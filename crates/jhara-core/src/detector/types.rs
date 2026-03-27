// detector/types.rs
//
// Every type in the detection layer. Kept in one file so the relationship
// between types is immediately visible. No logic lives here — only data.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::SystemTime;

// ─────────────────────────────────────────────────────────────────────────────
// SafetyTier
// ─────────────────────────────────────────────────────────────────────────────

/// How safely a given artifact directory can be deleted.
///
/// The ordering is meaningful: Safe < Caution < Risky < Blocked.
/// A tier can be upgraded (made stricter) by context signals such as a dirty
/// git working tree, but never downgraded. This ensures safety logic cannot
/// be bypassed by a UI state bug.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafetyTier {
    /// Regenerated automatically on the next build. Deleting these never
    /// requires developer intervention beyond re-running the build command.
    /// Examples: `node_modules/`, `target/`, `dist/`, `.next/`.
    Safe,

    /// Recoverable but expensive, or contains data with historical value
    /// that cannot be derived from the current source tree.
    /// Examples: Xcode Archives (crash symbolication), Conda envs with
    /// manually installed packages. Requires checkbox confirmation.
    Caution,

    /// Contains live infrastructure state or credentials that cannot be
    /// recovered from version control. Requires a per-item confirmation
    /// dialog. Batch deletion of Risky items is never permitted.
    /// Examples: Docker volumes, Vagrant machine data.
    Risky,

    /// Never presented to the user as a deletion candidate.
    /// The deletion engine refuses to act on Blocked paths regardless of
    /// how the request was constructed.
    /// Examples: `terraform.tfstate`, `.env`, `*.pem`.
    Blocked,
}

/// A more granular safety evaluation result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum SafetyRating {
    /// Perfectly safe to delete.
    Safe,
    /// Safe to delete, but recommended to verify first.
    Caution(String),
    /// blocked from deletion.
    Block(String),
}

impl SafetyTier {
    pub fn display_name(&self) -> &'static str {
        match self {
            SafetyTier::Safe => "Safe",
            SafetyTier::Caution => "Caution",
            SafetyTier::Risky => "Risky",
            SafetyTier::Blocked => "Protected",
        }
    }

    /// True when automated cleanup rules are allowed to act on this tier.
    /// Only Safe-tier artifacts can be removed without any confirmation.
    pub fn allows_automation(&self) -> bool {
        matches!(self, SafetyTier::Safe)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Ecosystem
// ─────────────────────────────────────────────────────────────────────────────

/// The development ecosystem that produced a set of artifacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ecosystem {
    // Languages and runtimes
    NodeJs,
    Bun,
    Deno,
    PythonPip,
    PythonPoetry,
    PythonConda,
    Ruby,
    Php,
    JavaMaven,
    JavaGradle,
    Kotlin,
    Scala,
    Clojure,
    Groovy,
    Go,
    Rust,
    CCpp,
    Zig,
    Elixir,
    Haskell,
    OCaml,
    FSharp,
    SwiftSpm,
    Dart,
    R,
    Julia,
    Nim,
    Crystal,
    D,
    DotNet,
    UnrealEngine,
    VisualStudio,

    // JS Frameworks
    Next,
    Nuxt,
    SvelteKit,
    Angular,
    Gatsby,
    Remix,
    Astro,
    TanstackStart,
    Solid,
    Qwik,
    Vue,
    Stencil,
    Ember,
    Vite,

    // Backend frameworks
    Django,
    FastApi,
    Flask,
    Phoenix,
    Rails,
    Laravel,
    SpringBoot,
    NestJs,

    // Mobile
    ReactNative,
    Expo,
    Flutter,
    Capacitor,
    Kmm,
    AndroidGradle,
    NativeScript,

    // Apple toolchain
    Xcode,
    XcodeArchives,
    XcodeSimulators,
    XcodeDeviceSupport,
    CocoaPods,
    Carthage,
    Fastlane,

    // DevOps
    Docker,
    Kubernetes,
    Terraform,
    Pulumi,
    Vagrant,
    Ansible,
    Bazel,
    Buck2,
    Pants,

    // Monorepo tooling
    Turborepo,
    Nx,
    CargoWorkspace,
    PnpmWorkspace,
    Lerna,

    // Global cache buckets (not tied to a single project)
    NpmCache,
    YarnCache,
    PnpmStore,
    CargoCache,
    PipCache,
    PoetryCache,
    MavenCache,
    GradleCache,
    RubyGems,
    Composer,
    Homebrew,
    CocoaPodsCache,
    CarthageCache,
    GoCache,
    SwiftPmCache,
    PubCache,
    MixCache,
    StackCache,

    // Data Science
    MLflow,
    Wandb,
    Triton,

    // Ghost sources
    GhostGitHistory,
    GhostShellHistory,
    GhostGitignore,
    GhostSymlink,
}

