/// A single filesystem entry produced by the scanner.
///
/// All fields are cross-platform. Platform-specific logic for deriving
/// physical size and file identity lives in `scanner::platform`.
#[derive(Debug, Clone)]
pub struct ScanNode {
    /// Full path to this entry.
    pub path: std::path::PathBuf,

    /// File or directory name (last component of `path`).
    pub name: String,

    /// Inode number on Unix (`st_ino`).
    /// On Windows: lower 64 bits of `FILE_ID_128` from `FILE_ID_INFO`.
    /// Zero if the platform could not determine the value.
    pub inode: u64,

    /// Device ID on Unix (`st_dev`).
    /// On Windows: `VolumeSerialNumber` from `FILE_ID_INFO`.
    /// Together with `inode` forms a unique file identity key for hard-link dedup.
    pub device_id: u64,

    /// Bytes actually allocated on disk.
    ///
    /// Unix:    `st_blocks * 512` (physical 512-byte sectors).
    /// Windows: logical size rounded up to volume cluster size.
    ///          Sparse/compressed files queried via `GetCompressedFileSizeW`.
    ///          OneDrive `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS` files → 0 (skipped).
    pub physical_size: u64,

    /// Logical byte count (`st_size` / `Metadata::len()`).
    pub logical_size: u64,

    /// Seconds since Unix epoch (from `stat.st_mtime` / `Metadata::modified()`).
    pub modification_secs: i64,

    /// Nanosecond sub-second component of modification time.
    pub modification_nanos: u32,

    /// Hard-link count (`st_nlink` / `Metadata::number_of_links()`).
    /// A value > 1 means this inode is shared — dedup via `InodeTracker`.
    pub link_count: u32,

    /// Entry classification used to drive traversal decisions and size rollup.
    pub kind: NodeKind,
}

/// Classification of a filesystem entry as produced by the scanner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// A regular file.
    File,
    /// A directory, emitted before its children (pre-order).
    DirPre,
    /// A directory, emitted after all children have been processed (post-order).
    /// Used to finalize size rollups.
    DirPost,
    /// A symbolic link. The scanner does not follow symlinks (`FTS_PHYSICAL` parity).
    Symlink,
    /// Anything else: device files, named pipes, sockets, etc.
    Other,
}

/// Aggregate statistics produced at the end of a scan.
#[derive(Debug, Default, Clone)]
pub struct ScanStats {
    /// Total entries visited (files + directories).
    pub total_entries: u64,
    /// Total unique physical bytes (after inode dedup).
    pub total_physical_bytes: u64,
    /// Total logical bytes.
    pub total_logical_bytes: u64,
    /// Number of hard-linked files whose size was skipped (duplicate inodes).
    pub deduped_entries: u64,
    /// Number of paths skipped due to iCloud / OneDrive placeholder status.
    pub skipped_cloud_entries: u64,
    /// Number of root directories that produced errors during traversal.
    pub error_count: u64,
    /// Artifact directory candidates found during single-pass detection.
    pub artifact_candidates: std::sync::Arc<std::sync::Mutex<Vec<crate::detector::artifact_scan::ArtifactCandidate>>>,
}

/// Errors that can be produced by `jhara-core`.
#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("I/O error at {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },

    #[error("Scan was cancelled by the caller")]
    Cancelled,

    #[error("Root path does not exist: {0}")]
    RootNotFound(std::path::PathBuf),
}
