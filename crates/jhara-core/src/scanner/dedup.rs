/// Windows-specific hard-link deduplication via `FILE_ID_INFO`.
///
/// On Windows, `Metadata::file_index()` returns a legacy 64-bit composite
/// file ID that is reliable for most purposes but is derived from
/// `BY_HANDLE_FILE_INFORMATION.nFileIndexHigh/Low`. Modern Windows exposes
/// the full 128-bit `FILE_ID_128` through `GetFileInformationByHandleEx`
/// with `FileIdInfo`. We use this for hard-link dedup because:
///
///   - NTFS 128-bit file IDs are guaranteed unique per-volume.
///   - The legacy 64-bit index can theoretically collide on large volumes.
///   - pnpm hard links are the primary scenario; accurate dedup matters.
///
/// The cost of opening a file handle is non-trivial (~1µs per file on NVMe),
/// so we only open handles for files where `link_count > 1` — i.e. files
/// that are actually hard-linked. Files with a single link never need the
/// expensive query.
///
/// On non-Windows platforms this module is empty and the type alias resolves
/// to the standard Unix identity.

#[cfg(windows)]
pub use windows_impl::query_file_id;

#[cfg(not(windows))]
/// No-op on non-Windows platforms. Unix identity comes from MetadataExt for free.
pub fn query_file_id(_path: &std::path::Path) -> Option<(u64, u64)> {
    None
}

#[cfg(windows)]
mod windows_impl {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;

    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        FileIdInfo, GetFileInformationByHandleEx, FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_INFO,
    };

    /// Open `path` with `FILE_FLAG_BACKUP_SEMANTICS` (required for directories)
    /// and query the full 128-bit file ID. Returns `(volume_serial, file_id_low)`
    /// where `file_id_low` is the lower 64 bits of `FILE_ID_128.Identifier`.
    ///
    /// Returns `None` if the handle cannot be opened or the query fails —
    /// callers should fall back to the legacy 64-bit identity in that case.
    pub fn query_file_id(path: &Path) -> Option<(u64, u64)> {
        // FILE_FLAG_BACKUP_SEMANTICS lets us open directories too.
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
            .open(path)
            .ok()?;

        let mut info = FILE_ID_INFO::default();
        let handle = HANDLE(file.as_raw_handle() as isize);

        let ok = unsafe {
            GetFileInformationByHandleEx(
                handle,
                FileIdInfo,
                &mut info as *mut FILE_ID_INFO as *mut _,
                std::mem::size_of::<FILE_ID_INFO>() as u32,
            )
        };

        if ok.is_err() {
            return None;
        }

        // FILE_ID_128.Identifier is a [u8; 16]. Split into two u64s.
        let bytes = info.FileId.Identifier;
        let low = u64::from_le_bytes(bytes[0..8].try_into().unwrap());

        Some((info.VolumeSerialNumber as u64, low))
    }
}

#[cfg(test)]
#[cfg(windows)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn file_id_is_consistent_for_same_file() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "dedup test").unwrap();
        let id1 = query_file_id(tmp.path());
        let id2 = query_file_id(tmp.path());
        assert!(id1.is_some());
        assert_eq!(id1, id2, "FILE_ID_INFO should be stable across calls");
    }
}
