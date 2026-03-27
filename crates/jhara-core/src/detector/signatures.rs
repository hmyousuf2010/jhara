// detector/signatures.rs
//
// The complete catalog of project signatures Jhara knows about.
//
// All entries use &'static str so the built-in database is stored in the
// read-only data segment with zero heap allocation. User-supplied JSON
// signatures (community additions) use the owned types in types.rs.
//
// Adding a new ecosystem:
//   1. Add a case to `Ecosystem` in types.rs
//   2. Add a `ProjectSignature` entry here
//   3. Add a test entry in tests/detector_tests.rs
//   4. Optionally add a JSON entry in data/signatures/ for users to see the schema

use crate::detector::types::{ArtifactPath, Ecosystem, ProjectSignature, SafetyTier};

// ─────────────────────────────────────────────────────────────────────────────
// Convenience constructors so the table below stays readable
// ─────────────────────────────────────────────────────────────────────────────

const fn safe(path: &'static str) -> ArtifactPath {
    ArtifactPath::new(path, SafetyTier::Safe)
}

const fn risky(path: &'static str) -> ArtifactPath {
    ArtifactPath::new(path, SafetyTier::Risky)
}

const fn blocked(path: &'static str) -> ArtifactPath {
    ArtifactPath::new(path, SafetyTier::Blocked)
}

const fn sig(
    filename: &'static str,
    ecosystem: Ecosystem,
    artifact_paths: &'static [ArtifactPath],
) -> ProjectSignature {
    ProjectSignature {
        filename,
        ecosystem,
        artifact_paths,
        content_key: None,
        priority: 0,
        stale_threshold_days: 60,
    }
}

