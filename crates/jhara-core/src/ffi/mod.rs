/// # jhara_core C FFI surface
///
/// All exported symbols follow the `jhara_` prefix convention.
/// This module is the **only** place `unsafe` code is permitted to cross the
/// FFI boundary; all other crates remain safe Rust.
///
/// ## Thread Safety
///
/// `jhara_scan_start` spawns a Rayon thread pool internally.  The `callback`
/// may be invoked from any of those threads **concurrently**.  Swift callers
/// must ensure their callback implementation is thread-safe (dispatch to an
/// actor or use a lock).
///
/// ## Ownership Rules
///
/// ```text
///  Rust owns                          Swift borrows (read-only)
///  ─────────────────────────────────  ──────────────────────────────────────
///  JharaScanHandle                    *mut JharaScanHandle (opaque handle)
///  ScanNodeC arena                    ScanNodeC* inside callback only
///  NUL-terminated strings             path/name pointers inside callback only
/// ```
///
/// Swift **must** call `jhara_core_scan_free` exactly once per handle.
/// Calling it on a handle returned by a failed `jhara_core_scan_start` (null)
/// is a no-op.
pub mod types;

use std::ffi::{c_char, c_void, CStr};
use std::ptr;
use std::path::PathBuf;
use std::sync::Arc;
use types::{ScanNodeBatchC, ScanNodeOwned};
use crate::classifier::RuleEngine;
use crate::cleaner::{DeletionCoordinator};

/// Callback type for asynchronous scan results.
pub type JharaScanCallback = unsafe extern "C" fn(batch: ScanNodeBatchC, ctx: *mut c_void);

// ── Opaque handle ─────────────────────────────────────────────────────────────

/// Opaque handle to an in-progress or completed scan session.
///
/// Heap-allocated by `jhara_core_scan_start`, freed by `jhara_core_scan_free`.
/// Never dereference from Swift; treat as an opaque token.
pub struct JharaScanHandle {
    /// Roots passed to this scan (owned).
    _roots: Vec<String>,
    /// Paths to skip (e.g. iCloud containers passed from Swift).
    _skip_list: Vec<String>,
    /// Whether a cancellation has been requested.
    pub cancelled: Arc<std::sync::atomic::AtomicBool>,
    /// Accumulated scan results, populated after the scan completes.
    results: Arc<std::sync::Mutex<Vec<types::ScanNodeOwned>>>,
    /// Root paths for this scan session — used by jhara_core_projects_results_json.
    root_paths: Arc<std::sync::Mutex<Vec<PathBuf>>>,
    /// The original callback function address (stored as usize for Send safety).
    pub callback_addr: usize,
    /// The original context pointer (stored as usize for Send safety).
    pub context: usize,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Converts a C string pointer to an owned `String`.
///
/// Returns `None` if `ptr` is null or contains invalid UTF-8.
///
/// # Safety
/// Caller guarantees `ptr` is either null or points to a valid NUL-terminated
/// C string that will remain valid for the duration of this call.
unsafe fn c_str_to_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: caller upholds the pointer validity invariant.
    CStr::from_ptr(ptr).to_str().ok().map(String::from)
}

/// Converts a `**c_char` array of length `count` into a `Vec<String>`.
///
/// Null pointers and invalid UTF-8 entries are silently skipped.
///
/// # Safety
/// `ptr` must be null or point to a valid array of `count` C string pointers.
unsafe fn c_string_array(ptr: *const *const c_char, count: usize) -> Vec<String> {
    if ptr.is_null() || count == 0 {
        return Vec::new();
    }
    // SAFETY: caller guarantees `ptr` points to `count` valid elements.
    std::slice::from_raw_parts(ptr, count)
        .iter()
        .filter_map(|&p| c_str_to_string(p))
        .collect()
}

// ── Public FFI exports ────────────────────────────────────────────────────────

