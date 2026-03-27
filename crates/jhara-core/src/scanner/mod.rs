pub mod dedup;
pub mod inode;
pub mod platform;
pub mod tree;
pub mod types;

pub use tree::ScanTree;
pub use types::{NodeKind, ScanError, ScanNode, ScanStats};

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use jwalk::WalkDirGeneric;
use rayon::prelude::*;

use inode::InodeTracker;
use platform::{file_identity, modification_time, physical_size, query_cluster_size};

/// How many `ScanNode`s to accumulate before invoking the callback.
///
/// Larger batches reduce FFI crossing overhead (critical for the Swift
/// integration) and reduce channel contention on the callback side.
/// 1 024 was chosen based on benchmarks: smaller values (128) produce
/// measurable actor mailbox pressure in Swift; larger values (4 096) add
/// latency to the first UI update without further throughput gain.
const BATCH_SIZE: usize = 1_024;

/// Controls issued to an in-progress scan.
///
/// The scanner checks `cancelled` at the start of each directory entry.
/// Cancellation is best-effort: partially processed batches may still
/// arrive in the callback after cancellation is set.
pub struct ScanHandle {
    cancelled: Arc<AtomicBool>,
}

impl ScanHandle {
    /// Signal the scan to stop. The callback will stop receiving batches
    /// shortly after this is called. Already-queued batches may still fire.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    /// Returns `true` if cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

/// Configuration for a scan session.
pub struct ScanConfig {
    /// Root directories to scan concurrently.
    pub roots: Vec<PathBuf>,

    /// Paths to skip entirely during traversal.
    ///
    /// On macOS, populated by Swift's iCloudGuard pre-scan using
    /// `URLResourceKey.isUbiquitousItemKey`. The scanner checks
    /// each directory against this set using `Path::starts_with()`
    /// (component-aware, not a raw string prefix) before descending.
    /// (component-aware, not a raw string prefix)
    /// preventing false matches like /home/dev/project matching /home/dev/proj.
    ///
    /// Typically contains top-level iCloud-managed directories like
    /// `~/Documents` or `~/Desktop` when iCloud Drive is enabled.
    pub skip_list: HashSet<PathBuf>,

    /// Staleness threshold in days (for later use by the classifier).
    /// Stored here so the scan context carries it through the pipeline.
    pub stale_threshold_days: u32,

    /// Directory names that should be pruned (skipped recursively).
    pub prune_names: HashSet<String>,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            roots: Vec::new(),
            skip_list: HashSet::new(),
            stale_threshold_days: 90,
            prune_names: HashSet::new(),
        }
    }
}

/// Execute a filesystem scan across all roots in `config`.
///
/// Calls `callback` with non-overlapping batches of `ScanNode`s as they
/// are discovered. The callback is invoked from a rayon worker thread —
/// it must be `Send + Sync`. On the macOS side, the C FFI callback
/// dispatches to the Swift actor via `Task { await coordinator.receive() }`.
///
/// Returns a `ScanHandle` that can be used to cancel the scan, and
/// `ScanStats` when the scan completes (or is cancelled).
///
/// # Errors
///
/// Returns `ScanError::RootNotFound` if any root does not exist.
/// Individual directory read errors are counted in `ScanStats::error_count`
/// and do not abort the scan.
pub fn scan<F>(config: ScanConfig, callback: F) -> Result<(ScanHandle, ScanStats), ScanError>
where
    F: Fn(Vec<ScanNode>) + Send + Sync + 'static,
{
    // Validate roots before starting
    for root in &config.roots {
        if !root.exists() {
            return Err(ScanError::RootNotFound(root.clone()));
        }
    }

    let cancelled = Arc::new(AtomicBool::new(false));
    let handle = ScanHandle {
        cancelled: Arc::clone(&cancelled),
    };

    let skip_list = Arc::new(config.skip_list);
    let prune_names = Arc::new(config.prune_names);
    let callback = Arc::new(callback);

    // Collect stats across all root scans
    let stats = scan_roots(config.roots, skip_list, prune_names, callback, cancelled);

    Ok((handle, stats))
}