// ─────────────────────────────────────────────────────────────────────────────
// ArtifactPath
// ─────────────────────────────────────────────────────────────────────────────

/// A single artifact directory definition.
///
/// `relative_path` is relative to the project root for project-level artifacts,
/// or relative to `$HOME` for global caches (`is_global == true`).
///
/// Uses `&'static str` for all string fields so the built-in database has
/// zero heap allocation per entry. User-provided JSON signatures use
/// `ArtifactPathOwned` instead (see below).
#[derive(Debug, Clone, Copy)]
pub struct ArtifactPath {
    /// Path relative to the project root (or `$HOME` if `is_global`).
    pub relative_path: &'static str,

    /// How safely this artifact can be deleted.
    pub safety_tier: SafetyTier,

    /// True for global caches in the user's home directory rather than
    /// inside individual project trees.
    pub is_global: bool,

    /// The command to run to regenerate this artifact after deletion.
    /// Shown to the user in the confirmation UI.
    pub recovery_command: Option<&'static str>,

    /// Typical lower bound of artifact size in megabytes.
    /// Used for heuristic prioritization of cleanup suggestions.
    pub typical_size_mb_min: u32,

    /// Typical upper bound of artifact size in megabytes.
    pub typical_size_mb_max: u32,

    /// True if the scanner should stop recursive traversal immediately
    /// upon reaching this directory to prevent system hangs.
    pub is_prunable: bool,
}

impl ArtifactPath {
    pub const fn new(relative_path: &'static str, safety_tier: SafetyTier) -> Self {
        ArtifactPath {
            relative_path,
            safety_tier,
            is_global: false,
            recovery_command: None,
            typical_size_mb_min: 0,
            typical_size_mb_max: 0,
            is_prunable: false,
        }
    }

    pub const fn prune(mut self) -> Self {
        self.is_prunable = true;
        self
    }

    pub const fn global(mut self) -> Self {
        self.is_global = true;
        self
    }

    pub const fn recovery(mut self, cmd: &'static str) -> Self {
        self.recovery_command = Some(cmd);
        self
    }

    pub const fn size_mb(mut self, min: u32, max: u32) -> Self {
        self.typical_size_mb_min = min;
        self.typical_size_mb_max = max;
        self
    }
}

/// Owned equivalent of `ArtifactPath`, used when loading signatures from JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactPathOwned {
    pub relative_path: String,
    pub safety_tier: SafetyTier,
    #[serde(default)]
    pub is_global: bool,
    pub recovery_command: Option<String>,
    pub typical_size_mb_min: Option<u32>,
    pub typical_size_mb_max: Option<u32>,
    #[serde(default)]
    pub is_prunable: bool,
}

