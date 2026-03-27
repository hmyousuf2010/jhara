use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, RwLock};

/// A thread-safe cache for Git safety checks, persisting for the duration of a scan session.
#[derive(Clone, Default)]
pub struct GitSessionCache {
    // Stores project_root -> is_dirty
    cache: Arc<RwLock<HashMap<PathBuf, bool>>>,
}

impl GitSessionCache {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Checks if a working tree is dirty, utilizing the cache to avoid redundant subprocesses.
    pub fn has_dirty_working_tree(&self, repo_path: &Path) -> io::Result<bool> {
        // Fast path: Check cache using a read lock
        if let Ok(read_guard) = self.cache.read() {
            if let Some(&is_dirty) = read_guard.get(repo_path) {
                return Ok(is_dirty);
            }
        }

        // Cache miss: Execute git command
        let is_dirty = self.execute_git_status(repo_path)?;

        // Update cache
        if let Ok(mut write_guard) = self.cache.write() {
            write_guard.insert(repo_path.to_path_buf(), is_dirty);
        }

        Ok(is_dirty)
    }

    fn execute_git_status(&self, repo_path: &Path) -> io::Result<bool> {
        let output = Command::new("git")
            .current_dir(repo_path)
            .args(["status", "--porcelain"])
            .output()?;

        if !output.status.success() {
            return Err(io::Error::other(format!(
                "Git status failed for {:?}",
                repo_path
            )));
        }

        // Non-empty output implies uncommitted changes
        Ok(!output.stdout.is_empty())
    }
}