/// Starts an asynchronous filesystem scan.
///
/// ## Parameters
///
/// | Parameter    | Description                                             |
/// |--------------|---------------------------------------------------------|
/// | `roots`      | Array of NUL-terminated UTF-8 root paths to scan       |
/// | `root_count` | Length of `roots`                                       |
/// | `skip_list`  | Array of NUL-terminated paths to exclude (iCloud etc.) |
/// | `skip_count` | Length of `skip_list`                                   |
/// | `callback`   | Called with each batch of discovered nodes              |
/// | `ctx`        | Opaque context pointer forwarded to every callback      |
///
/// ## Returns
///
/// A non-null `*mut JharaScanHandle` on success, or **null** if:
/// - `roots` is null or `root_count` is zero
/// - Memory allocation fails
///
/// The caller must eventually call `jhara_core_scan_free` on the returned handle.
///
/// # Safety
/// All pointer parameters must satisfy their documented invariants.
/// The `callback` must be thread-safe (may be called from multiple threads).
// #[no_mangle] — exported via jhara-macos-ffi shim only
pub unsafe extern "C" fn jhara_core_scan_start(
    roots:       *const *const c_char,
    root_count:  usize,
    skip_list:   *const *const c_char,
    skip_count:  usize,
    callback:    extern "C" fn(batch: ScanNodeBatchC, ctx: *mut c_void),
    ctx:         *mut c_void,
) -> *mut JharaScanHandle {
    // Validate mandatory inputs.
    if roots.is_null() || root_count == 0 {
        return ptr::null_mut();
    }

    let root_strings = c_string_array(roots, root_count);
    if root_strings.is_empty() {
        return ptr::null_mut();
    }

    let skip_strings = c_string_array(skip_list, skip_count);

    let handle = Box::new(JharaScanHandle {
        _roots:    root_strings.clone(),
        _skip_list: skip_strings.clone(),
        cancelled:  Arc::new(std::sync::atomic::AtomicBool::new(false)),
        results:    Arc::new(std::sync::Mutex::new(Vec::new())),
        root_paths: Arc::new(std::sync::Mutex::new(
            root_strings.iter().map(PathBuf::from).collect()
        )),
        callback_addr: callback as usize,
        context: ctx as usize,
    });

    let handle_ptr = Box::into_raw(handle);
    let handle_ptr_usize = handle_ptr as usize; // Capture handle_ptr as usize

    // Build the ScanConfig from handle settings
    let roots_vec: Vec<PathBuf> = root_strings.into_iter().map(PathBuf::from).collect();
    let skip_vec: Vec<PathBuf> = skip_strings.into_iter().map(PathBuf::from).collect();

    std::thread::spawn(move || {
        // Reconstruct the raw pointers from usize inside the thread
        let handle = unsafe { &mut *(handle_ptr_usize as *mut JharaScanHandle) };
        let callback_addr = handle.callback_addr;
        let context_usize = handle.context;

        let cancelled_for_scan = Arc::clone(&handle.cancelled);
        let results_for_scan = Arc::clone(&handle.results);

        let mut config = crate::scanner::ScanConfig {
            roots: roots_vec,
            skip_list: skip_vec.into_iter().collect(),
            stale_threshold_days: 60,
            prune_names: std::collections::HashSet::new(),
        };
        
        // Add default prunes
        config.prune_names.insert(".git".to_string());
        
        // Populate prunes from signature database
        for sig in crate::detector::signatures::PROJECT_SIGNATURES {
            for art in sig.artifact_paths {
                if art.is_prunable {
                    config.prune_names.insert(art.relative_path.to_string());
                }
            }
        }

        let _ = crate::scanner::scan(config, move |batch| {
            // Check for cancellation
            if cancelled_for_scan.load(std::sync::atomic::Ordering::Relaxed) {
                return; 
            }

            // Convert back to function pointer and context
            let callback: JharaScanCallback = unsafe { std::mem::transmute(callback_addr as *const c_void) };
            let ctx = context_usize as *mut c_void;

            // Per-batch string allocation: collect owned CStrings first so
            // the Vec is fully allocated before we take any pointers from it.
            // Swift callback is synchronous — copies all data before returning.
            let batch_strings: Vec<(std::ffi::CString, std::ffi::CString)> = batch
                .iter()
                .map(|node| {
                    let path_str = node.path.to_string_lossy();
                    let p = std::ffi::CString::new(path_str.as_ref())
                        .unwrap_or_else(|_| std::ffi::CString::new("<invalid>").unwrap());
                    let n = std::ffi::CString::new(node.name.as_str())
                        .unwrap_or_else(|_| std::ffi::CString::new("<invalid>").unwrap());
                    (p, n)
                })
                .collect();

            let mut c_nodes = Vec::with_capacity(batch.len());
            for (i, node) in batch.iter().enumerate() {
                let path_c = batch_strings[i].0.as_ptr();
                let name_c = batch_strings[i].1.as_ptr();

                let kind_u8 = types::NodeKind::from(node.kind).as_u8();

                // Real-time detection signal — look up exact tier from signature database.
                // Do NOT collapse to binary; node_modules (Safe) and terraform.tfstate
                // (Blocked) must carry different tiers.
                let name_str = node.name.as_str();
                let safety_tier: u8 = crate::detector::signatures::PROJECT_SIGNATURES
                    .iter()
                    .flat_map(|sig| sig.artifact_paths.iter())
                    .find(|art| art.relative_path == name_str)
                    .map(|art| match art.safety_tier {
                        crate::detector::SafetyTier::Safe    => 0,
                        crate::detector::SafetyTier::Caution => 1,
                        crate::detector::SafetyTier::Risky   => 2,
                        crate::detector::SafetyTier::Blocked => 3,
                    })
                    .unwrap_or(255); // 255 = not a known artifact

                // Store in handle's results for tree_physical_size queries.
                {
                    let path_str = node.path.to_string_lossy().into_owned();
                    let mut guard = results_for_scan.lock().unwrap();
                    guard.push(ScanNodeOwned {
                        path: path_str,
                        name: node.name.clone(),
                        inode: node.inode,
                        physical_size: node.physical_size as i64,
                        logical_size: node.logical_size as i64,
                        modification_secs: node.modification_secs,
                        modification_nanos: node.modification_nanos,
                        link_count: node.link_count as u16,
                        kind: kind_u8,
                        is_ghost: false,
                        safety_tier,
                        safety_rating: 0,
                    });
                }

                c_nodes.push(types::ScanNodeC {
                    path: path_c,
                    name: name_c,
                    inode: node.inode,
                    physical_size: node.physical_size as i64,
                    logical_size: node.logical_size as i64,
                    modification_secs: node.modification_secs,
                    modification_nanos: node.modification_nanos,
                    link_count: node.link_count as u16,
                    kind: kind_u8,
                    is_ghost: 0,
                    safety_tier,
                    safety_rating: 0,
                    _reserved: [0u8; 6],
                });
            }
            // batch_strings drops here — Swift already copied all data.

            let batch_c = ScanNodeBatchC {
                nodes: c_nodes.as_ptr(),
                count: c_nodes.len(),
            };

            unsafe { callback(batch_c, ctx) };
        });
    });

    handle_ptr
}

