// detector/frameworks.rs
//
// Identifies JS framework-specific artifact directories by reading the
// `dependencies` and `devDependencies` fields of package.json.
//
// Why this is a separate pass from signatures.rs:
//   Every package.json triggers Node.js detection with node_modules/ as the
//   primary artifact. But Next.js also produces .next/, SvelteKit produces
//   .svelte-kit/, Astro produces .astro/, and so on. We cannot list all of
//   these in the base Node.js signature because then we would flag absent
//   directories on projects that do not use those frameworks.
//
//   This module reads only the two dependency maps (not the full file, not
//   the lock file, not node_modules/). Total cost: one JSON parse per
//   package.json found by the scanner. We do NOT resolve the npm dependency
//   graph; we simply ask "does this file declare X as a dependency?"
//
// Design: FrameworkRule is a flat struct. Rules are evaluated in order and
// all matching rules contribute to the result (a single project can match
// Next.js AND Turborepo simultaneously, for example). The caller merges the
// returned artifact lists into the DetectedProject.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

use crate::detector::types::{ArtifactPath, Ecosystem, SafetyTier};

// ─────────────────────────────────────────────────────────────────────────────
// Public output type
// ─────────────────────────────────────────────────────────────────────────────

/// An additional artifact directory implied by a JS framework dependency.
#[derive(Debug, Clone)]
pub struct FrameworkArtifact {
    /// Path relative to the project root.
    pub relative_path: &'static str,
    pub safety_tier: SafetyTier,
    pub ecosystem: Ecosystem,
    pub recovery_command: &'static str,
}

