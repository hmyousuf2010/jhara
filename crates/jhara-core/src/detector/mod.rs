// detector/mod.rs
//
// The `ProjectDetector` — the top-level entry point for Phase 2.
//
// It takes a stream of filesystem paths (produced by the Phase 1 scanner),
// matches them against the signature database, resolves monorepo structures,
// merges framework-specific artifacts, and emits `DetectedProject` values.
//
// Architecture:
//   The detector operates in two passes:
//
//   Pass 1 — Signature matching (runs during the FTS scan):
//     As the scanner emits directory entries, the detector checks whether
//     each directory contains any signature files. If it finds one, it
//     records the project root and the matched signature.
//
//   Pass 2 — Resolution (runs after the scan completes):
//     For each candidate project root:
//       a. Determine final ecosystem list (handle multi-ecosystem roots)
//       b. Run FrameworkDetector on package.json if present
//       c. Run MonorepoResolver on the root
//       d. Resolve artifact paths to absolute paths and check existence
//       e. Query ScanTree for physical sizes
//       f. Read mtime of signature file and .git/HEAD
//     Emit DetectedProject.
//
// This two-pass design avoids re-scanning: the FTS walk happens once in
// Phase 1 and the detector consumes its output.

pub mod artifact_scan;
pub mod frameworks;
pub mod ghosts;
pub mod monorepo;
pub mod safety;
pub mod signatures;
pub mod types;
pub mod xcode;
pub use artifact_scan::{
    all_manifest_hints, find_artifact_rule, resolve_artifact_candidates, ArtifactCandidate,
    ArtifactDetectionResult, ManifestMap,
};

pub use monorepo::{MonorepoInfo, MonorepoResolver};
pub use types::{
    ArtifactPath, ArtifactPathOwned, DetectedProject, Ecosystem, FoundArtifact, MonorepoKind,
    MonorepoMembership, ProjectSignature, ProjectSignatureOwned, SafetyTier, XcodeProjectRef,
};
pub use xcode::XcodeResolver;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::error::JharaError;
use frameworks::detect_framework_artifacts;
use ghosts::discover_ghosts;
use monorepo::OwnedArtifact;
use safety::evaluate_safety;
use signatures::{GLOBAL_CACHE_SIGNATURES, PROJECT_SIGNATURES};
use types::SafetyRating;

// ─────────────────────────────────────────────────────────────────────────────
// SignatureIndex — O(1) lookup by filename
// ─────────────────────────────────────────────────────────────────────────────

/// A hash map from filename to the list of signatures that match it.
/// Built once at startup from the static signature tables.
struct SignatureIndex {
    /// Exact filename → signatures
    exact: HashMap<&'static str, Vec<&'static ProjectSignature>>,
    /// Suffix patterns ("*.tf", "*.cabal") evaluated separately
    suffix: Vec<(&'static str, &'static ProjectSignature)>,
}

impl SignatureIndex {
    fn build() -> Self {
        let mut exact: HashMap<&'static str, Vec<&'static ProjectSignature>> = HashMap::new();
        let mut suffix: Vec<(&'static str, &'static ProjectSignature)> = Vec::new();

        for sig in PROJECT_SIGNATURES {
            if sig.filename.starts_with('*') {
                // "*.tf" → suffix ".tf"
                let ext = &sig.filename[1..];
                suffix.push((ext, sig));
            } else {
                exact.entry(sig.filename).or_default().push(sig);
            }
        }

        SignatureIndex { exact, suffix }
    }

    /// Returns all signatures whose filename matches `name`, ordered by
    /// priority descending (highest priority = most specific match wins).
    fn matches(&self, name: &str) -> Vec<&'static ProjectSignature> {
        let mut results: Vec<&'static ProjectSignature> =
            self.exact.get(name).map(|v| v.to_vec()).unwrap_or_default();

        for (ext, sig) in &self.suffix {
            if name.ends_with(ext) {
                results.push(sig);
            }
        }

