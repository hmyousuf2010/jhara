// detector/monorepo.rs
//
// Detects monorepo structures and resolves their shared artifact directories.
//
// Why monorepos need special handling:
//   A PNPM workspace with 50 packages has one root node_modules/ and hard
//   links into each package's node_modules/. A naive scanner double-counts
//   every shared inode. The InodeTracker in the scanner layer handles the
//   counting correctly, but the detector still needs to know it is looking
//   at a monorepo so it can attribute artifacts correctly in the UI (e.g.,
//   "shared root node_modules" vs "MyApp's local node_modules").
//
//   Cargo workspaces unify ALL compilation into a single target/ at the
//   workspace root. There are no per-crate target/ directories. If we find
//   a Cargo.toml with [workspace] at the root, we should not look for target/
//   in sub-crates — the root one is the only one that matters.
//
//   Turborepo and Nx both maintain local computation caches that can grow
//   to several gigabytes and are entirely safe to delete. These caches are
//   not covered by the standard Node.js signature because they only appear
//   in monorepo setups.

use std::fs;
use std::path::{Path, PathBuf};

use crate::detector::types::{Ecosystem, MonorepoKind, MonorepoMembership, SafetyTier};
use crate::error::JharaError;

// ─────────────────────────────────────────────────────────────────────────────
// MonorepoResolver
// ─────────────────────────────────────────────────────────────────────────────

/// Inspects a directory to determine whether it is the root of a supported
/// monorepo structure, and returns additional artifact paths that belong to
/// the monorepo layer above any individual project.
pub struct MonorepoResolver;

impl MonorepoResolver {
    /// Checks whether `dir` is a monorepo root and returns a descriptor.
    ///
    /// Returns `None` if no monorepo structure is detected.
    /// Returns `Some(MonorepoInfo)` with the kind and shared artifact paths.
    pub fn resolve(dir: &Path) -> Result<Option<MonorepoInfo>, JharaError> {
        // Check in priority order: more specific tools first.

        // Turborepo: turbo.json at root
        if dir.join("turbo.json").is_file() {
            return Ok(Some(Self::turborepo_info(dir)));
        }

        // Nx: nx.json at root
        if dir.join("nx.json").is_file() {
            return Ok(Some(Self::nx_info(dir)));
        }

        // Lerna (often used alongside npm/yarn workspaces)
        if dir.join("lerna.json").is_file() {
            return Ok(Some(Self::lerna_info(dir)));
        }

        // PNPM workspace
        if dir.join("pnpm-workspace.yaml").is_file() {
            return Ok(Some(Self::pnpm_workspace_info(dir)));
        }

        // npm / Yarn workspace — detected via "workspaces" key in package.json
        if let Some(info) = Self::check_npm_yarn_workspace(dir)? {
            return Ok(Some(info));
        }

        // Cargo workspace — [workspace] section in root Cargo.toml
        if let Some(info) = Self::check_cargo_workspace(dir)? {
            return Ok(Some(info));
        }

        // Melos (Dart/Flutter monorepo)
        if dir.join("melos.yaml").is_file() {
            return Ok(Some(Self::melos_info(dir)));
        }

        Ok(None)
    }

    // ── Turborepo ──────────────────────────────────────────────────────────

    fn turborepo_info(root: &Path) -> MonorepoInfo {
        MonorepoInfo {
            kind: MonorepoKind::Turborepo,
            root: root.to_path_buf(),
            shared_artifacts: vec![
                OwnedArtifact {
                    relative_path: ".turbo".to_string(),
                    safety_tier: SafetyTier::Safe,
                    ecosystem: Ecosystem::Turborepo,
                    recovery_command: Some("turbo build (auto-regenerated)".to_string()),
                },
                OwnedArtifact {
                    relative_path: "node_modules/.cache/turbo".to_string(),
                    safety_tier: SafetyTier::Safe,
                    ecosystem: Ecosystem::Turborepo,
                    recovery_command: Some("turbo build (auto-regenerated)".to_string()),
                },
                // Root node_modules is shared across all packages
                OwnedArtifact {
                    relative_path: "node_modules".to_string(),
                    safety_tier: SafetyTier::Safe,
                    ecosystem: Ecosystem::NodeJs,
                    recovery_command: Some("npm install / pnpm install / yarn".to_string()),
                },
            ],
        }
    }

    // ── Nx ─────────────────────────────────────────────────────────────────

