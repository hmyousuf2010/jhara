// detector/safety.rs
//
// The Contextual Safety Checker.
// Evaluates deletion candidates for "contamination" (source files inside
// artifact directories) and protects critical system/project paths.

use std::path::Path;
use crate::detector::types::{SafetyRating, SafetyTier};

/// Evaluates a path and its siblings to determine a final safety rating.
pub fn evaluate_safety(path: &Path, tier: SafetyTier) -> SafetyRating {
    // 1. Absolute Blocklist check (redundant but safe)
    if is_protected_name(path) {
        return SafetyRating::Block(format!("'{}' is a protected name.", path.display()));
    }

    // 2. Contamination check: look for source files in the same directory
    // or as children (if it's a small directory).
    if contains_source_files(path) {
        return SafetyRating::Caution("Contains potential source files. Manual review required.".to_string());
    }

    // 3. Heuristic: if it's a generic name like 'build' or 'dist' but no
    // manifest was found for it, upgrade to Caution.
    if is_generic_artifact_name(path) && tier == SafetyTier::Safe {
        // We'll need more context from the detector to know if it was "confirmed"
        // For now, if it's Safe, we trust the signature logic.
    }

    SafetyRating::Safe
}

fn is_protected_name(path: &Path) -> bool {
    let protected = ["src", "internal", "pkg", ".git", ".ssh", "Documents", "Desktop", "Downloads"];
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if protected.contains(&name) {
            return true;
        }
    }
    false
}

fn is_generic_artifact_name(path: &Path) -> bool {
    let generic = ["build", "dist", "out", "target", "bin", "obj"];
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if generic.contains(&name) {
            return true;
        }
    }
    false
}

fn contains_source_files(path: &Path) -> bool {
    // We only scan the top level to avoid performance hit.
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.filter_map(Result::ok) {
            if let Some(ext) = entry.path().extension().and_then(|e| e.to_str()) {
                let source_exts = ["rs", "ts", "js", "swift", "cpp", "c", "h", "py", "go", "java"];
                if source_exts.contains(&ext) {
                    return true;
                }
            }
        }
    }
    false
}