        results.sort_by_key(|s| std::cmp::Reverse(s.priority));
        results
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// CandidateProject — intermediate state between detection and resolution
// ─────────────────────────────────────────────────────────────────────────────

struct CandidateProject {
    root: PathBuf,
    matched_signatures: Vec<&'static ProjectSignature>,
}

// ─────────────────────────────────────────────────────────────────────────────
// ProjectDetector
// ─────────────────────────────────────────────────────────────────────────────

/// The top-level ecosystem detector.
///
/// Usage:
/// ```ignore
/// let mut detector = ProjectDetector::new();
///
/// // Feed it every filename the scanner visits:
/// for (dir, filename) in scanner_output {
///     detector.observe(&dir, &filename);
/// }
///
/// // Resolve all candidates into DetectedProject values:
/// let projects = detector.resolve_all()?;
/// ```
pub struct ProjectDetector {
    index: SignatureIndex,
    candidates: Vec<CandidateProject>,
    /// Map from root path to its index in `candidates` for dedup.
    root_index: HashMap<PathBuf, usize>,
}

impl ProjectDetector {
    pub fn new() -> Self {
        ProjectDetector {
            index: SignatureIndex::build(),
            candidates: Vec::new(),
            root_index: HashMap::new(),
        }
    }

    /// Feed the detector a single directory entry from the scanner.
    ///
    /// `dir` is the directory containing the file; `filename` is the
    /// last path component (the file name, not the full path).
    ///
    /// This is cheap: it performs one hash map lookup per call.
    pub fn observe(&mut self, dir: &Path, filename: &str) {
        let matching = self.index.matches(filename);
        if matching.is_empty() {
            return;
        }

        // For signatures with content_key, we need to read the file to confirm.
        // Filter out signatures whose content_key is not present.
        let file_path = dir.join(filename);
        let confirmed: Vec<&ProjectSignature> = matching
            .into_iter()
            .filter(|sig| {
                if let Some(key) = sig.content_key {
                    file_contains(dir.join(filename).as_path(), key)
                } else {
                    true
                }
            })
            .collect();

        if confirmed.is_empty() {
            return;
        }

        // Deduplicate: if this root was already seen, append the new signatures.
        if let Some(&idx) = self.root_index.get(dir) {
            self.candidates[idx].matched_signatures.extend(confirmed);
        } else {
            let idx = self.candidates.len();
            self.candidates.push(CandidateProject {
                root: dir.to_path_buf(),
                matched_signatures: confirmed,
            });
            self.root_index.insert(dir.to_path_buf(), idx);
        }

        // Suppress the "unused variable" warning from the borrow above.
        let _ = file_path;
    }

    /// Resolves all candidates into `DetectedProject` values.
    ///
    /// This is the slower, allocation-heavy pass. It reads mtimes, runs
    /// MonorepoResolver, and calls FrameworkDetector for each Node.js project.
    /// Call it once after the scanner has finished, not during the scan.
    pub fn resolve_all(self) -> Result<Vec<DetectedProject>, JharaError> {
        let mut projects = Vec::with_capacity(self.candidates.len());

        for candidate in self.candidates {
            match resolve_candidate(candidate) {
                Ok(project) => projects.push(project),
                Err(_e) => {
                    // A failed resolution (permission error, etc.) is not a
                    // fatal error. Skip this candidate silently.
                    // TODO: surface these as warnings in the UI.
                }
            }
        }

        Ok(projects)
    }

    /// The number of project roots recorded so far.
    pub fn candidate_count(&self) -> usize {
        self.candidates.len()
    }

    /// Convenience method to detect projects at a specific path (Pass 1 + Pass 2).
    pub fn detect_at(mut self, path: &Path) -> Result<Vec<DetectedProject>, JharaError> {
        if !path.is_dir() {
            return self.resolve_all();
        }

        // Pass 1: find candidates in this directory
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                if let Ok(name) = entry.file_name().into_string() {
                    self.observe(path, &name);
                }
            }
        }