/// Requests cancellation of an in-progress scan.
///
/// This is advisory — the scan may emit one more callback batch after this
/// call returns.  Safe to call multiple times; safe to call on a completed
/// scan (no-op).
///
/// # Safety
/// `handle` must be a valid pointer returned by `jhara_scan_start` that has
/// not yet been freed via `jhara_scan_free`.
// #[no_mangle] — exported via jhara-macos-ffi shim only
pub unsafe extern "C" fn jhara_core_scan_cancel(handle: *mut JharaScanHandle) {
    if handle.is_null() {
        return;
    }
    (*handle)
        .cancelled
        .store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Frees all resources associated with a scan handle.
///
/// After this call `handle` is a dangling pointer; the caller must not use it.
/// Passing a null pointer is a safe no-op.
///
/// # Safety
/// `handle` must be either null or a valid pointer returned by
/// `jhara_scan_start` that has not previously been freed.
// #[no_mangle] — exported via jhara-macos-ffi shim only
pub unsafe extern "C" fn jhara_core_scan_free(handle: *mut JharaScanHandle) {
    if handle.is_null() {
        return;
    }
    // Re-box and immediately drop, freeing the allocation and its contents.
    drop(Box::from_raw(handle));
}

/// Returns the total physical size (bytes) of the subtree rooted at `path`.
///
/// ## Returns
///
/// - `>= 0` — cumulative `physical_size` of all nodes under `path`
/// - `-1`   — `handle` or `path` is null, path not found, or scan incomplete
///
/// # Safety
/// `handle` must be a valid, non-freed handle.
/// `path` must be a valid NUL-terminated C string or null.
// #[no_mangle] — exported via jhara-macos-ffi shim only
pub unsafe extern "C" fn jhara_core_tree_physical_size(
    handle: *mut JharaScanHandle,
    path:   *const c_char,
) -> i64 {
    if handle.is_null() || path.is_null() {
        return -1;
    }

    let query = match c_str_to_string(path) {
        Some(s) => s,
        None    => return -1,
    };

    let results = match (*handle).results.lock() {
        Ok(guard) => guard,
        Err(_)    => return -1,
    };

    let total: i64 = results
        .iter()
        .filter(|node: &&types::ScanNodeOwned| node.path.starts_with(&query))
        .map(|node: &types::ScanNodeOwned| node.physical_size)
        .sum();

    total
}

// ── Classification & Deletion ────────────────────────────────────────────────

/// Deletes a list of absolute paths permanently.
///
/// ## Returns
/// - `0` on success
/// - `-1` if `paths` is null or `count` is 0
/// - `> 0` total number of errors encountered during deletion
///
/// # Safety
/// `paths` must be a valid array of NUL-terminated strings.
// #[no_mangle] — exported via jhara-macos-ffi shim only
pub unsafe extern "C" fn jhara_core_delete_paths(
    paths: *const *const c_char,
    count: usize,
) -> i32 {
    if paths.is_null() || count == 0 {
        return -1;
    }

    let path_strings = c_string_array(paths, count);
    let path_bufs: Vec<PathBuf> = path_strings.into_iter().map(PathBuf::from).collect();

    match DeletionCoordinator::delete_batch(&path_bufs) {
        Ok(stats) => stats.errors.len() as i32,
        Err(_) => -1,
    }
}

/// Classifies a project at the given root.
/// Returns a JSON string (caller must free with `jhara_core_string_free`).
// #[no_mangle] — exported via jhara-macos-ffi shim only
pub unsafe extern "C" fn jhara_core_project_classify(
    project_root: *const c_char,
) -> *mut c_char {
    let root_str = match c_str_to_string(project_root) {
        Some(s) => s,
        None => return ptr::null_mut(),
    };

    let root_path = PathBuf::from(root_str);
    
    // 1. Detect
    let detector = crate::detector::ProjectDetector::new();
    let projects = match detector.detect_at(&root_path) {
        Ok(p) => p,
        Err(_) => return ptr::null_mut(),
    };

    let project = match projects.first() {
        Some(p) => p,
        None => return ptr::null_mut(),
    };

    // 2. Classify
    let classified = RuleEngine::classify(project);

    // 3. Serialize
    match serde_json::to_string(&classified) {
        Ok(json) => {
            let c_str = std::ffi::CString::new(json).unwrap();
            c_str.into_raw()
        }
        Err(_) => ptr::null_mut(),
    }
}

/// Frees a string allocated by the core library (e.g. from `jhara_core_project_classify`).
// #[no_mangle] — exported via jhara-macos-ffi shim only
pub unsafe extern "C" fn jhara_core_string_free(s: *mut c_char) {
    if s.is_null() { return; }
    let _ = std::ffi::CString::from_raw(s);
}

/// Returns a JSON array of all detected projects for the scanned roots.
///
/// Feeds the accumulated scan results into `ProjectDetector` so the full
/// signature database is used — not a shallow `read_dir` probe.
/// Physical sizes are back-filled from the scan tree before serialisation.
/// Caller must free the returned string with `jhara_core_string_free`.
/// Returns null on any error.
///
/// # Safety
/// `handle` must be a valid non-null pointer from `jhara_core_scan_start`.
// #[no_mangle] — exported via jhara-macos-ffi shim only
pub unsafe extern "C" fn jhara_core_projects_results_json(
    handle: *const JharaScanHandle,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let h = &*handle;

    // ── Pass 1: feed every scanned file into the detector ─────────────────
    // We use the accumulated results vec (populated during the scan) so we
    // don't re-walk the filesystem.  Each file entry is fed as
    // observe(parent_dir, filename) — exactly what the scanner would do.
    let results_guard = match h.results.lock() {
        Ok(g) => g,
        Err(_) => return ptr::null_mut(),
    };

    let mut detector = crate::detector::ProjectDetector::new();
    for node in results_guard.iter() {
        // kind 1 = File, kind 2 = Directory — observe both so wildcard
        // signatures (*.tf, *.cabal) and directory-name signatures both fire.
        let path = std::path::PathBuf::from(&node.path);
        if let Some(parent) = path.parent() {
            detector.observe(parent, &node.name);
        }
    }

    // ── Build a size lookup: prefix → cumulative physical bytes ───────────
    // We collect this now while we still hold the results lock, then drop
    // the lock before the allocation-heavy resolve_all() pass.
    //
    // Key insight: `jhara_core_tree_physical_size` already does a prefix
    // sum over the results vec.  We replicate that logic here so we can
    // inline sizes into the DetectedProject JSON without an extra FFI round-
    // trip per artifact.
    //
    // We store (path_string → physical_bytes) for every node so that
    // resolve_all's FoundArtifact paths can be looked up in O(n) total.
    let size_map: std::collections::HashMap<String, i64> = results_guard
        .iter()
        .map(|n| (n.path.clone(), n.physical_size))
        .collect();

    // Drop the lock before the expensive resolution pass.
    drop(results_guard);

    // ── Pass 2: resolve candidates → DetectedProject ──────────────────────
    let mut projects = detector.resolve_all().unwrap_or_default();

    // ── Pass 3: back-fill physical_size_bytes from scan tree ──────────────
    // resolve_candidate() sets physical_size_bytes = 0 because it has no
    // reference to the scan tree.  We do a prefix-sum here instead.
    for project in &mut projects {
        for artifact in &mut project.artifacts {
            let prefix = artifact.absolute_path.to_string_lossy().into_owned();
            let total: i64 = size_map
                .iter()
                .filter(|(p, _)| p.starts_with(&prefix))
                .map(|(_, &s)| s)
                .sum();
            if total > 0 {
                artifact.physical_size_bytes = total as u64;
            }
        }
    }

    match serde_json::to_string(&projects) {
        Ok(json) => match std::ffi::CString::new(json) {
            Ok(cs) => cs.into_raw(),
            Err(_) => ptr::null_mut(),
        },
        Err(_) => ptr::null_mut(),
    }
}

/// Returns a JSON array of global developer-tool caches found under `home_dir`.
///
/// Physical sizes are back-filled from the scan-tree results so the UI can
/// display accurate disk usage without re-walking the filesystem.
/// Caller must free the returned string with `jhara_core_string_free`.
/// Returns null on any error.
///
/// # Safety
/// `handle` must be a valid non-null pointer from `jhara_core_scan_start`.
/// `home_dir` must be a valid NUL-terminated UTF-8 C string.
// #[no_mangle] — exported via jhara-macos-ffi shim only
pub unsafe extern "C" fn jhara_core_global_caches_json(
    handle:   *const JharaScanHandle,
    home_dir: *const c_char,
) -> *mut c_char {
    if handle.is_null() || home_dir.is_null() {
        return ptr::null_mut();
    }
    let h = &*handle;

    let home_str = match c_str_to_string(home_dir) {
        Some(s) => s,
        None    => return ptr::null_mut(),
    };
    let home_path = std::path::PathBuf::from(home_str);

    // ── Build size lookup from accumulated scan results ────────────────────
    let size_map: std::collections::HashMap<String, i64> = match h.results.lock() {
        Ok(guard) => guard.iter().map(|n| (n.path.clone(), n.physical_size)).collect(),
        Err(_)    => return ptr::null_mut(),
    };

    // ── Detect global caches via the Rust engine ───────────────────────────
    let mut caches = crate::detector::detect_global_caches(&home_path);

    // ── Back-fill physical sizes from scan tree ────────────────────────────
    for cache in &mut caches {
        let prefix = cache.absolute_path.to_string_lossy().into_owned();
        let total: i64 = size_map
            .iter()
            .filter(|(p, _)| p.starts_with(&prefix))
            .map(|(_, &s)| s)
            .sum();
        if total > 0 {
            cache.physical_size_bytes = total as u64;
        }
    }

    match serde_json::to_string(&caches) {
        Ok(json) => match std::ffi::CString::new(json) {
            Ok(cs) => cs.into_raw(),
            Err(_) => ptr::null_mut(),
        },
        Err(_) => ptr::null_mut(),
    }
}

/// Returns a JSON array of Intel (x86_64) artifacts found on Apple Silicon —
/// i.e. paths under `/usr/local/` that only exist for Rosetta/Intel Homebrew.
///
/// These are Safe-tier: deleting them never affects native arm64 toolchains.
/// Physical sizes are back-filled from the scan-tree results.
/// Caller must free the returned string with `jhara_core_string_free`.
/// Returns null on any error or if no orphans are found.
///
/// # Safety
/// `handle` must be a valid non-null pointer from `jhara_core_scan_start`.
// #[no_mangle] — exported via jhara-macos-ffi shim only
pub unsafe extern "C" fn jhara_core_orphan_scan_json(
    handle: *const JharaScanHandle,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let h = &*handle;

    // Intel Homebrew lives under /usr/local/ on x86_64 Macs.
    // On Apple Silicon these paths are Rosetta-only; native arm64 Homebrew
    // installs to /opt/homebrew/. Anything under /usr/local/Cellar,
    // /usr/local/opt, /usr/local/lib, /usr/local/bin is safe to remove
    // once the user has migrated to arm64 Homebrew.
    const INTEL_PREFIXES: &[&str] = &[
        "/usr/local/Cellar/",
        "/usr/local/opt/",
        "/usr/local/lib/",
        "/usr/local/bin/",
    ];

    let results_guard = match h.results.lock() {
        Ok(g)  => g,
        Err(_) => return ptr::null_mut(),
    };

    // Collect all nodes that fall under an Intel prefix, summing their
    // physical sizes. We group by the top-level entry under /usr/local/
    // (e.g. /usr/local/Cellar/node) so the UI shows one artifact per
    // formula rather than thousands of individual files.
    let mut grouped: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for node in results_guard.iter() {
        let is_intel = INTEL_PREFIXES.iter().any(|prefix| node.path.starts_with(prefix));
        if !is_intel {
            continue;
        }
        // Key = the first two path components after /usr/local/
        // e.g. "/usr/local/Cellar/node/20.0.0/bin/node" → "/usr/local/Cellar/node"
        let parts: Vec<&str> = node.path.splitn(6, '/').collect();
        // parts: ["", "usr", "local", "Cellar", "node", ...]
        let key = if parts.len() >= 5 {
            format!("/{}/{}/{}/{}", parts[1], parts[2], parts[3], parts[4])
        } else {
            node.path.clone()
        };
        *grouped.entry(key).or_insert(0) += node.physical_size;
    }
    drop(results_guard);

    if grouped.is_empty() {
        return ptr::null_mut();
    }

    // Build FoundArtifact-compatible JSON objects so Swift can decode them
    // with the same FoundArtifactDecoded struct it uses for projects/caches.
    let artifacts: Vec<serde_json::Value> = grouped
        .into_iter()
        .map(|(path, size)| {
            let name = std::path::Path::new(&path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone());
            serde_json::json!({
                "absolute_path":       path,
                "safety_tier":         "safe",
                "physical_size_bytes": size,
                "recovery_command":    "brew install --formula <name>  # after migrating to arm64 Homebrew",
                "is_ghost":            false,
                "ecosystem":           "homebrew_intel",
                "name":                name,
            })
        })
        .collect();

    match serde_json::to_string(&artifacts) {
        Ok(json) => match std::ffi::CString::new(json) {
            Ok(cs) => cs.into_raw(),
            Err(_) => ptr::null_mut(),
        },
        Err(_) => ptr::null_mut(),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    extern "C" fn count_batches(batch: ScanNodeBatchC, ctx: *mut c_void) {
        let counter = unsafe { &*(ctx as *const AtomicUsize) };
        // Even empty batches increment the counter — we want to confirm the
        // callback was invoked at all.
        counter.fetch_add(1, Ordering::Relaxed);
        let _ = batch; // suppress unused warning
    }

    #[test]
    fn null_roots_returns_null_handle() {
        let handle = unsafe {
            jhara_core_scan_start(
                ptr::null(),
                0,
                ptr::null(),
                0,
                count_batches,
                ptr::null_mut(),
            )
        };
        assert!(handle.is_null());
    }

    #[test]
    fn valid_root_returns_non_null_handle_and_invokes_callback() {
        let root = CString::new("/tmp").unwrap();
        let root_ptr: *const c_char = root.as_ptr();
        let roots_arr: [*const c_char; 1] = [root_ptr];

        let counter = Arc::new(AtomicUsize::new(0));
        let ctx = Arc::as_ptr(&counter) as *mut c_void;

        let handle = unsafe {
            jhara_core_scan_start(
                roots_arr.as_ptr(),
                1,
                ptr::null(),
                0,
                count_batches,
                ctx,
            )
        };

        assert!(!handle.is_null());

        // Wait up to 1s for the async scan to start and invoke the callback
        let start = std::time::Instant::now();
        while counter.load(Ordering::Relaxed) == 0 && start.elapsed().as_millis() < 1000 {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        assert!(counter.load(Ordering::Relaxed) >= 1, "callback was not invoked within 1s");

        unsafe { jhara_core_scan_free(handle) };
    }

    #[test]
    fn cancel_on_null_handle_is_safe() {
        unsafe { jhara_core_scan_cancel(ptr::null_mut()) };
    }

    #[test]
    fn free_on_null_handle_is_safe() {
        unsafe { jhara_core_scan_free(ptr::null_mut()) };
    }

    #[test]
    fn tree_physical_size_null_guard() {
        assert_eq!(unsafe { jhara_core_tree_physical_size(ptr::null_mut(), ptr::null()) }, -1);
    }
}
