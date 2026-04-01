<div align="center">
  <h1>Jhara (ঝরা)</h1>
  <p><b>A disk cleaner for developers that actually understands what it's looking at.</b></p>

  [![Status](https://img.shields.io/badge/Status-Alpha%20%2F%20In%20Development-orange.svg)](#development-status)
  [![License](https://img.shields.io/badge/License-Apache_2.0-blue.svg?logo=apache)](LICENSE)
  <br />
  [![Rust](https://img.shields.io/badge/Rust-1.77%2B-orange?logo=rust&logoColor=white)](#)
  [![Swift](https://img.shields.io/badge/Swift-6.0-orange?logo=swift&logoColor=white)](#)
  [![TypeScript](https://img.shields.io/badge/TypeScript-5.x-blue?logo=typescript&logoColor=white)](#)
  [![Tauri](https://img.shields.io/badge/Tauri-v2-24C8D8?logo=tauri&logoColor=white)](#)
  <br />
  [![macOS](https://img.shields.io/badge/macOS-14%2B-lightgray?logo=apple&logoColor=white)](#)
  [![Linux](https://img.shields.io/badge/Linux-Supported-lightgray?logo=linux&logoColor=white)](#)
  [![Windows](https://img.shields.io/badge/Windows-Supported-lightgray?logo=windows&logoColor=white)](#)

  <br />
</div>

Jhara is a disk cleaner built on one idea: developer disks fill up in predictable, structured ways. A tool that understands those patterns can clean safely. Where a general disk analyzer sees an opaque directory tree, Jhara sees a Rust project with a stale `target/` folder, a Node monorepo with hoisted `node_modules`, or an Xcode workspace accumulating DerivedData for two years.

The name comes from the Bengali word ঝরা, meaning to shed or fall away. Like leaves at the end of a season.

> [!NOTE]
> **Status:** Paused for exams. 
> 
> To be totally honest, Jhara isn't ready for an MVP. I had to drop everything for my exams. That means the `main` branch might be broken right now. 
> 
> Here's what's actually built, though to be clear, the whole thing is currently broken and might not even run. The Rust scanner, ecosystem detection, and the SwiftUI app are technically done. But the background automation is just a pile of draft files. I haven't even started the Windows and Linux apps. Most importantly, the core scanner hasn't been battle-tested enough for me to guarantee it won't delete something it shouldn't. 
> 
> Don't run this on a drive you care about yet. 
> 
> Also, please don't open any PRs right now. I'm completely offline and won't be able to review them. Just check back in a few months when I'm active again.


## Table of contents

- [Why Jhara exists](#why-jhara-exists)
- [How it works](#how-it-works)
- [Architecture](#architecture)
- [Ecosystem coverage](#ecosystem-coverage)
- [Development status](#development-status)
- [Planned usage](#planned-usage)
- [Building from source](#building-from-source)
- [Project structure](#project-structure)
- [Design decisions](#design-decisions)
- [Distribution model](#distribution-model)
- [Contributing](#contributing)
- [License](#license)
- [Acknowledgements](#acknowledgements)


## Why Jhara exists

If you've been writing software long enough, you know the feeling. You open storage settings, look at the bar, and wonder how 256 GB disappeared. The culprits aren't mysterious. `~/.cargo/registry` grows every time you fetch a new crate. Xcode DerivedData builds up a separate cache for every project you've ever opened. The `node_modules` directory in a project you abandoned eight months ago still sits on your SSD, fully intact.

The problem isn't that these files are hard to delete. It's that it's hard to know which ones are safe to delete. A naive approach, just deleting all `target/` directories, will break a project currently compiling in the background. Deleting the wrong Docker volume destroys a local database that took two hours to seed.

Existing tools split into two camps. General disk analyzers like DaisyDisk are excellent at showing you what's large, but they have no idea what any of it means from a development perspective. Dedicated cleaners like CleanMyMac are subscription products with a business model the developer community has consistently rejected. And their closed-source nature makes it hard to trust them with full disk access on a machine full of source code and secrets.

Jhara is built on three things. First, understand the domain: every artifact gets classified by how easy it is to recover, not just how large it is. Second, be conservative: do nothing by default, require explicit confirmation before touching anything outside the safest tier. Third, be transparent: the tool is open source, so you can read exactly what it does before giving it access to your filesystem.


## How it works

Jhara uses three layers to go from a raw filesystem scan to an actionable cleanup report.

### Layer 1: project detection

The scanner core is written in Rust, using `jwalk` for parallel filesystem traversal with rayon-based concurrency. On macOS, before the Rust scanner starts, a Swift layer pre-scans directories using `URLResourceKey.isUbiquitousItemKey` to detect iCloud-managed paths and passes a skip-list to the Rust core. This prevents the scanner from triggering dataless file hydration on iCloud Drive directories, which is a silent, destructive side effect a pure-Rust traversal can't guard against.

The scanner looks for project signature files: `package.json`, `Cargo.toml`, `go.mod`, `build.gradle`, `Package.swift`, and others. These identify project boundaries and map each project to its known artifact directories.

### Layer 2: safety classification

Every artifact directory gets a tier before anything else happens.

**Safe** directories are ephemeral artifacts any build tool will regenerate. `node_modules/`, Cargo's `target/`, Go's module cache, pip's download cache. You can delete them today and they come back next time you run your build command.

**Caution** directories contain artifacts that are expensive to recreate or have historical value. Xcode Archives are a good example. Deleting them doesn't affect your source code, but it means you can't re-symbolicate crash reports from older releases. Conda environments with manually installed packages also land here.

**Risky** directories contain state you can't recover from a version control checkout. Docker volumes, Terraform state files, Vagrant machine metadata. These require explicit, per-item confirmation regardless of how old they are.

### Layer 3: staleness analysis

Size alone is a poor metric for cleanup priority. A 10 GB `target/` directory in a project you're actively working on shouldn't be touched. The same directory in a project you abandoned a year ago is fair game. Jhara determines staleness by looking at the modification time of the project's root descriptor file and the `.git/HEAD` file. If both are older than a configurable threshold (90 days by default), the project is classified as inactive.

One note worth making: `kMDItemLastUsedDate`, the Spotlight metadata attribute tracking when a file was last opened, only updates when you open a file through a GUI app. Running `cargo build` or `npm install` in the terminal updates no Spotlight metadata. Jhara doesn't use it.


## Architecture

Jhara is a monorepo with a Rust core library, a native macOS app, a cross-platform Tauri app for Linux and Windows, a TypeScript server, and a Next.js web dashboard.

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
│   │       ├── Scanner/     iCloudGuard, FSEventsMonitor, DiskUsageReporter
│   │       ├── UI/          SwiftUI views, NSStatusItem, treemap
│   │       ├── Automation/  SMAppService background agent
│   │       └── Cleaner/     TrashCoordinator, GitSafetyCheck
│   │
│   ├── tauri/               Tauri v2 app, Linux and Windows only
│   │   ├── next.config.mjs  output: 'export', static build for WebView
│   │   ├── src/             Next.js frontend (React)
│   │   └── src-tauri/       Rust backend, calls jhara-core directly (no FFI)
│   │
│   ├── server/              TypeScript server (Hono, tRPC), license validation
│   └── web/                 Next.js web dashboard, license portal
│
└── packages/
    ├── api/                 tRPC router definitions
    ├── auth/                Authentication logic (Better Auth)
    ├── db/                  Prisma schema and database client
    ├── ui/                  Shared React component library (shadcn/ui)
    ├── env/                 Type-safe environment variable validation
    └── config/              Shared TypeScript configuration
```

### Core separation

`jhara-core` (Rust) has everything with no platform-specific dependency: the filesystem traversal engine (`jwalk` + rayon), inode-based deduplication (`HashSet<(u64, u64)>`), path-segment interned scan tree (`ustr` + `Vec<TreeNode>`), ecosystem detection map (80+ project types), safety classifier, staleness checker, and deletion engine. It exposes a C FFI surface generated by `cbindgen`.

The macOS Swift layer is a thin orchestration shell. It does three things Rust can't: pre-scan directories for iCloud Drive status using `URLResourceKey.isUbiquitousItemKey`, listen for filesystem changes with FSEvents and trigger targeted Rust re-scans, and query `volumeAvailableCapacityForImportantUsage` for accurate free space. Everything else, UI, automation via `SMAppService`, Keychain storage, Sparkle updates, lives here too.

The Tauri app (Linux and Windows) calls `jhara-core` directly from its Rust backend. No FFI boundary: the same crate, called natively. The Next.js frontend talks to the backend via Tauri commands and receives scan progress as batched events.

The scanner logic is written once, in Rust, and tested once. macOS and Linux/Windows share identical scanning behavior and ecosystem coverage.

### Why not Tauri on macOS

The macOS app uses Swift for reasons you can't solve in Tauri. `URLResourceKey.isUbiquitousItemKey` is an Apple CoreServices API with no Rust equivalent, and bridging it via `objc2` breaks across macOS updates. `SMAppService` integrates with System Settings under General > Login Items, which only works through Swift's `ServiceManagement` framework. And `NSStatusItem` with the panel behavior expected from a macOS menu bar app is significantly more polished in native Swift than through Tauri's tray abstraction.

Tauri is used where it actually makes sense: Linux and Windows, where there are no deep system APIs to call and one codebase covering both platforms is a real win.


## Ecosystem coverage

Jhara understands artifacts from the following ecosystems. The full detection map lives in `jhara-core` and is shared across all platforms.

### Languages and runtimes

| Ecosystem | Global cache | Project artifacts | Safety |
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

### Mobile development

| Ecosystem | Artifact paths | Safety | Typical size |
|-----------|---------------|--------|-------------|
| Xcode / SwiftUI | `~/Library/Developer/Xcode/DerivedData/` | Safe | 5-50 GB |
| iOS Simulator Runtimes | `~/Library/Developer/CoreSimulator/Caches/` | Caution | 5-20 GB |
| Xcode Archives | `~/Library/Developer/Xcode/Archives/` | Caution | Variable |
| Android (Gradle) | `~/.gradle/caches/`, `.cxx/`, `build/` | Safe | 5-20 GB |
| React Native | `android/app/build/`, `ios/Pods/` | Safe | 2-8 GB |
| Flutter | `.dart_tool/`, `build/`, `ios/Pods/` | Safe | 1-5 GB |

### DevOps and infrastructure

Docker cleanup goes through the Docker API (`docker system prune`), not direct filesystem deletion. Touching virtual disk files while the daemon runs risks corruption. Terraform's `.terraform/` directory is safe to delete, but `terraform.tfstate` files are on an absolute blocklist and will never be touched under any circumstance.


## Development status

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


## Planned usage

> None of this is functional yet. This describes where the tool is headed.

### Menu bar application (macOS)

Jhara lives in the menu bar. Clicking the icon opens a panel with a treemap of your developer directories, color-coded by safety tier rather than raw size. A 2 GB `node_modules` shows up green (Safe, one click to remove). A 500 MB set of Conda environments shows up amber (Caution, review before removing).

### Scan and clean

```
Open Jhara from menu bar
→ Click "Scan Developer Directories"
→ Scan completes in 3-8 seconds depending on project count
→ Review results grouped by project
→ Select what to remove, or use "Remove All Safe Items"
→ Items go to Trash, not permanent deletion
```

### Pro: automation rules

The Pro tier adds a background automation engine via `SMAppService`. You configure rules like "remove node_modules from any project not touched in 60 days" and they run on a schedule, with a notification summary after each run.

```
Settings > Automation > Add Rule
→ Choose: node_modules, target (Rust), DerivedData, etc.
→ Set staleness threshold: 30, 60, or 90 days
→ Set schedule: daily, weekly, or on system wake
→ Notification: "Jhara removed 12.4 GB from 8 inactive projects"
```

Rules are stored locally. Nothing goes to any server.

### Linux and Windows (Tauri)

The Linux and Windows apps share the same Next.js frontend and connect to the same `jhara-core` Rust backend. Scanning behavior, ecosystem coverage, and safety classifications are identical to macOS. Platform differences: no iCloud guard (no equivalent exists), background automation uses systemd user units on Linux and HKCU registry or Task Scheduler on Windows.


## Building from source

### Prerequisites

- Rust 1.77 or later
- macOS 14+ with Xcode 16+ (for the macOS app)
- Node.js 22 or later
- pnpm 9 or later

### Rust core

```bash
git clone https://github.com/hmyousuf/jhara.git
cd jhara

# Build jhara-core
cargo build --release -p jhara-core

# Run tests
cargo test -p jhara-core
```

### macOS application

```bash
# Build the macOS FFI static library
cargo build --release --target aarch64-apple-darwin -p jhara-macos-ffi
cargo build --release --target x86_64-apple-darwin -p jhara-macos-ffi
lipo -create -output target/libjhara_universal.a \
  target/aarch64-apple-darwin/release/libjhara_macos_ffi.a \
  target/x86_64-apple-darwin/release/libjhara_macos_ffi.a

# Open in Xcode
open apps/macos/jhara.xcodeproj
```

### Tauri app (Linux / Windows)

```bash
cd apps/tauri
pnpm install
pnpm tauri dev       # Development
pnpm tauri build     # Production
```

### JavaScript packages and web dashboard

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


## Project structure

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
│   ├── mod.rs               SafetyClassifier, combines all signals
│   ├── staleness.rs         mtime-based activity analysis, .git/HEAD awareness
│   └── blocklist.rs         Absolute never-delete path patterns
└── ffi/
    ├── mod.rs               C FFI exports
    └── types.rs             ScanNodeC #[repr(C)], batched callback interface

apps/macos/jhara/
├── Scanner/
│   ├── iCloudGuard.swift    Pre-scan iCloud detection, skip-list for Rust
│   ├── FSEventsMonitor.swift Directory change detection, trigger Rust re-scan
│   └── DiskUsageReporter.swift volumeAvailableCapacityForImportantUsage
├── UI/                      SwiftUI views, treemap (Canvas), NSStatusItem
├── Automation/              SMAppService registration, XPC, notification actions
└── Cleaner/                 TrashCoordinator, GitSafetyChecker

apps/tauri/
├── src/                     Next.js frontend
└── src-tauri/src/
    ├── main.rs
    └── commands/
        ├── scan.rs          Calls jhara-core directly (same Rust process, no FFI)
        └── clean.rs
```


## Design decisions

### Rust core, not Swift scanner

The original plan was to write the scanner in Swift using `fts_open`. Three things changed that. The scan logic, inode tracking, path interning, ecosystem detection, safety classification, has no macOS dependency and should be written once, not twice. The Linux and Windows Tauri app needs the same logic, and rewriting it would create a maintenance problem. And `jwalk` with rayon gives parallel directory traversal that's competitive with a single-threaded `fts_open` wrapper on multi-core machines.

Swift keeps only the three things that genuinely need macOS APIs: iCloud path detection, FSEvents monitoring, and volume capacity queries.

### iCloud guard architecture

`URLResourceKey.isUbiquitousItemKey` is a CoreServices API only accessible from Swift or Objective-C. The pattern chosen: Swift enumerates top-level home directories with `FileManager.enumerator` using `.skipsSubdirectoryDescendants`, checks `isUbiquitousItemKey` for each, and serializes the results as a flat array of C strings passed to the Rust scanner before traversal begins. Inside `jwalk`'s `process_read_dir` hook, Rust does an O(1) `HashSet<PathBuf>` lookup. If a directory is in the skip-list, `jwalk` never descends into it.

This was chosen over `objc2`-based bridging from Rust because `objc2` depends on Apple's internal framework memory layouts, which break on major macOS versions. The pre-scan pattern is predictable, testable, and requires no unsafe Objective-C interop in the hot path.

### ScanNodeC batched FFI callbacks

The C FFI delivers results in batches of 1,024 nodes per callback invocation rather than one per call. At one million files, a per-node callback would queue one million messages into the Swift actor's mailbox. Actors process mailboxes sequentially, so this floods the executor, causes memory spikes, and starves the UI thread. With 1,024-node batches, the FFI boundary is crossed about 1,000 times per scan. The Tauri frontend mirrors this with batched `tauri::Window::emit()` calls.

### ustr + flat Vec for scan tree

`ScanTree` stores one million path strings in about 18 MB using two techniques. Path segments are interned with `ustr`, which maintains a lock-free global cache and returns null-terminated pointers usable directly as `*const c_char` at the FFI boundary. Tree nodes are stored in a flat `Vec<TreeNode>` where each node holds its parent's index. Size rollups happen in a single reverse-order pass after the scan completes: O(N), cache-friendly, no recursive calls, no per-insertion locking.

### mtime over kMDItemLastUsedDate

`kMDItemLastUsedDate` only updates when a file is opened through a GUI app via LaunchServices. Running `npm run build` or `cargo test` in the terminal updates no Spotlight metadata. Jhara uses the POSIX `mtime` of the project's root descriptor file combined with the `mtime` of `.git/HEAD`, which updates on every commit, checkout, and branch operation.

### GRDB over SwiftData

SwiftData's predicate support and multi-threaded access story are still incomplete as of macOS 15. GRDB.swift gives direct, efficient SQLite access, correct concurrent read behavior (needed because the background automation agent and the foreground app both read the same database), and has been production-proven in macOS apps for years.

### Distribution via signed DMG

The Mac App Store sandbox prevents apps from accessing paths outside the user's container without an explicit file picker. An app that can only scan directories you individually select isn't a disk manager. Jhara ships as an Apple Developer ID-signed, notarized `.dmg`, requesting Full Disk Access through the standard macOS permission flow.


## Distribution model

**Free tier (open source):** Full scanning and manual cleanup across all 80+ ecosystem types. No limits on what you can scan or remove.

**Pro tier ($12.99 one-time, Lemon Squeezy):** Background automation via `SMAppService` (macOS) and systemd/Task Scheduler (Linux/Windows). Configurable staleness rules, scheduled runs, notification summaries. One-time payment, no subscription. Two-machine activation limit.

**Why open source the core:** A tool requesting Full Disk Access on a machine full of source code and secrets should be auditable. 80+ ecosystem types requires community contributions to stay accurate. And honestly, a well-maintained open-source tool is a better portfolio signal than a closed-source app anyway.


## Contributing

Jhara is in early development and contributions are welcome at every level.

Before starting:

1. Read [CONTRIBUTING.md](CONTRIBUTING.md) for setup instructions
2. Check the [roadmap](ROADMAP.md) to understand which phase is current
3. Open an issue or start a discussion before beginning large changes

A few rules that apply everywhere: new ecosystem detection entries need at least one test covering the detection signature, artifact paths, and safety classification rationale. Performance-sensitive changes need before/after measurements. Bengali text and comments are welcome alongside English, this project has roots in South Asian developer culture.


## License

Apache License 2.0. See [LICENSE](LICENSE).

The Pro automation feature requires a license key purchased through the Jhara website. The license validation code is open source and auditable.


## Acknowledgements

[DaisyDisk](https://daisydiskapp.com/) for the treemap visualization approach. [DevCleaner for Xcode](https://github.com/vashpan/xcode-dev-cleaner) for careful handling of Xcode's opaque directory structure. [dua-cli](https://github.com/Byron/dua-cli) for demonstrating parallel Rust disk scanning and hard-link deduplication via composite inode keys. [Pearcleaner](https://github.com/alienator88/Pearcleaner) for the Sentinel Monitor pattern. [GRDB.swift](https://github.com/groue/GRDB.swift) for making SQLite pleasant from Swift. [Sparkle](https://sparkle-project.org/) for secure, signed app updates outside the App Store.

---

*Author: H.M. Yousuf*
*Repository: [github.com/hmyousuf/jhara](https://github.com/hmyousuf/jhara)*