impl From<&ArtifactPath> for ArtifactPathOwned {
    fn from(a: &ArtifactPath) -> Self {
        ArtifactPathOwned {
            relative_path: a.relative_path.to_string(),
            safety_tier: a.safety_tier,
            is_global: a.is_global,
            recovery_command: a.recovery_command.map(|s| s.to_string()),
            typical_size_mb_min: if a.typical_size_mb_min > 0 {
                Some(a.typical_size_mb_min)
            } else {
                None
            },
            typical_size_mb_max: if a.typical_size_mb_max > 0 {
                Some(a.typical_size_mb_max)
            } else {
                None
            },
            is_prunable: a.is_prunable,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ProjectSignature
// ─────────────────────────────────────────────────────────────────────────────

/// The definition of what to look for on disk to identify a project of
/// a given ecosystem type.
///
/// Detection: look for `filename` at each directory level during traversal.
/// If found, optionally confirm by checking `content_key` is present
/// somewhere in the file's text content (used to disambiguate files that
/// share a name across ecosystems).
#[derive(Debug, Clone, Copy)]
pub struct ProjectSignature {
    /// The filename that identifies this project type.
    pub filename: &'static str,

    /// The ecosystem this signature identifies.
    pub ecosystem: Ecosystem,

    /// The artifact paths associated with this project type.
    pub artifact_paths: &'static [ArtifactPath],

    /// An optional string that must appear in the file's content for
    /// the match to be confirmed. Allows disambiguating signatures that
    /// share filenames (e.g., `build.gradle` vs `build.gradle.kts`).
    pub content_key: Option<&'static str>,

    /// Priority for disambiguation when multiple signatures match the same
    /// directory. Higher priority wins. Default 0.
    pub priority: i32,

    /// Default staleness threshold in days. Projects inactive for longer
    /// than this are candidates for automated cleanup.
    pub stale_threshold_days: u32,
}

/// Owned equivalent for JSON-loaded signatures.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSignatureOwned {
    pub filename: String,
    pub ecosystem: Ecosystem,
    pub artifact_paths: Vec<ArtifactPathOwned>,
    pub content_key: Option<String>,
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "default_stale_threshold")]
    pub stale_threshold_days: u32,
}

fn default_stale_threshold() -> u32 {
    60
}

// ─────────────────────────────────────────────────────────────────────────────
// DetectedProject
// ─────────────────────────────────────────────────────────────────────────────

/// A project found on disk by the `ProjectDetector`, with all its artifact
/// directories resolved and sized.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedProject {
    /// Absolute path to the project root (the directory containing the
    /// signature file that triggered detection).
    pub root_path: PathBuf,

    /// The ecosystems detected at this root. Usually one. Multi-ecosystem
    /// roots occur with React Native (JS + native) or mixed-language repos.
    pub ecosystems: Vec<Ecosystem>,

    /// Artifact directories that exist on disk and belong to this project.
    /// Sized using ScanTree queries rather than re-scanning.
    pub artifacts: Vec<FoundArtifact>,

    /// Modification time of the signature file (`package.json`, `Cargo.toml`, etc).
    /// One of the two inputs to the staleness calculation.
    #[serde(with = "serde_system_time")]
    pub signature_mtime: SystemTime,

    /// Modification time of `.git/HEAD`, if this root is a git repository.
    /// `.git/HEAD` changes on every commit, checkout, and fetch — making it
    /// the most reliable activity signal for CLI-driven workflows.
    #[serde(with = "serde_system_time_opt")]
    pub git_head_mtime: Option<SystemTime>,

    /// If this root is part of a monorepo, the kind and root of that monorepo.
    pub monorepo: Option<MonorepoMembership>,
}

impl DetectedProject {
    /// The most recent of `signature_mtime` and `git_head_mtime`.
    /// This is the value compared against the staleness threshold.
    pub fn last_activity(&self) -> SystemTime {
        match self.git_head_mtime {
            Some(git) => self.signature_mtime.max(git),
            None => self.signature_mtime,
        }
    }

    /// True when `last_activity` is older than `threshold_days`.
    pub fn is_stale(&self, threshold_days: u32) -> bool {
        let threshold = std::time::Duration::from_secs(threshold_days as u64 * 86_400);
        self.last_activity()
            .elapsed()
            .map(|age| age > threshold)
            .unwrap_or(false)
    }