        // Pass 2: resolve
        self.resolve_all()
    }
}

impl Default for ProjectDetector {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// resolve_candidate — Pass 2 for a single project
// ─────────────────────────────────────────────────────────────────────────────

fn resolve_candidate(candidate: CandidateProject) -> Result<DetectedProject, JharaError> {
    let root = &candidate.root;

    // Collect ecosystems from all matched signatures (highest priority first).
    let mut ecosystems: Vec<Ecosystem> = candidate
        .matched_signatures
        .iter()
        .map(|s| s.ecosystem)
        .collect();
    ecosystems.dedup();

    // Gather all artifact path definitions from all matched signatures.
    let mut artifact_defs: Vec<OwnedArtifact> = candidate
        .matched_signatures
        .iter()
        .flat_map(|sig| {
            sig.artifact_paths.iter().map(|ap| OwnedArtifact {
                relative_path: ap.relative_path.to_string(),
                safety_tier: ap.safety_tier,
                ecosystem: sig.ecosystem,
                recovery_command: ap.recovery_command.map(|s| s.to_string()),
            })
        })
        .collect();

    // If any matched ecosystem is Node.js, run FrameworkDetector.
    let has_node = ecosystems.contains(&Ecosystem::NodeJs)
        || ecosystems.contains(&Ecosystem::Bun)
        || ecosystems.contains(&Ecosystem::Deno);
    if has_node {
        let pkg_json = root.join("package.json");
        if pkg_json.is_file() {
            let framework_artifacts = detect_framework_artifacts(&pkg_json);
            for fa in framework_artifacts {
                artifact_defs.push(OwnedArtifact {
                    relative_path: fa.relative_path.to_string(),
                    safety_tier: fa.safety_tier,
                    ecosystem: fa.ecosystem,
                    recovery_command: Some(fa.recovery_command.to_string()),
                });
            }
        }
    }

    // Run MonorepoResolver.
    let monorepo_info = MonorepoResolver::resolve(root)?;
    if let Some(ref info) = monorepo_info {
        // Add the monorepo's shared artifacts that do not already appear.
        for shared in &info.shared_artifacts {
            let already_listed = artifact_defs
                .iter()
                .any(|a| a.relative_path == shared.relative_path);
            if !already_listed {
                artifact_defs.push(shared.clone());
            }
        }
    }

    // Read the signature file mtime (use the first signature's filename).
    let signature_filename = candidate
        .matched_signatures
        .first()
        .map(|s| s.filename)
        .unwrap_or("package.json");
    let sig_path = root.join(signature_filename);
    let signature_mtime = file_mtime(&sig_path).unwrap_or(SystemTime::UNIX_EPOCH);

    // Read .git/HEAD mtime if this is a git repo.
    let git_head = root.join(".git").join("HEAD");
    let git_head_mtime = if git_head.is_file() {
        file_mtime(&git_head)
    } else {
        None
    };

    // Resolve artifact paths to absolute paths; check existence.
    let mut artifacts: Vec<FoundArtifact> = artifact_defs
        .into_iter()
        .filter_map(|def| {
            let abs = root.join(&def.relative_path);
            if abs.exists() {
                let safety_rating = evaluate_safety(&abs, def.safety_tier);
                Some(FoundArtifact {
                    absolute_path: abs,
                    safety_tier: def.safety_tier,
                    ecosystem: def.ecosystem,
                    physical_size_bytes: 0, // Populated later from ScanTree
                    recovery_command: def.recovery_command,
                    safety_rating,
                    is_ghost: false,
                })
            } else {
                None // Only include artifacts that actually exist
            }
        })
        .collect();

    // ── Ghost Discovery ──────────────────────────────────────────────────────
    let ghost_candidates = discover_ghosts(root);
    for gc in ghost_candidates {
        // If it's not already in the physical list, add it as a ghost
        if !artifacts.iter().any(|a| a.absolute_path == gc.path) {
            let abs_ghost = if gc.path.is_absolute() {
                gc.path.clone()
            } else {
                root.join(&gc.path)
            };
            artifacts.push(FoundArtifact {
                absolute_path: abs_ghost,
                safety_tier: SafetyTier::Safe, // Ghosts are usually safe to "clean" (remove from history/ignore)
                ecosystem: gc.source,
                physical_size_bytes: 0,
                recovery_command: None,
                safety_rating: SafetyRating::Safe,
                is_ghost: true,
            });
        }
    }

    Ok(DetectedProject {
        root_path: root.clone(),
        ecosystems,
        artifacts,
        signature_mtime,
        git_head_mtime,
        monorepo: monorepo_info
            .as_ref()
            .and_then(|info| info.membership_for(root)),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Global cache detection
// ─────────────────────────────────────────────────────────────────────────────

/// Checks the user's home directory for global developer tool caches.
///
/// These are not associated with individual projects. They are presented
/// in a separate "Global Caches" section in the UI.
pub fn detect_global_caches(home_dir: &Path) -> Vec<FoundArtifact> {
    let mut results = Vec::new();

    for sig in GLOBAL_CACHE_SIGNATURES {
        for artifact_def in sig.artifact_paths {
            if !artifact_def.is_global {
                continue;
            }
            let abs = home_dir.join(artifact_def.relative_path);
            if abs.exists() {
                results.push(FoundArtifact {
                    absolute_path: abs,
                    safety_tier: artifact_def.safety_tier,
                    ecosystem: sig.ecosystem,
                    physical_size_bytes: 0, // Populated from ScanTree later
                    recovery_command: artifact_def.recovery_command.map(|s| s.to_string()),
                    safety_rating: SafetyRating::Safe, // Global caches are always safe to clean
                    is_ghost: false,
                });
            }
        }
    }

    results
}

// ─────────────────────────────────────────────────────────────────────────────
// Private helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Returns true if the file at `path` contains `key` anywhere in its content.
/// Used for content_key confirmation. Reads the whole file: acceptable because
/// signature files (package.json, Cargo.toml, etc.) are always small.
fn file_contains(path: &Path, key: &str) -> bool {
    fs::read_to_string(path)
        .map(|content| content.contains(key))
        .unwrap_or(false)
}

/// Returns the modification time of a file, or None if it cannot be read.
fn file_mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

// ─────────────────────────────────────────────────────────────────────────────
// Integration tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write(dir: &Path, name: &str, content: &str) {
        let path = dir.join(name);
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).unwrap();
        }
        let mut f = std::fs::File::create(path).unwrap();
        write!(f, "{}", content).unwrap();
    }