fn scan_roots(
    roots: Vec<PathBuf>,
    skip_list: Arc<HashSet<PathBuf>>,
    prune_names: Arc<HashSet<String>>,
    callback: Arc<impl Fn(Vec<ScanNode>) + Send + Sync + 'static>,
    cancelled: Arc<AtomicBool>,
) -> ScanStats {
    // Capture root device IDs for cross-device boundary detection (FTS_XDEV parity).
    // If a directory's device ID differs from its root's, it is a mount point
    // (network share, external drive, Time Machine volume) and should not be descended.
    let root_devices: Vec<(PathBuf, u64)> = roots
        .iter()
        .map(|root| {
            let device = std::fs::metadata(root)
                .map(|m| file_identity(&m).device_id)
                .unwrap_or(0);
            (root.clone(), device)
        })
        .collect();

    // Query cluster size once per root (Windows only; returns 4096 on Unix).
    // Stored alongside the root device for use in physical_size().
    let cluster_sizes: Vec<u64> = roots.iter().map(|root| query_cluster_size(root)).collect();

    // Scan all roots in parallel using rayon.
    // Each root gets its own InodeTracker — hard-link dedup is per-root.
    // Cross-root dedup is intentionally omitted: a file hard-linked across
    // two different subtrees is unusual and the complexity isn't worth it.
    let per_root_stats: Vec<ScanStats> = root_devices
        .into_par_iter()
        .zip(cluster_sizes.into_par_iter())
        .map(|((root, root_device), cluster_size)| {
            if cancelled.load(Ordering::Relaxed) {
                return ScanStats::default();
            }
            scan_single_root(
                &root,
                root_device,
                cluster_size,
                &skip_list,
                &prune_names,
                callback.as_ref(),
                &cancelled,
            )
        })
        .collect();

    // Merge per-root stats into a single aggregate
    per_root_stats
        .into_iter()
        .fold(ScanStats::default(), |mut acc, s| {
            acc.total_entries += s.total_entries;
            acc.total_physical_bytes += s.total_physical_bytes;
            acc.total_logical_bytes += s.total_logical_bytes;
            acc.deduped_entries += s.deduped_entries;
            acc.skipped_cloud_entries += s.skipped_cloud_entries;
            acc.error_count += s.error_count;
            acc
        })
}

