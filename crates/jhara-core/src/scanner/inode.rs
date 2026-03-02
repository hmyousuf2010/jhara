use std::collections::HashSet;

/// Tracks which (device_id, inode) pairs have already been counted
/// during a scan session.
///
/// Hard-linked files share an inode. PNPM's content-addressable store
/// creates hard links from `~/.pnpm/store/` into each project's
/// `node_modules/`, meaning two projects' `node_modules/lodash` entries
/// may point to the same physical disk blocks. Without deduplication,
/// summing file sizes counts those blocks once per link, inflating
/// disk usage by 3–10× in large monorepos.
///
/// This tracker maintains a `HashSet` of `(device_id, inode)` pairs
/// seen so far. Calling `should_count` returns `true` the first time
/// a pair is seen and inserts it; subsequent calls for the same pair
/// return `false`. Size accumulation should only happen when `should_count`
/// returns `true`.
///
/// On Windows, NTFS does not expose POSIX inodes. The `device_id` field
/// holds the `VolumeSerialNumber` and `inode` holds the lower 64 bits of
/// the 128-bit `FILE_ID_128` returned by `GetFileInformationByHandleEx`.
/// See `scanner::platform::windows::file_identity` for the query path.
pub struct InodeTracker {
    seen: HashSet<(u64, u64)>,
}

impl InodeTracker {
    /// Create a new tracker. `capacity` is the initial allocation for the
    /// internal HashSet — pass an estimate of the number of hard-linked
    /// files expected in the scan to avoid rehashing.
    ///
    /// For a typical developer home directory, 8 192 is a reasonable default
    /// (matching the original Swift implementation).
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            seen: HashSet::with_capacity(capacity),
        }
    }

    /// Returns `true` if this `(device_id, inode)` pair is new and the
    /// caller should count its size. Returns `false` if the pair has
    /// already been seen (hard link to a previously counted inode).
    ///
    /// # Example
    ///
    /// ```
    /// use jhara_core::scanner::inode::InodeTracker;
    ///
    /// let mut tracker = InodeTracker::with_capacity(64);
    /// assert!(tracker.should_count(1, 42));   // first time → count it
    /// assert!(!tracker.should_count(1, 42));  // same inode → skip
    /// assert!(tracker.should_count(1, 43));   // different inode → count
    /// assert!(tracker.should_count(2, 42));   // same inode, different device → count
    /// ```
    #[inline]
    pub fn should_count(&mut self, device_id: u64, inode: u64) -> bool {
        self.seen.insert((device_id, inode))
    }

    /// Number of unique inodes seen so far.
    #[inline]
    pub fn unique_count(&self) -> usize {
        self.seen.len()
    }

    /// Reset the tracker for reuse across multiple scans without
    /// reallocating the backing storage.
    #[inline]
    pub fn reset(&mut self) {
        self.seen.clear();
    }
}

impl Default for InodeTracker {
    fn default() -> Self {
        Self::with_capacity(8_192)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_inode_is_counted() {
        let mut t = InodeTracker::default();
        assert!(t.should_count(1, 100));
    }

    #[test]
    fn duplicate_inode_same_device_skipped() {
        let mut t = InodeTracker::default();
        assert!(t.should_count(1, 100));
        assert!(!t.should_count(1, 100));
        assert!(!t.should_count(1, 100)); // idempotent
    }

    #[test]
    fn same_inode_different_device_counted() {
        let mut t = InodeTracker::default();
        assert!(t.should_count(1, 100));
        // Same inode number on a different device is a different file
        assert!(t.should_count(2, 100));
    }

    #[test]
    fn unique_count_tracks_insertions() {
        let mut t = InodeTracker::default();
        t.should_count(1, 1);
        t.should_count(1, 2);
        t.should_count(1, 1); // duplicate
        assert_eq!(t.unique_count(), 2);
    }

    #[test]
    fn reset_clears_state() {
        let mut t = InodeTracker::default();
        t.should_count(1, 99);
        t.reset();
        assert_eq!(t.unique_count(), 0);
        assert!(t.should_count(1, 99)); // should count again after reset
    }
}
