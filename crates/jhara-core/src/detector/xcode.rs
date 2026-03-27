// detector/xcode.rs
//
// Resolves Xcode DerivedData directories back to their originating projects.
//
// The problem:
//   Xcode stores compilation caches in ~/Library/Developer/Xcode/DerivedData/
//   under directory names like "MyApp-abcdefghijklmnop" — a hash of the
//   project path concatenated to the project name. There is no way to know
//   from the directory name alone which .xcodeproj or .xcworkspace created it.
//
// The solution:
//   Each DerivedData directory contains an info.plist that Xcode writes when
//   it creates the directory. The plist has a "WorkspacePath" key whose value
//   is the full path to the .xcodeproj or .xcworkspace that owns this cache.
//
//   If that path still exists on disk → the project is active; use the
//   project's git/file mtime for the staleness calculation.
//
//   If that path no longer exists → the DerivedData entry is orphaned.
//   Orphaned entries are Safe regardless of age: there is no project to
//   corrupt, and there is no staleness threshold that could make them risky.
//
// The plist format:
//   Xcode writes binary plist format. The `plist` crate handles both binary
//   and XML formats transparently, so we do not need to branch on format.

use std::fs;
use std::path::{Path, PathBuf};

use crate::detector::types::XcodeProjectRef;
use crate::error::JharaError;

// ─────────────────────────────────────────────────────────────────────────────
// XcodeResolver
// ─────────────────────────────────────────────────────────────────────────────

pub struct XcodeResolver;

impl XcodeResolver {
    /// Resolves a single DerivedData directory to its originating Xcode project.
    ///
    /// - `derived_data_dir`: One of the hash-named directories directly inside
    ///   `~/Library/Developer/Xcode/DerivedData/` (not the DerivedData root itself).
    ///
    /// Returns an `XcodeProjectRef` that describes whether the project still
    /// exists. Returns an error only if the plist file exists but is malformed
    /// (missing or unreadable plist returns an orphaned ref, not an error).
    pub fn resolve(derived_data_dir: &Path) -> Result<XcodeProjectRef, JharaError> {
        let info_plist = derived_data_dir.join("info.plist");

        if !info_plist.is_file() {
            // No info.plist → either an old format or a partially-created entry.
            // Treat as orphaned.
            return Ok(XcodeProjectRef {
                derived_data_path: derived_data_dir.to_path_buf(),
                project_path: None,
                is_orphaned: true,
            });
        }

        // Parse the plist. The `plist` crate handles binary and XML formats.
        let value = plist::from_file(&info_plist)
            .map_err(|e| JharaError::plist(info_plist.to_string_lossy(), e))?;

        let workspace_path: Option<String> = Self::extract_workspace_path(&value);

        match workspace_path {
            None => {
                // plist exists but has no WorkspacePath — unusual, treat as orphaned.
                Ok(XcodeProjectRef {
                    derived_data_path: derived_data_dir.to_path_buf(),
                    project_path: None,
                    is_orphaned: true,
                })
            }
            Some(path_str) => {
                let project_path = PathBuf::from(&path_str);
                let exists = project_path.exists();
                Ok(XcodeProjectRef {
                    derived_data_path: derived_data_dir.to_path_buf(),
                    project_path: Some(project_path),
                    is_orphaned: !exists,
                })
            }
        }
    }

