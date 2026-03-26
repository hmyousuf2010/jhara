# Jhara master roadmap

## From zero to a cross-platform developer disk cleaner

> **Project:** Jhara (ঝরা, "to shed" in Bengali)
> **Author:** H.M. Yousuf
> **Repository:** [github.com/hmyousuf/jhara](https://github.com/hmyousuf/jhara)
> **License:** MIT
> **Mission:** Build a Rust-core disk space manager for developers. It scans filesystem trees, classifies build artifacts by safety tier, and reclaims space without touching anything important. Ship a native macOS menu bar app as the primary product and a Tauri app covering Linux and Windows, both powered by the same Rust scanner core.


## Architecture summary

```
jhara-core (Rust crate)
    Scanner: jwalk + rayon parallel traversal
    InodeTracker: HashSet<(DeviceId, InodeId)> hard-link dedup
    ScanTree: ustr path interning + flat Vec<TreeNode> + O(N) rollup
    Detector: 80+ ecosystem signature map
    Classifier: safety tiers, staleness engine, blocklist
    Cleaner: deletion planner, trash coordination
    FFI: C FFI surface via cbindgen, batched ScanNodeC callbacks

apps/macos (Swift 6 + SwiftUI)
    iCloudGuard: pre-scan skip-list via URLResourceKey.isUbiquitousItemKey
    FSEventsMonitor: incremental re-scan triggers
    DiskUsageReporter: volumeAvailableCapacityForImportantUsage
    UI: NSStatusItem, NSPanel, SwiftUI treemap, scan results
    Automation: SMAppService, XPC, UNNotification actions
    Cleaner: TrashCoordinator, GitSafetyChecker

apps/tauri (Tauri v2 + Next.js, Linux and Windows only)
    src-tauri: Rust backend calling jhara-core directly (no FFI)
    src: Next.js static export, React frontend
    Automation: systemd user units (Linux), HKCU + WM_POWERBROADCAST (Windows)
```

What changed from v1: the original plan wrote the scanner in Swift using `fts_open`. The scanner now lives entirely in Rust (`jhara-core`). Swift keeps only the three macOS-specific concerns Rust can't address: iCloud path detection, FSEvents monitoring, and volume capacity queries. Linux and Windows are covered by a single Tauri v2 app sharing the same Rust core with no duplication.


## Confirmed technical decisions

These were validated through research before implementation. The reasoning matters, because someone picking this up later should understand why each choice was made.

### Decision 1: jhara-core in Rust, not Swift

The scanner logic, inode tracking, path interning, ecosystem detection, safety classification, has no macOS dependency. Writing it in Swift would make it unavailable to Linux and Windows without rewriting it a second time. In Rust, all three platforms share one codebase, tested once. `jwalk` with rayon gives parallel traversal competitive with `fts_open` on multi-core hardware. On macOS APFS, the kernel's global directory enumeration lock caps parallel traversal at roughly 50,000-80,000 files/sec regardless of language, so the performance ceiling is identical.

### Decision 2: jwalk with manual FTS_XDEV parity

`jwalk` doesn't have a built-in cross-device boundary guard equivalent to `FTS_XDEV`. The implementation captures the root directory's device ID via `MetadataExt::dev()` and checks each child directory in `process_read_dir`, removing entries with a different device ID from the traversal queue. `jwalk` defaults to `follow_links(false)`, fulfilling `FTS_PHYSICAL`. Thread safety equivalent to `FTS_NOCHDIR` is implicit since rayon workers don't call `chdir(2)`.

### Decision 3: iCloud guard as Swift pre-scan skip-list

`URLResourceKey.isUbiquitousItemKey` is a CoreServices API only accessible from Swift or Objective-C. Using `objc2` to call it from Rust breaks across macOS versions. The pattern: Swift enumerates top-level home directories with `FileManager.enumerator(.skipsSubdirectoryDescendants)`, checks `isUbiquitousItemKey` for each, serializes the results as `*const *const c_char`, and passes this skip-list to the Rust scanner before traversal starts. Inside `jwalk`'s `process_read_dir`, Rust does O(1) `HashSet<PathBuf>` lookups using `Path::starts_with()` (component-aware, not string prefix) to prune the descent queue.

### Decision 4: batched FFI callbacks (1,024 nodes per call)

A per-node FFI callback for one million files queues one million messages into the Swift actor's mailbox. Actors process mailboxes sequentially, so this causes memory spikes and executor flooding. The FFI delivers results in 1,024-node batches. Swift receives `(*const ScanNodeC, count: usize)` per callback, processes synchronously, and updates the scan tree. The Tauri frontend mirrors this with batched `tauri::Window::emit()` calls.

### Decision 5: ustr + flat Vec for scan tree

Path segment interning with `ustr` reduces RAM for a 1M-file tree from ~250 MB (naive String storage) to ~18 MB. `ustr` strings are null-terminated, so they can be passed as `*const c_char` at the FFI boundary without `CString` allocation. Tree nodes live in a flat `Vec<TreeNode>` where each node stores its parent index. Size rollup is a single reverse-order pass after scan completion: O(N), cache-friendly, no recursion, no per-insertion locking.

### Decision 6: composite (DeviceId, InodeId) for hard-link dedup

PNPM's content-addressable store creates hard links. Summing file sizes naively inflates disk usage by 3-10x in a large monorepo. The dedup tracker maintains a `HashSet<(u64, u64)>` keyed on `(MetadataExt::dev(), MetadataExt::ino())`. On Windows, NTFS lacks POSIX inodes, so files with `number_of_links() > 1` require opening a handle to query `FILE_ID_INFO` via `GetFileInformationByHandleEx`. Files with `link_count == 1` skip the handle entirely.

### Decision 7: bumpalo arena for FFI path strings

Allocating a `CString` per node in a 1M-node scan means 1M `malloc/free` pairs. The scanner uses a `bumpalo::Bump` arena for all path string allocations. The arena is pinned for the scan duration; `*const c_char` pointers in `ScanNodeC` remain valid for the entire traversal and are freed in O(1) when the arena drops.

### Decision 8: ScanNodeC explicit padding

The `ScanNodeC` struct is ordered largest-to-smallest fields (8-byte pointers, then 8-byte integers, then 4-byte, 2-byte, 1-byte) with one explicit `_padding: u8` field to reach a 64-byte total. This removes compiler-dependent alignment ambiguity across `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`, and `x86_64-pc-windows-msvc`.

### Decision 9: post-scan size rollup, not propagate-on-insert

The original Swift `ScanTree` called `propagateUp()` on every node insertion: O(N * depth) with heavy parent-node lock contention in a parallel scan. The Rust tree defers all rollup until scan completion. A single reverse-order pass over `Vec<TreeNode>` updates each node's parent: O(N), one cache-friendly array traversal, zero locking.

### Decision 10: mtime of descriptor + .git/HEAD for staleness

`kMDItemLastUsedDate` only updates via LaunchServices GUI file opens. Terminal-based workflows, `cargo build`, `npm install`, `git commit`, don't update it. Jhara reads `mtime` of the root descriptor file (`package.json`, `Cargo.toml`, etc.) and `mtime` of `.git/HEAD` (updated on every commit, checkout, and branch operation). The more recent of the two is `lastActivityDate`. Default threshold: 90 days.

### Decision 11: one Tauri project for Linux and Windows

A single `apps/tauri/` directory with `#[cfg(target_os)]` conditional compilation covers both Linux and Windows. Linux uses systemd user units for background automation and D-Bus `PrepareForSleep` for wake detection (`zbus` crate). Windows uses `WM_POWERBROADCAST` with `PBT_APMRESUMEAUTOMATIC` via an invisible Win32 sink window. System tray uses `tauri-plugin-positioner` with a `.desktop` file fallback on Linux (GNOME Wayland tray support requires AppIndicator extension; the app degrades gracefully to a standard window).

### Decision 12: SMAppService for macOS background automation

`SMAppService.agent(plistName:).register()` registers the background agent so it appears in System Settings > General > Login Items and Extensions. The user can see it, toggle it, and remove it without touching the terminal. The background agent is a separate lightweight executable bundled with the app. It links only `jhara-core`, the rule engine, and the XPC communication layer, not the full SwiftUI stack. Agent-to-foreground-app communication uses XPC.

### Decision 13: GRDB over SwiftData

SwiftData's predicate support and concurrent access behavior are still incomplete as of macOS 15. GRDB.swift gives direct SQLite access, correct concurrent reads (needed because the background agent and foreground app share the same database), and has been production-tested in macOS apps for years. Higher initial setup cost, lower long-term maintenance cost.

### Decision 14: Lemon Squeezy + 7-day offline cache

Lemon Squeezy handles license key generation, activation, deactivation, per-machine limits, VAT, and sales tax globally, so there's no need for a custom license validation backend. The macOS app stores the activation token in the Keychain. If the validation server is unreachable, the app falls back to the cached result if the last successful validation was within 7 days. After 7 days without connectivity, Pro features suspend with a clear message.

### Decision 15: Next.js static export in Tauri

Tauri v2 has no embedded Node.js server. Next.js 15 must use `output: 'export'`. The App Router works under static export for client-side navigation; server features (API routes, middleware) aren't supported. `next/image` requires `images: { unoptimized: true }`. `trailingSlash: true` ensures WebView routing resolves to explicit `index.html` files. `frontendDist: "../out"` in `tauri.conf.json`.


## Table of contents

1. [Project north stars](#1-project-north-stars)
2. [Monorepo architecture](#2-monorepo-architecture)
3. [Phase 0: tooling and CI/CD](#3-phase-0-tooling-and-cicd)
4. [Phase 1: jhara-core scanner engine](#4-phase-1-jhara-core-scanner-engine)
5. [Phase 2: ecosystem detection map](#5-phase-2-ecosystem-detection-map)
6. [Phase 3: safety analysis and staleness engine](#6-phase-3-safety-analysis-and-staleness-engine)
7. [Phase 4: C FFI surface](#7-phase-4-c-ffi-surface)
8. [Phase 5: Swift macOS integration layer](#8-phase-5-swift-macos-integration-layer)
9. [Phase 6: SwiftUI menu bar application](#9-phase-6-swiftui-menu-bar-application)
10. [Phase 7: deletion engine and safety protocols](#10-phase-7-deletion-engine-and-safety-protocols)
11. [Phase 8: SMAppService automation engine](#11-phase-8-smappservice-automation-engine)
12. [Phase 9: Tauri app (Linux and Windows)](#12-phase-9-tauri-app-linux-and-windows)
13. [Phase 10: web dashboard and authentication](#13-phase-10-web-dashboard-and-authentication)
14. [Phase 11: license integration](#14-phase-11-license-integration)
15. [Phase 12: distribution and notarization](#15-phase-12-distribution-and-notarization)
16. [Phase 13: open source release](#16-phase-13-open-source-release)
17. [Risk registry](#17-risk-registry)
18. [Dependency graph and critical path](#18-dependency-graph-and-critical-path)
19. [Appendix A: technical specifications](#appendix-a-technical-specifications)
20. [Appendix B: recommended Cargo.toml](#appendix-b-recommended-cargotoml)


## 1. Project north stars

### Quantitative targets

| Metric | Target |
|--------|--------|
| Scan time (500K files, macOS) | Under 8 seconds |
| Scan time (2M files, large monorepo) | Under 20 seconds |
| Peak memory during scan | Under 150 MB |
| Ecosystem coverage | 80+ project types |
| False positive rate | Zero |
| FFI crossings per 1M-file scan | ~1,000 (batched, not 1M) |

### Qualitative targets

The tool runs on a developer's machine for six months without a single incident of data loss or project corruption. The detection map covers enough ecosystems that the average developer finds at least one meaningful cleanup on the first scan without configuring anything. The Rust core is written once and works the same on macOS, Linux, and Windows.


## 2. Monorepo architecture

```
jhara/
├── Cargo.toml                       Rust workspace root
├── package.json                     pnpm workspace root
├── pnpm-workspace.yaml
├── turbo.json
├── biome.json
│
├── crates/
│   ├── jhara-core/
│   │   ├── Cargo.toml
│   │   ├── build.rs                 cbindgen header generation
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── types.rs             ScanNode, ScanError
│   │       ├── scanner/
│   │       │   ├── mod.rs           jwalk traversal, skip-list, FTS_XDEV parity
│   │       │   ├── inode.rs         InodeTracker — HashSet<(u64,u64)>
│   │       │   └── dedup.rs         Windows FILE_ID_INFO path
│   │       ├── tree.rs              ScanTree: ustr interning, flat Vec, O(N) rollup
│   │       ├── detector/
│   │       │   ├── mod.rs           ProjectDetector, monorepo resolution
│   │       │   ├── signatures.rs    Ecosystem signature database
│   │       │   └── frameworks.rs    package.json parsing
│   │       ├── classifier/
│   │       │   ├── mod.rs           SafetyClassifier
│   │       │   ├── staleness.rs     mtime + .git/HEAD analysis
│   │       │   └── blocklist.rs     Absolute never-delete patterns
│   │       ├── cleaner/
│   │       │   ├── mod.rs           DeletionPlan, staged ordering
│   │       │   └── git.rs           git status --porcelain via Process
│   │       └── ffi/
│   │           ├── mod.rs           jhara_scan_start, jhara_scan_free
│   │           └── types.rs         ScanNodeC #[repr(C)], ScanNodeBatchC
│   │
│   └── jhara-macos-ffi/
│       ├── Cargo.toml               crate-type = ["staticlib"]
│       └── src/lib.rs               Re-exports jhara-core FFI for Xcode linkage
│
├── apps/
│   ├── macos/
│   │   └── jhara/
│   │       ├── Scanner/
│   │       │   ├── iCloudGuard.swift
│   │       │   ├── FSEventsMonitor.swift
│   │       │   └── DiskUsageReporter.swift
│   │       ├── UI/
│   │       ├── Automation/
│   │       └── Cleaner/
│   │
│   ├── tauri/
│   │   ├── package.json
│   │   ├── next.config.mjs          output:'export', unoptimized images, trailingSlash
│   │   ├── src/                     Next.js React frontend
│   │   └── src-tauri/
│   │       ├── Cargo.toml           jhara-core as dependency
│   │       ├── tauri.conf.json      frontendDist:"../out"
│   │       └── src/
│   │           ├── main.rs
│   │           └── commands/
│   │               ├── scan.rs      jhara-core called directly (no FFI)
│   │               └── clean.rs
│   │
│   ├── server/
│   └── web/
│
└── packages/
    ├── api/
    ├── auth/
    ├── db/
    ├── ui/
    ├── env/
    └── config/
```


## 3. Phase 0: tooling and CI/CD

**Status:** Complete

### JavaScript/TypeScript toolchain

| Tool | Version | Purpose |
|------|---------|---------|
| pnpm | 9+ | Package management |
| Turborepo | Latest | Task caching and orchestration |
| TypeScript | 5.x | Type checking |
| Biome | Latest | Lint and format |
| Vitest | Latest | Unit testing |

### Rust toolchain

| Tool | Version | Purpose |
|------|---------|---------|
| Rust | 1.77+ | Language |
| cbindgen | 0.26+ | C header generation from Rust |
| sccache | Latest | Compiler cache for CI |
| cargo-nextest | Latest | Faster test runner |

### CI/CD matrix

```yaml
# Triggered on PR and merge to main
jobs:
  rust:
    matrix: [ubuntu-latest, macos-14, windows-latest]
    steps: cargo test -p jhara-core

  macos-app:
    runs-on: macos-14
    steps: build libjhara_universal.a → xcodebuild test

  tauri:
    matrix: [ubuntu-latest, windows-latest]
    steps:
      ubuntu: apt-get libwebkit2gtk-4.1-dev libappindicator3-dev → pnpm tauri build
      windows: pnpm tauri build

  web:
    runs-on: ubuntu-latest
    steps: pnpm build (server + web packages)
```

### Deliverables

- [x] All existing packages build cleanly
- [x] Biome configuration, zero warnings
- [x] GitHub Actions CI on every PR
- [x] CONTRIBUTING.md with setup instructions
- [x] Root `Cargo.toml` workspace definition
- [x] `crates/jhara-core/` skeleton with `cargo test` passing
- [ ] `sccache` integrated in CI


## 4. Phase 1: jhara-core scanner engine

**Duration:** Weeks 2-6
**Blocking:** Phases 2, 3, 4, 5

This phase produces a correct, performant, cross-platform filesystem traversal engine in Rust. No FFI yet. The output is a Rust library testable with `cargo test` and a `jhara-cli` binary for interactive benchmarking.

### 1.1 ScanNode (Rust)

Ports `ScanNode.swift` to Rust. All fields must be derivable on macOS, Linux, and Windows.

```rust
pub struct ScanNode {
    pub path: PathBuf,
    pub name: String,
    pub inode: u64,               // st_ino on Unix; FILE_ID_INFO low bits on Windows
    pub device_id: u64,           // st_dev on Unix; VolumeSerialNumber on Windows
    pub physical_size: u64,       // st_blocks*512 on Unix; cluster-rounded on Windows
    pub logical_size: u64,        // st_size / Metadata::len()
    pub modification_secs: i64,   // Unix epoch seconds
    pub modification_nanos: u32,  // Fractional nanoseconds
    pub link_count: u32,          // st_nlink / number_of_links()
    pub kind: NodeKind,           // File, DirPre, DirPost, Symlink, Other
}
```

### 1.2 InodeTracker (Rust)

Ports `InodeTracker.swift` to Rust.

```rust
pub struct InodeTracker {
    seen: HashSet<(u64, u64)>,  // (device_id, inode)
}

impl InodeTracker {
    pub fn should_count(&mut self, device: u64, inode: u64) -> bool {
        self.seen.insert((device, inode))
    }
}
```

On Windows, files with `number_of_links() == 1` skip the `FILE_ID_INFO` query entirely. Only files with `link_count > 1` open a handle to retrieve the 128-bit file ID.

### 1.3 ScanTree (Rust)

Ports `ScanTree.swift` to Rust with better rollup performance.

```rust
pub struct TreeNode {
    pub name: Ustr,              // ustr interned path segment
    pub parent_idx: Option<usize>,
    pub physical_size: u64,
    pub logical_size: u64,
    pub child_count: u32,
}

pub struct ScanTree {
    pub nodes: Vec<TreeNode>,
    pub path_index: HashMap<PathBuf, usize>,
}

impl ScanTree {
    pub fn rollup(&mut self) {
        // Single O(N) reverse pass — no per-insertion locking
        for i in (1..self.nodes.len()).rev() {
            let (phys, log) = (self.nodes[i].physical_size, self.nodes[i].logical_size);
            if let Some(p) = self.nodes[i].parent_idx {
                self.nodes[p].physical_size += phys;
                self.nodes[p].logical_size += log;
            }
        }
    }
}
```

### 1.4 jwalk scanner

```rust
pub fn scan(
    roots: &[PathBuf],
    skip_list: HashSet<PathBuf>,  // iCloud paths from Swift pre-scan
    callback: impl Fn(Vec<ScanNode>) + Send + Sync,
) -> Result<ScanStats, ScanError> {
    // Concurrent root scanning via rayon TaskPool
    // FTS_XDEV parity: capture root device_id, filter in process_read_dir
    // FTS_PHYSICAL parity: follow_links(false) — jwalk default
    // Skip-list: O(1) HashSet<PathBuf> lookup via Path::starts_with()
    // iNode dedup: InodeTracker per scan session
    // Batched callback: accumulate 1024 nodes, then call
}
```

### 1.5 Windows physical size

```rust
#[cfg(windows)]
fn physical_size(path: &Path, meta: &Metadata, cluster_size: u64) -> u64 {
    use std::os::windows::fs::MetadataExt;
    let attrs = meta.file_attributes();
    let is_special = (attrs & 0x800) != 0 || (attrs & 0x200) != 0
                  || (attrs & 0x00400000) != 0;  // RECALL_ON_DATA_ACCESS = OneDrive
    if is_special {
        return 0;  // Skip OneDrive placeholders entirely
    }
    let len = meta.len();
    if len == 0 { 0 } else { ((len - 1) / cluster_size + 1) * cluster_size }
}
```

### 1.6 Performance targets

| Operation | Target |
|-----------|--------|
| 500K files, macOS APFS | Under 4 seconds |
| 2M files, large monorepo | Under 12 seconds |
| Peak memory, 2M-file tree | Under 150 MB |
| Incremental FSEvents update | Under 500 ms |

### Deliverables

- [x] `crates/jhara-core/src/types.rs` — ScanNode, NodeKind
- [x] `crates/jhara-core/src/scanner/inode.rs` — InodeTracker
- [x] `crates/jhara-core/src/scanner/mod.rs` — jwalk traversal, skip-list, FTS_XDEV
- [x] `crates/jhara-core/src/scanner/dedup.rs` — Windows FILE_ID_INFO path
- [x] `crates/jhara-core/src/tree.rs` — ScanTree, rollup
- [x] `crates/jhara-cli/` — binary for interactive testing and benchmarking
- [x] Unit tests: symlink cycles, hard links, cross-device boundaries, empty dirs
- [x] Windows tests: OneDrive placeholder detection, NTFS hard-link dedup
- [x] Benchmark harness against generated test directory trees


## 5. Phase 2: ecosystem detection map

**Duration:** Weeks 4-8 (overlaps Phase 1)
**Blocking:** Phase 3

### 2.1 Signature architecture

```rust
pub struct ProjectSignature {
    pub filename: &'static str,
    pub ecosystem: Ecosystem,
    pub artifact_paths: &'static [ArtifactPath],
}

pub struct ArtifactPath {
    pub relative_path: &'static str,
    pub safety_tier: SafetyTier,
    pub is_global: bool,
    pub recovery_command: Option<&'static str>,
}

pub enum SafetyTier { Safe, Caution, Risky, Blocked }
```

### 2.2 Framework detection via package.json

Parses `dependencies` and `devDependencies` to determine which artifact directories to include (`.next/`, `.svelte-kit/`, `.astro/`, etc.). Lightweight: only reads the two dependency maps, doesn't resolve the full npm graph.

### 2.3 Monorepo resolution

- Turborepo: `turbo.json` at root → local cache at `.turbo/`, `node_modules/.cache/turbo/`
- PNPM workspace: `pnpm-workspace.yaml` → root `node_modules/` with hard-linked sub-packages; inode dedup is critical here
- Nx: `nx.json` → `.nx/cache/`
- Cargo workspace: `[workspace]` in root `Cargo.toml` → single `target/` at workspace root

### 2.4 Xcode DerivedData reverse lookup

```rust
pub fn resolve_xcode_project(derived_data_dir: &Path) -> Option<PathBuf> {
    let info_plist = derived_data_dir.join("info.plist");
    // Parse WorkspacePath from info.plist
    // If path no longer exists: orphaned, Safe regardless of age
    // If path exists: use for staleness check
}
```

### Deliverables

- [x] `crates/jhara-core/src/detector/signatures.rs` — 80+ ecosystem entries
- [x] `crates/jhara-core/src/detector/frameworks.rs` — package.json dependency parsing
- [x] `crates/jhara-core/src/detector/mod.rs` — MonorepoResolver, XcodeDerivedDataResolver
- [x] JSON data files for signatures (community-updateable without Rust recompilation)
- [x] Unit tests for every ecosystem with example directory structures


## 6. Phase 3: safety analysis and staleness engine

**Duration:** Weeks 6-9

### 3.1 Staleness checker

```rust
pub struct StalenessResult {
    pub project_root: PathBuf,
    pub last_activity: SystemTime,    // max(mtime(descriptor), mtime(.git/HEAD))
    pub is_stale: bool,
    pub has_dirty_working_tree: bool,
    pub confidence: Confidence,       // High if .git exists, Medium otherwise
}
```

### 3.2 Git safety check

Runs `git status --porcelain` via `std::process::Command` before any deletion in a git repository. Cached per project per scan session. Non-empty output = dirty working tree = warn before proceeding.

### 3.3 Absolute blocklist

```rust
pub const BLOCKLIST_PATTERNS: &[&str] = &[
    "terraform.tfstate",
    "terraform.tfstate.backup",
    ".terraform/terraform.tfstate",
    ".vagrant/machines",
    "*.pem",
    "*.key",
    ".env",
    ".env.local",
    ".env.production",
    ".env.staging",
];
```

Blocklist patterns are checked before safety tier classification. A file matching any pattern is excluded from all cleanup operations regardless of its enclosing directory's classification.

### 3.4 Apple Silicon orphan detection (macOS only, Swift layer)

Detects legacy x86_64 artifacts on Apple Silicon: Intel Homebrew at `/usr/local/Cellar/`, x86_64-only DerivedData entries, Intel-only simulator runtimes. Detected via `sysctl hw.optional.arm64`. Flagged as a dedicated cleanup category, Safe to remove unconditionally.

### Deliverables

- [x] `crates/jhara-core/src/classifier/staleness.rs`
- [x] `crates/jhara-core/src/classifier/mod.rs`
- [x] `crates/jhara-core/src/classifier/blocklist.rs`
- [x] `crates/jhara-core/src/cleaner/git.rs`
- [x] Tests: clean repo, dirty repo, no-git project, blocklist pattern matching


## 7. Phase 4: C FFI surface

**Duration:** Weeks 7-10
**Blocking:** Phase 5

### 4.1 ScanNodeC layout

```rust
#[repr(C)]
pub struct ScanNodeC {
    // 8-byte pointers (arena-owned, valid for scan lifetime)
    pub path: *const c_char,
    pub name: *const c_char,
    // 8-byte integers
    pub inode: u64,
    pub physical_size: i64,
    pub logical_size: i64,
    pub modification_secs: i64,
    // Smaller fields grouped to minimize padding
    pub modification_nanos: u32,
    pub link_count: u16,
    pub kind: u8,
    pub _padding: u8,
}
// Total: 64 bytes, explicit layout, no compiler-dependent padding

#[repr(C)]
pub struct ScanNodeBatchC {
    pub nodes: *const ScanNodeC,
    pub count: usize,
}
```

### 4.2 FFI exports

```rust
#[no_mangle]
pub extern "C" fn jhara_scan_start(
    roots: *const *const c_char,
    root_count: usize,
    skip_list: *const *const c_char,    // iCloud paths from Swift
    skip_count: usize,
    callback: extern "C" fn(ScanNodeBatchC, ctx: *mut c_void),
    ctx: *mut c_void,
) -> *mut JharaScanHandle;

#[no_mangle]
pub extern "C" fn jhara_scan_cancel(handle: *mut JharaScanHandle);

#[no_mangle]
pub extern "C" fn jhara_scan_free(handle: *mut JharaScanHandle);

// Tree query functions (called after scan completes)
#[no_mangle]
pub extern "C" fn jhara_tree_physical_size(
    handle: *mut JharaScanHandle,
    path: *const c_char,
) -> i64;
```

### 4.3 cbindgen configuration

`build.rs` runs cbindgen to generate `jhara_core.h`. The header is imported into Swift via the Objective-C Bridging Header.

### 4.4 Early validation test

Before wiring the full scanner: create a mock `ScanNodeC` array with recognizable hex values (`inode: 0xDEADBEEFCAFEBABE`). Pass over the FFI boundary. Assert in a Swift unit test that every numerical property decodes exactly as instantiated. This catches struct alignment issues before any scanner code is written.

### Deliverables

- [x] `crates/jhara-core/src/ffi/types.rs` — ScanNodeC, ScanNodeBatchC
- [x] `crates/jhara-core/src/ffi/mod.rs` — all `extern "C"` exports
- [x] `crates/jhara-core/build.rs` — cbindgen header generation
- [x] `crates/jhara-macos-ffi/` — staticlib re-export for Xcode
- [x] Xcode build script: `cargo build` both architectures → `lipo` → `libjhara_universal.a`
- [x] Bridging header importing `jhara_core.h`
- [x] Swift unit test asserting struct alignment correctness


## 8. Phase 5: Swift macOS integration layer

**Duration:** Weeks 9-11
**Depends on:** Phase 4

This phase replaces the old Swift scanner files with the thin orchestration layer that calls into `jhara-core`.

### Files being deleted from Swift

```
apps/macos/jhara/Scanner/
  FTSScanner.swift      → replaced by Rust jhara-core/src/scanner/
  ScanNode.swift        → replaced by Rust jhara-core/src/types.rs
  InodeTracker.swift    → replaced by Rust jhara-core/src/scanner/inode.rs
  ScanTree.swift        → replaced by Rust jhara-core/src/tree.rs
```

### Files remaining / refactored in Swift

**iCloudGuard.swift** (refactored for pre-scan pattern):

```swift
// Called BEFORE jhara_scan_start, not during traversal
func buildSkipList(homeURL: URL) -> [String] {
    var skipPaths: [String] = []
    let keys: [URLResourceKey] = [.isUbiquitousItemKey, .ubiquitousItemDownloadingStatusKey]
    guard let enumerator = FileManager.default.enumerator(
        at: homeURL,
        includingPropertiesForKeys: keys,
        options: [.skipsSubdirectoryDescendants]
    ) else { return [] }
    for case let url as URL in enumerator {
        guard let vals = try? url.resourceValues(forKeys: Set(keys)) else { continue }
        if vals.isUbiquitousItem == true {
            skipPaths.append(url.path)
        }
    }
    return skipPaths
}
```

**FSEventsMonitor.swift** (refactored to trigger Rust incremental re-scan):

The FSEvents callback computes the minimal covering ancestor set from the changed paths using `Path::starts_with()`-equivalent logic, then calls `jhara_scan_start()` with only the affected subtree roots.

**DiskUsageReporter.swift** (unchanged): queries `volumeAvailableCapacityForImportantUsage`.

### Swift actor receiving batches

```swift
actor ScanCoordinator {
    private var tree: [ScanNodeProxy] = []

    // Called from C FFI callback (arrives on Rust/rayon thread)
    // Dispatch to actor for safe sequential processing
    func receive(batch: UnsafeBufferPointer<ScanNodeC>) {
        for node in batch {
            tree.append(ScanNodeProxy(from: node))
        }
    }
}

// The C callback bridges to the Swift actor
let ctx = Unmanaged.passRetained(coordinator).toOpaque()
jhara_scan_start(roots, rootCount, skipList, skipCount, { batch, ctx in
    let coordinator = Unmanaged<ScanCoordinator>.fromOpaque(ctx!).takeUnretainedValue()
    Task { await coordinator.receive(batch: UnsafeBufferPointer(start: batch.nodes, count: batch.count)) }
}, ctx)
```

### Deliverables

- [x] Delete FTSScanner.swift, ScanNode.swift, InodeTracker.swift, ScanTree.swift
- [x] Refactored iCloudGuard.swift — pre-scan only, no traversal-time checks
- [x] Refactored FSEventsMonitor.swift — minimal covering set, Rust re-scan trigger
- [x] ScanCoordinator.swift — Swift actor receiving batched FFI callbacks
- [x] Integration test: scan a local test directory, verify counts match `du` output


## 9. Phase 6: SwiftUI menu bar application

**Duration:** Weeks 10-14
**Depends on:** Phase 5

### Application architecture

No Dock icon. `NSStatusItem` for menu bar presence. `NSPanel` main window, floats above desktop, dismisses on focus loss. All data in Swift actors observed via `@Observable`.

### Treemap visualization

Squarified algorithm, implemented with SwiftUI `Canvas`. Color by safety tier: green (Safe), amber (Caution), red (Risky). Click to navigate to list entry. Hover tooltip with path, size, last activity, classification reason.

### Project-centric results view

Results grouped by project, not artifact type. Each project shows total reclaimable size, expanded to constituent artifacts with individual sizes and safety tiers. "Remove Safe Items" button per project. "Review All" for Caution and Risky items.

### Deletion confirmation flows

Safe: one-click, summary before execution. Caution: explicit checkbox per category with explanation of what deletion means. Risky: per-item dialog, no batch removal.

### Deliverables

- [x] Menu bar app, no Dock icon, NSStatusItem + NSPanel
- [x] ScanView with progress ring during scan
- [x] ResultsView with Squarified treemap and project-grouped list
- [x] Deletion confirmation flows for all three safety tiers
- [x] Dark mode, full VoiceOver support
- [x] Apple Silicon orphan detection UI


## 10. Phase 7: deletion engine and safety protocols

**Duration:** Weeks 12-15
**Depends on:** Phase 6

### Deletion order (level 1 to level 5)

1. Pure caches (`node_modules/.cache/`, `~/.npm/_cacache/`) — sub-minute recovery
2. Build outputs (`dist/`, `.next/`, `build/`) — build-script recovery
3. Dependency directories (`node_modules/`, `vendor/`, `.venv/`) — package manager recovery
4. Heavy compilation outputs (Rust `target/`, Xcode DerivedData) — full recompile
5. Stateful items (Docker volumes, Conda environments) — explicit per-item confirmation

All deletions use `FileManager.default.trashItem()`. Items go to Trash, not permanent deletion. Large directories (10K+ files) are enumerated in batches to prevent Finder progress from hanging.

### Git safety before any deletion

`jhara-core`'s git checker runs `git status --porcelain` before touching any project with a `.git/` directory. Non-empty output triggers a warning and requires explicit confirmation even for Safe-tier artifacts.

### Deliverables

- [x] `DeletionPlan.swift` — staged ordering from ScanTree output
- [x] `TrashCoordinator.swift` — timeout handling, batch enumeration, cloud-sync path guard
- [x] Background deletion with real-time progress
- [x] Pre-deletion summary, post-deletion summary with reclaimed space
- [x] Tests: interrupted deletion, partial completion recovery


## 11. Phase 8: SMAppService automation engine

**Duration:** Weeks 14-18
**Pro tier feature**
**Status:** Drafted. All Swift files authored and staged in `phase-8/` directory (`JharaAutomationAgent.swift`, `RuleEngine.swift`, `SMAppServiceManager.swift`, `XPCProtocol.swift`, `NotificationManager.swift`, `RuleEditingView.swift`, `GRDBSchema.swift`). Next step: copy files into `apps/macos/jhara/Automation/`, add them to the Xcode project/target, and wire up the XPC connection.

### Rule model

```swift
struct AutomationRule: Codable, Identifiable {
    let id: UUID
    var name: String
    var isEnabled: Bool
    var ecosystems: Set<Ecosystem>
    var artifactTypes: Set<ArtifactType>
    var staleThresholdDays: Int
    var safetyTierLimit: SafetyTier
    var schedule: Schedule   // .onWake, .daily(hour:), .weekly(dayOfWeek:hour:)
    var notificationBehavior: NotificationBehavior
}
```

### SMAppService registration

```swift
import ServiceManagement

SMAppService.agent(plistName: "com.hmyousuf.jhara.automation.plist").register()
// Appears in: System Settings > General > Login Items and Extensions
```

### Notification actions

"Clean Now" sends an XPC message to the agent, which runs cleanup in the background. "View Details" opens the main app and navigates to scan results. "Remind Tomorrow" snoozes and reschedules for the next wake cycle.

### Deliverables

- [ ] `JharaAutomationAgent` separate lightweight executable target
- [ ] `RuleEngine.swift` evaluating rules against scan state
- [ ] SMAppService registration and unregistration
- [ ] XPC protocol and implementation
- [ ] Notification with action buttons
- [ ] Rule editing UI in Settings
- [ ] GRDB schema for automation rules and scan history


## 12. Phase 9: Tauri app (Linux and Windows)

**Duration:** Weeks 14-20
**Parallel with:** Phases 8, 10
**Status:** Not started. `apps/desktop/src-tauri/` directory exists but is empty. Next.js frontend scaffold exists at `apps/desktop/` but there's no Tauri backend, no `tauri.conf.json`, no Rust commands. Note: the directory is named `apps/desktop/` in the repo rather than `apps/tauri/`.

### Frontend configuration

```js
// next.config.mjs
export default {
  output: 'export',
  images: { unoptimized: true },
  trailingSlash: true,
  assetPrefix: process.env.NODE_ENV === 'development'
    ? `http://localhost:3000`
    : undefined,
};
```

```json
// tauri.conf.json (excerpt)
{
  "build": {
    "frontendDist": "../out",
    "devUrl": "http://localhost:3000",
    "beforeDevCommand": "pnpm dev",
    "beforeBuildCommand": "pnpm build"
  }
}
```

### Rust backend (src-tauri)

`jhara-core` is a direct Cargo dependency, no FFI boundary. Tauri commands call `jhara_core::scan()` directly. Progress events are emitted to the frontend in 1,024-node batches via `tauri::Window::emit()`.

### Linux system tray

`tauri-plugin-positioner` for tray positioning. `.desktop` file deployed to `~/.local/share/applications/` as fallback for GNOME Wayland environments without AppIndicator. `tauri-plugin-single-instance` to surface the app window if launched while the agent is already running.

### Linux background automation

systemd user unit (`~/.config/systemd/user/jhara-agent.service`). Wake detection via D-Bus `PrepareForSleep` signal (boolean `false` = system resumed) using `zbus` crate.

```rust
// Wake detection on Linux
let stream = proxy.receive_prepare_for_sleep().await?;
while let Some(signal) = stream.next().await {
    if !signal.args()?.start {
        // System resumed — trigger rule evaluation
    }
}
```

### Windows background automation

HKCU Run registry key for login persistence. Wake detection via invisible Win32 sink window receiving `WM_POWERBROADCAST` with `PBT_APMRESUMEAUTOMATIC`. OneDrive placeholder guard: check `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS` before any file access during traversal.

### Linux packaging

Primary: `.deb` and `.AppImage` (no sandbox, full dotfile access). Flatpak (future): must declare explicit `--filesystem=~/.cargo:ro`, `--filesystem=~/.npm:ro`, etc. The default `--filesystem=home` excludes dotfiles.

### Deliverables

- [ ] `apps/tauri/` Tauri v2 project with Next.js static export
- [ ] `src-tauri/` Rust backend calling jhara-core directly
- [ ] Scan command streaming 1,024-node batches to frontend
- [ ] Linux: systemd user unit, D-Bus wake detection, tray + `.desktop` fallback
- [ ] Windows: HKCU Run, invisible sink window for power events, OneDrive guard
- [ ] DEB, AppImage builds in CI
- [ ] Windows NSIS installer in CI


## 13. Phase 10: web dashboard and authentication

**Duration:** Weeks 3-10 (in parallel, already partially built)
**Status:** Scaffolded but incomplete. Email/password auth is wired via BetterAuth + Prisma. Route pages exist (`/`, `/login`, `/success`, `/dashboard`). But: (1) the landing page is a health-check stub, not a product page; (2) GitHub OAuth plugin isn't configured; (3) integration uses Polar for payments, not Lemon Squeezy as spec'd; (4) tRPC router only has `healthCheck` + `privateData`; (5) Prisma schema has only BetterAuth tables, no `license` or `machine_activation` tables.

| Route | Purpose |
|-------|---------|
| `/` | Landing page |
| `/login` | Email/password + GitHub OAuth |
| `/success` | Post-purchase, shows license key |
| `/dashboard` | License status, machine activations |
| `/dashboard/machines` | Per-machine deactivation |

License validation flow: macOS app sends `POST /api/license/activate` → server validates against Lemon Squeezy → records activation → returns token → app stores in Keychain. Subsequent launches: local Keychain check (7-day offline cache) before network call.

### Deliverables

- [ ] Landing page
- [ ] Auth flow (email + GitHub OAuth via Better Auth)
- [ ] Dashboard with license status and machine list
- [ ] tRPC routes for activate, verify, deactivate
- [ ] Prisma migrations for license and machine_activation tables


## 14. Phase 11: license integration

**Duration:** Weeks 10-13

**Lemon Squeezy products:**
- Jhara Pro (single): $12.99, 2-machine activation limit
- Jhara Pro (team): $39.99, 5-machine activation limit

**Keychain storage (macOS):** activation token + last-validated timestamp. 7-day offline grace period.

**Tauri (Linux/Windows):** license token stored in OS keychain via `keyring` crate.

### Deliverables

- [ ] Lemon Squeezy products configured with sandbox testing
- [ ] Webhook handler for purchase and refund events
- [ ] `LicenseKeychainManager.swift` (macOS)
- [ ] `keyring` integration for Tauri (Linux/Windows)
- [ ] 7-day offline cache with clear messaging on expiry
- [ ] Activation UI in macOS Settings and Tauri Settings


## 15. Phase 12: distribution and notarization

**Duration:** Weeks 18-20

**macOS:** Developer ID Application certificate. Signed `.dmg` with `create-dmg`. Notarized with `notarytool`. Sparkle 2 for EdDSA-signed updates.

**Linux:** `.deb` and `.AppImage` via Tauri bundler. Package repository (future).

**Windows:** NSIS installer via Tauri bundler. Microsoft code signing (future).

### GitHub Actions release pipeline (on tag v*.*.*)

```yaml
# macOS: xcodebuild archive → Developer ID sign → notarize → staple → DMG
# Linux: tauri build → DEB + AppImage artifacts
# Windows: tauri build → NSIS installer
# All: upload to GitHub Releases + update Sparkle appcast
```

### Deliverables

- [ ] Developer ID cert in CI secrets
- [ ] Notarization pipeline via `notarytool`
- [ ] `create-dmg` config for macOS DMG
- [ ] Tauri bundler config for Linux and Windows
- [ ] Sparkle appcast update on release
- [ ] Notarization verified against fresh macOS VM


## 16. Phase 13: open source release

**Duration:** Weeks 20-22

### Pre-release checklist

- [ ] Rust: `clippy` clean, `cargo test` 100% pass, no `unwrap()` in production paths
- [ ] Swift: SwiftLint clean, zero `TODO:` in release commits
- [ ] Test coverage: scanner, safety classifier, rule engine above 70%
- [ ] No hardcoded credentials anywhere (CI check via truffleHog or similar)
- [ ] CHANGELOG.md, architecture diagram
- [ ] CONTRIBUTING.md includes template for adding new ecosystem types

### Deliverables

- [ ] v1.0.0 tagged, signed, notarized, on GitHub Releases
- [ ] Jhara website (static) live
- [ ] Lemon Squeezy storefront live
- [ ] Hacker News, developer community announcement


## 17. Risk registry

| ID | Risk | Probability | Impact | Mitigation |
|----|------|-------------|--------|------------|
| R1 | ScanNodeC struct alignment differs between Rust and Swift | Medium | Critical | Early validation test with hex-valued mock struct before scanner wiring |
| R2 | Swift actor flooding from unbounded FFI callbacks | High | High | 1,024-node batching; proven in isolation before scan integration |
| R3 | iCloud hydration triggered by Rust traversal | Medium | High | iCloudGuard pre-scan skip-list; integration test against iCloud-enabled directory |
| R4 | OneDrive FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS triggers downloads on Windows | Medium | High | Attribute check before any stat; Windows-specific CI test with mock reparse points |
| R5 | GNOME Wayland tray silent failure on Linux | High | Medium | Mandatory `.desktop` file fallback; `tauri-plugin-single-instance` for window recovery |
| R6 | Windows Defender 100x slowdown on node_modules scan | Medium | Medium | Benchmark on Windows with Defender active; only stat directory entries, never read file contents |
| R7 | jwalk has no FTS_XDEV equivalent, scanner crosses into network volume | Low | Medium | Manual device ID check in `process_read_dir`; unit test with tmpfs mount |
| R8 | bumpalo arena paths freed before Swift copies them | Low | Critical | Arena lifetime tied to scan handle; explicit drop after scan completion acknowledged by Swift |
| R9 | Cargo workspace conflicts with pnpm workspace in CI | Medium | Low | sccache for Rust; Turborepo outputs declared for cargo artifacts |
| R10 | Community contribution adds wrong safety classification | Medium | Medium | Safety classification requires reviewer approval; mandatory test per ecosystem entry |


## 18. Dependency graph and critical path

```
Phase 0 (Tooling)
  └── Phase 1 (jhara-core Scanner)
        ├── Phase 2 (Ecosystem Detector) — can start during Phase 1
        │     └── Phase 3 (Safety/Staleness)
        │           └── Phase 4 (C FFI Surface)
        │                 └── Phase 5 (Swift Integration Layer)
        │                       └── Phase 6 (SwiftUI App)
        │                             ├── Phase 7 (Deletion Engine)
        │                             │     └── Phase 8 (Automation, Pro)
        │                             └── Phase 11 (License, parallel)
        │
        └── Phase 9 (Tauri Linux/Windows) — can start after Phase 3
              (no FFI needed — jhara-core is a direct Cargo dependency)

Phase 0 → Phase 10 (Web Dashboard) — independent, runs in parallel
Phase 10 → Phase 11 (License Integration)
Phase 7 + Phase 11 → Phase 12 (Distribution)
Phase 12 → Phase 13 (Release)
```

**Critical path:** Phases 1 → 2 → 3 → 4 → 5 → 6 → 7 → 8 → 12 → 13

**Key advantage of v2 architecture:** Phase 9 (Tauri) can start as soon as Phase 3 is stable because Tauri calls `jhara-core` directly with no FFI. It doesn't block on Phase 4 (FFI). Linux and Windows development can proceed in parallel with the macOS FFI integration work.


## Appendix A: technical specifications

### Key APIs used

| Platform | API | Purpose |
|----------|-----|---------|
| Rust (all) | `jwalk` 0.8 | Parallel filesystem traversal |
| Rust (all) | `rayon` 1.10 | Work-stealing thread pool |
| Rust (all) | `ustr` 0.10 | Lock-free path segment interning |
| Rust (all) | `bumpalo` 3.16 | Arena allocator for FFI path strings |
| Rust (Unix) | `MetadataExt::ino()`, `::dev()` | Inode and device ID |
| Rust (Windows) | `GetFileInformationByHandleEx(FileIdInfo)` | File identity for dedup |
| Rust (Windows) | `GetDiskFreeSpaceW` | Cluster size for physical size calc |
| Rust (Linux) | `zbus` | D-Bus PrepareForSleep wake detection |
| Rust (Windows) | `windows-rs` | WM_POWERBROADCAST, invisible sink window |
| Swift (macOS) | `URLResourceKey.isUbiquitousItemKey` | iCloud path detection |
| Swift (macOS) | `FSEvents` | Directory change notifications |
| Swift (macOS) | `volumeAvailableCapacityForImportantUsage` | Accurate free space |
| Swift (macOS) | `SMAppService` | Background agent registration |
| Swift (macOS) | `FileManager.trashItem()` | Safe deletion to Trash |
| Swift (macOS) | `Process` | `git status --porcelain` |
| Swift (macOS) | Keychain Services | License token storage |
| Swift (macOS) | Sparkle 2 | EdDSA-signed app updates |
| Swift (macOS) | GRDB.swift | SQLite, automation rules and scan history |

### macOS minimum deployment target

macOS 14 Sonoma. `SMAppService` was introduced in macOS 13 but had documented edge cases in early point releases. Requiring macOS 14 gives a stable implementation.

### Safety tier reference

| Tier | Description | Deletion requirement |
|------|-------------|---------------------|
| Safe | Auto-regenerated by build tool | One-click with summary |
| Caution | Expensive to rebuild or has historical value | Checkbox confirmation per category |
| Risky | Non-recoverable from VCS | Per-item dialog |
| Blocked | Never delete under any circumstance | Not presented to user |


## Appendix B: recommended Cargo.toml

```toml
[workspace]
members = [
    "crates/jhara-core",
    "crates/jhara-macos-ffi",
    "apps/tauri/src-tauri",
]
resolver = "2"

# crates/jhara-core/Cargo.toml
[package]
name = "jhara-core"
version = "0.1.0"
edition = "2021"

[lib]
name = "jhara_core"
crate-type = ["lib"]   # staticlib exposed via jhara-macos-ffi

[dependencies]
# Traversal and parallelism
jwalk = "0.8.1"
rayon = "1.10.0"

# String interning and arena allocation
ustr = "0.10.0"
bumpalo = "3.16.0"

# Cross-platform file identity
same-file = "1.0.6"

# Error handling
thiserror = "1.0"

[target.'cfg(windows)'.dependencies]
# Physical size on NTFS/ReFS
filesize = "0.2.0"
# Windows kernel bindings
windows = { version = "0.57.0", features = [
    "Win32_Storage_FileSystem",
    "Win32_System_SystemInformation",
    "Win32_Foundation",
] }

[target.'cfg(target_os = "linux")'.dependencies]
# D-Bus for PrepareForSleep wake detection
zbus = { version = "4.0", default-features = false, features = ["tokio"] }

[build-dependencies]
cbindgen = "0.26.0"

[dev-dependencies]
tempfile = "3.10"
cargo-nextest = "0.9"
```

---

*Author: H.M. Yousuf*
*Repository: [github.com/hmyousuf/jhara](https://github.com/hmyousuf/jhara)*
*License: MIT*
*Last updated: March 2026*
*Document version: 2.0.0*
*Status: Phase 0 complete, Phase 1 in progress*
