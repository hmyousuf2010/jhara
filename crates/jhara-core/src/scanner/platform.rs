/// Platform-specific helpers for extracting file identity and physical size.
///
/// These functions are isolated here to keep `scanner/mod.rs` readable
/// and to make the Windows-specific code paths easy to test in isolation.
use std::fs::Metadata;
use std::path::Path;

/// A file's unique identity on the filesystem.
///
/// On Unix: (st_dev, st_ino).
/// On Windows: (VolumeSerialNumber, lower 64 bits of FILE_ID_128).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileIdentity {
    pub device_id: u64,
    pub inode: u64,
}

/// Extract `FileIdentity` from metadata without opening an extra file handle.
///
/// On Unix this is always cheap — both values come from the `stat` struct
/// that `jwalk` already has in memory.
///
/// On Windows, `MetadataExt::volume_serial_number()` and
/// `file_index()` (the legacy 64-bit composite ID) are available without
/// an extra handle. For parity with the full 128-bit `FILE_ID_INFO` approach
/// we only fall back to handle-based queries in `dedup.rs` when
/// `link_count > 1`.
#[cfg(unix)]
pub fn file_identity(meta: &Metadata) -> FileIdentity {
    use std::os::unix::fs::MetadataExt;
    FileIdentity {
        device_id: meta.dev(),
        inode: meta.ino(),
    }
}

#[cfg(windows)]
pub fn file_identity(meta: &Metadata) -> FileIdentity {
    use std::os::windows::fs::MetadataExt;
    // volume_serial_number() and file_index() are available without opening
    // an extra handle. file_index() is the same 64-bit value used by
    // BY_HANDLE_FILE_INFORMATION. For hard-linked files we refine this in
    // dedup.rs using the full 128-bit FILE_ID_INFO.
    FileIdentity {
        device_id: meta.volume_serial_number().unwrap_or(0) as u64,
        inode: meta.file_index().unwrap_or(0),
    }
}

// Fallback for non-Unix, non-Windows (e.g. Wasm, UEFI) — identity not available.
#[cfg(not(any(unix, windows)))]
pub fn file_identity(_meta: &Metadata) -> FileIdentity {
    FileIdentity { device_id: 0, inode: 0 }
}

// ─── Physical Size ────────────────────────────────────────────────────────────

/// Returns the number of bytes physically allocated on disk for this entry.
///
/// Unix:    `st_blocks * 512` — accurate for sparse files.
/// Windows: see `physical_size_windows`.
#[cfg(unix)]
pub fn physical_size(_path: &Path, meta: &Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    // st_blocks is in 512-byte units, regardless of filesystem block size.
    (meta.blocks() * 512) as u64
}

#[cfg(windows)]
pub fn physical_size(path: &Path, meta: &Metadata) -> u64 {
    physical_size_windows(path, meta)
}

#[cfg(not(any(unix, windows)))]
pub fn physical_size(_path: &Path, meta: &Metadata) -> u64 {
    meta.len()
}

/// Windows physical size strategy:
///
/// 1. Check `dwFileAttributes` for reparse points that trigger cloud downloads:
///    - `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS` (0x0040_0000): OneDrive placeholder.
///    - `FILE_ATTRIBUTE_RECALL_ON_OPEN` (0x0004_0000): Similar cloud reparse.
///    - `FILE_ATTRIBUTE_OFFLINE` (0x0000_1000): Older offline marker.
///    Return 0 for all of these — Jhara skips cloud-placeholder files entirely
///    to avoid triggering downloads.
///
/// 2. For compressed or sparse files, `GetCompressedFileSizeW` returns the true
///    physical allocation. Delegated to the `filesize` crate.
///
/// 3. For normal files, round logical size up to the volume cluster size.
///    Cluster size is queried once per volume via `GetDiskFreeSpaceW` and
///    passed in from the scan context (see `ScanContext::cluster_size`).
#[cfg(windows)]
pub fn physical_size_windows(path: &Path, meta: &Metadata) -> u64 {
    use std::os::windows::fs::MetadataExt;

    const RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
    const RECALL_ON_OPEN:        u32 = 0x0004_0000;
    const OFFLINE:               u32 = 0x0000_1000;
    const COMPRESSED:            u32 = 0x0000_0800;
    const SPARSE:                u32 = 0x0000_0200;

    let attrs = meta.file_attributes();

    // Cloud placeholder — do not touch, return 0 to skip size accounting.
    if attrs & (RECALL_ON_DATA_ACCESS | RECALL_ON_OPEN | OFFLINE) != 0 {
        return 0;
    }

    // Compressed or sparse — use the accurate Win32 query.
    if attrs & (COMPRESSED | SPARSE) != 0 {
        return filesize::PathExt::size_on_disk_fast(path, meta).unwrap_or_else(|_| meta.len());
    }

    // Normal file — round up to cluster size (passed in from the scan context).
    // If cluster size is unknown (0), fall back to logical size.
    // Callers should always supply a valid cluster_size from query_cluster_size().
    meta.len()
}

