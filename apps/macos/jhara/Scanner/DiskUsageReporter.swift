// DiskUsageReporter.swift
// Jhara
//
// Queries the correct disk usage and available capacity values for display.
//
// There are three "available space" numbers on macOS. They are all different
// and they mean different things. Picking the wrong one produces either
// pessimistic or optimistic figures that confuse users.
//
// volumeAvailableCapacity (NSURLVolumeAvailableCapacityKey):
//   Raw free blocks on the volume. This is what `df` reports. It does NOT
//   include space that the OS can reclaim immediately if needed. On a typical
//   developer Mac with Time Machine enabled, this number can be 30-50 GB
//   lower than what the user can actually recover, because Time Machine local
//   snapshots occupy "free" space until the OS needs it.
//
// volumeAvailableCapacityForImportantUsage (NSURLVolumeAvailableCapacityForImportantUsageKey):
//   Free space + immediately purgeable space (local TM snapshots, iCloud
//   cache files). This is what the About This Mac storage bar shows. This is
//   the number Jhara should display as "Available" because it reflects reality
//   from the user's perspective - they can recover this space by clicking
//   one button.
//
// volumeAvailableCapacityForOpportunisticUsage:
//   The space available for background, non-critical writes. Lower than the
//   above. Not useful for Jhara's purpose.
//
// File sizes - two values:
//   NSURLFileSizeKey (logical size): The size of the file's content.
//     e.g., a 1 MB file reports 1 MB. Two CoW clones both report 1 MB each.
//   NSURLFileAllocatedSizeKey (physical allocation): The actual blocks
//     reserved on disk. On APFS with CoW, two clones share blocks until
//     one is modified, so their allocated sizes reflect the shared portion.
//     This is what will be reclaimed by deletion.
//
//   For displaying "how much will I reclaim?", use NSURLFileAllocatedSizeKey.
//   For displaying "how large is this file?", use NSURLFileSizeKey.

import Foundation

// MARK: - DiskUsageReporter

/// Queries volume-level disk space metrics and per-file physical allocation.
///
/// All queries are non-throwing by design. Disk capacity queries can fail
/// if the volume is temporarily unavailable or permissions change mid-scan.
/// In those cases we return nil and let the caller decide how to handle it.
public enum DiskUsageReporter {

    // MARK: - Volume capacity

    /// Returns the total capacity of the volume at the given path, in bytes.
    ///
    /// - Parameter path: Any path on the volume you want to query.
    ///   Typically the user's home directory.
    public static func totalCapacity(at path: String) -> Int64? {
        let url = URL(fileURLWithPath: path)
        guard let values = try? url.resourceValues(forKeys: [.volumeTotalCapacityKey]),
              let bytes = values.volumeTotalCapacity else {
            return nil
        }
        return Int64(bytes)
    }

    /// Returns the available capacity including purgeable space (local Time
    /// Machine snapshots, iCloud cached files). This is the number to show
    /// to users as "Available Storage" because it reflects what they can
    /// actually recover.
    ///
    /// This matches the value shown in About This Mac > Storage.
    ///
    /// - Parameter path: Any path on the volume you want to query.
    public static func availableCapacity(at path: String) -> Int64? {
        let url = URL(fileURLWithPath: path)
        guard let values = try? url.resourceValues(forKeys: [
            .volumeAvailableCapacityForImportantUsageKey
        ]),
              let bytes = values.volumeAvailableCapacityForImportantUsage else {
            return nil
        }
        return bytes
    }

    /// Returns the raw free space without purgeable space included.
    /// This is lower than `availableCapacity` on machines with Time Machine
    /// local snapshots. Only use this for diagnostics.
    public static func rawFreeCapacity(at path: String) -> Int64? {
        let url = URL(fileURLWithPath: path)
        guard let values = try? url.resourceValues(forKeys: [.volumeAvailableCapacityKey]),
              let bytes = values.volumeAvailableCapacity else {
            return nil
        }
        return Int64(bytes)
    }

    /// Returns a `CapacitySnapshot` bundling total, available, and used space.
    ///
    /// - Parameter path: Any path on the target volume.
    public static func capacitySnapshot(at path: String) -> CapacitySnapshot? {
        guard let total = totalCapacity(at: path),
              let available = availableCapacity(at: path) else {
            return nil
        }
        return CapacitySnapshot(
            total: total,
            available: available,
            used: total - available
        )
    }

    // MARK: - Per-file physical allocation

    /// Returns the physical block allocation for a single file, in bytes.
    ///
    /// On APFS, this reflects the actual on-disk space used after
    /// Copy-on-Write sharing is accounted for. Use this (not the logical
    /// size) when computing how much space a deletion will reclaim.
    ///
    /// For directories, this returns the size of the directory metadata
    /// block, not the sum of its contents. Use ScanTree for directory totals.
    ///
    /// - Parameter url: The file URL to query.
    /// - Returns: Physical allocation in bytes, or nil if the query fails.
    public static func physicalAllocation(at url: URL) -> Int64? {
        guard let values = try? url.resourceValues(forKeys: [.fileAllocatedSizeKey]),
              let bytes = values.fileAllocatedSize else {
            return nil
        }
        return Int64(bytes)
    }

    /// Returns both logical and physical sizes for a file.
    /// Useful for diagnostics that want to show "apparent vs. actual" size.
    public static func fileSizes(at url: URL) -> FileSizes? {
        guard let values = try? url.resourceValues(forKeys: [
            .fileSizeKey,
            .fileAllocatedSizeKey,
        ]) else {
            return nil
        }

        let logical = values.fileSize.map { Int64($0) }
        let physical = values.fileAllocatedSize.map { Int64($0) }

        guard let l = logical, let p = physical else { return nil }
        return FileSizes(logical: l, physical: p)
    }
}

// MARK: - Supporting types

extension DiskUsageReporter {

    /// A point-in-time snapshot of a volume's capacity figures.
    public struct CapacitySnapshot: Sendable {
        /// Total formatted capacity of the volume (e.g., 512 GB).
        public let total: Int64
        /// Available space including immediately purgeable data.
        /// This is what the user can actually recover right now.
        public let available: Int64
        /// total - available. The space currently in use.
        public let used: Int64

        /// The fraction of the volume currently in use (0.0 to 1.0).
        public var usedFraction: Double {
            guard total > 0 else { return 0 }
            return Double(used) / Double(total)
        }

        /// Returns true when the volume is critically full.
        /// The threshold is 15% free, which scales with drive size:
        /// 38 GB on 256 GB, 150 GB on 1 TB.
        public var isCriticallyFull: Bool {
            usedFraction > 0.85
        }
    }

    /// Logical and physical sizes for a single file.
    public struct FileSizes: Sendable {
        /// The size of the file's content stream. This is what you see in
        /// Finder's Get Info panel.
        public let logical: Int64
        /// The actual disk space reserved for this file. May be lower than
        /// logical for APFS CoW clones that share blocks with other entries.
        public let physical: Int64

        /// True when this file's physical footprint is less than its logical
        /// size, indicating APFS CoW block sharing is in effect.
        public var isSharedOnDisk: Bool {
            physical < logical
        }
    }
}
