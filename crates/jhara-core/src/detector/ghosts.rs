// detector/ghosts.rs
//
// The Ghost Detection Engine.
// Detects artifacts that existed but have been deleted, or are referenced
// in developer logs/history but aren't currently on the filesystem.

use crate::detector::types::{Ecosystem, GhostCandidate};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Discovers ghost candidates from various system sources.
pub fn discover_ghosts(root: &Path) -> Vec<GhostCandidate> {
    let mut candidates = Vec::new();

    // 1. Git History
    if let Ok(git_ghosts) = from_git_history(root) {
        candidates.extend(git_ghosts);
    }

    // 2. Shell History (Home directory level, but filtered by root)
    if let Some(home) = dirs::home_dir() {
        candidates.extend(from_shell_history(&home, root));
    }

    // 3. Gitignore hints
    candidates.extend(from_gitignore(root));

    // 4. Dangling Symlinks
    candidates.extend(from_dangling_symlinks(root));

    candidates
}

fn from_git_history(root: &Path) -> Result<Vec<GhostCandidate>, std::io::Error> {
    let output = Command::new("git")
        .args(["log", "--diff-filter=D", "--summary", "--pretty=format:"])
        .current_dir(root)
        .output()?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut ghosts = Vec::new();
    let mut seen = HashSet::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.starts_with("delete mode") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                let path_str = parts[4..].join(" ");
                let path = PathBuf::from(path_str);

                // We only care about directory markers or significant files
                // that hint at a project's existence.
                if !seen.contains(&path) {
                    ghosts.push(GhostCandidate {
                        path: path.clone(),
                        source: Ecosystem::GhostGitHistory,
                        confidence_boost: 0.8,
                    });
                    seen.insert(path);
                }
            }
        }
    }

    Ok(ghosts)
}

fn from_shell_history(home: &Path, filter_root: &Path) -> Vec<GhostCandidate> {
    let mut candidates = Vec::new();
    let history_files = [".zsh_history", ".bash_history", ".fish_history"];

    for file_name in &history_files {
        let path = home.join(file_name);
        if let Ok(content) = fs::read_to_string(&path) {
            for line in content.lines() {
                // Heuristic: look for 'rm -rf' or 'rmdir' or 'mv' commands
                if line.contains("rm -rf") || line.contains("rmdir") {
                    if let Some(target) = extract_path_from_rm(line) {
                        let target_path = PathBuf::from(target);
                        if target_path.starts_with(filter_root) {
                            candidates.push(GhostCandidate {
                                path: target_path,
                                source: Ecosystem::GhostShellHistory,
                                confidence_boost: 0.5,
                            });
                        }
                    }
                }
            }
        }
    }

    candidates
}

fn extract_path_from_rm(line: &str) -> Option<&str> {
    // Basic extraction logic
    if let Some(idx) = line.rfind("rm -rf ") {
        return Some(line[idx + 7..].trim());
    }
    if let Some(idx) = line.rfind("rmdir ") {
        return Some(line[idx + 6..].trim());
    }
    None
}

fn from_gitignore(root: &Path) -> Vec<GhostCandidate> {
    let mut candidates = Vec::new();
    let gitignore = root.join(".gitignore");

    if let Ok(content) = fs::read_to_string(gitignore) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // If the ignored path doesn't exist, it's a ghost candidate
            let path = root.join(line);
            if !path.exists() {
                // Only consider it a ghost if it has a "famous" name
                if is_famous_artifact_name(line) {
                    candidates.push(GhostCandidate {
                        path: PathBuf::from(line),
                        source: Ecosystem::GhostGitignore,
                        confidence_boost: 0.3,
                    });
                }
            }
        }
    }

    candidates
}

fn is_famous_artifact_name(name: &str) -> bool {
    let famous = [
        "node_modules",
        "target",
        "build",
        "dist",
        "out",
        "vendor",
        ".venv",
        "bin",
        "obj",
    ];
    famous.contains(&name)
}

fn from_dangling_symlinks(root: &Path) -> Vec<GhostCandidate> {
    let mut candidates = Vec::new();
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.filter_map(Result::ok) {
            if let Ok(meta) = entry.metadata() {
                if meta.file_type().is_symlink() {
                    if let Ok(target) = fs::read_link(entry.path()) {
                        if !target.exists() {
                            candidates.push(GhostCandidate {
                                path: entry.path(),
                                source: Ecosystem::GhostSymlink,
                                confidence_boost: 0.9,
                            });
                        }
                    }
                }
            }
        }
    }
    candidates
}