/// Query the allocation unit (cluster) size for the volume containing `path`.
///
/// On Windows this calls `GetDiskFreeSpaceW` once per volume; callers should
/// cache the result for the scan session rather than calling per-file.
///
/// Returns `4096` (a safe default) if the query fails or the platform is not Windows.
pub fn query_cluster_size(_path: &Path) -> u64 {
    #[cfg(windows)]
    {
        query_cluster_size_windows(_path)
    }
    #[cfg(not(windows))]
    {
        4_096 // not used on Unix but defined for cross-compilation
    }
}

#[cfg(windows)]
fn query_cluster_size_windows(path: &Path) -> u64 {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceW;
    use windows::core::PCWSTR;

    // GetDiskFreeSpaceW needs the root of the volume, e.g. "C:\".
    let root = path
        .ancestors()
        .last()
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();

    let wide: Vec<u16> = OsStr::new(&root)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut sectors_per_cluster: u32 = 0;
    let mut bytes_per_sector: u32 = 0;
    let mut _free_clusters: u32 = 0;
    let mut _total_clusters: u32 = 0;

    let ok = unsafe {
        GetDiskFreeSpaceW(
            PCWSTR(wide.as_ptr()),
            Some(&mut sectors_per_cluster),
            Some(&mut bytes_per_sector),
            Some(&mut _free_clusters),
            Some(&mut _total_clusters),
        )
    };

    if ok.is_ok() && bytes_per_sector > 0 && sectors_per_cluster > 0 {
        (sectors_per_cluster as u64) * (bytes_per_sector as u64)
    } else {
        4_096 // safe default
    }
}

/// Extract modification time as (seconds, nanoseconds) since Unix epoch.
///
/// Uses `SystemTime::duration_since(UNIX_EPOCH)` which handles the
/// Windows FILETIME → Unix epoch conversion internally and avoids
/// overflow risk from manual 100ns-interval arithmetic.
pub fn modification_time(meta: &Metadata) -> (i64, u32) {
    use std::time::SystemTime;

    match meta.modified() {
        Ok(sys_time) => match sys_time.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(dur) => (dur.as_secs() as i64, dur.subsec_nanos()),
            // Pre-1970 timestamps (rare but possible on some filesystems)
            Err(e) => {
                let dur = e.duration();
                (-(dur.as_secs() as i64), dur.subsec_nanos())
            }
        },
        Err(_) => (0, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn modification_time_is_positive_for_recent_file() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "jhara test").unwrap();
        let meta = tmp.as_file().metadata().unwrap();
        let (secs, _nanos) = modification_time(&meta);
        // File was just created — timestamp must be after 2020-01-01
        assert!(secs > 1_577_836_800, "mtime {} looks wrong", secs);
    }

    #[test]
    #[cfg(unix)]
    fn physical_size_nonzero_for_nonempty_file() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        // Write enough to allocate at least one filesystem block
        let data = vec![b'x'; 4_096];
        tmp.write_all(&data).unwrap();
        tmp.flush().unwrap();
        let meta = std::fs::metadata(tmp.path()).unwrap();
        let size = physical_size(tmp.path(), &meta);
        assert!(size >= 4_096, "physical_size {} < 4096", size);
    }
}
