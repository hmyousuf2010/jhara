<div align="center">
  <h1>Jhara (ঝরা)</h1>
  <p><b>A developer disk cleaner that understands your projects — native macOS, cross-platform core.</b></p>

  [![Status](https://img.shields.io/badge/Status-Pre--Alpha%20%2F%20Under%20Development-red.svg)](#development-status)
  [![License](https://img.shields.io/badge/License-Apache_2.0-blue.svg?logo=apache)](LICENSE)
  <br />
  [![Rust](https://img.shields.io/badge/Rust-1.77%2B-orange?logo=rust&logoColor=white)](#)
  [![Swift](https://img.shields.io/badge/Swift-6.0-orange?logo=swift&logoColor=white)](#)
  <br />
  [![macOS](https://img.shields.io/badge/macOS-14%2B-lightgray?logo=apple&logoColor=white)](#)
  [![Linux](https://img.shields.io/badge/Linux-Supported-lightgray?logo=linux&logoColor=white)](#)
  [![Windows](https://img.shields.io/badge/Windows-Supported-lightgray?logo=windows&logoColor=white)](#)

  <br />
</div>

Jhara is a developer disk cleaner built around one idea: a developer's disk fills up in predictable, structured ways, and a tool that understands those patterns can clean it safely. Where a general-purpose disk manager sees an opaque directory tree, Jhara sees a Rust project with a stale `target/` folder, a Node monorepo with hoisted `node_modules`, or an Xcode workspace accumulating DerivedData for two years.

The name comes from the Bengali word ঝরা, meaning to shed or fall away — like leaves falling from a tree at the end of a season. Files you no longer need, discarded cleanly.

> **Status:** Jhara is in early development. The monorepo scaffold, web dashboard structure, and macOS app shell exist. The Rust core and scanner engine are under active development. If you are here for the tool, check back in a few months. If you are here to contribute, the roadmap describes exactly what needs to be done.


## Table of Contents

- [Why Jhara Exists](#why-jhara-exists)
- [How It Works](#how-it-works)
- [Architecture](#architecture)
- [Ecosystem Coverage](#ecosystem-coverage)
- [Development Status](#development-status)
- [Planned Usage](#planned-usage)
- [Building from Source](#building-from-source)
- [Project Structure](#project-structure)
- [Design Decisions](#design-decisions)
- [Distribution Model](#distribution-model)
- [Contributing](#contributing)
- [License](#license)
- [Acknowledgements](#acknowledgements)


## Why Jhara Exists

If you have been writing software long enough, you know the feeling. You open storage settings, look at the bar, and wonder how 256 gigabytes disappeared. The culprits are not mysterious. `~/.cargo/registry` grows every time you fetch a new crate. Xcode DerivedData accumulates a separate build cache for every project you have ever opened. The `node_modules` directory in a project you abandoned eight months ago still sits on your SSD, fully intact.

The problem is not that these files are hard to delete. It is that it is hard to know which ones are safe to delete. A naive approach — just deleting all `target/` directories — will break a project currently compiling in the background. Deleting the wrong Docker volume destroys a local database that took two hours to seed.

Existing tools fall into two categories. General disk analyzers like DaisyDisk are excellent at showing you what is large, but they have no understanding of what any of it means from a development perspective. Dedicated cleaners like CleanMyMac are subscription products with a business model the developer community has consistently rejected, and their closed-source nature makes it impossible to trust them with full disk access on a machine full of source code and secrets.

Jhara is built on three principles. First, understand the domain: every artifact is classified by how easy it is to recover, not just how large it is. Second, be conservative: default to doing nothing, require explicit confirmation before touching anything outside the safest tier. Third, be transparent: the tool is open source, so you can read exactly what it does before granting it access to your filesystem.


## How It Works

Jhara uses a three-layer approach to go from a raw filesystem scan to a safe, actionable cleanup report.

### Layer 1: Project Detection

The scanner core is written in Rust, using `jwalk` for parallel filesystem traversal with rayon-based concurrency. On macOS, before the Rust scanner starts, a thin Swift layer pre-scans directories using `URLResourceKey.isUbiquitousItemKey` to detect iCloud-managed paths and passes a skip-list to the Rust core. This prevents the scanner from triggering dataless file hydration on iCloud Drive directories — a silent, destructive side effect that a pure-Rust traversal cannot guard against.

The scanner looks for project signature files — `package.json`, `Cargo.toml`, `go.mod`, `build.gradle`, `Package.swift`, and others — to identify project boundaries and maps each project to its known artifact directories.

### Layer 2: Safety Classification

Every artifact directory gets assigned to one of three tiers before anything else happens.

**Safe** directories are ephemeral artifacts that any build tool will regenerate automatically. `node_modules/`, Cargo's `target/`, Go's module cache, pip's download cache: all fall into this tier. You can delete them today and they come back the next time you run your build command.

**Caution** directories contain artifacts that are expensive to recreate or contain historical data with real value. Xcode Archives are a good example. Deleting them does not affect your source code, but it means you can no longer re-symbolicate crash reports from older releases. Conda environments with manually installed packages also fall here.

**Risky** directories contain state that cannot be recovered from a version control checkout. Docker volumes, Terraform state files, Vagrant machine metadata: these require explicit, per-item confirmation regardless of how old they are.

### Layer 3: Staleness Analysis

Size alone is a poor metric for cleanup priority. A 10 GB `target/` directory in a project you are actively working on should not be touched. The same directory in a project you abandoned a year ago is fair game. Jhara determines staleness by looking at the modification time of the project's root descriptor file and the `.git/HEAD` file. If both are older than a configurable threshold (90 days by default), the project is classified as inactive.

One important note: `kMDItemLastUsedDate` — the Spotlight metadata attribute that tracks when a file was last opened — is only updated when a user opens a file through a GUI application. Running `cargo build` or `npm install` in the terminal updates no Spotlight metadata whatsoever. Jhara does not use it.


## Architecture

Jhara is structured as a monorepo with a Rust core library, a native macOS application, a cross-platform Tauri application for Linux and Windows, a TypeScript server, and a Next.js web dashboard.

```
jhara/
├── Cargo.toml               Rust workspace root
├── crates/
│   ├── jhara-core/          Rust library: scanner, detector, classifier, deletion engine
│   └── jhara-macos-ffi/     Rust staticlib target for Xcode linkage (C FFI surface)
│
├── apps/
│   ├── macos/               Native macOS app (Swift 6, SwiftUI)
│   │   └── jhara/
│   │       ├── Scanner/     iCloudGuard, FSEventsMonitor, DiskUsageReporter (macOS-only)
│   │       ├── UI/          SwiftUI views, NSStatusItem, treemap
│   │       ├── Automation/  SMAppService background agent
│   │       └── Cleaner/     TrashCoordinator, GitSafetyCheck
│   │
│   ├── tauri/               Tauri v2 app — Linux and Windows only
│   │   ├── next.config.mjs  output: 'export', static build for WebView
│   │   ├── src/             Next.js frontend (React)
│   │   └── src-tauri/       Rust backend, calls jhara-core directly (no FFI)
│   │
│   ├── server/              TypeScript server (Hono, tRPC) — license validation
│   └── web/                 Next.js web dashboard — license portal
│
└── packages/
    ├── api/                 tRPC router definitions
    ├── auth/                Authentication logic (Better Auth)
    ├── db/                  Prisma schema and database client
    ├── ui/                  Shared React component library (shadcn/ui)
    ├── env/                 Type-safe environment variable validation
    └── config/              Shared TypeScript configuration
```

### Core Separation

**jhara-core (Rust)** contains everything that has no platform-specific dependency: the filesystem traversal engine (`jwalk` + rayon), inode-based deduplication (`HashSet<(u64, u64)>`), path-segment interned scan tree (`ustr` + `Vec<TreeNode>`), ecosystem detection map (80+ project types), safety classifier, staleness checker, and deletion engine. It exposes a C FFI surface generated by `cbindgen`.

**macOS Swift layer** is a thin orchestration shell. It does three things that Rust cannot: pre-scan directories for iCloud Drive status using `URLResourceKey.isUbiquitousItemKey`, listen for filesystem changes with FSEvents and trigger targeted Rust re-scans, and query `volumeAvailableCapacityForImportantUsage` for accurate free space reporting. Everything else — UI, automation scheduling via `SMAppService`, Keychain storage, Sparkle updates — lives here as well.

**Tauri app (Linux/Windows)** calls `jhara-core` directly from its Rust backend process. No FFI boundary: the same crate, called natively. The Next.js frontend communicates with the backend via Tauri commands and receives scan progress as batched events.

This means the scanner logic is written exactly once, in Rust, and tested once. The macOS and Linux/Windows platforms share identical scanning behavior and ecosystem coverage.

### Why Not Tauri on macOS

The macOS application uses Swift for three reasons that cannot be solved in Tauri. First, `URLResourceKey.isUbiquitousItemKey` is an Apple CoreServices API — there is no Rust equivalent, and using `objc2` to bridge it from Rust is fragile across macOS updates. Second, `SMAppService` (the background automation API) integrates with System Settings under General > Login Items; this is only achievable through Swift's `ServiceManagement` framework. Third, `NSStatusItem` and the SwiftUI panel behavior expected from a macOS menu bar app is significantly more polished using native APIs than through Tauri's tray abstraction.

Tauri is used where it excels: Linux and Windows, where there are no deep system APIs to call, and where a single codebase covering both platforms is a meaningful simplification.


## Ecosystem Coverage

Jhara understands artifacts produced by the following ecosystems. The full detection map lives in `jhara-core` and is shared across all platforms.

### Languages and Runtimes

| Ecosystem | Global Cache | Project Artifacts | Safety |
|-----------|-------------|-------------------|--------|
| Node.js | `~/.npm/_cacache/` | `node_modules/`, `node_modules/.cache/` | Safe |
| Bun | `~/.bun/install/cache/` | `node_modules/` | Safe |
| Deno | `~/.deno/`, `~/Library/Caches/deno` | `vendor/` | Safe |
| Python (pip/uv) | `~/Library/Caches/pip/`, `~/.cache/uv/` | `.venv/`, `venv/` | Safe |
| Python (Conda) | `~/.conda/pkgs/` | `envs/` | Caution |
| Ruby | `~/.gem/`, `~/.rbenv/versions/` | `vendor/bundle/` | Safe |
| PHP | `~/.composer/cache/` | `vendor/` | Safe |
| Java (Maven) | `~/.m2/repository/` | `target/` | Safe |
| Java (Gradle) | `~/.gradle/caches/` | `build/`, `.gradle/` | Safe |
| Go | `~/go/pkg/mod/`, `~/Library/Caches/go-build/` | `bin/`, `pkg/` | Safe |
| Rust | `~/.cargo/registry/`, `~/.cargo/git/` | `target/` | Safe |
| C / C++ | None | `build/`, `out/`, `cmake-build-*/` | Safe |
| Swift (SPM) | `~/Library/Caches/org.swift.swiftpm/` | `.build/` | Safe |
| Dart / Flutter | `~/.pub-cache/` | `.dart_tool/`, `build/` | Safe |
| Elixir | `~/.mix/` | `_build/`, `deps/` | Safe |
| Haskell | `~/.stack/`, `~/.cabal/` | `.stack-work/` | Safe |
| Zig | `~/.cache/zig/` | `zig-cache/`, `zig-out/` | Safe |

### Mobile Development

| Ecosystem | Artifact Paths | Safety | Typical Size |
|-----------|---------------|--------|-------------|
| Xcode / SwiftUI | `~/Library/Developer/Xcode/DerivedData/` | Safe | 5–50 GB |
| iOS Simulator Runtimes | `~/Library/Developer/CoreSimulator/Caches/` | Caution | 5–20 GB |
| Xcode Archives | `~/Library/Developer/Xcode/Archives/` | Caution | Variable |
| Android (Gradle) | `~/.gradle/caches/`, `.cxx/`, `build/` | Safe | 5–20 GB |
| React Native | `android/app/build/`, `ios/Pods/` | Safe | 2–8 GB |
| Flutter | `.dart_tool/`, `build/`, `ios/Pods/` | Safe | 1–5 GB |

### DevOps and Infrastructure

Docker cleanup is handled exclusively through the Docker API (`docker system prune`) rather than direct filesystem deletion. Touching virtual disk files while the daemon is running risks corruption. Terraform's `.terraform/` directory is safe to delete, but `terraform.tfstate` files are on an absolute blocklist and are never touched under any circumstance.


## Development Status

| Phase | Description | Status |
|-------|-------------|--------|
| Phase 0 | Monorepo bootstrap, tooling, CI/CD | Complete |
| Phase 1 | jhara-core: Rust scanner (jwalk, inode dedup, scan tree) | In Progress |
| Phase 2 | jhara-core: Ecosystem detection map (80+ project types) | Not started |
| Phase 3 | jhara-core: Safety analysis and staleness engine | Not started |
| Phase 4 | C FFI surface + cbindgen header generation | Not started |
| Phase 5 | Swift macOS: iCloudGuard pre-scan + FFI integration | Not started |
| Phase 6 | SwiftUI menu bar application | Not started |
| Phase 7 | Deletion engine and safety protocols | Not started |
| Phase 8 | SMAppService automation engine (Pro tier) | Not started |
| Phase 9 | Tauri app (Linux + Windows) | Not started |
| Phase 10 | Web dashboard and authentication | In Progress |
| Phase 11 | License integration (Lemon Squeezy) | Not started |
| Phase 12 | Distribution, notarization, and packaging | Not started |
| Phase 13 | Open source release | Not started |


## Planned Usage

> The following describes the intended user experience. None of this is functional yet.

### Menu Bar Application (macOS)

Jhara lives in the menu bar. Clicking the icon opens a panel showing a treemap of your developer directories, color-coded by safety tier rather than raw size. A 2 GB `node_modules` directory appears green (Safe, one click to remove). A 500 MB set of Conda environments appears amber (Caution, review before removing).

### Scan and Clean

```
Open Jhara from menu bar
→ Click "Scan Developer Directories"
→ Scan completes in 3 to 8 seconds depending on project count
→ Review results grouped by project
→ Select what to remove, or use "Remove All Safe Items"
→ Items are moved to Trash (not permanently deleted)
```

### Pro: Automation Rules

The Pro tier adds a background automation engine powered by `SMAppService`. Configure rules like "remove node_modules from any project not modified in 60 days" and have them run automatically on a schedule, with a notification summary after each run.

```
Settings > Automation > Add Rule
→ Choose: node_modules, target (Rust), DerivedData, etc.
→ Set staleness threshold: 30, 60, or 90 days
→ Set schedule: daily, weekly, or on system wake
→ Notification: "Jhara removed 12.4 GB from 8 inactive projects"
```

Rules are stored locally. No data is sent to any server.

### Linux and Windows (Tauri)

The Linux and Windows applications share the same Next.js frontend and connect to the same `jhara-core` Rust backend. The scanning behavior, ecosystem coverage, and safety classifications are identical to the macOS version. Platform-specific differences: no iCloud guard (no equivalent on Linux/Windows), background automation uses systemd user units on Linux and HKCU registry or Task Scheduler on Windows.


## Building from Source

### Prerequisites

- Rust 1.77 or later
- macOS 14+ with Xcode 16+ (for the macOS app)
- Node.js 22 or later
- pnpm 9 or later

### Rust Core

```bash
git clone https://github.com/hmyousuf/jhara.git
cd jhara

# Build jhara-core
cargo build --release -p jhara-core

# Run tests
cargo test -p jhara-core
```

### macOS Application

```bash
# Build the macOS FFI static library first
cargo build --release --target aarch64-apple-darwin -p jhara-macos-ffi
cargo build --release --target x86_64-apple-darwin -p jhara-macos-ffi
lipo -create -output target/libjhara_universal.a \
  target/aarch64-apple-darwin/release/libjhara_macos_ffi.a \
  target/x86_64-apple-darwin/release/libjhara_macos_ffi.a

# Open in Xcode
open apps/macos/jhara.xcodeproj
```

### Tauri App (Linux / Windows)

```bash
cd apps/tauri
pnpm install
pnpm tauri dev       # Development
pnpm tauri build     # Production
```

### JavaScript Packages and Web Dashboard

```bash
pnpm install
pnpm dev             # Start all JS packages in dev mode
pnpm build           # Build all packages
```

### Database

```bash
cd packages/db
docker compose up -d
pnpm db:push
```


## Project Structure

```
crates/jhara-core/src/
├── lib.rs
├── types.rs                 ScanNode, ScanError (shared across all platforms)
├── scanner/
│   ├── mod.rs               jwalk traversal, FTS_XDEV parity, skip-list handling
│   ├── inode.rs             InodeTracker — HashSet<(u64, u64)> with device ID
│   └── dedup.rs             Windows FILE_ID_INFO hard-link deduplication
├── tree.rs                  ScanTree — ustr path interning, flat Vec<TreeNode>, O(N) rollup
├── detector/
│   ├── mod.rs               ProjectDetector, signature priority, monorepo resolution
│   ├── signatures.rs        Ecosystem signature database
│   └── frameworks.rs        package.json dependency parsing for framework detection
├── classifier/
│   ├── mod.rs               SafetyClassifier — combines all signals
│   ├── staleness.rs         mtime-based activity analysis, .git/HEAD awareness
│   └── blocklist.rs         Absolute never-delete path patterns
└── ffi/
    ├── mod.rs               C FFI exports
    └── types.rs             ScanNodeC #[repr(C)], batched callback interface

apps/macos/jhara/
├── Scanner/
│   ├── iCloudGuard.swift    Pre-scan iCloud detection → skip-list for Rust
│   ├── FSEventsMonitor.swift Directory change detection → trigger Rust re-scan
│   └── DiskUsageReporter.swift volumeAvailableCapacityForImportantUsage
├── UI/                      SwiftUI views, treemap (Canvas), NSStatusItem
├── Automation/              SMAppService registration, XPC, notification actions
└── Cleaner/                 TrashCoordinator, GitSafetyChecker

apps/tauri/
├── src/                     Next.js frontend (shared with no macOS-specific UI)
└── src-tauri/src/
    ├── main.rs
    └── commands/
        ├── scan.rs          Calls jhara-core directly (same Rust process, no FFI)
        └── clean.rs
```


## Design Decisions

### Rust Core, Not Swift Scanner

The original implementation planned to write the scanner in Swift using `fts_open`. The decision to move the scanner to Rust was made for three reasons. First, the scan logic — inode tracking, path interning, ecosystem detection, safety classification — has no macOS dependency and should be written once, not twice. Second, the Linux and Windows Tauri app needs the same logic, and rewriting it in a third language would be a maintenance burden. Third, `jwalk` with rayon provides parallel directory traversal that is competitive with or faster than a single-threaded `fts_open` wrapper on multi-core machines.

The Swift layer retains only the three things that genuinely require macOS APIs: iCloud path detection, FSEvents monitoring, and volume capacity queries.

### iCloud Guard Architecture

`URLResourceKey.isUbiquitousItemKey` is a CoreServices API accessible only from Swift or Objective-C. The chosen pattern is a pre-scan: Swift enumerates top-level home directories using `FileManager.enumerator` with `.skipsSubdirectoryDescendants`, checks `isUbiquitousItemKey` for each, and serializes the results as a flat array of C strings passed to the Rust scanner before traversal begins. Inside `jwalk`'s `process_read_dir` hook, Rust performs an O(1) `HashSet<PathBuf>` lookup. If a directory is in the skip-list, `jwalk` never descends into it, meaning no child paths ever reach the iCloud check.

This was chosen over `objc2`-based Objective-C bridging from Rust because `objc2` requires depending on Apple's internal framework memory layouts, which break on major macOS versions. The pre-scan pattern is predictable, testable, and requires no unsafe Objective-C interop in the hot path.

### ScanNodeC Batched FFI Callbacks

The C FFI delivers scan results in batches of 1,024 nodes per callback invocation rather than one node per call. At one million files, a per-node callback would queue one million messages into the Swift actor's mailbox. Swift actors process mailboxes sequentially — this floods the executor, causes memory spikes, and starves the UI thread. With 1,024-node batches, the FFI boundary is crossed approximately 1,000 times per scan. The Swift actor unpacks each batch synchronously and passes it to the scan tree. The same batching logic applies to the Tauri frontend via `tauri::Window::emit()`.

### ustr + Flat Vec for Scan Tree

`ScanTree` stores one million path strings in approximately 18 MB using two techniques. Path segments are interned with `ustr`, which maintains a lock-free global cache and returns null-terminated pointers usable directly as `*const c_char` at the FFI boundary. Tree nodes are stored in a flat `Vec<TreeNode>` where each node holds its parent's index. Size rollups happen in a single reverse-order pass after the scan completes — O(N), cache-friendly, with no recursive calls or per-insertion locking.

### mtime Over kMDItemLastUsedDate

`kMDItemLastUsedDate` is only updated when a file is opened through a GUI application via LaunchServices. Running `npm run build` or `cargo test` in the terminal updates no Spotlight metadata whatsoever. Jhara uses the POSIX `mtime` of the project's root descriptor file combined with the `mtime` of `.git/HEAD`, which is updated on every commit, checkout, and branch operation.

### GRDB Over SwiftData

SwiftData's predicate support and multi-threaded access story remain incomplete as of macOS 15. GRDB.swift provides direct, efficient SQLite access, correct concurrent read behavior (required because the background automation agent and the foreground app both read the same database), and has been production-proven in macOS apps for years.

### Distribution via Signed DMG

The Mac App Store sandbox prevents apps from accessing paths outside the user's container without an explicit file picker. An app that can only scan directories you individually pick is not a disk manager. Jhara is distributed as an Apple Developer ID-signed, notarized `.dmg`, requesting Full Disk Access through the standard macOS permission flow.


## Distribution Model

**Free tier (open source):** Full scanning and manual cleanup across all 80+ ecosystem types. No limitations on what you can scan or remove. The free tier is a complete manual disk manager.

**Pro tier ($12.99 one-time, Lemon Squeezy):** Background automation via `SMAppService` (macOS) and systemd/Task Scheduler (Linux/Windows). Configurable staleness rules, scheduled runs, and notification summaries. One-time payment, no subscription. Two-machine activation limit.

**Why open source the core:** A tool requesting Full Disk Access on a machine full of source code and secrets should be auditable. Eighty-plus ecosystem types requires community contributions to stay accurate. And a well-maintained, starred open-source tool is a better portfolio signal than a closed-source app.


## Contributing

Jhara is in early development and contributions are welcome at every level.

Before starting:

1. Read [CONTRIBUTING.md](CONTRIBUTING.md) for setup instructions
2. Check the [roadmap](ROADMAP.md) to understand which phase is current
3. Open an issue or start a discussion before beginning large changes

Rules that apply everywhere:

New ecosystem detection entries need at least one test covering the detection signature, artifact paths, and safety classification rationale. Performance-sensitive changes need before/after measurements. Bengali text and comments are welcome alongside English — this project has roots in South Asian developer culture.


## License

Jhara is released under the [Apache License 2.0](LICENSE).

The Pro automation feature requires a license key purchased through the Jhara website. The license validation code is open source and auditable.


## Acknowledgements

[DaisyDisk](https://daisydiskapp.com/) for the treemap visualization approach. [DevCleaner for Xcode](https://github.com/vashpan/xcode-dev-cleaner) for careful handling of Xcode's opaque directory structure. [dua-cli](https://github.com/Byron/dua-cli) for demonstrating parallel Rust disk scanning and hard-link deduplication via composite inode keys. [Pearcleaner](https://github.com/alienator88/Pearcleaner) for the Sentinel Monitor pattern. [GRDB.swift](https://github.com/groue/GRDB.swift) for making SQLite pleasant from Swift. [Sparkle](https://sparkle-project.org/) for secure, signed app updates outside the App Store.

---

*Author: H.M. Yousuf*
*Repository: [github.com/hmyousuf/jhara](https://github.com/hmyousuf/jhara)*