/// Scan a single root directory tree.
///
/// Uses `jwalk::WalkDirGeneric` with a `process_read_dir` hook for:
///   - Cross-device filtering (FTS_XDEV parity)
///   - Skip-list pruning (iCloud / OneDrive paths)
fn scan_single_root(
    root: &Path,
    root_device: u64,
    _cluster_size: u64,
    skip_list: &HashSet<PathBuf>,
    prune_names: &HashSet<String>,
    callback: &(impl Fn(Vec<ScanNode>) + Send + Sync),
    cancelled: &AtomicBool,
) -> ScanStats {
    let mut stats = ScanStats::default();
    let mut tracker = InodeTracker::default();
    let mut batch: Vec<ScanNode> = Vec::with_capacity(BATCH_SIZE);

    // `WalkDirGeneric` with `((), ())` state — we don't use the client state
    // feature, but the type parameter is required.
    let walker = WalkDirGeneric::<((), ())>::new(root)
        .follow_links(false) // FTS_PHYSICAL parity: never follow symlinks
        .skip_hidden(false) // Developer caches live in dotfiles (.cargo, .npm…)
        .process_read_dir({
            let skip_list = skip_list.clone();
            let skip_list_owned: HashSet<PathBuf> = skip_list.iter().cloned().collect();
            let prune_names_owned: HashSet<String> = prune_names.iter().cloned().collect();
            move |_depth, dir_path, _state, children| {
                // If the current directory being read is in the prune list,
                // clear all its children to stop recursion immediately.
                if let Some(name) = dir_path.file_name() {
                    let name_str = name.to_string_lossy();
                    if prune_names_owned.contains(name_str.as_ref()) {
                        children.clear();
                        return;
                    }
                }

                // Remove children that cross a device boundary (FTS_XDEV parity).
                // Also remove children whose path is in the skip-list.
                children.retain(|entry_result| {
                    let Ok(entry) = entry_result else { return true };

                    // Cross-device boundary check
                    if entry.file_type().is_dir() {
                        if let Ok(meta) = entry.metadata() {
                            let id = file_identity(&meta);
                            if root_device != 0 && id.device_id != 0 && id.device_id != root_device
                            {
                                return false; // different volume — prune
                            }
                        }
                    }

                    // Skip-list check: if this entry's path starts with any
                    // skip-list entry, prune it from the traversal queue.
                    // `Path::starts_with` compares components, not raw string bytes,
                    // preventing false matches like /home/dev/project matching /home/dev/proj.
                    let entry_path = dir_path.join(entry.file_name());
                    for skip in &skip_list_owned {
                        if entry_path.starts_with(skip) {
                            return false;
                        }
                    }

                    true
                });
            }
        });

    for entry_result in walker {
        if cancelled.load(Ordering::Relaxed) {
            break;
        }

        let entry = match entry_result {
            Ok(e) => e,
            Err(_) => {
                stats.error_count += 1;
                continue;
            }
        };

        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => {
                stats.error_count += 1;
                continue;
            }
        };

        let ft = entry.file_type();

        let kind = if ft.is_dir() {
            // jwalk emits each directory once in pre-order.
            // DirPost is not natively supported; we emit DirPre here.
            // The ScanTree's O(N) reverse-pass rollup does not need DirPost.
            NodeKind::DirPre
        } else if ft.is_symlink() {
            NodeKind::Symlink
        } else if ft.is_file() {
            NodeKind::File
        } else {
            NodeKind::Other
        };

        let identity = file_identity(&meta);
        let link_count = link_count_from_meta(&meta);

        // For hard-linked files, check dedup.
        // Files with link_count == 1 are never hard-linked; skip the set lookup.
        let should_count = if link_count > 1 {
            // On Windows, refine identity via FILE_ID_INFO for files with links.
            #[cfg(windows)]
            let refined = {
                if let Some((dev, id)) = dedup::query_file_id(entry.path()) {
                    (dev, id)
                } else {
                    (identity.device_id, identity.inode)
                }
            };
            #[cfg(not(windows))]
            let refined = (identity.device_id, identity.inode);

            tracker.should_count(refined.0, refined.1)
        } else {
            true // link_count == 1: always count, no dedup needed
        };

        let phys = if should_count {
            physical_size(entry.path().as_path(), &meta)
        } else {
            stats.deduped_entries += 1;
            0
        };

        // OneDrive / offline placeholders return 0 from physical_size_windows.
        // Track them in skipped_cloud_entries.
        #[cfg(windows)]
        if phys == 0 && ft.is_file() {
            use std::os::windows::fs::MetadataExt;
            const RECALL: u32 = 0x0040_0000;
            const OFFLINE: u32 = 0x0000_1000;
            if meta.file_attributes() & (RECALL | OFFLINE) != 0 {
                stats.skipped_cloud_entries += 1;
            }
        }

        let (mod_secs, mod_nanos) = modification_time(&meta);

        let name = entry.file_name().to_string_lossy().into_owned();

        let node = ScanNode {
            path: entry.path().to_path_buf(),
            name,
            inode: identity.inode,
            device_id: identity.device_id,
            physical_size: phys,
            logical_size: meta.len(),
            modification_secs: mod_secs,
            modification_nanos: mod_nanos,
            link_count,
            kind,
        };

        stats.total_entries += 1;
        stats.total_physical_bytes += phys;
        stats.total_logical_bytes += meta.len();

        batch.push(node);

        if batch.len() >= BATCH_SIZE {
            callback(std::mem::replace(
                &mut batch,
                Vec::with_capacity(BATCH_SIZE),
            ));
        }
    }

    // Flush remaining nodes
    if !batch.is_empty() {
        callback(batch);
    }

    stats
}

/// Extract the hard-link count from `Metadata` in a cross-platform way.
#[cfg(unix)]
fn link_count_from_meta(meta: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::MetadataExt;
    meta.nlink() as u32
}

#[cfg(windows)]
fn link_count_from_meta(meta: &std::fs::Metadata) -> u32 {
    use std::os::windows::fs::MetadataExt;
    meta.number_of_links().unwrap_or(1)
}

