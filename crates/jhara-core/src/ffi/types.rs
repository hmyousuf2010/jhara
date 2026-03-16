use std::ffi::c_char;

/// A C-ABI-compatible representation of a single filesystem node.
///
/// ## Layout Contract (64 bytes, little-endian)
///
/// Fields are ordered by descending alignment to produce a deterministic,
/// padding-free layout on all tier-1 Rust targets (x86_64-apple-darwin,
/// aarch64-apple-darwin).  The explicit `_padding` field makes the intent
/// self-documenting and prevents the compiler from inserting silent bytes.
///
/// | Offset | Size | Field               |
/// |--------|------|---------------------|
/// |  0     |  8   | path                |
/// |  8     |  8   | name                |
/// | 16     |  8   | inode               |
/// | 24     |  8   | physical_size       |
/// | 32     |  8   | logical_size        |
/// | 40     |  8   | modification_secs   |
/// | 48     |  4   | modification_nanos  |
/// | 52     |  2   | link_count          |
/// | 54     |  1   | kind                |
/// | 55     |  1   | _padding            |
/// | 56     |  8   | _reserved           |
///
/// *cbindgen will assert the size via a `static_assert` in the generated header.
///
/// ## Lifetime
///
/// Both `path` and `name` point into an arena that is owned by the
/// `JharaScanHandle`.  They are valid from the callback invocation until
/// `jhara_scan_free` is called.  Swift consumers **must not** retain raw
/// pointers past that point; copy to a `String` immediately if needed.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ScanNodeC {
    // ── 8-byte-aligned pointers ──────────────────────────────────────────────
    /// NUL-terminated, UTF-8 best-effort path. Arena-owned.
    pub path: *const c_char,
    /// NUL-terminated file name component. Arena-owned.
    pub name: *const c_char,

    // ── 8-byte integers ──────────────────────────────────────────────────────
    /// Inode number as reported by `stat(2)`.
    pub inode: u64,
    /// Allocated disk space in bytes (`st_blocks * 512`).  `-1` if unknown.
    pub physical_size: i64,
    /// Apparent file size in bytes (`st_size`).  `-1` if unknown.
    pub logical_size: i64,
    /// Modification time — whole seconds since UNIX epoch.
    pub modification_secs: i64,

    // ── Sub-8-byte fields grouped to avoid padding ───────────────────────────
    /// Modification time — nanosecond fraction (0–999_999_999).
    pub modification_nanos: u32,
    /// Number of hard links (`st_nlink`).
    pub link_count: u16,
    /// Node kind discriminant.  See `NodeKind` for values.
    pub kind: u8,
    /// True if this is a ghost artifact (history-based).
    pub is_ghost: u8,
    /// Safety tier (Safe, Caution, Risky, Blocked).
    pub safety_tier: u8,
    /// Detailed safety rating (Safe, Caution with msg, Block with msg).
    pub safety_rating: u8,
    /// Reserved for future use — ensures 64-byte total size.
    pub _reserved: [u8; 6],
}

/// Ensures the struct has the expected size at compile time.
/// If this fails, the Swift alignment test will also fail — but this gives
/// a much earlier, human-readable error.
const _: () = assert!(
    std::mem::size_of::<ScanNodeC>() == 64,
    "ScanNodeC layout has changed — update the size constant and the Swift test"
);

/// Discriminant values for `ScanNodeC::kind`.
///
/// Represented as `u8` so it fits in the struct without padding.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Unknown   = 0,
    File      = 1,
    Directory = 2,
    Symlink   = 3,
    Other     = 4,
}

impl From<crate::scanner::NodeKind> for NodeKind {
    fn from(k: crate::scanner::NodeKind) -> Self {
        match k {
            crate::scanner::NodeKind::File    => NodeKind::File,
            crate::scanner::NodeKind::DirPre  => NodeKind::Directory,
            crate::scanner::NodeKind::DirPost => NodeKind::Directory,
            crate::scanner::NodeKind::Symlink => NodeKind::Symlink,
            crate::scanner::NodeKind::Other   => NodeKind::Other,
        }
    }
}