    /// Scans the DerivedData root and returns refs for every sub-directory.
    ///
    /// - `derived_data_root`: `~/Library/Developer/Xcode/DerivedData/`
    ///
    /// Sub-directories that fail to resolve (malformed plist) are included
    /// as orphaned refs rather than causing the whole scan to fail.
    pub fn resolve_all(derived_data_root: &Path) -> Vec<XcodeProjectRef> {
        let Ok(entries) = fs::read_dir(derived_data_root) else {
            return vec![];
        };

        entries
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().is_dir())
            .filter(|entry| {
                // Skip non-project directories like "ModuleCache.noindex"
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                // Hashed DerivedData entries contain a hyphen separating
                // the project name from the hash. Plain names without hyphens
                // are utility directories.
                name_str.contains('-')
            })
            .map(|entry| {
                Self::resolve(&entry.path()).unwrap_or_else(|_| XcodeProjectRef {
                    derived_data_path: entry.path(),
                    project_path: None,
                    is_orphaned: true,
                })
            })
            .collect()
    }

    /// Returns the total disk size in bytes of all orphaned DerivedData entries
    /// in `derived_data_root`. Useful for reporting before presenting cleanup UI.
    pub fn orphaned_size_bytes(derived_data_root: &Path) -> u64 {
        Self::resolve_all(derived_data_root)
            .into_iter()
            .filter(|r| r.is_orphaned)
            .map(|r| dir_size_bytes(&r.derived_data_path))
            .sum()
    }

    // ── Private helpers ────────────────────────────────────────────────────

    /// Extracts the `WorkspacePath` string from a parsed plist value.
    ///
    /// The info.plist structure is:
    ///   {
    ///     "WorkspacePath" = "/Users/dev/MyApp/MyApp.xcodeproj";
    ///     "LastAccessedDate" = <timestamp>;
    ///     ...
    ///   }
    ///
    /// The plist crate deserializes this into a `plist::Value::Dictionary`.
    fn extract_workspace_path(value: &plist::Value) -> Option<String> {
        let dict = value.as_dictionary()?;
        let workspace = dict.get("WorkspacePath")?;
        workspace.as_string().map(|s| s.to_string())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Utilities
// ─────────────────────────────────────────────────────────────────────────────

/// Recursively sums the sizes of all files under `path`.
/// Not precise for APFS CoW scenarios, but accurate enough for heuristic
/// reporting of "how much space will I save".
fn dir_size_bytes(path: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries
        .filter_map(|e| e.ok())
        .map(|e| {
            let p = e.path();
            if p.is_dir() {
                dir_size_bytes(&p)
            } else {
                p.metadata().map(|m| m.len()).unwrap_or(0)
            }
        })
        .sum()
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use plist::Dictionary;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_info_plist(dir: &Path, workspace_path: Option<&str>) {
        let plist_path = dir.join("info.plist");
        let mut dict = Dictionary::new();
        if let Some(path) = workspace_path {
            dict.insert(
                "WorkspacePath".to_string(),
                plist::Value::String(path.to_string()),
            );
        }
        dict.insert(
            "LastAccessedDate".to_string(),
            plist::Value::String("2024-01-01".to_string()),
        );
        let value = plist::Value::Dictionary(dict);
        // Write as XML plist (human-readable in tests)
        value.to_file_xml(&plist_path).unwrap();
    }

    fn make_derived_data_dir(name: &str) -> (TempDir, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let dd_dir = tmp.path().join(name);
        std::fs::create_dir_all(&dd_dir).unwrap();
        (tmp, dd_dir)
    }

    // ── resolve: project exists ───────────────────────────────────────────────

    #[test]
    fn resolves_to_existing_project() {
        let (tmp, dd_dir) = make_derived_data_dir("MyApp-abcdef1234567890");

        // Create a fake .xcodeproj so the path exists
        let project_dir = tmp.path().join("MyApp.xcodeproj");
        std::fs::create_dir_all(&project_dir).unwrap();

        write_info_plist(&dd_dir, Some(project_dir.to_str().unwrap()));

        let result = XcodeResolver::resolve(&dd_dir).unwrap();
        assert!(
            !result.is_orphaned,
            "Project exists on disk; should not be orphaned"
        );
        assert_eq!(result.project_path.unwrap(), project_dir);
        assert_eq!(result.derived_data_path, dd_dir);
    }

    // ── resolve: project deleted (orphaned) ──────────────────────────────────

    #[test]
    fn marks_orphaned_when_project_does_not_exist() {
        let (_tmp, dd_dir) = make_derived_data_dir("DeletedApp-abcdef1234567890");

        write_info_plist(&dd_dir, Some("/Users/dev/DeletedApp/DeletedApp.xcodeproj"));

        let result = XcodeResolver::resolve(&dd_dir).unwrap();
        assert!(
            result.is_orphaned,
            "Project path does not exist; should be orphaned"
        );
        assert!(
            result.project_path.is_some(),
            "project_path should be Some even when orphaned"
        );
    }

    // ── resolve: no info.plist ────────────────────────────────────────────────

    #[test]
    fn orphaned_when_no_info_plist() {
        let (_tmp, dd_dir) = make_derived_data_dir("NoInfoPlist-abc123");
        // Do NOT write info.plist

        let result = XcodeResolver::resolve(&dd_dir).unwrap();
        assert!(result.is_orphaned);
        assert!(result.project_path.is_none());
    }

    // ── resolve: plist without WorkspacePath ─────────────────────────────────

    #[test]
    fn orphaned_when_plist_has_no_workspace_path() {
        let (_tmp, dd_dir) = make_derived_data_dir("NoWorkspace-abc123");
        write_info_plist(&dd_dir, None); // plist exists, no WorkspacePath key

        let result = XcodeResolver::resolve(&dd_dir).unwrap();
        assert!(result.is_orphaned);
    }

    // ── resolve_all ───────────────────────────────────────────────────────────

    #[test]
    fn resolve_all_scans_multiple_entries() {
        let tmp = TempDir::new().unwrap();
        let derived_data_root = tmp.path();

        // Three hash-named subdirectories
        for name in &["App1-aaaa", "App2-bbbb", "App3-cccc"] {
            let dd_dir = derived_data_root.join(name);
            std::fs::create_dir_all(&dd_dir).unwrap();
            write_info_plist(&dd_dir, Some("/nonexistent/path.xcodeproj"));
        }

        // One utility directory that should be skipped (no hyphen)
        let util = derived_data_root.join("ModuleCache.noindex");
        std::fs::create_dir_all(&util).unwrap();

        let refs = XcodeResolver::resolve_all(derived_data_root);
        assert_eq!(
            refs.len(),
            3,
            "Should resolve 3 hash-named entries, skipping ModuleCache"
        );
        assert!(
            refs.iter().all(|r| r.is_orphaned),
            "All three have nonexistent project paths"
        );
    }

    // ── orphaned_size_bytes ───────────────────────────────────────────────────

    #[test]
    fn orphaned_size_bytes_sums_orphaned_directories() {
        let tmp = TempDir::new().unwrap();
        let derived_data_root = tmp.path();

        // Create one orphaned DerivedData entry with some files
        let orphaned = derived_data_root.join("DeadApp-dead1234");
        std::fs::create_dir_all(&orphaned).unwrap();
        write_info_plist(&orphaned, Some("/nonexistent/DeadApp.xcodeproj"));

        // Write some content to give it non-zero size
        let mut f = std::fs::File::create(orphaned.join("large_cache_file")).unwrap();
        write!(f, "{}", "x".repeat(4096)).unwrap();

        let size = XcodeResolver::orphaned_size_bytes(derived_data_root);
        assert!(
            size > 0,
            "Orphaned directory should contribute non-zero bytes"
        );
    }

    // ── XcodeProjectRef fields ────────────────────────────────────────────────

    #[test]
    fn project_ref_derived_data_path_is_absolute() {
        let (_tmp, dd_dir) = make_derived_data_dir("AbsPath-abc123");
        write_info_plist(&dd_dir, Some("/nonexistent/path.xcodeproj"));

        let result = XcodeResolver::resolve(&dd_dir).unwrap();
        assert!(result.derived_data_path.is_absolute());
    }
}
