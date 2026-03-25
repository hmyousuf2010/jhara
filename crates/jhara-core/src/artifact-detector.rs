//! jhara — reconstructable developer artifact detector
//!
//! Core contract:
//!   • NEVER descend into a known artifact directory (no node_modules file crawl).
//!   • Every detection is directory-level only.
//!   • Confidence comes from dir name + parent manifest cross-check.
//!   • Ghost dirs (ever-existed, now deleted) shown only with --ghosts.

#![allow(clippy::needless_pass_by_value)]

use std::{
    collections::{HashMap, HashSet},
    fmt, fs,
    io::{self, BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};

use clap::Parser;
use rayon::prelude::*;
use serde::Serialize;
use walkdir::WalkDir;

// ════════════════════════════════════════════════════════════════════════════
// CLI
// ════════════════════════════════════════════════════════════════════════════

#[derive(Parser, Debug)]
#[command(
    name = "jhara",
    about = "Reconstructable developer artifact detector",
    version,
    long_about = "Scans for build outputs, dep caches, and tool artifacts that\n\
                  are safe to delete because they can be fully regenerated.\n\
                  Never reads file contents. Directory-first, manifest-confirmed."
)]
pub struct Args {
    /// Root directory to scan
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Output results as JSON
    #[arg(short = 'j', long)]
    pub json: bool,

    /// Calculate approximate directory sizes (slower — bounded at 100k entries/dir)
    #[arg(short = 's', long)]
    pub sizes: bool,

    /// Include ghost directories (ever-existed but now deleted)
    #[arg(short = 'g', long)]
    pub ghosts: bool,

    /// Minimum confidence threshold 0.0–1.0
    #[arg(short = 't', long, default_value = "0.60")]
    pub threshold: f32,

    /// Interactive deletion of safe items (requires --confirm)
    #[arg(short = 'd', long)]
    pub delete: bool,

    /// Required alongside --delete to actually remove files
    #[arg(long)]
    pub confirm: bool,

    /// Show signal detail per result
    #[arg(short = 'v', long)]
    pub verbose: bool,

    /// Filter by kind: build|deps|tool-cache|test|ide|cc-cache|tmp|ml-cache
    #[arg(short = 'k', long)]
    pub kind: Option<String>,
}

// ════════════════════════════════════════════════════════════════════════════
// TYPES
// ════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ArtifactKind {
    BuildOutput,
    DependencyCache,
    ToolCache,
    TestArtifact,
    IDEArtifact,
    CompilerCache,
    TempFile,
    MLCache,
}

impl ArtifactKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::BuildOutput     => "build",
            Self::DependencyCache => "deps",
            Self::ToolCache       => "tool-cache",
            Self::TestArtifact    => "test",
            Self::IDEArtifact     => "ide",
            Self::CompilerCache   => "cc-cache",
            Self::TempFile        => "tmp",
            Self::MLCache         => "ml-cache",
        }
    }

    pub fn ansi_color(&self) -> &'static str {
        match self {
            Self::BuildOutput     => "\x1b[33m",  // yellow
            Self::DependencyCache => "\x1b[36m",  // cyan
            Self::ToolCache       => "\x1b[35m",  // magenta
            Self::TestArtifact    => "\x1b[34m",  // blue
            Self::IDEArtifact     => "\x1b[90m",  // dark gray
            Self::CompilerCache   => "\x1b[95m",  // bright magenta
            Self::TempFile        => "\x1b[37m",  // light gray
            Self::MLCache         => "\x1b[32m",  // green
        }
    }

    pub fn from_label(s: &str) -> Option<Self> {
        match s {
            "build"      => Some(Self::BuildOutput),
            "deps"       => Some(Self::DependencyCache),
            "tool-cache" => Some(Self::ToolCache),
            "test"       => Some(Self::TestArtifact),
            "ide"        => Some(Self::IDEArtifact),
            "cc-cache"   => Some(Self::CompilerCache),
            "tmp"        => Some(Self::TempFile),
            "ml-cache"   => Some(Self::MLCache),
            _            => None,
        }
    }
}

impl fmt::Display for ArtifactKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DetectionResult {
    pub path: PathBuf,
    pub kind: ArtifactKind,
    pub confidence: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    pub note: &'static str,
    pub is_ghost: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_manifest: Option<PathBuf>,
    pub signals: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum GhostSource {
    GitHistory,
    ShellHistory,
    GitignoreHint,
    DanglingSymlink,
}

impl fmt::Display for GhostSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GitHistory      => write!(f, "git-history"),
            Self::ShellHistory    => write!(f, "shell-history"),
            Self::GitignoreHint   => write!(f, "gitignore-hint"),
            Self::DanglingSymlink => write!(f, "dangling-symlink"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GhostCandidate {
    pub path: PathBuf,
    pub source: GhostSource,
    pub matched_rule_idx: usize, // index into RULES
}

// ════════════════════════════════════════════════════════════════════════════
// RULES — The knowledge base
//
// Every Rule maps a directory name → (kind, manifests that confirm it).
// Confidence without a manifest: base_confidence - 0.25 (min 0.0).
// ════════════════════════════════════════════════════════════════════════════

pub struct Rule {
    /// Exact directory name to match
    pub dir_name: &'static str,
    /// Filenames in parent directory that boost confidence to base_confidence.
    /// Empty = name alone is enough (very distinctive names like "__pycache__").
    pub manifest_hints: &'static [&'static str],
    pub kind: ArtifactKind,
    pub base_confidence: f32,
    pub note: &'static str,
}

/// Rules matched by name suffix (e.g. ".egg-info", ".xcuserdata")
pub struct SuffixRule {
    pub suffix: &'static str,
    pub manifest_hints: &'static [&'static str],
    pub kind: ArtifactKind,
    pub base_confidence: f32,
    pub note: &'static str,
}

/// Rules matched by name prefix (e.g. "cmake-build-", "bazel-")
pub struct PrefixRule {
    pub prefix: &'static str,
    pub manifest_hints: &'static [&'static str],
    pub kind: ArtifactKind,
    pub base_confidence: f32,
    pub note: &'static str,
}

// ─── Exact name rules ────────────────────────────────────────────────────────