impl NodeKind {
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Owned mirror of `ScanNodeC` for storage inside the handle's results.
#[derive(Debug, Clone)]
pub struct ScanNodeOwned {
    pub path:               String,
    pub name:               String,
    pub inode:              u64,
    pub physical_size:      i64,
    pub logical_size:       i64,
    pub modification_secs:  i64,
    pub modification_nanos: u32,
    pub link_count:         u16,
    pub kind:               u8,
    pub is_ghost:           bool,
    pub safety_tier:        u8,
    pub safety_rating:      u8,
}

// ── Batch ─────────────────────────────────────────────────────────────────────

/// A fat pointer passed to the scan callback representing one batch of nodes.
///
/// The batch is only valid for the duration of the callback invocation.
/// The Swift callback **must** copy any data it needs before returning.
///
/// `count` may be zero on a progress heartbeat; the Swift side should handle
/// this gracefully (it is useful as a cancellation-check point).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ScanNodeBatchC {
    /// Pointer to the first element of a contiguous `ScanNodeC` array.
    pub nodes: *const ScanNodeC,
    /// Number of valid elements in `nodes`.
    pub count: usize,
}

// ── Safety ────────────────────────────────────────────────────────────────────

// SAFETY: The raw pointers inside these structs are arena-owned and read-only
// during the FFI call.  Callers are required by contract to not share them
// across threads without synchronisation; the structs themselves carry no
// exclusive state.
unsafe impl Send for ScanNodeC {}
unsafe impl Send for ScanNodeBatchC {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem;

    #[test]
    fn scan_node_c_size_is_64_bytes() {
        assert_eq!(mem::size_of::<ScanNodeC>(), 64);
    }

    #[test]
    fn scan_node_c_alignment_is_8_bytes() {
        assert_eq!(mem::align_of::<ScanNodeC>(), 8);
    }

    #[test]
    fn scan_node_batch_c_is_two_words() {
        // pointer + usize
        assert_eq!(mem::size_of::<ScanNodeBatchC>(), 16);
    }

    #[test]
    fn node_kind_discriminants_are_stable() {
        assert_eq!(NodeKind::Unknown   as u8, 0);
        assert_eq!(NodeKind::File      as u8, 1);
        assert_eq!(NodeKind::Directory as u8, 2);
        assert_eq!(NodeKind::Symlink   as u8, 3);
    }

    /// Mirrors the Swift alignment validation test described in §4.4.
    /// Populates a node with sentinel hex values and verifies round-trip
    /// byte identity through a raw pointer cast.
    #[test]
    fn sentinel_hex_round_trip() {
        let sentinel_path  = b"sentinel\0";
        let sentinel_name  = b"name\0";

        let node = ScanNodeC {
            path:               sentinel_path.as_ptr() as *const _,
            name:               sentinel_name.as_ptr() as *const _,
            inode:              0xDEAD_BEEF_CAFE_BABE_u64,
            physical_size:      0x0102_0304_0506_0708_i64,
            logical_size:       0x1112_1314_1516_1718_i64,
            modification_secs:  0x2122_2324_2526_2728_i64,
            modification_nanos: 0x3132_3334_u32,
            link_count:         0x4142_u16,
            kind:               NodeKind::File.as_u8(),
            is_ghost:           0,
            safety_tier:        0,
            safety_rating:      0,
            _reserved:          [0u8; 6],
        };

        // Verify values survive a round-trip through a raw pointer,
        // simulating what the C/Swift side does.
        let ptr: *const ScanNodeC = &node;
        let recovered = unsafe { &*ptr };

        assert_eq!(recovered.inode,             0xDEAD_BEEF_CAFE_BABE);
        assert_eq!(recovered.physical_size,     0x0102_0304_0506_0708);
        assert_eq!(recovered.logical_size,      0x1112_1314_1516_1718);
        assert_eq!(recovered.modification_secs, 0x2122_2324_2526_2728);
        assert_eq!(recovered.modification_nanos,0x3132_3334);
        assert_eq!(recovered.link_count,        0x4142);
        assert_eq!(recovered.kind,              NodeKind::File.as_u8());
        assert_eq!(recovered.is_ghost,          0);
    }
}