impl FrameworkArtifact {
    pub fn to_artifact_path(&self) -> ArtifactPath {
        ArtifactPath::new(self.relative_path, self.safety_tier).recovery(self.recovery_command)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Rule definition
// ─────────────────────────────────────────────────────────────────────────────

struct FrameworkRule {
    /// Package names to look for in `dependencies` or `devDependencies`.
    /// The rule matches if ANY of these keys is present.
    package_keys: &'static [&'static str],
    artifacts: &'static [FrameworkArtifact],
}

impl FrameworkRule {
    fn matches(&self, deps: &HashMap<String, serde_json::Value>) -> bool {
        self.package_keys.iter().any(|key| deps.contains_key(*key))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Artifact slices — &'static so FrameworkRule can reference them
// ─────────────────────────────────────────────────────────────────────────────

static NEXT_ARTIFACTS: &[FrameworkArtifact] = &[
    FrameworkArtifact {
        relative_path: ".next",
        safety_tier: SafetyTier::Safe,
        ecosystem: Ecosystem::Next,
        recovery_command: "next build",
    },
    FrameworkArtifact {
        relative_path: ".next/cache",
        safety_tier: SafetyTier::Safe,
        ecosystem: Ecosystem::Next,
        recovery_command: "next build",
    },
];

static NUXT_ARTIFACTS: &[FrameworkArtifact] = &[
    FrameworkArtifact {
        relative_path: ".nuxt",
        safety_tier: SafetyTier::Safe,
        ecosystem: Ecosystem::Nuxt,
        recovery_command: "nuxt build",
    },
    FrameworkArtifact {
        relative_path: ".output",
        safety_tier: SafetyTier::Safe,
        ecosystem: Ecosystem::Nuxt,
        recovery_command: "nuxt build",
    },
];

static SVELTEKIT_ARTIFACTS: &[FrameworkArtifact] = &[FrameworkArtifact {
    relative_path: ".svelte-kit",
    safety_tier: SafetyTier::Safe,
    ecosystem: Ecosystem::SvelteKit,
    recovery_command: "svelte-kit build",
}];

static ANGULAR_ARTIFACTS: &[FrameworkArtifact] = &[
    FrameworkArtifact {
        relative_path: "dist",
        safety_tier: SafetyTier::Safe,
        ecosystem: Ecosystem::Angular,
        recovery_command: "ng build",
    },
    FrameworkArtifact {
        relative_path: ".angular/cache",
        safety_tier: SafetyTier::Safe,
        ecosystem: Ecosystem::Angular,
        recovery_command: "ng build",
    },
];

static GATSBY_ARTIFACTS: &[FrameworkArtifact] = &[
    FrameworkArtifact {
        relative_path: ".cache",
        safety_tier: SafetyTier::Safe,
        ecosystem: Ecosystem::Gatsby,
        recovery_command: "gatsby build",
    },
    FrameworkArtifact {
        relative_path: "public",
        safety_tier: SafetyTier::Safe,
        ecosystem: Ecosystem::Gatsby,
        recovery_command: "gatsby build",
    },
];

static REMIX_ARTIFACTS: &[FrameworkArtifact] = &[
    FrameworkArtifact {
        relative_path: "build",
        safety_tier: SafetyTier::Safe,
        ecosystem: Ecosystem::Remix,
        recovery_command: "remix build",
    },
    FrameworkArtifact {
        relative_path: "public/build",
        safety_tier: SafetyTier::Safe,
        ecosystem: Ecosystem::Remix,
        recovery_command: "remix build",
    },
];

static ASTRO_ARTIFACTS: &[FrameworkArtifact] = &[
    FrameworkArtifact {
        relative_path: ".astro",
        safety_tier: SafetyTier::Safe,
        ecosystem: Ecosystem::Astro,
        recovery_command: "astro build",
    },
    FrameworkArtifact {
        relative_path: "dist",
        safety_tier: SafetyTier::Safe,
        ecosystem: Ecosystem::Astro,
        recovery_command: "astro build",
    },
];

static TANSTACK_ARTIFACTS: &[FrameworkArtifact] = &[
    FrameworkArtifact {
        relative_path: ".vinxi",
        safety_tier: SafetyTier::Safe,
        ecosystem: Ecosystem::TanstackStart,
        recovery_command: "tss build",
    },
    FrameworkArtifact {
        relative_path: "dist",
        safety_tier: SafetyTier::Safe,
        ecosystem: Ecosystem::TanstackStart,
        recovery_command: "tss build",
    },
];

static SOLID_ARTIFACTS: &[FrameworkArtifact] = &[FrameworkArtifact {
    relative_path: "dist",
    safety_tier: SafetyTier::Safe,
    ecosystem: Ecosystem::Solid,
    recovery_command: "vite build",
}];

static QWIK_ARTIFACTS: &[FrameworkArtifact] = &[
    FrameworkArtifact {
        relative_path: "dist",
        safety_tier: SafetyTier::Safe,
        ecosystem: Ecosystem::Qwik,
        recovery_command: "qwik build",
    },
    FrameworkArtifact {
        relative_path: "tmp",
        safety_tier: SafetyTier::Safe,
        ecosystem: Ecosystem::Qwik,
        recovery_command: "qwik build",
    },
];

static STENCIL_ARTIFACTS: &[FrameworkArtifact] = &[
    FrameworkArtifact {
        relative_path: "dist",
        safety_tier: SafetyTier::Safe,
        ecosystem: Ecosystem::Stencil,
        recovery_command: "stencil build",
    },
    FrameworkArtifact {
        relative_path: "www",
        safety_tier: SafetyTier::Safe,
        ecosystem: Ecosystem::Stencil,
        recovery_command: "stencil build",
    },
    FrameworkArtifact {
        relative_path: ".stencil",
        safety_tier: SafetyTier::Safe,
        ecosystem: Ecosystem::Stencil,
        recovery_command: "stencil build",
    },
];

static EMBER_ARTIFACTS: &[FrameworkArtifact] = &[
    FrameworkArtifact {
        relative_path: "dist",
        safety_tier: SafetyTier::Safe,
        ecosystem: Ecosystem::Ember,
        recovery_command: "ember build",
    },
    FrameworkArtifact {
        relative_path: "tmp",
        safety_tier: SafetyTier::Safe,
        ecosystem: Ecosystem::Ember,
        recovery_command: "ember build",
    },
];

// Vite is a bundler used by many frameworks. When detected without a more
// specific framework, we include the standard dist/ output.
static VITE_ARTIFACTS: &[FrameworkArtifact] = &[FrameworkArtifact {
    relative_path: "dist",
    safety_tier: SafetyTier::Safe,
    ecosystem: Ecosystem::Vite,
    recovery_command: "vite build",
}];

// Webpack (used by many older projects and some CRA setups)
static WEBPACK_CACHE_ARTIFACTS: &[FrameworkArtifact] = &[FrameworkArtifact {
    relative_path: "node_modules/.cache/webpack",
    safety_tier: SafetyTier::Safe,
    ecosystem: Ecosystem::NodeJs,
    recovery_command: "webpack build (auto-regenerated)",
}];

// Turbopack (Next.js 13+ uses this internally)
static TURBOPACK_CACHE_ARTIFACTS: &[FrameworkArtifact] = &[FrameworkArtifact {
    relative_path: "node_modules/.cache/turbopack",
    safety_tier: SafetyTier::Safe,
    ecosystem: Ecosystem::Next,
    recovery_command: "next build (auto-regenerated)",
}];

// Parcel
static PARCEL_ARTIFACTS: &[FrameworkArtifact] = &[
    FrameworkArtifact {
        relative_path: ".parcel-cache",
        safety_tier: SafetyTier::Safe,
        ecosystem: Ecosystem::Vite,
        recovery_command: "parcel build (auto-regenerated)",
    },
    FrameworkArtifact {
        relative_path: "dist",
        safety_tier: SafetyTier::Safe,
        ecosystem: Ecosystem::Vite,
        recovery_command: "parcel build",
    },
];

// NestJS (Node.js backend framework — produces dist/ like a frontend bundler)
static NESTJS_ARTIFACTS: &[FrameworkArtifact] = &[FrameworkArtifact {
    relative_path: "dist",
    safety_tier: SafetyTier::Safe,
    ecosystem: Ecosystem::NestJs,
    recovery_command: "nest build",
}];

// Turborepo (monorepo tool — adds .turbo/ cache at the workspace level)
// Detected via turbo.json in signatures.rs; these are the package-level caches
static TURBO_PKG_ARTIFACTS: &[FrameworkArtifact] = &[FrameworkArtifact {
    relative_path: ".turbo",
    safety_tier: SafetyTier::Safe,
    ecosystem: Ecosystem::Turborepo,
    recovery_command: "turbo build (auto-regenerated)",
}];

// ─────────────────────────────────────────────────────────────────────────────
// Rule table
// ─────────────────────────────────────────────────────────────────────────────

static FRAMEWORK_RULES: &[FrameworkRule] = &[
    FrameworkRule {
        package_keys: &["next"],
        artifacts: NEXT_ARTIFACTS,
    },
    FrameworkRule {
        package_keys: &["nuxt", "nuxt3"],
        artifacts: NUXT_ARTIFACTS,
    },
    FrameworkRule {
        package_keys: &["@sveltejs/kit"],
        artifacts: SVELTEKIT_ARTIFACTS,
    },
    FrameworkRule {
        package_keys: &["@angular/core"],
        artifacts: ANGULAR_ARTIFACTS,
    },
    FrameworkRule {
        package_keys: &["gatsby"],
        artifacts: GATSBY_ARTIFACTS,
    },
    FrameworkRule {
        package_keys: &["@remix-run/react", "@remix-run/node", "@remix-run/serve"],
        artifacts: REMIX_ARTIFACTS,
    },
    FrameworkRule {
        package_keys: &["astro"],
        artifacts: ASTRO_ARTIFACTS,
    },
    FrameworkRule {
        package_keys: &["@tanstack/start"],
        artifacts: TANSTACK_ARTIFACTS,
    },
    FrameworkRule {
        package_keys: &["solid-js"],
        artifacts: SOLID_ARTIFACTS,
    },
    FrameworkRule {
        package_keys: &["@builder.io/qwik"],
        artifacts: QWIK_ARTIFACTS,
    },
    FrameworkRule {
        package_keys: &["@stencil/core"],
        artifacts: STENCIL_ARTIFACTS,
    },
    FrameworkRule {
        package_keys: &["ember-cli"],
        artifacts: EMBER_ARTIFACTS,
    },
    FrameworkRule {
        package_keys: &["vite"],
        artifacts: VITE_ARTIFACTS,
    },
    FrameworkRule {
        package_keys: &["webpack", "webpack-cli"],
        artifacts: WEBPACK_CACHE_ARTIFACTS,
    },
    FrameworkRule {
        package_keys: &["parcel"],
        artifacts: PARCEL_ARTIFACTS,
    },
    FrameworkRule {
        package_keys: &["@nestjs/core"],
        artifacts: NESTJS_ARTIFACTS,
    },
    FrameworkRule {
        package_keys: &["turbo"],
        artifacts: TURBO_PKG_ARTIFACTS,
    },
    FrameworkRule {
        package_keys: &["next", "@next/bundle-analyzer"],
        artifacts: TURBOPACK_CACHE_ARTIFACTS,
    },
];

// ─────────────────────────────────────────────────────────────────────────────
// Minimal package.json deserializer
// ─────────────────────────────────────────────────────────────────────────────

/// We only need `dependencies` and `devDependencies`. The rest of the file
/// is ignored. Using a dedicated struct rather than `serde_json::Value` for
/// the top-level parse avoids allocating the full package.json object graph.
#[derive(Deserialize)]
struct PackageJsonDeps {
    #[serde(default)]
    dependencies: HashMap<String, serde_json::Value>,
    #[serde(rename = "devDependencies", default)]
    dev_dependencies: HashMap<String, serde_json::Value>,
    // peerDependencies matters for libraries. Optional but included.
    #[serde(rename = "peerDependencies", default)]
    peer_dependencies: HashMap<String, serde_json::Value>,
}

impl PackageJsonDeps {
    /// Merges all three dependency maps into one for rule evaluation.
    /// Allocation: three HashMaps merged into one. Acceptable — this runs
    /// once per package.json, not in a hot loop.
    fn all_deps(self) -> HashMap<String, serde_json::Value> {
        let mut merged = self.dependencies;
        merged.extend(self.dev_dependencies);
        merged.extend(self.peer_dependencies);
        merged
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/// Reads the `package.json` at `path` and returns the additional artifact
/// paths implied by whatever frameworks its dependencies declare.
///
/// Returns an empty `Vec` if:
///   - The file cannot be read (permissions, not found)
///   - The file is not valid JSON
///   - No known frameworks are declared as dependencies
///
/// Failures are silent rather than surfaced as errors because a missing or
/// malformed `package.json` is not a scanning failure — the file just does
/// not contribute framework-specific artifacts.
pub fn detect_framework_artifacts(package_json_path: &Path) -> Vec<FrameworkArtifact> {
    let data = match std::fs::read(package_json_path) {
        Ok(d) => d,
        Err(_) => return vec![],
    };

    let parsed: PackageJsonDeps = match serde_json::from_slice(&data) {
        Ok(p) => p,
        Err(_) => return vec![],
    };

    let all_deps = parsed.all_deps();

    let mut results: Vec<FrameworkArtifact> = Vec::new();
    for rule in FRAMEWORK_RULES {
        if rule.matches(&all_deps) {
            results.extend_from_slice(rule.artifacts);
        }
    }
    results
}

/// Returns true if the package.json at `path` declares a dependency on
/// `package_name` in any of the three dependency maps.
///
/// Used by the `ProjectDetector` for fast single-key checks without needing
/// the full artifact list (e.g., confirming React Native vs. a plain React app).
pub fn has_dependency(package_json_path: &Path, package_name: &str) -> bool {
    let data = match std::fs::read(package_json_path) {
        Ok(d) => d,
        Err(_) => return false,
    };

    let parsed: PackageJsonDeps = match serde_json::from_slice(&data) {
        Ok(p) => p,
        Err(_) => return false,
    };

    parsed.dependencies.contains_key(package_name)
        || parsed.dev_dependencies.contains_key(package_name)
        || parsed.peer_dependencies.contains_key(package_name)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_package_json(content: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "{}", content).unwrap();
        f
    }

    #[test]
    fn detects_next_js() {
        let f = write_package_json(r#"{"dependencies": {"next": "14.0.0"}}"#);
        let artifacts = detect_framework_artifacts(f.path());
        assert!(
            artifacts.iter().any(|a| a.relative_path == ".next"),
            "Expected .next/ artifact for Next.js dependency"
        );
    }

    #[test]
    fn detects_nuxt_from_devdeps() {
        let f = write_package_json(r#"{"devDependencies": {"nuxt": "3.0.0"}}"#);
        let artifacts = detect_framework_artifacts(f.path());
        assert!(
            artifacts.iter().any(|a| a.relative_path == ".nuxt"),
            "Expected .nuxt/ artifact for Nuxt devDependency"
        );
    }

    #[test]
    fn detects_sveltekit() {
        let f = write_package_json(r#"{"devDependencies": {"@sveltejs/kit": "2.0.0"}}"#);
        let artifacts = detect_framework_artifacts(f.path());
        assert!(
            artifacts.iter().any(|a| a.relative_path == ".svelte-kit"),
            "Expected .svelte-kit/ artifact"
        );
    }

    #[test]
    fn detects_multiple_frameworks_in_one_project() {
        // A project might use Vite as bundler AND Turbo for caching
        let f = write_package_json(
            r#"{"dependencies": {"vite": "5.0.0"}, "devDependencies": {"turbo": "1.0.0"}}"#,
        );
        let artifacts = detect_framework_artifacts(f.path());
        let paths: Vec<_> = artifacts.iter().map(|a| a.relative_path).collect();
        assert!(paths.contains(&"dist"), "Expected dist/ from Vite");
        assert!(paths.contains(&".turbo"), "Expected .turbo/ from Turbo");
    }

    #[test]
    fn returns_empty_for_unknown_deps() {
        let f = write_package_json(r#"{"dependencies": {"some-obscure-lib": "1.0.0"}}"#);
        let artifacts = detect_framework_artifacts(f.path());
        assert!(artifacts.is_empty());
    }

    #[test]
    fn returns_empty_for_empty_package_json() {
        let f = write_package_json(r#"{}"#);
        let artifacts = detect_framework_artifacts(f.path());
        assert!(artifacts.is_empty());
    }

    #[test]
    fn returns_empty_for_invalid_json() {
        let f = write_package_json("this is not json");
        let artifacts = detect_framework_artifacts(f.path());
        assert!(
            artifacts.is_empty(),
            "Should silently return empty for malformed JSON"
        );
    }

    #[test]
    fn returns_empty_for_nonexistent_file() {
        let path = Path::new("/tmp/jhara_test_does_not_exist_package_json_abc123");
        let artifacts = detect_framework_artifacts(path);
        assert!(artifacts.is_empty());
    }

    #[test]
    fn has_dependency_detects_in_peer_dependencies() {
        let f = write_package_json(r#"{"peerDependencies": {"react": "^18.0.0"}}"#);
        assert!(has_dependency(f.path(), "react"));
        assert!(!has_dependency(f.path(), "vue"));
    }

    #[test]
    fn all_framework_artifacts_are_safe_tier() {
        // Every framework-specific artifact should be Safe — they are all
        // build outputs that can be regenerated. If a Caution or Risky
        // artifact ever ends up here it is almost certainly a mistake.
        let f = write_package_json(
            r#"{
            "dependencies": {
                "next": "14.0.0",
                "nuxt": "3.0.0",
                "@sveltejs/kit": "2.0.0",
                "gatsby": "5.0.0"
            }
        }"#,
        );
        let artifacts = detect_framework_artifacts(f.path());
        for a in &artifacts {
            assert_eq!(
                a.safety_tier,
                SafetyTier::Safe,
                "Framework artifact {} should be Safe, found {:?}",
                a.relative_path,
                a.safety_tier
            );
        }
    }
}