    pub fn total_artifact_size_bytes(&self) -> u64 {
        self.artifacts.iter().map(|a| a.physical_size_bytes).sum()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// FoundArtifact
// ─────────────────────────────────────────────────────────────────────────────

/// A concrete artifact directory that exists on disk for a specific project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FoundArtifact {
    /// Absolute path to this artifact directory.
    pub absolute_path: PathBuf,

    /// Safety tier inherited from the signature definition.
    /// May be upgraded (made stricter) by context — e.g., dirty git tree
    /// upgrades Safe to Caution for that session.
    pub safety_tier: SafetyTier,

    /// The ecosystem this artifact belongs to.
    pub ecosystem: Ecosystem,

    /// Physical size in bytes, queried from the ScanTree.
    /// 0 if the tree has not been populated yet (pre-scan).
    pub physical_size_bytes: u64,

    /// Command to regenerate this artifact after deletion.
    pub recovery_command: Option<String>,

    /// The safety evaluation result.
    pub safety_rating: SafetyRating,

    /// True if this artifact is a "ghost" (detected via history, not on disk).
    pub is_ghost: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// MonorepoMembership / MonorepoKind
// ─────────────────────────────────────────────────────────────────────────────

/// Describes the monorepo structure a project belongs to, if any.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GhostCandidate {
    pub path: PathBuf,
    pub source: Ecosystem, // Reusing Ecosystem for ghost sources
    pub confidence_boost: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonorepoMembership {
    pub kind: MonorepoKind,
    /// Absolute path to the monorepo root (one level up from the workspace config).
    pub root: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonorepoKind {
    Turborepo,
    Nx,
    PnpmWorkspace,
    NpmWorkspace,
    YarnWorkspace,
    Lerna,
    CargoWorkspace,
    /// Melos workspace (Dart/Flutter monorepos).
    Melos,
}

impl MonorepoKind {
    pub fn display_name(&self) -> &'static str {
        match self {
            MonorepoKind::Turborepo => "Turborepo",
            MonorepoKind::Nx => "Nx",
            MonorepoKind::PnpmWorkspace => "PNPM Workspace",
            MonorepoKind::NpmWorkspace => "npm Workspace",
            MonorepoKind::YarnWorkspace => "Yarn Workspace",
            MonorepoKind::Lerna => "Lerna",
            MonorepoKind::CargoWorkspace => "Cargo Workspace",
            MonorepoKind::Melos => "Melos",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// XcodeProjectRef
// ─────────────────────────────────────────────────────────────────────────────

/// The result of resolving a DerivedData directory back to its originating
/// Xcode project via the embedded `info.plist`.
#[derive(Debug, Clone)]
pub struct XcodeProjectRef {
    /// The DerivedData directory (e.g., `~/Library/.../DerivedData/MyApp-abc123/`).
    pub derived_data_path: PathBuf,

    /// The originating `.xcodeproj` or `.xcworkspace` path, if the plist
    /// could be read and the path still exists on disk.
    pub project_path: Option<PathBuf>,

    /// True when `project_path` is `None` because the project no longer
    /// exists on disk. Orphaned entries are Safe regardless of age.
    pub is_orphaned: bool,
}

mod serde_system_time {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::{SystemTime, UNIX_EPOCH};

    pub fn serialize<S>(time: &SystemTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let dur = time.duration_since(UNIX_EPOCH).unwrap_or_default();
        (dur.as_secs_f64()).serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<SystemTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = f64::deserialize(deserializer)?;
        Ok(UNIX_EPOCH + std::time::Duration::from_secs_f64(secs))
    }
}

mod serde_system_time_opt {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::{SystemTime, UNIX_EPOCH};

    pub fn serialize<S>(time: &Option<SystemTime>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match time {
            Some(t) => {
                let dur = t.duration_since(UNIX_EPOCH).unwrap_or_default();
                (dur.as_secs_f64()).serialize(serializer)
            }
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<SystemTime>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt_secs: Option<f64> = Option::deserialize(deserializer)?;
        Ok(opt_secs.map(|secs| UNIX_EPOCH + std::time::Duration::from_secs_f64(secs)))
    }
}