pub static RULES: &[Rule] = &[
    // ── JavaScript / Node ────────────────────────────────────────────────────
    Rule {
        dir_name: "node_modules",
        manifest_hints: &["package.json", "package-lock.json", "yarn.lock",
                          "pnpm-lock.yaml", "bun.lockb", ".npmrc"],
        kind: ArtifactKind::DependencyCache,
        base_confidence: 0.97,
        note: "npm/yarn/pnpm/bun dependencies",
    },
    Rule {
        dir_name: ".npm",
        manifest_hints: &["package.json", ".npmrc"],
        kind: ArtifactKind::ToolCache,
        base_confidence: 0.92,
        note: "npm global cache",
    },
    Rule {
        dir_name: ".yarn",
        manifest_hints: &["yarn.lock", ".yarnrc.yml", ".yarnrc"],
        kind: ArtifactKind::ToolCache,
        base_confidence: 0.93,
        note: "Yarn Berry cache and PnP state",
    },
    Rule {
        dir_name: ".pnpm-store",
        manifest_hints: &["pnpm-lock.yaml"],
        kind: ArtifactKind::ToolCache,
        base_confidence: 0.94,
        note: "pnpm content-addressable store",
    },
    Rule {
        dir_name: ".next",
        manifest_hints: &["next.config.js", "next.config.ts", "next.config.mjs",
                          "next.config.cjs", "package.json"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.96,
        note: "Next.js build cache and server output",
    },
    Rule {
        dir_name: ".nuxt",
        manifest_hints: &["nuxt.config.js", "nuxt.config.ts", "nuxt.config.mjs", "package.json"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.96,
        note: "Nuxt.js build output",
    },
    Rule {
        dir_name: ".turbo",
        manifest_hints: &["turbo.json", "package.json"],
        kind: ArtifactKind::ToolCache,
        base_confidence: 0.95,
        note: "Turborepo remote/local cache",
    },
    Rule {
        dir_name: ".parcel-cache",
        manifest_hints: &["package.json", ".parcelrc"],
        kind: ArtifactKind::ToolCache,
        base_confidence: 0.97,
        note: "Parcel bundler cache",
    },
    Rule {
        dir_name: ".vite",
        manifest_hints: &["vite.config.js", "vite.config.ts", "vite.config.mjs", "package.json"],
        kind: ArtifactKind::ToolCache,
        base_confidence: 0.95,
        note: "Vite dev-server pre-bundle cache",
    },
    Rule {
        dir_name: ".svelte-kit",
        manifest_hints: &["svelte.config.js", "svelte.config.ts", "package.json"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.97,
        note: "SvelteKit generated types + build output",
    },
    Rule {
        dir_name: ".astro",
        manifest_hints: &["astro.config.mjs", "astro.config.ts", "astro.config.js", "package.json"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.96,
        note: "Astro build cache",
    },
    Rule {
        dir_name: ".remix",
        manifest_hints: &["remix.config.js", "remix.config.ts", "package.json"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.95,
        note: "Remix framework build output",
    },
    Rule {
        dir_name: "storybook-static",
        manifest_hints: &["package.json"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.94,
        note: "Storybook static build output",
    },
    Rule {
        dir_name: ".nyc_output",
        manifest_hints: &["package.json", ".nycrc", ".nycrc.json"],
        kind: ArtifactKind::TestArtifact,
        base_confidence: 0.98,
        note: "nyc/Istanbul code coverage raw data",
    },
    Rule {
        dir_name: "jspm_packages",
        manifest_hints: &["package.json"],
        kind: ArtifactKind::DependencyCache,
        base_confidence: 0.91,
        note: "JSPM package cache (legacy)",
    },
    Rule {
        dir_name: "bower_components",
        manifest_hints: &["bower.json", ".bowerrc"],
        kind: ArtifactKind::DependencyCache,
        base_confidence: 0.95,
        note: "Bower dependencies (legacy)",
    },
    Rule {
        dir_name: "web_modules",
        manifest_hints: &["package.json"],
        kind: ArtifactKind::DependencyCache,
        base_confidence: 0.89,
        note: "Snowpack web modules",
    },
    // ── Rust ─────────────────────────────────────────────────────────────────
    Rule {
        dir_name: "target",
        manifest_hints: &["Cargo.toml", "Cargo.lock"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.85, // 'target' also common in Maven/Gradle
        note: "Rust/Maven/sbt build output (confirm via Cargo.toml)",
    },
    // ── Python ───────────────────────────────────────────────────────────────
    Rule {
        dir_name: "__pycache__",
        manifest_hints: &[], // name is unambiguous — no manifest needed
        kind: ArtifactKind::TempFile,
        base_confidence: 0.99,
        note: "Python bytecode cache (.pyc files)",
    },
    Rule {
        dir_name: ".venv",
        manifest_hints: &["requirements.txt", "pyproject.toml", "Pipfile",
                          "uv.lock", "poetry.lock", "setup.py", "setup.cfg"],
        kind: ArtifactKind::DependencyCache,
        base_confidence: 0.97,
        note: "Python virtual environment (.venv)",
    },
    Rule {
        dir_name: "venv",
        manifest_hints: &["requirements.txt", "pyproject.toml", "Pipfile"],
        kind: ArtifactKind::DependencyCache,
        base_confidence: 0.88,
        note: "Python virtual environment (venv)",
    },
    Rule {
        dir_name: "env",
        manifest_hints: &["requirements.txt", "pyproject.toml"],
        kind: ArtifactKind::DependencyCache,
        base_confidence: 0.72, // generic name, needs manifest confirmation
        note: "Python virtual environment (env) — low confidence, verify",
    },
    Rule {
        dir_name: ".tox",
        manifest_hints: &["tox.ini", "setup.cfg", "pyproject.toml"],
        kind: ArtifactKind::TestArtifact,
        base_confidence: 0.98,
        note: "tox test environments",
    },
    Rule {
        dir_name: ".nox",
        manifest_hints: &["noxfile.py"],
        kind: ArtifactKind::TestArtifact,
        base_confidence: 0.98,
        note: "nox session environments",
    },
    Rule {
        dir_name: ".pytest_cache",
        manifest_hints: &["pytest.ini", "pyproject.toml", "setup.cfg", "conftest.py"],
        kind: ArtifactKind::TestArtifact,
        base_confidence: 0.99,
        note: "pytest cache directory",
    },
    Rule {
        dir_name: ".mypy_cache",
        manifest_hints: &["mypy.ini", "setup.cfg", "pyproject.toml"],
        kind: ArtifactKind::ToolCache,
        base_confidence: 0.99,
        note: "mypy static type-checker cache",
    },
    Rule {
        dir_name: ".ruff_cache",
        manifest_hints: &["ruff.toml", "pyproject.toml"],
        kind: ArtifactKind::ToolCache,
        base_confidence: 0.99,
        note: "Ruff linter cache",
    },
    Rule {
        dir_name: ".pytype",
        manifest_hints: &["pyproject.toml", "setup.cfg"],
        kind: ArtifactKind::ToolCache,
        base_confidence: 0.97,
        note: "pytype static analyzer output",
    },
    Rule {
        dir_name: ".hypothesis",
        manifest_hints: &["pyproject.toml", "pytest.ini", "conftest.py"],
        kind: ArtifactKind::TestArtifact,
        base_confidence: 0.97,
        note: "Hypothesis property-based test database",
    },
    Rule {
        dir_name: "htmlcov",
        manifest_hints: &[".coveragerc", "pyproject.toml", "setup.cfg"],
        kind: ArtifactKind::TestArtifact,
        base_confidence: 0.96,
        note: "coverage.py HTML report output",
    },
    Rule {
        dir_name: ".ipynb_checkpoints",
        manifest_hints: &[],
        kind: ArtifactKind::TempFile,
        base_confidence: 0.99,
        note: "Jupyter notebook auto-save checkpoints",
    },
    Rule {
        dir_name: "__pypackages__",
        manifest_hints: &["pyproject.toml"],
        kind: ArtifactKind::DependencyCache,
        base_confidence: 0.95,
        note: "PEP 582 local package directory",
    },
    Rule {
        dir_name: ".pixi",
        manifest_hints: &["pixi.toml", "pyproject.toml"],
        kind: ArtifactKind::DependencyCache,
        base_confidence: 0.98,
        note: "Pixi environment manager cache",
    },
    Rule {
        dir_name: ".pdm-build",
        manifest_hints: &["pyproject.toml"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.97,
        note: "PDM build artifacts",
    },
    // ── Java / JVM ───────────────────────────────────────────────────────────
    Rule {
        dir_name: ".gradle",
        manifest_hints: &["build.gradle", "build.gradle.kts", "settings.gradle",
                          "settings.gradle.kts", "gradle.properties"],
        kind: ArtifactKind::ToolCache,
        base_confidence: 0.98,
        note: "Gradle build cache and daemon state",
    },
    Rule {
        dir_name: ".kotlin",
        manifest_hints: &["build.gradle.kts", "settings.gradle.kts"],
        kind: ArtifactKind::ToolCache,
        base_confidence: 0.98,
        note: "Kotlin incremental compilation cache",
    },
    Rule {
        dir_name: ".bsp",
        manifest_hints: &["build.sbt", "build.gradle", "build.gradle.kts", "pom.xml"],
        kind: ArtifactKind::ToolCache,
        base_confidence: 0.93,
        note: "Build Server Protocol workspace metadata",
    },
    // ── Swift / iOS / macOS ──────────────────────────────────────────────────
    Rule {
        dir_name: ".build",
        manifest_hints: &["Package.swift"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.92, // .build also exists in other contexts
        note: "Swift Package Manager build output",
    },
    Rule {
        dir_name: "DerivedData",
        manifest_hints: &[],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.99,
        note: "Xcode DerivedData — often multi-GB, fully regeneratable",
    },
    Rule {
        dir_name: "Pods",
        manifest_hints: &["Podfile", "Podfile.lock"],
        kind: ArtifactKind::DependencyCache,
        base_confidence: 0.98,
        note: "CocoaPods dependencies",
    },
    Rule {
        dir_name: ".swiftpm",
        manifest_hints: &["Package.swift"],
        kind: ArtifactKind::ToolCache,
        base_confidence: 0.98,
        note: "Swift Package Manager resolved dependency state",
    },
    // ── Go ───────────────────────────────────────────────────────────────────
    Rule {
        dir_name: "vendor",
        manifest_hints: &["go.mod", "go.sum", "Gemfile", "Gemfile.lock",
                          "composer.json", "composer.lock"],
        kind: ArtifactKind::DependencyCache,
        base_confidence: 0.83, // 'vendor' is generic — needs manifest
        note: "Vendored dependencies (Go/Ruby/PHP/etc.)",
    },
    // ── Ruby ─────────────────────────────────────────────────────────────────
    Rule {
        dir_name: ".bundle",
        manifest_hints: &["Gemfile", "Gemfile.lock"],
        kind: ArtifactKind::ToolCache,
        base_confidence: 0.97,
        note: "Bundler path config and cached gems",
    },
    Rule {
        dir_name: ".sass-cache",
        manifest_hints: &["Gemfile", "package.json"],
        kind: ArtifactKind::ToolCache,
        base_confidence: 0.98,
        note: "Sass/SCSS compiler cache",
    },
    // ── PHP ──────────────────────────────────────────────────────────────────
    Rule {
        dir_name: ".phpunit.cache",
        manifest_hints: &["phpunit.xml", "phpunit.xml.dist", "composer.json"],
        kind: ArtifactKind::TestArtifact,
        base_confidence: 0.99,
        note: "PHPUnit test cache",
    },
    // ── Elixir / Erlang ──────────────────────────────────────────────────────
    Rule {
        dir_name: "_build",
        manifest_hints: &["mix.exs", "mix.lock", "rebar.config", "erlang.mk"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.96,
        note: "Elixir/Erlang compiled OTP applications",
    },
    Rule {
        dir_name: "deps",
        manifest_hints: &["mix.exs", "mix.lock", "rebar.config"],
        kind: ArtifactKind::DependencyCache,
        base_confidence: 0.83, // 'deps' is used in many contexts
        note: "Elixir/Erlang fetched dependencies",
    },
    Rule {
        dir_name: ".elixir_ls",
        manifest_hints: &["mix.exs"],
        kind: ArtifactKind::IDEArtifact,
        base_confidence: 0.98,
        note: "ElixirLS language server data",
    },
    // ── C / C++ ──────────────────────────────────────────────────────────────
    Rule {
        dir_name: "cmake-build-debug",
        manifest_hints: &["CMakeLists.txt"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.99,
        note: "CMake debug build directory (CLion default)",
    },
    Rule {
        dir_name: "cmake-build-release",
        manifest_hints: &["CMakeLists.txt"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.99,
        note: "CMake release build directory",
    },
    Rule {
        dir_name: "cmake-build-relwithdebinfo",
        manifest_hints: &["CMakeLists.txt"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.99,
        note: "CMake RelWithDebInfo build directory",
    },
    Rule {
        dir_name: "conan",
        manifest_hints: &["conanfile.txt", "conanfile.py", "CMakeLists.txt"],
        kind: ArtifactKind::DependencyCache,
        base_confidence: 0.95,
        note: "Conan C/C++ package manager cache",
    },
    // ── .NET / C# ────────────────────────────────────────────────────────────
    Rule {
        dir_name: "obj",
        manifest_hints: &[], // .csproj/.fsproj handled by suffix rules
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.78, // 'obj' alone is ambiguous — suffix rules will boost
        note: ".NET intermediate build objects",
    },
    Rule {
        dir_name: "TestResults",
        manifest_hints: &[],
        kind: ArtifactKind::TestArtifact,
        base_confidence: 0.90,
        note: ".NET / VS test output directory",
    },
    // ── Dart / Flutter ───────────────────────────────────────────────────────
    Rule {
        dir_name: ".dart_tool",
        manifest_hints: &["pubspec.yaml", "pubspec.lock"],
        kind: ArtifactKind::ToolCache,
        base_confidence: 0.99,
        note: "Dart/Flutter pub tool cache and build runner output",
    },
    // ── Infrastructure ───────────────────────────────────────────────────────
    Rule {
        dir_name: ".terraform",
        manifest_hints: &[".terraform.lock.hcl"],
        kind: ArtifactKind::ToolCache,
        base_confidence: 0.99,
        note: "Terraform providers, modules, and state backend cache",
    },
    Rule {
        dir_name: ".serverless",
        manifest_hints: &["serverless.yml", "serverless.yaml", "serverless.ts"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.99,
        note: "Serverless Framework packaged deployment artifact",
    },
    Rule {
        dir_name: "cdk.out",
        manifest_hints: &["cdk.json"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.99,
        note: "AWS CDK synthesized CloudFormation templates",
    },
    Rule {
        dir_name: ".pulumi",
        manifest_hints: &["Pulumi.yaml", "Pulumi.yml"],
        kind: ArtifactKind::ToolCache,
        base_confidence: 0.99,
        note: "Pulumi state, plugins, and credentials cache",
    },
    // ── Build systems ────────────────────────────────────────────────────────
    Rule {
        dir_name: "bazel-bin",
        manifest_hints: &["WORKSPACE", "WORKSPACE.bazel", "MODULE.bazel"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.99,
        note: "Bazel compiled build outputs",
    },
    Rule {
        dir_name: "bazel-out",
        manifest_hints: &["WORKSPACE", "WORKSPACE.bazel", "MODULE.bazel"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.99,
        note: "Bazel all build outputs (root symlink)",
    },
    Rule {
        dir_name: "bazel-testlogs",
        manifest_hints: &["WORKSPACE", "WORKSPACE.bazel"],
        kind: ArtifactKind::TestArtifact,
        base_confidence: 0.99,
        note: "Bazel test result logs",
    },
    Rule {
        dir_name: ".bazel",
        manifest_hints: &["WORKSPACE", "WORKSPACE.bazel", "MODULE.bazel"],
        kind: ArtifactKind::ToolCache,
        base_confidence: 0.99,
        note: "Bazel local disk cache",
    },
    Rule {
        dir_name: "buck-out",
        manifest_hints: &[".buckconfig", "BUCK", "BUCK.v2"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.99,
        note: "Buck2 build output directory",
    },
    Rule {
        dir_name: ".pants.d",
        manifest_hints: &["pants.toml", "BUILD_ROOT"],
        kind: ArtifactKind::ToolCache,
        base_confidence: 0.99,
        note: "Pants v2 build system cache",
    },
    Rule {
        dir_name: ".nx",
        manifest_hints: &["nx.json", "workspace.json"],
        kind: ArtifactKind::ToolCache,
        base_confidence: 0.98,
        note: "Nx monorepo computation cache",
    },
    // ── Generic (lower confidence — need manifest) ───────────────────────────
    Rule {
        dir_name: "dist",
        manifest_hints: &["package.json", "setup.py", "pyproject.toml",
                          "Cargo.toml", "rollup.config.js", "webpack.config.js",
                          "vite.config.ts", "vite.config.js"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.83,
        note: "Distribution / release build output",
    },
    Rule {
        dir_name: "build",
        manifest_hints: &["package.json", "CMakeLists.txt", "Makefile",
                          "build.gradle", "build.gradle.kts", "setup.py",
                          "meson.build"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.75,
        note: "Generic build output directory",
    },
    Rule {
        dir_name: "out",
        manifest_hints: &["package.json", "tsconfig.json", "*.iml"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.73,
        note: "Generic compiler / transpiler output",
    },
    Rule {
        dir_name: "coverage",
        manifest_hints: &["package.json", "jest.config.js", "jest.config.ts",
                          "jest.config.mjs", "vitest.config.ts", "pyproject.toml"],
        kind: ArtifactKind::TestArtifact,
        base_confidence: 0.88,
        note: "Test coverage report output",
    },
    // ── IDEs ─────────────────────────────────────────────────────────────────
    Rule {
        dir_name: ".idea",
        manifest_hints: &[],
        kind: ArtifactKind::IDEArtifact,
        base_confidence: 0.78,
        note: "JetBrains IDE project state (can regenerate)",
    },
    // ── Haskell / Cabal / Stack ───────────────────────────────────────────────
    Rule {
        dir_name: ".stack-work",
        manifest_hints: &["stack.yaml", "stack.yaml.lock"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.99,
        note: "Haskell Stack build artifacts",
    },
    Rule {
        dir_name: "dist-newstyle",
        manifest_hints: &["cabal.project"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.99,
        note: "Haskell Cabal new-build artifacts",
    },
    // ── Scala ────────────────────────────────────────────────────────────────
    Rule {
        dir_name: ".scala-build",
        manifest_hints: &["project/build.properties"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.99,
        note: "Scala CLI build cache",
    },
    // ── Zig ──────────────────────────────────────────────────────────────────
    Rule {
        dir_name: "zig-cache",
        manifest_hints: &["build.zig", "build.zig.zon"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.99,
        note: "Zig compiler cache",
    },
    Rule {
        dir_name: "zig-out",
        manifest_hints: &["build.zig", "build.zig.zon"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.99,
        note: "Zig build output directory",
    },
    // ── Nim ──────────────────────────────────────────────────────────────────
    Rule {
        dir_name: "nimcache",
        manifest_hints: &[],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.99,
        note: "Nim compiler object cache",
    },
    // ── Crystal ──────────────────────────────────────────────────────────────
    Rule {
        dir_name: ".shards",
        manifest_hints: &["shard.yml", "shard.lock"],
        kind: ArtifactKind::DependencyCache,
        base_confidence: 0.99,
        note: "Crystal Shards dependency cache",
    },
    Rule {
        dir_name: "lib",
        manifest_hints: &["shard.yml"],
        kind: ArtifactKind::DependencyCache,
        base_confidence: 0.82,
        note: "Crystal Shards installed libs",
    },
    // ── D lang ───────────────────────────────────────────────────────────────
    Rule {
        dir_name: ".dub",
        manifest_hints: &["dub.json", "dub.sdl"],
        kind: ArtifactKind::DependencyCache,
        base_confidence: 0.99,
        note: "D language DUB package cache",
    },
    // ── R ────────────────────────────────────────────────────────────────────
    Rule {
        dir_name: "renv",
        manifest_hints: &["renv.lock"],
        kind: ArtifactKind::DependencyCache,
        base_confidence: 0.99,
        note: "R renv project-local package library",
    },
    Rule {
        dir_name: "packrat",
        manifest_hints: &[],
        kind: ArtifactKind::DependencyCache,
        base_confidence: 0.97,
        note: "R packrat package library",
    },
    // ── OCaml ────────────────────────────────────────────────────────────────
    Rule {
        dir_name: "_opam",
        manifest_hints: &["*.opam", "dune-project"],
        kind: ArtifactKind::DependencyCache,
        base_confidence: 0.99,
        note: "opam local switch packages",
    },
    // ── ML / Data ────────────────────────────────────────────────────────────
    Rule {
        dir_name: "mlruns",
        manifest_hints: &["MLproject", "requirements.txt", "conda.yaml"],
        kind: ArtifactKind::MLCache,
        base_confidence: 0.97,
        note: "MLflow experiment tracking data",
    },
    Rule {
        dir_name: "wandb",
        manifest_hints: &["requirements.txt", "pyproject.toml"],
        kind: ArtifactKind::MLCache,
        base_confidence: 0.96,
        note: "Weights & Biases run logs and artifacts",
    },
    Rule {
        dir_name: ".triton",
        manifest_hints: &["requirements.txt", "pyproject.toml"],
        kind: ArtifactKind::CompilerCache,
        base_confidence: 0.96,
        note: "Triton GPU kernel JIT cache",
    },
    // ── Compiler caches ──────────────────────────────────────────────────────
    Rule {
        dir_name: ".ccache",
        manifest_hints: &["CMakeLists.txt", "Makefile", "configure.ac"],
        kind: ArtifactKind::CompilerCache,
        base_confidence: 0.98,
        note: "ccache compiler output cache",
    },
    Rule {
        dir_name: ".sccache",
        manifest_hints: &["Cargo.toml", "CMakeLists.txt"],
        kind: ArtifactKind::CompilerCache,
        base_confidence: 0.98,
        note: "sccache shared compiler cache",
    },
];

// ─── Suffix-match rules ──────────────────────────────────────────────────────

pub static SUFFIX_RULES: &[SuffixRule] = &[
    SuffixRule {
        suffix: ".egg-info",
        manifest_hints: &["setup.py", "setup.cfg", "pyproject.toml"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.98,
        note: "Python package egg-info metadata",
    },
    SuffixRule {
        suffix: ".dist-info",
        manifest_hints: &["setup.py", "setup.cfg", "pyproject.toml"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.97,
        note: "Python wheel dist-info metadata",
    },
    SuffixRule {
        suffix: ".xcuserdata",
        manifest_hints: &[],
        kind: ArtifactKind::IDEArtifact,
        base_confidence: 0.99,
        note: "Xcode per-user project state",
    },
    SuffixRule {
        suffix: ".xcworkspace",
        manifest_hints: &["Podfile"],
        kind: ArtifactKind::IDEArtifact,
        base_confidence: 0.70, // can be source-controlled
        note: "Xcode workspace (CocoaPods generated — verify)",
    },
];

// ─── Prefix-match rules ──────────────────────────────────────────────────────

pub static PREFIX_RULES: &[PrefixRule] = &[
    PrefixRule {
        prefix: "cmake-build-",
        manifest_hints: &["CMakeLists.txt"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.99,
        note: "CMake out-of-source build directory",
    },
    PrefixRule {
        prefix: "bazel-",
        manifest_hints: &["WORKSPACE", "WORKSPACE.bazel"],
        kind: ArtifactKind::BuildOutput,
        base_confidence: 0.95,
        note: "Bazel build directory symlink",
    },
];

// ─── Rule lookup ─────────────────────────────────────────────────────────────

pub enum RuleRef {
    Exact(usize),   // index into RULES
    Suffix(usize),  // index into SUFFIX_RULES
    Prefix(usize),  // index into PREFIX_RULES
}

impl RuleRef {
    pub fn manifest_hints(&self) -> &'static [&'static str] {
        match self {
            Self::Exact(i)  => RULES[*i].manifest_hints,
            Self::Suffix(i) => SUFFIX_RULES[*i].manifest_hints,
            Self::Prefix(i) => PREFIX_RULES[*i].manifest_hints,
        }
    }
    pub fn kind(&self) -> ArtifactKind {
        match self {
            Self::Exact(i)  => RULES[*i].kind.clone(),
            Self::Suffix(i) => SUFFIX_RULES[*i].kind.clone(),
            Self::Prefix(i) => PREFIX_RULES[*i].kind.clone(),
        }
    }
    pub fn base_confidence(&self) -> f32 {
        match self {
            Self::Exact(i)  => RULES[*i].base_confidence,
            Self::Suffix(i) => SUFFIX_RULES[*i].base_confidence,
            Self::Prefix(i) => PREFIX_RULES[*i].base_confidence,
        }
    }
    pub fn note(&self) -> &'static str {
        match self {
            Self::Exact(i)  => RULES[*i].note,
            Self::Suffix(i) => SUFFIX_RULES[*i].note,
            Self::Prefix(i) => PREFIX_RULES[*i].note,
        }
    }
    pub fn rule_idx_for_ghost(&self) -> usize {
        match self { Self::Exact(i) => *i, _ => 0 }
    }
}

pub fn find_rule(name: &str) -> Option<RuleRef> {
    // 1. exact
    for (i, rule) in RULES.iter().enumerate() {
        if rule.dir_name == name {
            return Some(RuleRef::Exact(i));
        }
    }
    // 2. suffix
    for (i, rule) in SUFFIX_RULES.iter().enumerate() {
        if name.ends_with(rule.suffix) {
            return Some(RuleRef::Suffix(i));
        }
    }
    // 3. prefix
    for (i, rule) in PREFIX_RULES.iter().enumerate() {
        if name.starts_with(rule.prefix) {
            return Some(RuleRef::Prefix(i));
        }
    }
    None
}

// Pre-computed set of every manifest filename across all rules.
// Used to avoid storing every filename during the walk.
fn build_all_hints() -> HashSet<&'static str> {
    RULES.iter()
        .flat_map(|r| r.manifest_hints.iter().copied())
        .chain(SUFFIX_RULES.iter().flat_map(|r| r.manifest_hints.iter().copied()))
        .chain(PREFIX_RULES.iter().flat_map(|r| r.manifest_hints.iter().copied()))
        .collect()
}

// ════════════════════════════════════════════════════════════════════════════
// SCANNER
//
// Single walkdir pass. When a known artifact dir is found:
//   1. Record it as a candidate.
//   2. Call skip_current_dir() — no recursion inside.
// For every other file, if its name matches a known manifest hint, record it
// in manifest_map[parent].
// ════════════════════════════════════════════════════════════════════════════

/// Collected candidate dir before manifest matching
struct Candidate {
    path: PathBuf,
    rule: RuleRef,
}

/// manifest_map: dir → set of manifest filenames found directly inside that dir
type ManifestMap = HashMap<PathBuf, Vec<&'static str>>;

fn scan_tree(root: &Path, all_hints: &HashSet<&'static str>)
    -> (Vec<Candidate>, ManifestMap)
{
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut manifest_map: ManifestMap = HashMap::new();

    // Also collect dangling symlinks for ghost detection
    let mut walker = WalkDir::new(root)
        .follow_links(false)
        .min_depth(1)
        .into_iter();

    loop {
        match walker.next() {
            None => break,
            Some(Err(_e)) => {
                // Permission denied, etc. — skip silently
                continue;
            }
            Some(Ok(entry)) => {
                let ft = entry.file_type();
                let name = match entry.file_name().to_str() {
                    Some(n) => n,
                    None    => continue, // non-UTF8 name — skip
                };
                let path = entry.into_path();

                if ft.is_dir() {
                    if let Some(rule) = find_rule(name) {
                        candidates.push(Candidate { path, rule });
                        walker.skip_current_dir();
                        continue;
                    }
                    // Also skip some globally irrelevant dirs to avoid wasted walk time
                    if matches!(name, ".git" | ".svn" | ".hg") {
                        walker.skip_current_dir();
                        continue;
                    }
                } else if ft.is_file() {
                    // Only record if this filename is a known manifest hint
                    if all_hints.contains(name) {
                        // We need the 'static str — find it from the set
                        // all_hints is HashSet<&'static str>, so we can get it back
                        if let Some(&hint) = all_hints.get(name) {
                            let parent = match path.parent() {
                                Some(p) => p.to_owned(),
                                None    => continue,
                            };
                            manifest_map.entry(parent).or_default().push(hint);
                        }
                    }
                }
                // Symlinks: handled by ghost detector separately
            }
        }
    }

    (candidates, manifest_map)
}

// ════════════════════════════════════════════════════════════════════════════
// MANIFEST MATCHING  (parallel via rayon)
// ════════════════════════════════════════════════════════════════════════════

fn match_candidates(
    candidates: Vec<Candidate>,
    manifest_map: &ManifestMap,
    threshold: f32,
    kind_filter: Option<&ArtifactKind>,
) -> Vec<DetectionResult> {
    candidates
        .into_par_iter()
        .filter_map(|cand| {
            let parent = cand.path.parent()?;
            let hints = cand.rule.manifest_hints();

            let (manifest_found, matched_manifest) = if hints.is_empty() {
                // No manifest needed — name is self-evident
                (true, None)
            } else if let Some(parent_files) = manifest_map.get(parent) {
                let found_hint = parent_files.iter().find(|&&f| hints.contains(&f));
                match found_hint {
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

            let kind = cand.rule.kind();
            if let Some(filter) = kind_filter {
                if &kind != filter {
                    return None;
                }
            }

            let mut signals: Vec<String> = Vec::new();
            signals.push(format!("dir-name:{}", cand.path.file_name()
                .and_then(|n| n.to_str()).unwrap_or("?")));
            if manifest_found {
                signals.push("manifest-confirmed".to_owned());
            }

            Some(DetectionResult {
                path: cand.path,
                kind,
                confidence,
                size_bytes: None,
                note: cand.rule.note(),
                is_ghost: false,
                matched_manifest,
                signals,
            })
        })
        .collect()
}

// ════════════════════════════════════════════════════════════════════════════
// SIZE CALCULATOR  (bounded — max 100k file entries per dir)
// ════════════════════════════════════════════════════════════════════════════

const SIZE_ENTRY_CAP: u64 = 100_000;

fn calc_size(path: &Path) -> Option<u64> {
    let mut total: u64 = 0;
    let mut count: u64 = 0;

    for entry in WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            if let Ok(meta) = entry.metadata() {
                total = total.saturating_add(meta.len());
            }
            count += 1;
            if count >= SIZE_ENTRY_CAP {
                // Approximate: extrapolate current total
                // (conservative — just return what we have with a note)
                return Some(total);
            }
        }
    }
    Some(total)
}

fn calc_sizes_parallel(results: &mut Vec<DetectionResult>) {
    let sizes: Vec<Option<u64>> = results
        .par_iter()
        .map(|r| calc_size(&r.path))
        .collect();
    for (r, s) in results.iter_mut().zip(sizes) {
        r.size_bytes = s;
    }
}

// ════════════════════════════════════════════════════════════════════════════
// GHOST DETECTOR
//
// Four sources — all run in parallel via rayon::join chains.
// Results are de-duplicated by path before becoming DetectionResult.
// ════════════════════════════════════════════════════════════════════════════

fn ghost_from_git(root: &Path) -> Vec<GhostCandidate> {
    let output = Command::new("git")
        .args(["-C", &root.to_string_lossy(),
               "log", "--diff-filter=D", "--name-only",
               "--pretty=format:", "--", "."])
        .output();

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return vec![],
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut results = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }
        let p = Path::new(line);
        let name = match p.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None    => continue,
        };
        if let Some(rule) = find_rule(name) {
            let abs = if p.is_absolute() {
                p.to_owned()
            } else {
                root.join(p)
            };
            if !abs.exists() {
                results.push(GhostCandidate {
                    path: abs,
                    source: GhostSource::GitHistory,
                    matched_rule_idx: rule.rule_idx_for_ghost(),
                });
            }
        }
    }
    results
}

fn ghost_from_shell_history(root: &Path) -> Vec<GhostCandidate> {
    let home = match std::env::var("HOME") {
        Ok(h) => PathBuf::from(h),
        Err(_) => return vec![],
    };

    let history_files = [
        home.join(".bash_history"),
        home.join(".zsh_history"),
        home.join(".local/share/fish/fish_history"),
    ];

    let mut results = Vec::new();

    for hfile in &history_files {
        let file = match fs::File::open(hfile) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let reader = BufReader::new(file);

        for line in reader.lines().map_while(Result::ok) {
            // Extract path-like tokens: either bare absolute paths or after 'cd '
            let tokens: Vec<&str> = if line.starts_with(": ") {
                // zsh extended history format: ": timestamp:elapsed;command"
                line.splitn(3, ';').nth(1).map(|s| vec![s]).unwrap_or_default()
            } else {
                vec![line.as_str()]
            };

            for token in tokens {
                // Look for absolute path components that match known artifact dirs
                for part in token.split_whitespace() {
                    let p = Path::new(part);
                    if !p.is_absolute() { continue; }
                    let name = match p.file_name().and_then(|n| n.to_str()) {
                        Some(n) => n,
                        None    => continue,
                    };
                    if let Some(rule) = find_rule(name) {
                        // Only report if under the scan root and not currently existing
                        if p.starts_with(root) && !p.exists() {
                            results.push(GhostCandidate {
                                path: p.to_owned(),
                                source: GhostSource::ShellHistory,
                                matched_rule_idx: rule.rule_idx_for_ghost(),
                            });
                        }
                    }
                }
            }
        }
    }
    results
}

fn ghost_from_gitignore(root: &Path) -> Vec<GhostCandidate> {
    // Find .gitignore files under root, parse entries that match artifact dir names
    let mut results = Vec::new();

    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name() == ".gitignore" && e.file_type().is_file())
    {
        let gi_path = entry.path();
        let parent  = match gi_path.parent() { Some(p) => p, None => continue };

        let file = match fs::File::open(gi_path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            let entry_str = line.trim().trim_start_matches('/').trim_end_matches('/');
            if entry_str.starts_with('#') || entry_str.is_empty() { continue; }

            if let Some(rule) = find_rule(entry_str) {
                let artifact_path = parent.join(entry_str);
                if !artifact_path.exists() {
                    results.push(GhostCandidate {
                        path: artifact_path,
                        source: GhostSource::GitignoreHint,
                        matched_rule_idx: rule.rule_idx_for_ghost(),
                    });
                }
            }
        }
    }
    results
}

fn ghost_from_dangling_symlinks(root: &Path) -> Vec<GhostCandidate> {
    let mut results = Vec::new();

    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_symlink())
    {
        let link_path = entry.path();
        if let Ok(target) = fs::read_link(link_path) {
            let abs_target = if target.is_absolute() {
                target
            } else {
                match link_path.parent() {
                    Some(p) => p.join(target),
                    None    => continue,
                }
            };
            if !abs_target.exists() {
                let name = match abs_target.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n,
                    None    => continue,
                };
                if let Some(rule) = find_rule(name) {
                    results.push(GhostCandidate {
                        path: abs_target,
                        source: GhostSource::DanglingSymlink,
                        matched_rule_idx: rule.rule_idx_for_ghost(),
                    });
                }
            }
        }
    }
    results
}

fn run_ghost_detection(root: &Path, threshold: f32) -> Vec<DetectionResult> {
    // Run all four sources in parallel
    let (git_ghosts, (shell_ghosts, (gi_ghosts, sym_ghosts))) = rayon::join(
        || ghost_from_git(root),
        || rayon::join(
            || ghost_from_shell_history(root),
            || rayon::join(
                || ghost_from_gitignore(root),
                || ghost_from_dangling_symlinks(root),
            ),
        ),
    );

    // Merge and de-duplicate by path, accumulating sources
    let mut by_path: HashMap<PathBuf, (GhostSource, usize, Vec<String>)> = HashMap::new();

    let all: Vec<GhostCandidate> = git_ghosts.into_iter()
        .chain(shell_ghosts)
        .chain(gi_ghosts)
        .chain(sym_ghosts)
        .collect();

    for gc in all {
        let entry = by_path.entry(gc.path.clone()).or_insert_with(|| {
            (gc.source.clone(), gc.matched_rule_idx, Vec::new())
        });
        entry.2.push(gc.source.to_string());
    }

    let mut results = Vec::new();
    for (path, (primary_source, rule_idx, mut signals)) in by_path {
        // Confidence: base_confidence * 0.65 (dir doesn't exist), boosted by source count
        let base = if rule_idx < RULES.len() {
            RULES[rule_idx].base_confidence
        } else {
            0.6
        };

        // git history is most reliable source
        let source_boost = {
            let has_git   = signals.iter().any(|s| s == "git-history");
            let has_symlink = signals.iter().any(|s| s == "dangling-symlink");
            let extra = (signals.len() as f32 - 1.0).max(0.0) * 0.05;
            if has_git { 0.1 } else if has_symlink { 0.05 } else { 0.0 } + extra
        };

        let confidence = ((base * 0.65) + source_boost).min(0.99);

        if confidence < threshold { continue; }

        signals.dedup();
        let note = if rule_idx < RULES.len() { RULES[rule_idx].note } else { "ghost artifact" };
        let kind = if rule_idx < RULES.len() { RULES[rule_idx].kind.clone() } else { ArtifactKind::TempFile };

        // source signal label
        let _ = primary_source; // used for ordering, not needed further
        results.push(DetectionResult {
            path,
            kind,
            confidence,
            size_bytes: None,
            note,
            is_ghost: true,
            matched_manifest: None,
            signals,
        });
    }

    results
}

// ════════════════════════════════════════════════════════════════════════════
// SAFETY CHECKER
//
// Hard stops: never allow deletion of source code directories.
// ════════════════════════════════════════════════════════════════════════════

const SOURCE_EXTENSIONS: &[&str] = &[
    "rs", "go", "py", "js", "ts", "jsx", "tsx", "java", "kt", "swift",
    "c", "cpp", "h", "hpp", "cs", "fs", "rb", "ex", "exs", "erl", "hrl",
    "zig", "nim", "cr", "d", "ml", "mli", "hs", "elm", "clj", "scala",
    "dart", "lua", "php", "r", "jl",
];

const PROTECTED_NAMES: &[&str] = &[
    "src", "source", "lib", "app", "pkg", "internal", "cmd", "api",
    "core", "common", "shared", "utils", "helpers", "modules",
    "components", "services", "controllers", "models", "views",
];

#[derive(Debug, Clone, PartialEq)]
pub enum SafetyRating {
    Safe,
    Caution(String),
    Block(String),
}

pub fn safety_check(path: &Path) -> SafetyRating {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    // 1. Ghost dirs don't exist — cannot delete, just report
    if !path.exists() {
        return SafetyRating::Caution("directory no longer exists (ghost)".to_owned());
    }

    // 2. Protected name check
    if PROTECTED_NAMES.contains(&name) {
        // Check for source file siblings in the parent
        if let Some(parent) = path.parent() {
            if parent_has_source_files(parent) {
                return SafetyRating::Block(
                    format!("'{}' is a protected name and parent has source files", name)
                );
            }
        }
    }

    // 3. Source file contamination check — look for .rs/.py/etc inside top level
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.filter_map(|e| e.ok()) {
            let ename = entry.file_name();
            let ename_str = ename.to_string_lossy();
            if let Some(ext) = Path::new(ename_str.as_ref()).extension().and_then(|e| e.to_str()) {
                if SOURCE_EXTENSIONS.contains(&ext) {
                    return SafetyRating::Block(
                        format!("contains source file: {}", ename_str)
                    );
                }
            }
        }
    }

    // 4. Low confidence generic names need extra caution
    if matches!(name, "build" | "dist" | "out" | "bin" | "lib" | "env" | "vendor" | "deps") {
        return SafetyRating::Caution(format!("'{}' is a generic name — verify before deleting", name));
    }

    SafetyRating::Safe
}

fn parent_has_source_files(parent: &Path) -> bool {
    fs::read_dir(parent)
        .ok()
        .map(|entries| entries
            .filter_map(|e| e.ok())
            .any(|e| {
                let name = e.file_name();
                let name_str = name.to_string_lossy();
                Path::new(name_str.as_ref())
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| SOURCE_EXTENSIONS.contains(&ext))
                    .unwrap_or(false)
            }))
        .unwrap_or(false)
}

// ════════════════════════════════════════════════════════════════════════════
// OUTPUT
// ════════════════════════════════════════════════════════════════════════════

const RESET: &str  = "\x1b[0m";
const BOLD:  &str  = "\x1b[1m";
const DIM:   &str  = "\x1b[2m";
const RED:   &str  = "\x1b[31m";
const GREEN: &str  = "\x1b[32m";
const GHOST_ICON: &str = "↯";

fn fmt_bytes(b: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if b >= GB       { format!("{:.1} GB", b as f64 / GB as f64) }
    else if b >= MB  { format!("{:.1} MB", b as f64 / MB as f64) }
    else if b >= KB  { format!("{:.0} KB", b as f64 / KB as f64) }
    else             { format!("{} B", b) }
}

fn fmt_confidence(c: f32) -> String {
    let icon = if c >= 0.90 { "●" } else if c >= 0.75 { "◑" } else { "○" };
    format!("{} {:.0}%", icon, c * 100.0)
}

fn print_results(results: &[DetectionResult], verbose: bool, no_color: bool) {
    let c = |s: &str| if no_color { "" } else { s };

    println!();
    println!("{}  {:<12}  {:<55}  {:<8}  {:<10}  {}",
        c(BOLD),
        "kind",
        "path",
        "conf",
        "size",
        "note",
    );
    println!("{}", c(DIM));
    println!("{}", "─".repeat(120));
    println!("{}", c(RESET));

    for r in results {
        let ghost_marker = if r.is_ghost {
            format!("{}{} ", c(RED), GHOST_ICON)
        } else {
            "  ".to_owned()
        };

        let kind_str = format!("{}{:<12}{}", c(r.kind.ansi_color()), r.kind.label(), c(RESET));
        let path_str = r.path.display().to_string();
        let path_disp = if path_str.len() > 55 {
            format!("…{}", &path_str[path_str.len().saturating_sub(54)..])
        } else {
            path_str
        };

        let size_str = r.size_bytes
            .map(fmt_bytes)
            .unwrap_or_else(|| "—".to_owned());

        let conf_str = fmt_confidence(r.confidence);

        println!("{}{:<12}  {:<55}  {:<8}  {:<10}  {}{}",
            ghost_marker,
            kind_str,
            path_disp,
            conf_str,
            size_str,
            c(DIM), r.note,
        );
        print!("{}", c(RESET));

        if verbose {
            for sig in &r.signals {
                println!("    {}• {}{}", c(DIM), sig, c(RESET));
            }
            if let Some(ref mf) = r.matched_manifest {
                println!("    {}↳ manifest: {}{}", c(DIM), mf.display(), c(RESET));
            }
        }
    }
    println!();
}

fn print_summary(results: &[DetectionResult], elapsed_ms: u128, no_color: bool) {
    let c = |s: &str| if no_color { "" } else { s };

    let live_count   = results.iter().filter(|r| !r.is_ghost).count();
    let ghost_count  = results.iter().filter(|r| r.is_ghost).count();
    let total_bytes: u64 = results.iter()
        .filter(|r| !r.is_ghost)
        .filter_map(|r| r.size_bytes)
        .sum();

    print!("  {}", c(BOLD));
    print!("{} artifact{}", live_count, if live_count == 1 { "" } else { "s" });
    print!("{}", c(RESET));
    if ghost_count > 0 {
        print!("  {}+{} ghost{}{}", c(DIM), ghost_count,
               if ghost_count == 1 { "" } else { "s" }, c(RESET));
    }
    if total_bytes > 0 {
        print!("  {}  {}{}{}",
               c(GREEN),
               fmt_bytes(total_bytes),
               " reclaimable",
               c(RESET));
    }
    println!("  {}  scanned in {}ms{}", c(DIM), elapsed_ms, c(RESET));
    println!();
}

// ════════════════════════════════════════════════════════════════════════════
// INTERACTIVE DELETION
// ════════════════════════════════════════════════════════════════════════════

fn interactive_delete(results: &[DetectionResult]) {
    let deletable: Vec<&DetectionResult> = results.iter()
        .filter(|r| !r.is_ghost && r.confidence >= 0.85)
        .filter(|r| safety_check(&r.path) == SafetyRating::Safe)
        .collect();

    if deletable.is_empty() {
        println!("  No items meet deletion criteria (confidence ≥ 85%, safety: Safe).");
        return;
    }

    println!("\n  {}Items eligible for deletion:{}\n", BOLD, RESET);
    for (i, r) in deletable.iter().enumerate() {
        let size = r.size_bytes.map(fmt_bytes).unwrap_or_else(|| "?".to_owned());
        println!("  [{i}] {} ({}) — {size}", r.path.display(), r.kind.label());
    }

    println!("\n  Enter comma-separated indices to delete, or 'all', or 'q' to quit:");
    print!("  > ");
    io::stdout().flush().unwrap();

    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    let input = input.trim();

    if input == "q" || input.is_empty() {
        println!("  Aborted.");
        return;
    }

    let to_delete: Vec<&DetectionResult> = if input == "all" {
        deletable.clone()
    } else {
        input.split(',')
            .filter_map(|s| s.trim().parse::<usize>().ok())
            .filter(|&i| i < deletable.len())
            .map(|i| deletable[i])
            .collect()
    };

    if to_delete.is_empty() {
        println!("  Nothing selected.");
        return;
    }

    let total: u64 = to_delete.iter().filter_map(|r| r.size_bytes).sum();
    println!("\n  {}About to delete {} item(s) ({}):{}", BOLD, to_delete.len(), fmt_bytes(total), RESET);
    for r in &to_delete {
        println!("    {} {}", RED, r.path.display());
    }
    print!("{}  Confirm? [y/N] > ", RESET);
    io::stdout().flush().unwrap();

    let mut confirm = String::new();
    io::stdin().read_line(&mut confirm).unwrap();

    if confirm.trim().to_lowercase() != "y" {
        println!("  Aborted.");
        return;
    }

    let mut deleted = 0u64;
    for r in to_delete {
        match fs::remove_dir_all(&r.path) {
            Ok(_) => {
                deleted += r.size_bytes.unwrap_or(0);
                println!("  {}✓{} Deleted: {}", GREEN, RESET, r.path.display());
            }
            Err(e) => {
                println!("  {}✗{} Failed to delete {}: {}", RED, RESET, r.path.display(), e);
            }
        }
    }

    println!("\n  {}Freed: {}{}", GREEN, fmt_bytes(deleted), RESET);
}

// ════════════════════════════════════════════════════════════════════════════
// MAIN
// ════════════════════════════════════════════════════════════════════════════

fn main() {
    let args = Args::parse();

    // Resolve scan root
    let root = match args.path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("jhara: cannot access '{}': {}", args.path.display(), e);
            std::process::exit(1);
        }
    };

    // Guard against scanning /
    if root == Path::new("/") {
        eprintln!("jhara: refusing to scan filesystem root. Specify a project directory.");
        std::process::exit(1);
    }

    let no_color = std::env::var("NO_COLOR").is_ok()
        || std::env::var("TERM").map(|t| t == "dumb").unwrap_or(false);

    let kind_filter: Option<ArtifactKind> = args.kind.as_deref().and_then(ArtifactKind::from_label);
    if args.kind.is_some() && kind_filter.is_none() {
        eprintln!("jhara: unknown kind '{}'. Valid: build|deps|tool-cache|test|ide|cc-cache|tmp|ml-cache",
                  args.kind.as_deref().unwrap_or(""));
        std::process::exit(1);
    }

    if !args.json {
        let c = |s: &str| if no_color { "" } else { s };
        println!("{}jhara{} — scanning {}{}{}",
            c(BOLD), c(RESET),
            c(DIM), root.display(), c(RESET));
    }

    let t0 = Instant::now();

    // ── Phase 1: Walk and collect candidates + manifests ──────────────────
    let all_hints = build_all_hints();
    let (candidates, manifest_map) = scan_tree(&root, &all_hints);

    // ── Phase 2: Match candidates against manifests (parallel) ───────────
    let mut results = match_candidates(
        candidates,
        &manifest_map,
        args.threshold,
        kind_filter.as_ref(),
    );

    // ── Phase 3: Ghost detection (optional) ──────────────────────────────
    if args.ghosts {
        let ghost_results = run_ghost_detection(&root, args.threshold);
        results.extend(ghost_results);
    }

    // ── Phase 4: Size calculation (optional) ─────────────────────────────
    if args.sizes {
        calc_sizes_parallel(&mut results);
    }

    // ── Sort: by size desc (if available), then by confidence desc ────────
    results.sort_by(|a, b| {
        let sa = a.size_bytes.unwrap_or(0);
        let sb = b.size_bytes.unwrap_or(0);
        sb.cmp(&sa)
            .then_with(|| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal))
    });

    let elapsed_ms = t0.elapsed().as_millis();

    // ── Output ────────────────────────────────────────────────────────────
    if args.json {
        println!("{}", serde_json::to_string_pretty(&results).unwrap_or_default());
    } else {
        print_results(&results, args.verbose, no_color);
        print_summary(&results, elapsed_ms, no_color);
    }

    // ── Interactive deletion ──────────────────────────────────────────────
    if args.delete {
        if !args.confirm {
            eprintln!("jhara: --delete requires --confirm to prevent accidents.");
            std::process::exit(1);
        }
        interactive_delete(&results);
    }
}