const fn sig_with(
    filename: &'static str,
    ecosystem: Ecosystem,
    artifact_paths: &'static [ArtifactPath],
    content_key: &'static str,
    priority: i32,
    _stale_days: u32, // Renamed to _stale_days to fix unused variable warning
) -> ProjectSignature {
    ProjectSignature {
        filename,
        ecosystem,
        artifact_paths,
        content_key: Some(content_key),
        priority,
        stale_threshold_days: _stale_days, // Use the renamed variable
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Artifact path slices — must be &'static so ProjectSignature can hold them
// ─────────────────────────────────────────────────────────────────────────────

// Node.js
static NODE_ARTIFACTS: &[ArtifactPath] = &[
    safe("node_modules")
        .recovery("npm install")
        .size_mb(50, 2048)
        .prune(),
    safe("node_modules/.cache")
        .recovery("npm install")
        .size_mb(10, 500),
];

// Bun
static BUN_ARTIFACTS: &[ArtifactPath] = &[safe("node_modules")
    .recovery("bun install")
    .size_mb(100, 5120)
    .prune()];

// Deno
static DENO_ARTIFACTS: &[ArtifactPath] = &[safe("deno.lock").recovery("deno cache")];

// Storybook
static STORYBOOK_ARTIFACTS: &[ArtifactPath] = &[safe("storybook-static").size_mb(10, 500)];

// Go
static GO_ARTIFACTS: &[ArtifactPath] = &[
    safe("vendor").recovery("go mod vendor").prune(),
    safe("bin").recovery("go build").size_mb(10, 500).prune(),
    safe("pkg").recovery("go build").prune(),
];

// Python pip / uv / venv
static PYTHON_PIP_ARTIFACTS: &[ArtifactPath] = &[
    safe(".venv")
        .recovery("pip install -r requirements.txt")
        .size_mb(100, 2048)
        .prune(),
    safe("venv")
        .recovery("pip install -r requirements.txt")
        .size_mb(100, 2048)
        .prune(),
    safe("env")
        .recovery("pip install -r requirements.txt")
        .prune(),
    safe("__pycache__")
        .recovery("python (auto-generated)")
        .prune(),
    safe(".pytest_cache")
        .recovery("pytest (auto-generated)")
        .prune(),
    safe(".mypy_cache")
        .recovery("mypy (auto-generated)")
        .prune(),
    safe(".ruff_cache")
        .recovery("ruff (auto-generated)")
        .prune(),
];

// Python Poetry
static PYTHON_POETRY_ARTIFACTS: &[ArtifactPath] = &[
    safe(".venv").recovery("poetry install").size_mb(100, 3072),
    safe("__pycache__").recovery("python (auto-generated)"),
    safe(".pytest_cache").recovery("pytest (auto-generated)"),
    safe(".mypy_cache").recovery("mypy (auto-generated)"),
    safe("dist").recovery("poetry build"),
];

// Conda
static CONDA_ARTIFACTS: &[ArtifactPath] = &[ArtifactPath::new("envs", SafetyTier::Caution)
    .recovery("conda env create -f environment.yml")
    .size_mb(512, 10240)];

// Django
static DJANGO_ARTIFACTS: &[ArtifactPath] = &[
    safe("__pycache__").recovery("python (auto-generated)"),
    safe(".pytest_cache").recovery("pytest (auto-generated)"),
    safe("staticfiles").recovery("python manage.py collectstatic"),
    safe("media").recovery("user-uploaded content — review before deleting"),
];

// Ruby
static RUBY_ARTIFACTS: &[ArtifactPath] = &[
    safe("vendor/bundle")
        .recovery("bundle install")
        .size_mb(50, 2048),
    safe(".bundle").recovery("bundle install"),
];

// Rails
static RAILS_ARTIFACTS: &[ArtifactPath] = &[
    safe("tmp").recovery("rails server starts fresh"),
    safe("log").recovery("rails server starts fresh"),
    safe("public/assets").recovery("rails assets:precompile"),
    safe("public/packs").recovery("rails webpacker:compile"),
];

// PHP / Composer
static PHP_COMPOSER_ARTIFACTS: &[ArtifactPath] = &[safe("vendor")
    .recovery("composer install")
    .size_mb(10, 500)
    .prune()];

// Laravel
static LARAVEL_ARTIFACTS: &[ArtifactPath] = &[
    safe("storage/framework/cache").recovery("php artisan cache:clear"),
    safe("storage/framework/sessions").recovery("php artisan session:flush"),
    safe("storage/logs").recovery("php artisan log:clear"),
    safe("bootstrap/cache").recovery("php artisan config:clear"),
];

// Java Maven
static MAVEN_ARTIFACTS: &[ArtifactPath] = &[safe("target")
    .recovery("mvn compile")
    .size_mb(100, 5120)
    .prune()];

// Java Gradle
static GRADLE_ARTIFACTS: &[ArtifactPath] = &[
    safe("build")
        .recovery("gradle build")
        .size_mb(200, 10240)
        .prune(),
    safe(".gradle").recovery("gradle build").prune(),
];

// Kotlin
static KOTLIN_ARTIFACTS: &[ArtifactPath] = &[
    safe("build").recovery("gradle build").size_mb(200, 10240),
    safe(".gradle").recovery("gradle build"),
    safe(".kotlin").recovery("gradle build"),
];

// Scala sbt
static SCALA_ARTIFACTS: &[ArtifactPath] = &[
    safe("target").recovery("sbt compile").size_mb(100, 4096),
    safe("project/target").recovery("sbt compile"),
];

// Clojure (Leiningen)
static CLOJURE_LEIN_ARTIFACTS: &[ArtifactPath] = &[
    safe(".cpcache").recovery("lein deps"),
    safe("target").recovery("lein compile"),
];

// Clojure (deps.edn)
static CLOJURE_DEPS_ARTIFACTS: &[ArtifactPath] = &[safe(".cpcache").recovery("clj -P")];

// Groovy
static GROOVY_ARTIFACTS: &[ArtifactPath] = &[safe("build").recovery("groovyc").size_mb(10, 1024)];

// Rust
static RUST_ARTIFACTS: &[ArtifactPath] = &[safe("target")
    .recovery("cargo build")
    .size_mb(500, 15360)
    .prune()];

// C / C++ (CMake)
static CMAKE_ARTIFACTS: &[ArtifactPath] = &[
    safe("build")
        .recovery("cmake --build build")
        .size_mb(100, 10240),
    safe("cmake-build-debug").recovery("cmake --build ."),
    safe("cmake-build-release").recovery("cmake --build ."),
    safe("out").recovery("cmake --build ."),
];

// C / C++ (Makefile)
static MAKEFILE_ARTIFACTS: &[ArtifactPath] = &[
    safe("build").recovery("make").size_mb(50, 5120),
    safe("out").recovery("make"),
];

// Zig
static ZIG_ARTIFACTS: &[ArtifactPath] = &[
    safe("zig-cache").recovery("zig build").size_mb(50, 3072),
    safe("zig-out").recovery("zig build"),
];

// Elixir / Mix
static ELIXIR_ARTIFACTS: &[ArtifactPath] = &[
    safe("_build").recovery("mix compile").size_mb(50, 1024),
    safe("deps").recovery("mix deps.get").size_mb(50, 512),
];

// Erlang / rebar3
static REBAR_ARTIFACTS: &[ArtifactPath] = &[
    safe("_build").recovery("rebar3 compile"),
    safe("_deps").recovery("rebar3 get-deps"),
];

// Haskell / Stack
static HASKELL_STACK_ARTIFACTS: &[ArtifactPath] = &[safe(".stack-work")
    .recovery("stack build")
    .size_mb(200, 8192)];

// OCaml / Dune
static OCAML_ARTIFACTS: &[ArtifactPath] =
    &[safe("_build").recovery("dune build").size_mb(100, 5120)];

// F#
static FSHARP_ARTIFACTS: &[ArtifactPath] = &[
    safe("bin").recovery("dotnet build"),
    safe("obj").recovery("dotnet build").size_mb(50, 1024),
];

// Swift (SPM standalone project)
static SWIFT_SPM_ARTIFACTS: &[ArtifactPath] =
    &[safe(".build").recovery("swift build").size_mb(100, 5120)];

// Dart / Flutter
static DART_ARTIFACTS: &[ArtifactPath] = &[
    safe(".dart_tool").recovery("dart pub get"),
    safe("build").recovery("flutter build").size_mb(100, 4096),
    safe("ios/Pods").recovery("pod install").size_mb(200, 2048),
];

// R
static R_ARTIFACTS: &[ArtifactPath] = &[
    ArtifactPath::new(".Rhistory", SafetyTier::Caution).recovery("N/A — session history"),
    ArtifactPath::new(".RData", SafetyTier::Caution)
        .recovery("N/A — workspace data. Review before deleting."),
];

// Nim
static NIM_ARTIFACTS: &[ArtifactPath] =
    &[safe("nimcache").recovery("nimble build").size_mb(10, 512)];

// Crystal
static CRYSTAL_ARTIFACTS: &[ArtifactPath] = &[
    safe("lib").recovery("shards install").size_mb(20, 512),
    safe(".shards").recovery("shards install"),
];

// D (dub)
static D_ARTIFACTS: &[ArtifactPath] = &[safe(".dub").recovery("dub build").size_mb(10, 1024)];

// React Native (bare / Expo)
static REACT_NATIVE_ARTIFACTS: &[ArtifactPath] = &[
    safe("android/app/build")
        .recovery("cd android && ./gradlew build")
        .size_mb(500, 8192),
    safe("android/.cxx").recovery("cd android && ./gradlew build"),
    safe("ios/build").recovery("xcodebuild").size_mb(500, 5120),
    safe("ios/Pods")
        .recovery("cd ios && pod install")
        .size_mb(200, 3072),
    safe(".expo").recovery("expo start"),
    safe("node_modules")
        .recovery("npm install")
        .size_mb(200, 4096),
];

// Flutter
static FLUTTER_ARTIFACTS: &[ArtifactPath] = &[
    safe(".dart_tool").recovery("dart pub get"),
    safe("build").recovery("flutter build").size_mb(100, 5120),
    safe("ios/Pods")
        .recovery("cd ios && pod install")
        .size_mb(200, 2048),
    safe("android/app/build")
        .recovery("./gradlew build")
        .size_mb(200, 4096),
];

// Ionic / Capacitor
static CAPACITOR_ARTIFACTS: &[ArtifactPath] = &[
    safe("www").recovery("npm run build").size_mb(50, 1024),
    safe("ios/App/Pods").recovery("pod install"),
];

// .NET
static DOTNET_ARTIFACTS: &[ArtifactPath] = &[
    safe("bin").recovery("dotnet build").prune(),
    safe("obj").recovery("dotnet build").prune(),
];

// Mobile (Android)
static ANDROID_ARTIFACTS: &[ArtifactPath] = &[
    safe("app/build")
        .recovery("./gradlew build")
        .size_mb(500, 20480),
    safe(".cxx").recovery("./gradlew build"),
];

// Unreal Engine
static UNREAL_ARTIFACTS: &[ArtifactPath] = &[
    safe("Intermediate")
        .recovery("Unreal builds regenerate these")
        .prune(),
    safe("Saved")
        .recovery("Unreal builds regenerate some of these")
        .prune(),
    safe("Binaries")
        .recovery("Unreal builds regenerate these")
        .prune(),
];

// Visual Studio
static VISUAL_STUDIO_ARTIFACTS: &[ArtifactPath] = &[
    safe(".vs")
        .recovery("Visual Studio regenerates these")
        .prune(),
    safe(".ipch")
        .recovery("Visual Studio regenerates these")
        .prune(),
];

// Kotlin Multiplatform Mobile
static KMM_ARTIFACTS: &[ArtifactPath] = &[
    safe("build").recovery("gradle build").size_mb(200, 5120),
    safe(".kotlin").recovery("gradle build"),
    safe("iosApp/build").recovery("gradle build"),
];

// NativeScript
static NATIVESCRIPT_ARTIFACTS: &[ArtifactPath] = &[
    safe("platforms").recovery("ns build").size_mb(200, 4096),
    safe("hooks").recovery("ns build"),
    safe("node_modules")
        .recovery("npm install")
        .size_mb(200, 4096),
];

// Terraform
// .terraform/     → Safe (provider plugins, re-downloadable)
// *.tfstate       → Blocked (live infrastructure state, never touch)
static TERRAFORM_ARTIFACTS: &[ArtifactPath] = &[
    safe(".terraform")
        .recovery("terraform init")
        .size_mb(50, 500),
    safe(".terraform.lock.hcl").recovery("terraform init"),
    blocked("terraform.tfstate"),
    blocked("terraform.tfstate.backup"),
];

// Pulumi
static PULUMI_ARTIFACTS: &[ArtifactPath] = &[ArtifactPath::new(".pulumi", SafetyTier::Caution)
    .recovery("pulumi login")
    .size_mb(10, 500)];

// Vagrant
static VAGRANT_ARTIFACTS: &[ArtifactPath] = &[
    // .vagrant/machines tracks running VMs. Deleting orphans those VMs.
    risky(".vagrant/machines").recovery("vagrant up (re-provisions the VM)"),
];

// Bazel
static BAZEL_ARTIFACTS: &[ArtifactPath] = &[
    safe("bazel-bin").prune(),
    safe("bazel-out").prune(),
    safe("bazel-testlogs").prune(),
    safe("bazel-<project_name>").prune(),
];

// ML / Data Science
static ML_ARTIFACTS: &[ArtifactPath] = &[
    safe("mlruns").recovery("mlflow runs"),
    safe("wandb").recovery("weights & biases logs"),
    safe(".triton").recovery("triton cache"),
];

// Monorepo: Turborepo
static TURBO_ARTIFACTS: &[ArtifactPath] = &[
    safe(".turbo").recovery("turbo build").size_mb(10, 2048),
    safe("node_modules/.cache/turbo").recovery("turbo build"),
];

// Monorepo: Nx
static NX_ARTIFACTS: &[ArtifactPath] = &[
    safe(".nx/cache").recovery("nx build").size_mb(100, 10240),
    safe("node_modules/.cache/nx").recovery("nx build"),
];

// ─────────────────────────────────────────────────────────────────────────────
// Global cache paths — keyed by a sentinel name, looked up by home-relative path
// ─────────────────────────────────────────────────────────────────────────────

static NPM_CACHE_ARTIFACTS: &[ArtifactPath] =
    &[ArtifactPath::new(".npm/_cacache", SafetyTier::Safe)
        .global()
        .recovery("npm install (auto-regenerated)")
        .size_mb(200, 4096)];

static YARN_CACHE_ARTIFACTS: &[ArtifactPath] =
    &[
        ArtifactPath::new("Library/Caches/Yarn/v6", SafetyTier::Safe)
            .global()
            .recovery("yarn install (auto-regenerated)")
            .size_mb(200, 4096),
    ];

static PNPM_STORE_ARTIFACTS: &[ArtifactPath] =
    &[ArtifactPath::new("Library/pnpm/store/v3", SafetyTier::Safe)
        .global()
        .recovery("pnpm install (auto-regenerated)")
        .size_mb(500, 10240)];

static CARGO_CACHE_ARTIFACTS: &[ArtifactPath] = &[
    ArtifactPath::new(".cargo/registry", SafetyTier::Safe)
        .global()
        .recovery("cargo build (auto-fetched)")
        .size_mb(500, 10240),
    ArtifactPath::new(".cargo/git", SafetyTier::Safe)
        .global()
        .recovery("cargo build (auto-fetched)")
        .size_mb(100, 2048),
];

static PIP_CACHE_ARTIFACTS: &[ArtifactPath] = &[
    ArtifactPath::new("Library/Caches/pip", SafetyTier::Safe)
        .global()
        .recovery("pip install (auto-downloaded)")
        .size_mb(200, 10240),
    ArtifactPath::new(".cache/uv", SafetyTier::Safe)
        .global()
        .recovery("uv pip install (auto-downloaded)"),
];

static POETRY_CACHE_ARTIFACTS: &[ArtifactPath] =
    &[
        ArtifactPath::new("Library/Caches/pypoetry", SafetyTier::Safe)
            .global()
            .recovery("poetry install (auto-regenerated)")
            .size_mb(200, 5120),
    ];

static MAVEN_CACHE_ARTIFACTS: &[ArtifactPath] =
    &[ArtifactPath::new(".m2/repository", SafetyTier::Safe)
        .global()
        .recovery("mvn install (auto-downloaded)")
        .size_mb(500, 5120)];

static GRADLE_CACHE_ARTIFACTS: &[ArtifactPath] = &[
    ArtifactPath::new(".gradle/caches", SafetyTier::Safe)
        .global()
        .recovery("gradle build (auto-regenerated)")
        .size_mb(500, 15360),
    ArtifactPath::new(".gradle/daemon", SafetyTier::Safe)
        .global()
        .recovery("gradle build (auto-started)"),
    ArtifactPath::new(".gradle/wrapper", SafetyTier::Safe)
        .global()
        .recovery("gradle wrapper (auto-downloaded)"),
];

static RUBYGEMS_ARTIFACTS: &[ArtifactPath] = &[
    ArtifactPath::new(".gem", SafetyTier::Safe)
        .global()
        .recovery("gem install (auto-downloaded)")
        .size_mb(200, 2048),
    ArtifactPath::new(".rbenv/versions", SafetyTier::Caution)
        .global()
        .recovery("rbenv install <version>")
        .size_mb(200, 2048),
];

static COMPOSER_CACHE_ARTIFACTS: &[ArtifactPath] =
    &[ArtifactPath::new(".composer/cache", SafetyTier::Safe)
        .global()
        .recovery("composer install (auto-downloaded)")
        .size_mb(100, 1024)];

static HOMEBREW_CACHE_ARTIFACTS: &[ArtifactPath] =
    &[
        ArtifactPath::new("Library/Caches/Homebrew", SafetyTier::Safe)
            .global()
            .recovery("brew install (auto-downloaded)")
            .size_mb(200, 10240),
    ];

static COCOAPODS_CACHE_ARTIFACTS: &[ArtifactPath] =
    &[
        ArtifactPath::new("Library/Caches/CocoaPods", SafetyTier::Safe)
            .global()
            .recovery("pod install (auto-downloaded)")
            .size_mb(200, 5120),
    ];

static CARTHAGE_CACHE_ARTIFACTS: &[ArtifactPath] =
    &[
        ArtifactPath::new("Library/Caches/org.carthage.CarthageKit", SafetyTier::Safe)
            .global()
            .recovery("carthage update (auto-downloaded)")
            .size_mb(100, 5120),
    ];

static GO_CACHE_ARTIFACTS: &[ArtifactPath] = &[
    ArtifactPath::new("Library/Caches/go-build", SafetyTier::Safe)
        .global()
        .recovery("go build (auto-regenerated)")
        .size_mb(500, 8192),
    ArtifactPath::new("go/pkg/mod", SafetyTier::Safe)
        .global()
        .recovery("go get (auto-downloaded)")
        .size_mb(500, 8192),
];

static SWIFT_PM_CACHE_ARTIFACTS: &[ArtifactPath] = &[
    ArtifactPath::new("Library/Caches/org.swift.swiftpm", SafetyTier::Safe)
        .global()
        .recovery("swift package resolve (auto-downloaded)")
        .size_mb(100, 5120),
    ArtifactPath::new("Library/org.swift.swiftpm", SafetyTier::Safe)
        .global()
        .recovery("swift package resolve"),
];

static PUB_CACHE_ARTIFACTS: &[ArtifactPath] = &[ArtifactPath::new(".pub-cache", SafetyTier::Safe)
    .global()
    .recovery("dart pub get (auto-downloaded)")
    .size_mb(100, 4096)];

static MIX_CACHE_ARTIFACTS: &[ArtifactPath] = &[ArtifactPath::new(".mix", SafetyTier::Safe)
    .global()
    .recovery("mix deps.get (auto-downloaded)")
    .size_mb(50, 1024)];

static STACK_CACHE_ARTIFACTS: &[ArtifactPath] = &[ArtifactPath::new(".stack", SafetyTier::Safe)
    .global()
    .recovery("stack build (auto-downloaded)")
    .size_mb(500, 8192)];

// Xcode: DerivedData (global, mapped back to projects by XcodeResolver)
static XCODE_DERIVED_DATA_ARTIFACTS: &[ArtifactPath] =
    &[
        ArtifactPath::new("Library/Developer/Xcode/DerivedData", SafetyTier::Safe)
            .global()
            .recovery("Xcode rebuilds automatically")
            .size_mb(1024, 51200),
    ];

// Xcode Archives — Caution because deleting prevents crash symbolication
static XCODE_ARCHIVES_ARTIFACTS: &[ArtifactPath] =
    &[
        ArtifactPath::new("Library/Developer/Xcode/Archives", SafetyTier::Caution)
            .global()
            .recovery("Cannot be regenerated — needed for crash symbolication of live releases")
            .size_mb(100, 20480),
    ];

// iOS Simulator caches (Safe) vs Devices (Caution — contain installed app sandboxes)
static XCODE_SIMULATORS_ARTIFACTS: &[ArtifactPath] = &[
    ArtifactPath::new("Library/Developer/CoreSimulator/Caches", SafetyTier::Safe)
        .global()
        .recovery("Downloaded automatically when simulator runtime is used")
        .size_mb(1024, 20480),
    ArtifactPath::new(
        "Library/Developer/CoreSimulator/Devices",
        SafetyTier::Caution,
    )
    .global()
    .recovery("Simulator devices must be re-created and apps re-installed"),
];

// DeviceSupport: extracted automatically when a device is connected
static XCODE_DEVICE_SUPPORT_ARTIFACTS: &[ArtifactPath] = &[ArtifactPath::new(
    "Library/Developer/Xcode/iOS DeviceSupport",
    SafetyTier::Safe,
)
.global()
.recovery("Extracted automatically when the device is reconnected")
.size_mb(500, 10240)];

// ─────────────────────────────────────────────────────────────────────────────
// The full signature table
// ─────────────────────────────────────────────────────────────────────────────

/// Every project-level signature. Evaluated against each directory during
/// traversal. The order here does not affect correctness — detection uses
/// a filename-indexed hash map — but keeping related ecosystems adjacent
/// makes the list easier to maintain.
pub static PROJECT_SIGNATURES: &[ProjectSignature] = &[
    // ── Node / JS ──────────────────────────────────────────────────────────
    sig("package.json", Ecosystem::NodeJs, NODE_ARTIFACTS),
    sig("package-lock.json", Ecosystem::NodeJs, NODE_ARTIFACTS),
    sig("yarn.lock", Ecosystem::NodeJs, NODE_ARTIFACTS),
    sig("pnpm-lock.yaml", Ecosystem::NodeJs, NODE_ARTIFACTS),
    sig(".storybook", Ecosystem::NodeJs, STORYBOOK_ARTIFACTS),
    sig("bun.lockb", Ecosystem::Bun, BUN_ARTIFACTS),
    sig("deno.json", Ecosystem::Deno, DENO_ARTIFACTS),
    sig("deno.jsonc", Ecosystem::Deno, DENO_ARTIFACTS),
    // ── Python ─────────────────────────────────────────────────────────────
    sig(
        "requirements.txt",
        Ecosystem::PythonPip,
        PYTHON_PIP_ARTIFACTS,
    ),
    sig(
        "pyproject.toml",
        Ecosystem::PythonPoetry,
        PYTHON_POETRY_ARTIFACTS,
    ),
    sig("environment.yml", Ecosystem::PythonConda, CONDA_ARTIFACTS),
    sig("setup.py", Ecosystem::PythonPip, PYTHON_PIP_ARTIFACTS),
    // manage.py distinguishes Django from plain Python; higher priority wins
    sig_with(
        "manage.py",
        Ecosystem::Django,
        DJANGO_ARTIFACTS,
        "django",
        10,
        45,
    ),
    // ── Ruby ───────────────────────────────────────────────────────────────
    sig("Gemfile", Ecosystem::Ruby, RUBY_ARTIFACTS),
    // config/routes.rb identifies a Rails app; higher priority than Gemfile alone
    sig_with(
        "config/routes.rb",
        Ecosystem::Rails,
        RAILS_ARTIFACTS,
        "routes.draw",
        10,
        60,
    ),
    // ── PHP ────────────────────────────────────────────────────────────────
    sig("composer.json", Ecosystem::Php, PHP_COMPOSER_ARTIFACTS),
    // artisan file is unique to Laravel
    sig_with(
        "artisan",
        Ecosystem::Laravel,
        LARAVEL_ARTIFACTS,
        "laravel",
        10,
        30,
    ),
    // ── Java / JVM ─────────────────────────────────────────────────────────
    sig("pom.xml", Ecosystem::JavaMaven, MAVEN_ARTIFACTS),
    sig("build.gradle", Ecosystem::JavaGradle, GRADLE_ARTIFACTS),
    sig("build.gradle.kts", Ecosystem::Kotlin, KOTLIN_ARTIFACTS),
    sig("build.sbt", Ecosystem::Scala, SCALA_ARTIFACTS),
    sig("project.clj", Ecosystem::Clojure, CLOJURE_LEIN_ARTIFACTS),
    sig("deps.edn", Ecosystem::Clojure, CLOJURE_DEPS_ARTIFACTS),
    // ── Go ─────────────────────────────────────────────────────────────────
    sig("go.mod", Ecosystem::Go, GO_ARTIFACTS),
    // ── Rust ───────────────────────────────────────────────────────────────
    sig("Cargo.toml", Ecosystem::Rust, RUST_ARTIFACTS),
    // ── C / C++ ────────────────────────────────────────────────────────────
    sig("CMakeLists.txt", Ecosystem::CCpp, CMAKE_ARTIFACTS),
    sig("Makefile", Ecosystem::CCpp, MAKEFILE_ARTIFACTS),
    sig(
        "meson.build",
        Ecosystem::CCpp,
        &[safe("builddir").recovery("meson setup builddir")],
    ),
    // ── Zig ────────────────────────────────────────────────────────────────
    sig("build.zig", Ecosystem::Zig, ZIG_ARTIFACTS),
    // ── Elixir ─────────────────────────────────────────────────────────────
    sig("mix.exs", Ecosystem::Elixir, ELIXIR_ARTIFACTS),
    sig("rebar.config", Ecosystem::Elixir, REBAR_ARTIFACTS),
    // ── Haskell ────────────────────────────────────────────────────────────
    sig("stack.yaml", Ecosystem::Haskell, HASKELL_STACK_ARTIFACTS),
    // ── OCaml ──────────────────────────────────────────────────────────────
    sig("dune-project", Ecosystem::OCaml, OCAML_ARTIFACTS),
    sig("*.opam", Ecosystem::OCaml, OCAML_ARTIFACTS),
    // ── F# ─────────────────────────────────────────────────────────────────
    sig("*.fsproj", Ecosystem::FSharp, FSHARP_ARTIFACTS),
    // ── Swift (SPM standalone) ─────────────────────────────────────────────
    sig("Package.swift", Ecosystem::SwiftSpm, SWIFT_SPM_ARTIFACTS),
    // ── Dart / Flutter ─────────────────────────────────────────────────────
    sig("pubspec.yaml", Ecosystem::Dart, DART_ARTIFACTS),
    sig("melos.yaml", Ecosystem::Dart, &[]), // Melos detected by MonorepoResolver
    // ── R ──────────────────────────────────────────────────────────────────
    sig("*.Rproj", Ecosystem::R, R_ARTIFACTS),
    // ── Nim ────────────────────────────────────────────────────────────────
    sig("*.nimble", Ecosystem::Nim, NIM_ARTIFACTS),
    // ── Crystal ────────────────────────────────────────────────────────────
    sig("shard.yml", Ecosystem::Crystal, CRYSTAL_ARTIFACTS),
    // ── D ──────────────────────────────────────────────────────────────────
    sig("dub.json", Ecosystem::D, D_ARTIFACTS),
    sig("dub.sdl", Ecosystem::D, D_ARTIFACTS),
    // ── Groovy ─────────────────────────────────────────────────────────────
    sig("*.groovy", Ecosystem::Groovy, GROOVY_ARTIFACTS),
    // ── Mobile: React Native ───────────────────────────────────────────────
    // app.json with "react-native" anywhere in it → React Native
    sig_with(
        "app.json",
        Ecosystem::ReactNative,
        REACT_NATIVE_ARTIFACTS,
        "react-native",
        20,
        30,
    ),
    // ── Mobile: Capacitor ──────────────────────────────────────────────────
    sig(
        "capacitor.config.json",
        Ecosystem::Capacitor,
        CAPACITOR_ARTIFACTS,
    ),
    sig(
        "ionic.config.json",
        Ecosystem::Capacitor,
        CAPACITOR_ARTIFACTS,
    ),
    // ── Mobile: Flutter ────────────────────────────────────────────────────
    // pubspec.yaml with "flutter:" → Flutter (higher priority than plain Dart)
    sig_with(
        "pubspec.yaml",
        Ecosystem::Flutter,
        FLUTTER_ARTIFACTS,
        "flutter:",
        10,
        30,
    ),
    // ── Mobile: Android bare Gradle project ────────────────────────────────
    sig_with(
        "settings.gradle",
        Ecosystem::AndroidGradle,
        ANDROID_ARTIFACTS,
        "android",
        5,
        45,
    ),
    // ── Mobile: KMM ────────────────────────────────────────────────────────
    sig_with(
        "build.gradle.kts",
        Ecosystem::Kmm,
        KMM_ARTIFACTS,
        "multiplatform",
        15,
        45,
    ),
    // ── Mobile: NativeScript ───────────────────────────────────────────────
    sig(
        "nsconfig.json",
        Ecosystem::NativeScript,
        NATIVESCRIPT_ARTIFACTS,
    ),
    // ── DevOps ─────────────────────────────────────────────────────────────
    sig("*.tf", Ecosystem::Terraform, TERRAFORM_ARTIFACTS),
    sig("Pulumi.yaml", Ecosystem::Pulumi, PULUMI_ARTIFACTS),
    sig("Vagrantfile", Ecosystem::Vagrant, VAGRANT_ARTIFACTS),
    // ── Monorepo tooling ───────────────────────────────────────────────────
    // These trigger additional MonorepoResolver analysis; artifact lists
    // may be extended after resolution.
    sig("turbo.json", Ecosystem::Turborepo, TURBO_ARTIFACTS),
    sig("nx.json", Ecosystem::Nx, NX_ARTIFACTS),
    sig("pnpm-workspace.yaml", Ecosystem::PnpmWorkspace, &[]),
    sig("lerna.json", Ecosystem::Lerna, &[]),
    // ── .NET ──────────────────────────────────────────────────────────────
    sig("*.sln", Ecosystem::DotNet, DOTNET_ARTIFACTS),
    sig("*.csproj", Ecosystem::DotNet, DOTNET_ARTIFACTS),
    // ── Bazel ─────────────────────────────────────────────────────────────
    sig(".bazelrc", Ecosystem::Bazel, BAZEL_ARTIFACTS),
    sig("BUILD", Ecosystem::Bazel, BAZEL_ARTIFACTS),
    sig("WORKSPACE", Ecosystem::Bazel, BAZEL_ARTIFACTS),
    // ── ML / Data Science ──────────────────────────────────────────────────
    sig("mlruns", Ecosystem::MLflow, ML_ARTIFACTS),
    sig("wandb", Ecosystem::Wandb, ML_ARTIFACTS),
    // ── Unreal ────────────────────────────────────────────────────────────
    sig("*.uproject", Ecosystem::UnrealEngine, UNREAL_ARTIFACTS),
    // ── Visual Studio ─────────────────────────────────────────────────────
    sig(".vs", Ecosystem::VisualStudio, VISUAL_STUDIO_ARTIFACTS),
];

/// Global cache signatures — keyed by a home-relative directory sentinel.
/// The `ProjectDetector` checks for these paths relative to `$HOME` at scan
/// startup rather than finding them via signature file detection.
pub static GLOBAL_CACHE_SIGNATURES: &[ProjectSignature] = &[
    // Sentinel filenames are arbitrary for global caches — the detector uses
    // the artifact paths directly rather than looking for these filenames.
    // We reuse the ProjectSignature type for consistency.
    sig("_cacache", Ecosystem::NpmCache, NPM_CACHE_ARTIFACTS),
    sig("Yarn", Ecosystem::YarnCache, YARN_CACHE_ARTIFACTS),
    sig("pnpm", Ecosystem::PnpmStore, PNPM_STORE_ARTIFACTS),
    sig("cargo", Ecosystem::CargoCache, CARGO_CACHE_ARTIFACTS),
    sig("pip", Ecosystem::PipCache, PIP_CACHE_ARTIFACTS),
    sig("pypoetry", Ecosystem::PoetryCache, POETRY_CACHE_ARTIFACTS),
    sig("m2", Ecosystem::MavenCache, MAVEN_CACHE_ARTIFACTS),
    sig("gradle", Ecosystem::GradleCache, GRADLE_CACHE_ARTIFACTS),
    sig("gem", Ecosystem::RubyGems, RUBYGEMS_ARTIFACTS),
    sig("composer", Ecosystem::Composer, COMPOSER_CACHE_ARTIFACTS),
    sig("Homebrew", Ecosystem::Homebrew, HOMEBREW_CACHE_ARTIFACTS),
    sig(
        "CocoaPods",
        Ecosystem::CocoaPodsCache,
        COCOAPODS_CACHE_ARTIFACTS,
    ),
    sig(
        "CarthageKit",
        Ecosystem::CarthageCache,
        CARTHAGE_CACHE_ARTIFACTS,
    ),
    sig("go-build", Ecosystem::GoCache, GO_CACHE_ARTIFACTS),
    sig(
        "org.swift.swiftpm",
        Ecosystem::SwiftPmCache,
        SWIFT_PM_CACHE_ARTIFACTS,
    ),
    sig("pub-cache", Ecosystem::PubCache, PUB_CACHE_ARTIFACTS),
    sig("mix", Ecosystem::MixCache, MIX_CACHE_ARTIFACTS),
    sig("stack", Ecosystem::StackCache, STACK_CACHE_ARTIFACTS),
    sig(
        "DerivedData",
        Ecosystem::Xcode,
        XCODE_DERIVED_DATA_ARTIFACTS,
    ),
    sig(
        "Archives",
        Ecosystem::XcodeArchives,
        XCODE_ARCHIVES_ARTIFACTS,
    ),
    sig(
        "CoreSimulator",
        Ecosystem::XcodeSimulators,
        XCODE_SIMULATORS_ARTIFACTS,
    ),
    sig(
        "iOS DeviceSupport",
        Ecosystem::XcodeDeviceSupport,
        XCODE_DEVICE_SUPPORT_ARTIFACTS,
    ),
];