    fn nx_info(root: &Path) -> MonorepoInfo {
        MonorepoInfo {
            kind: MonorepoKind::Nx,
            root: root.to_path_buf(),
            shared_artifacts: vec![
                OwnedArtifact {
                    relative_path: ".nx/cache".to_string(),
                    safety_tier: SafetyTier::Safe,
                    ecosystem: Ecosystem::Nx,
                    recovery_command: Some("nx build (auto-regenerated)".to_string()),
                },
                OwnedArtifact {
                    relative_path: "node_modules/.cache/nx".to_string(),
                    safety_tier: SafetyTier::Safe,
                    ecosystem: Ecosystem::Nx,
                    recovery_command: Some("nx build (auto-regenerated)".to_string()),
                },
                OwnedArtifact {
                    relative_path: "node_modules".to_string(),
                    safety_tier: SafetyTier::Safe,
                    ecosystem: Ecosystem::NodeJs,
                    recovery_command: Some("npm install / pnpm install".to_string()),
                },
            ],
        }
    }

    // ── Lerna ──────────────────────────────────────────────────────────────

    fn lerna_info(root: &Path) -> MonorepoInfo {
        MonorepoInfo {
            kind: MonorepoKind::Lerna,
            root: root.to_path_buf(),
            shared_artifacts: vec![
                // Lerna historically maintains per-package node_modules
                // (they are listed by the scanner under each package directory).
                // The root node_modules may be hoisted or not depending on config.
                OwnedArtifact {
                    relative_path: "node_modules".to_string(),
                    safety_tier: SafetyTier::Safe,
                    ecosystem: Ecosystem::NodeJs,
                    recovery_command: Some("lerna bootstrap".to_string()),
                },
            ],
        }
    }

    // ── PNPM workspace ─────────────────────────────────────────────────────

    fn pnpm_workspace_info(root: &Path) -> MonorepoInfo {
        MonorepoInfo {
            kind: MonorepoKind::PnpmWorkspace,
            root: root.to_path_buf(),
            shared_artifacts: vec![
                // PNPM hoists to root node_modules/ with hard links into
                // package-level node_modules/. The InodeTracker handles
                // the deduplication; we just list the root here.
                OwnedArtifact {
                    relative_path: "node_modules".to_string(),
                    safety_tier: SafetyTier::Safe,
                    ecosystem: Ecosystem::PnpmWorkspace,
                    recovery_command: Some("pnpm install".to_string()),
                },
                OwnedArtifact {
                    relative_path: "node_modules/.cache".to_string(),
                    safety_tier: SafetyTier::Safe,
                    ecosystem: Ecosystem::PnpmWorkspace,
                    recovery_command: Some("pnpm install (auto-regenerated)".to_string()),
                },
            ],
        }
    }

    // ── npm / Yarn workspace ────────────────────────────────────────────────

    fn check_npm_yarn_workspace(dir: &Path) -> Result<Option<MonorepoInfo>, JharaError> {
        let pkg_path = dir.join("package.json");
        if !pkg_path.is_file() {
            return Ok(None);
        }

        let content =
            fs::read(&pkg_path).map_err(|e| JharaError::io(pkg_path.to_string_lossy(), e))?;

        // We only need to know if "workspaces" key exists in the JSON.
        // Parse just enough to check this without allocating the full value tree.
        let json: serde_json::Value = serde_json::from_slice(&content)
            .map_err(|e| JharaError::json(pkg_path.to_string_lossy(), e))?;

        if json.get("workspaces").is_none() {
            return Ok(None);
        }

        // Distinguish npm vs Yarn by presence of yarn.lock
        let kind = if dir.join("yarn.lock").is_file() {
            MonorepoKind::YarnWorkspace
        } else {
            MonorepoKind::NpmWorkspace
        };

        Ok(Some(MonorepoInfo {
            kind,
            root: dir.to_path_buf(),
            shared_artifacts: vec![OwnedArtifact {
                relative_path: "node_modules".to_string(),
                safety_tier: SafetyTier::Safe,
                ecosystem: Ecosystem::NodeJs,
                recovery_command: Some(match kind {
                    MonorepoKind::YarnWorkspace => "yarn install".to_string(),
                    _ => "npm install".to_string(),
                }),
            }],
        }))
    }

    // ── Cargo workspace ─────────────────────────────────────────────────────