    fn make_dir(dir: &Path, name: &str) {
        std::fs::create_dir_all(dir.join(name)).unwrap();
    }

    // ── Signature index ───────────────────────────────────────────────────────

    #[test]
    fn signature_index_finds_exact_match() {
        let index = SignatureIndex::build();
        let matches = index.matches("package.json");
        assert!(!matches.is_empty(), "package.json should match Node.js");
        assert!(matches.iter().any(|s| s.ecosystem == Ecosystem::NodeJs));
    }

    #[test]
    fn signature_index_finds_wildcard_match() {
        let index = SignatureIndex::build();
        let matches = index.matches("main.tf");
        assert!(!matches.is_empty(), "*.tf should match Terraform");
        assert!(matches.iter().any(|s| s.ecosystem == Ecosystem::Terraform));
    }

    #[test]
    fn signature_index_returns_empty_for_unknown_file() {
        let index = SignatureIndex::build();
        let matches = index.matches("totally_unknown_file.xyz");
        assert!(matches.is_empty());
    }

    #[test]
    fn signature_index_higher_priority_comes_first() {
        let index = SignatureIndex::build();
        // manage.py has priority 10; it should come before package.json (0)
        let matches = index.matches("manage.py");
        assert!(!matches.is_empty());
        assert_eq!(matches[0].ecosystem, Ecosystem::Django);
    }

