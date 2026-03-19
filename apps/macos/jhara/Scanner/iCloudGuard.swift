// iCloudGuard.swift
// Jhara
//
// Prevents accidental iCloud file hydration during filesystem traversal.
//
// The danger: when a user enables "Desktop & Documents Folders" in iCloud
// Drive settings, macOS stores files that have not been downloaded locally
// as dataless placeholders. They appear as regular files in the directory
// listing but contain no local data.
//
// If any process calls open(), stat(), or even just tries to list the
// directory attributes of a dataless file, NSFileProvider intercepts the
// call and downloads the file from iCloud. For a developer with gigabytes
// of documents synced to iCloud, running fts_open over ~/Documents would
// silently trigger downloads of everything that was not already local.
//
// The fix is to check URLResourceKey.isUbiquitousItemKey before descending
// into any directory. If the directory is managed by iCloud (ubiquitous),
// we skip it entirely. We also check before the initial fts_open call on
// each root directory.
//
// Note: this check works for Apple's iCloud Drive. Third-party sync
// providers (Dropbox, Google Drive) do not use the ubiquitous item API
// and their files are always fully local. No special handling needed there.

import Foundation

// MARK: - iCloudGuard

/// A namespace for iCloud-awareness helpers used during filesystem traversal.
///
/// All methods are static because there is no state to maintain.
/// The checks are lightweight: a single URL resource value query.
public enum iCloudGuard {

    /// Scans the given home directory and returns a list of paths managed by
    /// iCloud Drive that should be skipped during retrieval to prevent hydration.
    ///
    /// This is called ONCE before starting the Rust scan.
    ///
    /// - Parameter homeURL: Typically the user's home directory URL.
    /// - Returns: An array of absolute path strings to skip.
    public static func buildSkipList(homeURL: URL) -> [String] {
        var skipPaths: [String] = []
        let keys: [URLResourceKey] = [
            .isUbiquitousItemKey,
            .ubiquitousItemDownloadingStatusKey
        ]
        
        guard let enumerator = FileManager.default.enumerator(
            at: homeURL,
            includingPropertiesForKeys: keys,
            options: [.skipsSubdirectoryDescendants, .skipsHiddenFiles]
        ) else {
            return []
        }
        
        for case let url as URL in enumerator {
            do {
                let values = try url.resourceValues(forKeys: Set(keys))
                
                // If it's ubiquitous, it's an iCloud-managed directory.
                if values.isUbiquitousItem == true {
                    // We skip the entire container to be safe.
                    skipPaths.append(url.path)
                    enumerator.skipDescendants()
                }
            } catch {
                // Skip on error to be safe.
                skipPaths.append(url.path)
                enumerator.skipDescendants()
            }
        }
        
        return skipPaths
    }
}