    fn check_cargo_workspace(dir: &Path) -> Result<Option<MonorepoInfo>, JharaError> {
        let cargo_path = dir.join("Cargo.toml");
        if !cargo_path.is_file() {
            return Ok(None);
        }

        let content = fs::read_to_string(&cargo_path)
            .map_err(|e| JharaError::io(cargo_path.to_string_lossy(), e))?;

        // Rather than pulling in a full TOML parser, we do a simple string
        // search for the workspace section header. This is robust enough:
        // a Cargo.toml with `[workspace]` on a line by itself IS a workspace
        // manifest. The alternative (toml crate) adds a heavy dependency for
        // a check that a 10-char string search handles correctly.
        if !content.lines().any(|line| line.trim() == "[workspace]") {
            return Ok(None);
        }

        Ok(Some(MonorepoInfo {
            kind: MonorepoKind::CargoWorkspace,
            root: dir.to_path_buf(),
            shared_artifacts: vec![
                // Cargo workspaces compile ALL crates into a single target/
                // at the workspace root. There are no per-crate target/ dirs.
                OwnedArtifact {
                    relative_path: "target".to_string(),
                    safety_tier: SafetyTier::Safe,
                    ecosystem: Ecosystem::CargoWorkspace,
                    recovery_command: Some("cargo build".to_string()),
                },
            ],
        }))
    }

    // ── Melos (Dart/Flutter) ────────────────────────────────────────────────

    fn melos_info(root: &Path) -> MonorepoInfo {
        MonorepoInfo {
            kind: MonorepoKind::Melos,
            root: root.to_path_buf(),
            shared_artifacts: vec![OwnedArtifact {
                relative_path: ".dart_tool".to_string(),
                safety_tier: SafetyTier::Safe,
                ecosystem: Ecosystem::Dart,
                recovery_command: Some("melos bootstrap".to_string()),
            }],
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// MonorepoInfo
// ─────────────────────────────────────────────────────────────────────────────

/// The result of a successful monorepo detection.
#[derive(Debug, Clone)]
pub struct MonorepoInfo {
    pub kind: MonorepoKind,
    /// The absolute path of the monorepo root.
    pub root: PathBuf,
    /// Artifact directories owned by the monorepo layer (not by any single
    /// package within it). These are resolved relative to `root`.
    pub shared_artifacts: Vec<OwnedArtifact>,
}

impl MonorepoInfo {
    pub fn membership_for(&self, package_root: &Path) -> Option<MonorepoMembership> {
        // A package is a member of this monorepo if its path starts with the
        // monorepo root and it is not the root itself.
        if package_root == self.root {
            return None;
        }
        if package_root.starts_with(&self.root) {
            Some(MonorepoMembership {
                kind: self.kind,
                root: self.root.clone(),
            })
        } else {
            None
        }
    }
}

/// Owned version of ArtifactPath for runtime-constructed artifact lists.
#[derive(Debug, Clone)]
pub struct OwnedArtifact {
    pub relative_path: String,
    pub safety_tier: SafetyTier,
    pub ecosystem: Ecosystem,
    pub recovery_command: Option<String>,
}

impl OwnedArtifact {
    pub fn absolute_path(&self, root: &Path) -> PathBuf {
        root.join(&self.relative_path)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_file(dir: &Path, name: &str, content: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "{}", content).unwrap();
    }

    // ── Turborepo ────────────────────────────────────────────────────────────

    #[test]
    fn detects_turborepo() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "turbo.json", r#"{"pipeline": {}}"#);

        let result = MonorepoResolver::resolve(tmp.path()).unwrap();
        assert!(result.is_some());
        let info = result.unwrap();
        assert_eq!(info.kind, MonorepoKind::Turborepo);
        assert!(
            info.shared_artifacts
                .iter()
                .any(|a| a.relative_path == ".turbo"),
            "Expected .turbo artifact"
        );
        assert!(
            info.shared_artifacts
                .iter()
                .any(|a| a.relative_path == "node_modules/.cache/turbo"),
            "Expected turbo node_modules cache artifact"
        );
    }

    // ── Nx ───────────────────────────────────────────────────────────────────

    #[test]
    fn detects_nx() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "nx.json", r#"{"version": 3}"#);

        let result = MonorepoResolver::resolve(tmp.path()).unwrap();
        assert!(result.is_some());
        let info = result.unwrap();
        assert_eq!(info.kind, MonorepoKind::Nx);
        assert!(info
            .shared_artifacts
            .iter()
            .any(|a| a.relative_path == ".nx/cache"));
    }

    // ── PNPM workspace ───────────────────────────────────────────────────────

    #[test]
    fn detects_pnpm_workspace() {
        let tmp = TempDir::new().unwrap();
        write_file(
            tmp.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - 'apps/*'\n",
        );

        let result = MonorepoResolver::resolve(tmp.path()).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().kind, MonorepoKind::PnpmWorkspace);
    }

    // ── npm workspace ────────────────────────────────────────────────────────

    #[test]
    fn detects_npm_workspace() {
        let tmp = TempDir::new().unwrap();
        write_file(
            tmp.path(),
            "package.json",
            r#"{"name": "root", "workspaces": ["packages/*"]}"#,
        );

        let result = MonorepoResolver::resolve(tmp.path()).unwrap();
        assert!(result.is_some());
        let info = result.unwrap();
        // No yarn.lock → npm workspace
        assert_eq!(info.kind, MonorepoKind::NpmWorkspace);
    }

