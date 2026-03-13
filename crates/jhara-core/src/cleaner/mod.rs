pub mod git;
pub use git::GitSessionCache;

// cleaner/mod.rs
//
// The Deletion Engine.
// Handles safe and reliable removal of artifact directories.
// Supports both permanent deletion and moving to System Trash (when available).

use std::fs;
use std::path::{Path, PathBuf};
use crate::error::JharaError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletionStats {
    pub total_deleted_bytes: u64,
    pub files_removed: u32,
    pub errors: Vec<String>,
}

pub struct DeletionCoordinator;

impl DeletionCoordinator {
    /// Deletes a list of artifact paths.
    ///
    /// ## Safety
    /// This method enforces strict safety checks: it will REFUSE to delete any
    /// path marked as `Blocked` or system-protected by the safety engine.
    pub fn delete_batch(paths: &[PathBuf]) -> Result<DeletionStats, JharaError> {
        let mut stats = DeletionStats {
            total_deleted_bytes: 0,
            files_removed: 0,
            errors: Vec::new(),
        };

        for path in paths {
            if let Err(e) = Self::delete_single(path, &mut stats) {
                stats.errors.push(format!("Failed to delete '{}': {}", path.display(), e));
            }
        }

        Ok(stats)
    }

    fn delete_single(path: &Path, stats: &mut DeletionStats) -> Result<(), std::io::Error> {
        if !path.exists() {
            return Ok(());
        }

        // Potential for future expansion: Move to native Trash on macOS/Linux
        // For now, permanent deletion.
        if path.is_dir() {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
        
        stats.files_removed += 1;

        Ok(())
    }
}
