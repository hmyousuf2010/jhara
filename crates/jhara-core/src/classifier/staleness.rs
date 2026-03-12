use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use crate::cleaner::git::GitSessionCache;

#[derive(Debug, PartialEq, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    High,   // Contains a .git directory
    Medium, // Standard descriptors exist but no version control
    Low,    // Fallback/heuristic detection only
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StalenessResult {
    pub project_root: PathBuf,
    pub last_activity: SystemTime,
    pub is_stale: bool,
    pub has_dirty_working_tree: bool,
    pub confidence: Confidence,
}

pub struct StalenessChecker {
    stale_threshold: Duration,
    git_cache: GitSessionCache,
}

impl StalenessChecker {
    /// Creates a new checker with a given threshold and a shared Git cache.
    pub fn new(stale_threshold_days: u64, git_cache: GitSessionCache) -> Self {
        Self {
            stale_threshold: Duration::from_secs(stale_threshold_days * 24 * 60 * 60),
            git_cache,
        }
    }

    /// Safely retrieves the modification time of a file, ignoring errors.
    fn safe_mtime(path: &Path) -> Option<SystemTime> {
        fs::metadata(path).and_then(|m| m.modified()).ok()
    }

    /// Evaluates the staleness of a project directory.
    pub fn evaluate(&self, project_root: &Path, descriptor_mtime: Option<SystemTime>) -> std::io::Result<StalenessResult> {
        let git_dir = project_root.join(".git");
        let git_head = git_dir.join("HEAD");

        let mut confidence = Confidence::Medium;
        let mut is_dirty = false;
        let mut git_mtime = None;

        if git_dir.is_dir() {
            confidence = Confidence::High;
            // Gracefully handle git status failures (e.g., git not installed) by defaulting to safe (dirty = true)
            is_dirty = self.git_cache.has_dirty_working_tree(project_root).unwrap_or(true);
            git_mtime = Self::safe_mtime(&git_head);
        }

        // Resolution: max(descriptor_mtime, git_mtime), fallback to dir mtime, fallback to UNIX EPOCH
        let last_activity = descriptor_mtime.into_iter()
            .chain(git_mtime)
            .max()
            .unwrap_or_else(|| Self::safe_mtime(project_root).unwrap_or(SystemTime::UNIX_EPOCH));

        let age = SystemTime::now()
            .duration_since(last_activity)
            .unwrap_or(Duration::ZERO);

        let is_stale = age > self.stale_threshold;

        Ok(StalenessResult {
            project_root: project_root.to_path_buf(),
            last_activity,
            is_stale,
            has_dirty_working_tree: is_dirty,
            confidence,
        })
    }
}