    // ── Yarn workspace ───────────────────────────────────────────────────────

    #[test]
    fn detects_yarn_workspace() {
        let tmp = TempDir::new().unwrap();
        write_file(
            tmp.path(),
            "package.json",
            r#"{"workspaces": ["packages/*"]}"#,
        );
        write_file(tmp.path(), "yarn.lock", "");

        let result = MonorepoResolver::resolve(tmp.path()).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().kind, MonorepoKind::YarnWorkspace);
    }

    // ── Cargo workspace ──────────────────────────────────────────────────────

    #[test]
    fn detects_cargo_workspace() {
        let tmp = TempDir::new().unwrap();
        write_file(
            tmp.path(),
            "Cargo.toml",
            "[workspace]\nmembers = [\"crates/*\"]\n",
        );

        let result = MonorepoResolver::resolve(tmp.path()).unwrap();
        assert!(result.is_some());
        let info = result.unwrap();
        assert_eq!(info.kind, MonorepoKind::CargoWorkspace);
        assert!(
            info.shared_artifacts
                .iter()
                .any(|a| a.relative_path == "target"),
            "Expected single shared target/ for Cargo workspace"
        );
    }

    #[test]
    fn plain_cargo_toml_is_not_workspace() {
        let tmp = TempDir::new().unwrap();
        write_file(
            tmp.path(),
            "Cargo.toml",
            "[package]\nname = \"myapp\"\nversion = \"0.1.0\"\n",
        );

        let result = MonorepoResolver::resolve(tmp.path()).unwrap();
        assert!(
            result.is_none(),
            "A plain Cargo.toml without [workspace] should not match"
        );
    }

    // ── Lerna ────────────────────────────────────────────────────────────────

    #[test]
    fn detects_lerna() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "lerna.json", r#"{"version": "1.0.0"}"#);

        let result = MonorepoResolver::resolve(tmp.path()).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().kind, MonorepoKind::Lerna);
    }

    // ── Melos ────────────────────────────────────────────────────────────────

    #[test]
    fn detects_melos() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "melos.yaml", "name: my_workspace\n");

        let result = MonorepoResolver::resolve(tmp.path()).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().kind, MonorepoKind::Melos);
    }

    // ── Priority (Turborepo wins over npm workspace) ──────────────────────────

    #[test]
    fn turborepo_takes_priority_over_npm_workspace() {
        let tmp = TempDir::new().unwrap();
        // Both present — Turborepo check runs first
        write_file(tmp.path(), "turbo.json", "{}");
        write_file(tmp.path(), "package.json", r#"{"workspaces": ["apps/*"]}"#);

        let result = MonorepoResolver::resolve(tmp.path()).unwrap();
        assert_eq!(result.unwrap().kind, MonorepoKind::Turborepo);
    }

    // ── No monorepo ───────────────────────────────────────────────────────────

    #[test]
    fn returns_none_for_plain_project() {
        let tmp = TempDir::new().unwrap();
        write_file(
            tmp.path(),
            "package.json",
            r#"{"name": "plain-app", "version": "1.0.0"}"#,
        );
        write_file(tmp.path(), "index.js", "console.log('hello')");

        let result = MonorepoResolver::resolve(tmp.path()).unwrap();
        assert!(
            result.is_none(),
            "Plain project without workspace config should not match"
        );
    }

    // ── membership_for ───────────────────────────────────────────────────────

    #[test]
    fn membership_identifies_sub_package() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "turbo.json", "{}");
        let info = MonorepoResolver::resolve(tmp.path()).unwrap().unwrap();

        let sub_pkg = tmp.path().join("packages").join("ui");
        let membership = info.membership_for(&sub_pkg);
        assert!(
            membership.is_some(),
            "Sub-package should be recognized as a member"
        );
        assert_eq!(membership.unwrap().kind, MonorepoKind::Turborepo);
    }

    #[test]
    fn membership_returns_none_for_root_itself() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "turbo.json", "{}");
        let info = MonorepoResolver::resolve(tmp.path()).unwrap().unwrap();

        // The root itself is not a "member" — it is the monorepo root.
        let membership = info.membership_for(tmp.path());
        assert!(membership.is_none());
    }

    #[test]
    fn membership_returns_none_for_unrelated_path() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "turbo.json", "{}");
        let info = MonorepoResolver::resolve(tmp.path()).unwrap().unwrap();

        let unrelated = Path::new("/tmp/some_other_project");
        let membership = info.membership_for(unrelated);
        assert!(membership.is_none());
    }
}