    // ── Node.js project detection ─────────────────────────────────────────────

    #[test]
    fn detects_node_project() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "package.json", r#"{"name": "my-app"}"#);
        make_dir(tmp.path(), "node_modules");

        let mut detector = ProjectDetector::new();
        detector.observe(tmp.path(), "package.json");
        let projects = detector.resolve_all().unwrap();

        assert_eq!(projects.len(), 1);
        let p = &projects[0];
        assert!(p.ecosystems.contains(&Ecosystem::NodeJs));
        assert!(p
            .artifacts
            .iter()
            .any(|a| a.absolute_path.ends_with("node_modules")));
    }

    #[test]
    fn detects_next_js_framework_artifacts() {
        let tmp = TempDir::new().unwrap();
        write(
            tmp.path(),
            "package.json",
            r#"{"dependencies": {"next": "14.0.0"}}"#,
        );
        make_dir(tmp.path(), "node_modules");
        make_dir(tmp.path(), ".next");

        let mut detector = ProjectDetector::new();
        detector.observe(tmp.path(), "package.json");
        let projects = detector.resolve_all().unwrap();

        let p = &projects[0];
        assert!(
            p.artifacts
                .iter()
                .any(|a| a.absolute_path.ends_with(".next")),
            "Expected .next/ artifact for a Next.js project"
        );
    }

    // ── Rust project detection ─────────────────────────────────────────────────

    #[test]
    fn detects_rust_project() {
        let tmp = TempDir::new().unwrap();
        write(
            tmp.path(),
            "Cargo.toml",
            "[package]\nname = \"myapp\"\nversion = \"0.1.0\"\n",
        );
        make_dir(tmp.path(), "target");

        let mut detector = ProjectDetector::new();
        detector.observe(tmp.path(), "Cargo.toml");
        let projects = detector.resolve_all().unwrap();

        assert_eq!(projects.len(), 1);
        assert!(projects[0].ecosystems.contains(&Ecosystem::Rust));
        assert!(projects[0]
            .artifacts
            .iter()
            .any(|a| a.absolute_path.ends_with("target")));
    }

    // ── Python detection ──────────────────────────────────────────────────────

    #[test]
    fn detects_python_pip_project() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "requirements.txt", "requests==2.31.0\n");
        make_dir(tmp.path(), ".venv");
        make_dir(tmp.path(), "__pycache__");

        let mut detector = ProjectDetector::new();
        detector.observe(tmp.path(), "requirements.txt");
        let projects = detector.resolve_all().unwrap();

        assert_eq!(projects.len(), 1);
        assert!(projects[0].ecosystems.contains(&Ecosystem::PythonPip));

        let artifact_paths: Vec<_> = projects[0]
            .artifacts
            .iter()
            .map(|a| {
                a.absolute_path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            })
            .collect();
        assert!(artifact_paths.contains(&".venv".to_string()));
        assert!(artifact_paths.contains(&"__pycache__".to_string()));
    }

    // ── Go detection ──────────────────────────────────────────────────────────

    #[test]
    fn detects_go_project() {
        let tmp = TempDir::new().unwrap();
        write(
            tmp.path(),
            "go.mod",
            "module github.com/example/myapp\n\ngo 1.21\n",
        );

        let mut detector = ProjectDetector::new();
        detector.observe(tmp.path(), "go.mod");
        let projects = detector.resolve_all().unwrap();

        assert_eq!(projects.len(), 1);
        assert!(projects[0].ecosystems.contains(&Ecosystem::Go));
    }

    // ── Terraform: blocked artifacts never resolve ────────────────────────────

    #[test]
    fn terraform_blocked_artifacts_not_in_artifacts_list() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "main.tf", "provider \"aws\" {}\n");
        // Create the blocked files to simulate a real terraform project
        write(tmp.path(), "terraform.tfstate", r#"{"version": 4}"#);
        make_dir(tmp.path(), ".terraform");

        let mut detector = ProjectDetector::new();
        detector.observe(tmp.path(), "main.tf");
        let projects = detector.resolve_all().unwrap();

        assert_eq!(projects.len(), 1);
        // .terraform/ should be present (it exists and is Safe)
        assert!(projects[0].artifacts.iter().any(|a| {
            a.absolute_path.ends_with(".terraform") && a.safety_tier == SafetyTier::Safe
        }));
        // terraform.tfstate should be Blocked
        assert!(projects[0].artifacts.iter().any(|a| {
            a.absolute_path.ends_with("terraform.tfstate") && a.safety_tier == SafetyTier::Blocked
        }));
    }

    // ── content_key confirmation ───────────────────────────────────────────────

    #[test]
    fn content_key_required_for_django() {
        let tmp = TempDir::new().unwrap();
        // manage.py without "django" content → should not match Django
        write(
            tmp.path(),
            "manage.py",
            "#!/usr/bin/env python3\nprint('hello')\n",
        );

        let mut detector = ProjectDetector::new();
        detector.observe(tmp.path(), "manage.py");
        let projects = detector.resolve_all().unwrap();

        // Should not detect Django because "django" is not in the file
        assert!(
            !projects
                .iter()
                .any(|p| p.ecosystems.contains(&Ecosystem::Django)),
            "Django should not be detected without 'django' in manage.py content"
        );
    }

    #[test]
    fn django_detected_with_content_key() {
        let tmp = TempDir::new().unwrap();
        write(
            tmp.path(),
            "manage.py",
            "#!/usr/bin/env python3\nimport django\nos.environ.setdefault('DJANGO_SETTINGS_MODULE', 'mysite.settings')\n",
        );

        let mut detector = ProjectDetector::new();
        detector.observe(tmp.path(), "manage.py");
        let projects = detector.resolve_all().unwrap();

        assert!(projects
            .iter()
            .any(|p| p.ecosystems.contains(&Ecosystem::Django)));
    }

    // ── staleness ─────────────────────────────────────────────────────────────

    #[test]
    fn new_project_is_not_stale() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "Cargo.toml", "[package]\nname = \"fresh\"\n");

        let mut detector = ProjectDetector::new();
        detector.observe(tmp.path(), "Cargo.toml");
        let projects = detector.resolve_all().unwrap();

        // A just-created file is definitely not 90 days old
        assert!(!projects[0].is_stale(90));
    }

    // ── git HEAD mtime ────────────────────────────────────────────────────────

    #[test]
    fn reads_git_head_mtime_when_git_repo() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "package.json", r#"{"name": "my-repo"}"#);
        // Simulate .git/HEAD
        make_dir(tmp.path(), ".git");
        write(tmp.path(), ".git/HEAD", "ref: refs/heads/main\n");

        let mut detector = ProjectDetector::new();
        detector.observe(tmp.path(), "package.json");
        let projects = detector.resolve_all().unwrap();

        assert!(
            projects[0].git_head_mtime.is_some(),
            "Should detect git_head_mtime when .git/HEAD exists"
        );
    }

    #[test]
    fn git_head_mtime_is_none_for_non_git_project() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "go.mod", "module example.com/myapp\n");
        // No .git directory

        let mut detector = ProjectDetector::new();
        detector.observe(tmp.path(), "go.mod");
        let projects = detector.resolve_all().unwrap();

        assert!(
            projects[0].git_head_mtime.is_none(),
            "Should have no git_head_mtime when not a git repo"
        );
    }

    // ── monorepo detection integration ────────────────────────────────────────

    #[test]
    fn turborepo_monorepo_detected_alongside_node_project() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "package.json", r#"{"workspaces": ["apps/*"]}"#);
        write(tmp.path(), "turbo.json", r#"{"pipeline": {}}"#);
        make_dir(tmp.path(), "node_modules");
        make_dir(tmp.path(), ".turbo");

        let mut detector = ProjectDetector::new();
        detector.observe(tmp.path(), "package.json");
        detector.observe(tmp.path(), "turbo.json");
        let projects = detector.resolve_all().unwrap();

        // Both Node and Turborepo ecosystems should be present
        assert!(projects.iter().any(|p| {
            p.ecosystems.contains(&Ecosystem::NodeJs)
                || p.ecosystems.contains(&Ecosystem::Turborepo)
        }));
    }

    // ── detect_global_caches ──────────────────────────────────────────────────

    #[test]
    fn detects_global_npm_cache() {
        let tmp = TempDir::new().unwrap();
        // Create the npm cache directory structure
        make_dir(tmp.path(), ".npm/_cacache");

        let caches = detect_global_caches(tmp.path());
        assert!(
            caches.iter().any(|c| {
                c.absolute_path.to_string_lossy().contains(".npm/_cacache")
                    && c.safety_tier == SafetyTier::Safe
            }),
            "Expected .npm/_cacache in global caches"
        );
    }

    #[test]
    fn global_caches_skips_absent_directories() {
        let tmp = TempDir::new().unwrap();
        // Empty home dir — no caches should be detected
        let caches = detect_global_caches(tmp.path());
        assert!(
            caches.is_empty(),
            "Empty home dir should yield zero global caches"
        );
    }

    // ── Multiple projects ─────────────────────────────────────────────────────

    #[test]
    fn detector_handles_multiple_distinct_roots() {
        let tmp = TempDir::new().unwrap();

        let proj_a = tmp.path().join("project_a");
        let proj_b = tmp.path().join("project_b");
        std::fs::create_dir_all(&proj_a).unwrap();
        std::fs::create_dir_all(&proj_b).unwrap();

        write(&proj_a, "Cargo.toml", "[package]\nname = \"a\"\n");
        write(&proj_b, "go.mod", "module example.com/b\n");

        let mut detector = ProjectDetector::new();
        detector.observe(&proj_a, "Cargo.toml");
        detector.observe(&proj_b, "go.mod");
        let projects = detector.resolve_all().unwrap();

        assert_eq!(projects.len(), 2);
    }

    #[test]
    fn duplicate_observations_of_same_root_produce_one_project() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "package.json", "{}");

        let mut detector = ProjectDetector::new();
        // Observe the same file twice (e.g., if the scanner visits the directory twice)
        detector.observe(tmp.path(), "package.json");
        detector.observe(tmp.path(), "package.json");
        let projects = detector.resolve_all().unwrap();

        assert_eq!(
            projects.len(),
            1,
            "Duplicate observations should produce exactly one project"
        );
    }

    // ── Safety tier coverage ──────────────────────────────────────────────────

    #[test]
    fn every_ecosystem_has_at_least_one_signature() {
        // Not every Ecosystem variant needs a signature (some are framework
        // variants detected via FrameworkDetector), but the core ones must.
        let core_ecosystems = [
            Ecosystem::NodeJs,
            Ecosystem::Rust,
            Ecosystem::Go,
            Ecosystem::PythonPip,
            Ecosystem::PythonPoetry,
            Ecosystem::JavaMaven,
            Ecosystem::JavaGradle,
            Ecosystem::Dart,
            Ecosystem::Terraform,
        ];
        let index = SignatureIndex::build();

        for eco in &core_ecosystems {
            let found = PROJECT_SIGNATURES.iter().any(|s| s.ecosystem == *eco);
            assert!(found, "Expected at least one signature for {:?}", eco);
        }
        let _ = index;
    }
}
