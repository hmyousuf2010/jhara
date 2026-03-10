//! artifact_scan.rs
//!
//! Single-pass artifact directory detector integrated from artifact-detector.rs.
//! Used by the scanner to skip known artifact dirs and record them as candidates.
//!
//! Key contract:
//!   - NEVER descend into a matched artifact directory.
//!   - Confidence = base_confidence when manifest found in parent; base - 0.25 otherwise.
//!   - All detection is directory-name-level only (no file content reads during scan).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use rayon::prelude::*;
use serde::Serialize;

// ── Rule types ────────────────────────────────────────────────────────────────

pub struct ArtifactRule {
    pub dir_name:        &'static str,
    pub manifest_hints:  &'static [&'static str],
    pub kind:            &'static str,   // "build" | "deps" | "tool-cache" | etc.
    pub base_confidence: f32,
    pub note:            &'static str,
}

pub struct ArtifactSuffixRule {
    pub suffix:          &'static str,
    pub manifest_hints:  &'static [&'static str],
    pub kind:            &'static str,
    pub base_confidence: f32,
    pub note:            &'static str,
}

pub struct ArtifactPrefixRule {
    pub prefix:          &'static str,
    pub manifest_hints:  &'static [&'static str],
    pub kind:            &'static str,
    pub base_confidence: f32,
    pub note:            &'static str,
}

// ── Rule lookup ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactRuleRef {
    Exact(usize),
    Suffix(usize),
    Prefix(usize),
}

impl ArtifactRuleRef {
    pub fn manifest_hints(&self) -> &'static [&'static str] {
        match self {
            Self::Exact(i)  => ARTIFACT_RULES[*i].manifest_hints,
            Self::Suffix(i) => ARTIFACT_SUFFIX_RULES[*i].manifest_hints,
            Self::Prefix(i) => ARTIFACT_PREFIX_RULES[*i].manifest_hints,
        }
    }
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Exact(i)  => ARTIFACT_RULES[*i].kind,
            Self::Suffix(i) => ARTIFACT_SUFFIX_RULES[*i].kind,
            Self::Prefix(i) => ARTIFACT_PREFIX_RULES[*i].kind,
        }
    }
    pub fn base_confidence(&self) -> f32 {
        match self {
            Self::Exact(i)  => ARTIFACT_RULES[*i].base_confidence,
            Self::Suffix(i) => ARTIFACT_SUFFIX_RULES[*i].base_confidence,
            Self::Prefix(i) => ARTIFACT_PREFIX_RULES[*i].base_confidence,
        }
    }
    pub fn note(&self) -> &'static str {
        match self {
            Self::Exact(i)  => ARTIFACT_RULES[*i].note,
            Self::Suffix(i) => ARTIFACT_SUFFIX_RULES[*i].note,
            Self::Prefix(i) => ARTIFACT_PREFIX_RULES[*i].note,
        }
    }
}

pub fn find_artifact_rule(name: &str) -> Option<ArtifactRuleRef> {
    for (i, rule) in ARTIFACT_RULES.iter().enumerate() {
        if rule.dir_name == name {
            return Some(ArtifactRuleRef::Exact(i));
        }
    }
    for (i, rule) in ARTIFACT_SUFFIX_RULES.iter().enumerate() {
        if name.ends_with(rule.suffix) {
            return Some(ArtifactRuleRef::Suffix(i));
        }
    }
    for (i, rule) in ARTIFACT_PREFIX_RULES.iter().enumerate() {
        if name.starts_with(rule.prefix) {
            return Some(ArtifactRuleRef::Prefix(i));
        }
    }
    None
}

/// Returns the set of all manifest filenames across all rules.
/// Used to decide which files to record during the walk.
pub fn all_manifest_hints() -> HashSet<&'static str> {
    ARTIFACT_RULES.iter()
        .flat_map(|r| r.manifest_hints.iter().copied())
        .chain(ARTIFACT_SUFFIX_RULES.iter().flat_map(|r| r.manifest_hints.iter().copied()))
        .chain(ARTIFACT_PREFIX_RULES.iter().flat_map(|r| r.manifest_hints.iter().copied()))
        .collect()
}

// ── Detection result ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactDetectionResult {
    pub path:             PathBuf,
    pub kind:             &'static str,
    pub confidence:       f32,
    pub note:             &'static str,
    pub manifest_found:   bool,
    pub matched_manifest: Option<PathBuf>,
}

// ── Internal candidate ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ArtifactCandidate {
    pub path: PathBuf,
    pub rule: ArtifactRuleRef,
}

/// manifest_map: parent dir → manifest filenames found directly inside
pub type ManifestMap = HashMap<PathBuf, Vec<&'static str>>;

// ── Resolve candidates against manifest map ───────────────────────────────────