#[cfg(not(any(unix, windows)))]
fn link_count_from_meta(_meta: &std::fs::Metadata) -> u32 {
    1
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_tree(dir: &Path) {
        // project/
        //   src/
        //     main.rs
        //   Cargo.toml
        //   target/
        //     debug/
        //       binary
        let src = dir.join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("main.rs"), b"fn main() {}").unwrap();
        fs::write(dir.join("Cargo.toml"), b"[package]").unwrap();
        let target_debug = dir.join("target").join("debug");
        fs::create_dir_all(&target_debug).unwrap();
        fs::write(target_debug.join("binary"), b"ELF").unwrap();
    }

    fn collect_scan(dir: &Path) -> (Vec<ScanNode>, ScanStats) {
        use std::sync::Mutex;
        let nodes: Arc<Mutex<Vec<ScanNode>>> = Arc::new(Mutex::new(Vec::new()));
        let nodes_cb = Arc::clone(&nodes);
        let config = ScanConfig {
            roots: vec![dir.to_path_buf()],
            ..Default::default()
        };
        let (_handle, stats) = scan(config, move |batch| {
            nodes_cb.lock().unwrap().extend(batch);
        })
        .unwrap();
        let result = nodes.lock().unwrap().drain(..).collect();
        (result, stats)
    }

    #[test]
    fn scan_discovers_expected_files() {
        let tmp = TempDir::new().unwrap();
        make_tree(tmp.path());
        let (nodes, stats) = collect_scan(tmp.path());
        let paths: Vec<_> = nodes.iter().map(|n| n.name.clone()).collect();
        assert!(paths.contains(&"main.rs".to_string()), "main.rs not found");
        assert!(
            paths.contains(&"Cargo.toml".to_string()),
            "Cargo.toml not found"
        );
        assert!(stats.total_entries > 0);
    }

    #[test]
    fn scan_error_on_missing_root() {
        let config = ScanConfig {
            roots: vec![PathBuf::from("/this/does/not/exist/jhara-test")],
            ..Default::default()
        };
        let result = scan(config, |_| {});
        assert!(matches!(result, Err(ScanError::RootNotFound(_))));
    }

    #[test]
    fn skip_list_excludes_directory() {
        let tmp = TempDir::new().unwrap();
        let skip_dir = tmp.path().join("skip_me");
        fs::create_dir_all(&skip_dir).unwrap();
        fs::write(skip_dir.join("secret.txt"), b"hidden").unwrap();
        fs::write(tmp.path().join("visible.txt"), b"shown").unwrap();

        let mut skip_list = HashSet::new();
        skip_list.insert(skip_dir.clone());

        let nodes: Arc<std::sync::Mutex<Vec<ScanNode>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let nodes_cb = Arc::clone(&nodes);
        let config = ScanConfig {
            roots: vec![tmp.path().to_path_buf()],
            skip_list,
            ..Default::default()
        };
        scan(config, move |batch| {
            nodes_cb.lock().unwrap().extend(batch);
        })
        .unwrap();

        let names: Vec<_> = nodes
            .lock()
            .unwrap()
            .iter()
            .map(|n| n.name.clone())
            .collect();
        assert!(
            names.contains(&"visible.txt".to_string()),
            "visible.txt should appear"
        );
        assert!(
            !names.contains(&"secret.txt".to_string()),
            "secret.txt should be skipped"
        );
    }

    #[test]
    fn cancellation_stops_scan() {
        let tmp = TempDir::new().unwrap();
        // Create a moderately large tree to give cancellation time to kick in
        for i in 0..100 {
            let dir = tmp.path().join(format!("dir_{}", i));
            fs::create_dir_all(&dir).unwrap();
            for j in 0..20 {
                fs::write(dir.join(format!("file_{}.txt", j)), b"data").unwrap();
            }
        }

        let count = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let count_cb = Arc::clone(&count);
        let config = ScanConfig {
            roots: vec![tmp.path().to_path_buf()],
            ..Default::default()
        };
        let (handle, _stats) = scan(config, move |batch| {
            count_cb.fetch_add(batch.len() as u64, Ordering::Relaxed);
        })
        .unwrap();

        handle.cancel();
        assert!(handle.is_cancelled());
        // We cannot assert exactly how many entries were processed (race condition
        // is inherent), but the test should complete quickly and not hang.
    }

    #[test]
    fn hard_link_dedup_counts_once() {
        #[cfg(unix)]
        {
            let tmp = TempDir::new().unwrap();
            let original = tmp.path().join("original.txt");
            let linked = tmp.path().join("linked.txt");
            fs::write(&original, b"shared content").unwrap();
            std::fs::hard_link(&original, &linked).unwrap();

            let nodes: Arc<std::sync::Mutex<Vec<ScanNode>>> =
                Arc::new(std::sync::Mutex::new(Vec::new()));
            let nodes_cb = Arc::clone(&nodes);
            let config = ScanConfig {
                roots: vec![tmp.path().to_path_buf()],
                ..Default::default()
            };
            let (_handle, stats) = scan(config, move |batch| {
                nodes_cb.lock().unwrap().extend(batch);
            })
            .unwrap();

            // One of the two hard links should have physical_size == 0 (deduped)
            let nodes = nodes.lock().unwrap();
            let zero_size_files: Vec<_> = nodes
                .iter()
                .filter(|n| n.kind == NodeKind::File && n.physical_size == 0)
                .collect();
            assert_eq!(
                zero_size_files.len(),
                1,
                "exactly one hard link should be deduped"
            );
            assert_eq!(stats.deduped_entries, 1);
        }
    }

    #[test]
    fn symlinks_are_not_followed() {
        #[cfg(unix)]
        {
            let tmp = TempDir::new().unwrap();
            let real_dir = tmp.path().join("real");
            fs::create_dir_all(&real_dir).unwrap();
            fs::write(real_dir.join("inside.txt"), b"real").unwrap();

            let link_dir = tmp.path().join("link_to_real");
            std::os::unix::fs::symlink(&real_dir, &link_dir).unwrap();

            let nodes: Arc<std::sync::Mutex<Vec<ScanNode>>> =
                Arc::new(std::sync::Mutex::new(Vec::new()));
            let nodes_cb = Arc::clone(&nodes);
            let config = ScanConfig {
                roots: vec![tmp.path().to_path_buf()],
                ..Default::default()
            };
            scan(config, move |batch| {
                nodes_cb.lock().unwrap().extend(batch);
            })
            .unwrap();

            // `inside.txt` should appear once (via `real/`), not twice
            let names: Vec<_> = nodes
                .lock()
                .unwrap()
                .iter()
                .map(|n| n.name.clone())
                .collect();
            let count = names.iter().filter(|n| n.as_str() == "inside.txt").count();
            assert_eq!(
                count, 1,
                "symlink followed — inside.txt found {} times",
                count
            );

            // The symlink entry itself should be NodeKind::Symlink
            let symlink_entries: Vec<_> = nodes
                .lock()
                .unwrap()
                .iter()
                .filter(|n| n.kind == NodeKind::Symlink)
                .map(|n| n.name.clone())
                .collect();
            assert!(symlink_entries.contains(&"link_to_real".to_string()));
        }
    }

    #[test]
    fn empty_directory_produces_one_dirpre_node() {
        let tmp = TempDir::new().unwrap();
        let empty = tmp.path().join("empty_dir");
        fs::create_dir_all(&empty).unwrap();

        let nodes: Arc<std::sync::Mutex<Vec<ScanNode>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let nodes_cb = Arc::clone(&nodes);
        let config = ScanConfig {
            roots: vec![empty.clone()],
            ..Default::default()
        };
        scan(config, move |batch| {
            nodes_cb.lock().unwrap().extend(batch);
        })
        .unwrap();

        let nodes = nodes.lock().unwrap();
        // The root itself should appear as DirPre
        let dirs: Vec<_> = nodes
            .iter()
            .filter(|n| n.kind == NodeKind::DirPre)
            .collect();
        assert!(
            !dirs.is_empty(),
            "expected at least one DirPre for the root"
        );
    }

    #[test]
    fn smart_pruning_skips_children_but_reports_dir() {
        let tmp = TempDir::new().unwrap();
        let target_dir = tmp.path().join("target");
        fs::create_dir_all(&target_dir).unwrap();
        fs::write(target_dir.join("debug_binary"), b"huge").unwrap();
        fs::write(tmp.path().join("main.rs"), b"code").unwrap();

        let mut prune_names = HashSet::new();
        prune_names.insert("target".to_string());

        let nodes: Arc<std::sync::Mutex<Vec<ScanNode>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let nodes_cb = Arc::clone(&nodes);
        let config = ScanConfig {
            roots: vec![tmp.path().to_path_buf()],
            prune_names,
            ..Default::default()
        };
        scan(config, move |batch| {
            nodes_cb.lock().unwrap().extend(batch);
        })
        .unwrap();

        let names: Vec<_> = nodes
            .lock()
            .unwrap()
            .iter()
            .map(|n| n.name.clone())
            .collect();
        assert!(
            names.contains(&"main.rs".to_string()),
            "main.rs should be found"
        );
        assert!(
            names.contains(&"target".to_string()),
            "target dir itself should be found"
        );
        assert!(
            !names.contains(&"debug_binary".to_string()),
            "target children should be pruned"
        );
    }
}