pub fn resolve_artifact_candidates(
    candidates: Vec<ArtifactCandidate>,
    manifest_map: &ManifestMap,
    threshold: f32,
) -> Vec<ArtifactDetectionResult> {
    candidates
        .into_par_iter()
        .filter_map(|cand| {
            let parent = cand.path.parent()?;
            let hints  = cand.rule.manifest_hints();

            let (manifest_found, matched_manifest) = if hints.is_empty() {
                (true, None)
            } else if let Some(parent_files) = manifest_map.get(parent) {
                let hit = parent_files.iter().find(|&&f| hints.contains(&f));
                match hit {
                    Some(&h) => (true, Some(parent.join(h))),
                    None     => (false, None),
                }
            } else {
                (false, None)
            };

            let confidence = if manifest_found {
                cand.rule.base_confidence()
            } else {
                (cand.rule.base_confidence() - 0.25).max(0.0)
            };

            if confidence < threshold {
                return None;
            }

            Some(ArtifactDetectionResult {
                path: cand.path,
                kind: cand.rule.kind(),
                confidence,
                note: cand.rule.note(),
                manifest_found,
                matched_manifest,
            })
        })
        .collect()
}

// ── Rule tables (ported from artifact-detector.rs) ───────────────────────────

pub static ARTIFACT_RULES: &[ArtifactRule] = &[
    ArtifactRule { dir_name: "node_modules", manifest_hints: &["package.json","package-lock.json","yarn.lock","pnpm-lock.yaml","bun.lockb",".npmrc"], kind: "deps",       base_confidence: 0.97, note: "npm/yarn/pnpm/bun dependencies" },
    ArtifactRule { dir_name: ".npm",         manifest_hints: &["package.json",".npmrc"],                                                              kind: "tool-cache", base_confidence: 0.92, note: "npm global cache" },
    ArtifactRule { dir_name: ".yarn",        manifest_hints: &["yarn.lock",".yarnrc.yml",".yarnrc"],                                                  kind: "tool-cache", base_confidence: 0.93, note: "Yarn Berry cache" },
    ArtifactRule { dir_name: ".pnpm-store",  manifest_hints: &["pnpm-lock.yaml"],                                                                    kind: "tool-cache", base_confidence: 0.94, note: "pnpm content-addressable store" },
    ArtifactRule { dir_name: ".next",        manifest_hints: &["next.config.js","next.config.ts","next.config.mjs","package.json"],                  kind: "build",      base_confidence: 0.96, note: "Next.js build cache" },
    ArtifactRule { dir_name: ".nuxt",        manifest_hints: &["nuxt.config.js","nuxt.config.ts","package.json"],                                    kind: "build",      base_confidence: 0.96, note: "Nuxt.js build output" },
    ArtifactRule { dir_name: ".turbo",       manifest_hints: &["turbo.json","package.json"],                                                         kind: "tool-cache", base_confidence: 0.95, note: "Turborepo cache" },
    ArtifactRule { dir_name: ".parcel-cache",manifest_hints: &["package.json",".parcelrc"],                                                          kind: "tool-cache", base_confidence: 0.97, note: "Parcel bundler cache" },
    ArtifactRule { dir_name: ".svelte-kit",  manifest_hints: &["svelte.config.js","svelte.config.ts","package.json"],                               kind: "build",      base_confidence: 0.97, note: "SvelteKit build output" },
    ArtifactRule { dir_name: "__pycache__",  manifest_hints: &[],                                                                                    kind: "tmp",        base_confidence: 0.99, note: "Python bytecode cache" },
    ArtifactRule { dir_name: ".venv",        manifest_hints: &["requirements.txt","pyproject.toml","Pipfile","uv.lock","poetry.lock"],               kind: "deps",       base_confidence: 0.97, note: "Python virtual environment" },
    ArtifactRule { dir_name: "venv",         manifest_hints: &["requirements.txt","pyproject.toml","Pipfile"],                                       kind: "deps",       base_confidence: 0.88, note: "Python virtual environment" },
    ArtifactRule { dir_name: ".pytest_cache",manifest_hints: &["pytest.ini","pyproject.toml","conftest.py"],                                         kind: "test",       base_confidence: 0.99, note: "pytest cache" },
    ArtifactRule { dir_name: ".mypy_cache",  manifest_hints: &["mypy.ini","pyproject.toml"],                                                         kind: "tool-cache", base_confidence: 0.99, note: "mypy cache" },
    ArtifactRule { dir_name: ".ruff_cache",  manifest_hints: &["ruff.toml","pyproject.toml"],                                                        kind: "tool-cache", base_confidence: 0.99, note: "Ruff linter cache" },
    ArtifactRule { dir_name: "target",       manifest_hints: &["Cargo.toml","Cargo.lock"],                                                           kind: "build",      base_confidence: 0.85, note: "Rust/Maven build output" },
    ArtifactRule { dir_name: ".gradle",      manifest_hints: &["build.gradle","build.gradle.kts","settings.gradle"],                                kind: "tool-cache", base_confidence: 0.98, note: "Gradle build cache" },
    ArtifactRule { dir_name: ".build",       manifest_hints: &["Package.swift"],                                                                     kind: "build",      base_confidence: 0.92, note: "Swift PM build output" },
    ArtifactRule { dir_name: "DerivedData",  manifest_hints: &[],                                                                                    kind: "build",      base_confidence: 0.99, note: "Xcode DerivedData" },
    ArtifactRule { dir_name: "Pods",         manifest_hints: &["Podfile","Podfile.lock"],                                                            kind: "deps",       base_confidence: 0.98, note: "CocoaPods dependencies" },
    ArtifactRule { dir_name: "_build",       manifest_hints: &["mix.exs","mix.lock"],                                                               kind: "build",      base_confidence: 0.96, note: "Elixir/Erlang build" },
    ArtifactRule { dir_name: ".terraform",   manifest_hints: &[".terraform.lock.hcl"],                                                              kind: "tool-cache", base_confidence: 0.99, note: "Terraform providers cache" },
    ArtifactRule { dir_name: ".stack-work",  manifest_hints: &["stack.yaml"],                                                                        kind: "build",      base_confidence: 0.99, note: "Haskell Stack artifacts" },
    ArtifactRule { dir_name: "zig-cache",    manifest_hints: &["build.zig"],                                                                         kind: "build",      base_confidence: 0.99, note: "Zig compiler cache" },
    ArtifactRule { dir_name: "zig-out",      manifest_hints: &["build.zig"],                                                                         kind: "build",      base_confidence: 0.99, note: "Zig build output" },
    ArtifactRule { dir_name: ".dart_tool",   manifest_hints: &["pubspec.yaml"],                                                                      kind: "tool-cache", base_confidence: 0.99, note: "Dart/Flutter pub cache" },
    ArtifactRule { dir_name: "dist",         manifest_hints: &["package.json","Cargo.toml","setup.py","pyproject.toml"],                             kind: "build",      base_confidence: 0.83, note: "Distribution build output" },
    ArtifactRule { dir_name: "build",        manifest_hints: &["package.json","CMakeLists.txt","Makefile","build.gradle"],                           kind: "build",      base_confidence: 0.75, note: "Generic build output" },
    ArtifactRule { dir_name: "vendor",       manifest_hints: &["go.mod","go.sum","Gemfile","composer.json"],                                         kind: "deps",       base_confidence: 0.83, note: "Vendored dependencies" },
    ArtifactRule { dir_name: ".nx",          manifest_hints: &["nx.json"],                                                                           kind: "tool-cache", base_confidence: 0.98, note: "Nx monorepo cache" },
    ArtifactRule { dir_name: "bazel-bin",    manifest_hints: &["WORKSPACE","MODULE.bazel"],                                                          kind: "build",      base_confidence: 0.99, note: "Bazel compiled outputs" },
    ArtifactRule { dir_name: "bazel-out",    manifest_hints: &["WORKSPACE","MODULE.bazel"],                                                          kind: "build",      base_confidence: 0.99, note: "Bazel all outputs" },
    ArtifactRule { dir_name: ".ccache",      manifest_hints: &["CMakeLists.txt","Makefile"],                                                         kind: "cc-cache",   base_confidence: 0.98, note: "ccache compiler cache" },
    ArtifactRule { dir_name: "nimcache",     manifest_hints: &[],                                                                                    kind: "build",      base_confidence: 0.99, note: "Nim compiler cache" },
    ArtifactRule { dir_name: ".ipynb_checkpoints", manifest_hints: &[],                                                                              kind: "tmp",        base_confidence: 0.99, note: "Jupyter checkpoint files" },
    ArtifactRule { dir_name: "coverage",     manifest_hints: &["package.json","jest.config.js","vitest.config.ts","pyproject.toml"],                kind: "test",       base_confidence: 0.88, note: "Test coverage output" },
    ArtifactRule { dir_name: "mlruns",       manifest_hints: &["requirements.txt"],                                                                  kind: "ml-cache",   base_confidence: 0.97, note: "MLflow experiment data" },
];

pub static ARTIFACT_SUFFIX_RULES: &[ArtifactSuffixRule] = &[
    ArtifactSuffixRule { suffix: ".egg-info",    manifest_hints: &["setup.py","pyproject.toml"], kind: "build",      base_confidence: 0.98, note: "Python egg-info" },
    ArtifactSuffixRule { suffix: ".dist-info",   manifest_hints: &["setup.py","pyproject.toml"], kind: "build",      base_confidence: 0.97, note: "Python dist-info" },
    ArtifactSuffixRule { suffix: ".xcuserdata",  manifest_hints: &[],                            kind: "ide",        base_confidence: 0.99, note: "Xcode per-user state" },
];

pub static ARTIFACT_PREFIX_RULES: &[ArtifactPrefixRule] = &[
    ArtifactPrefixRule { prefix: "cmake-build-", manifest_hints: &["CMakeLists.txt"], kind: "build", base_confidence: 0.99, note: "CMake out-of-source build" },
    ArtifactPrefixRule { prefix: "bazel-",       manifest_hints: &["WORKSPACE"],      kind: "build", base_confidence: 0.95, note: "Bazel build symlink" },
];
