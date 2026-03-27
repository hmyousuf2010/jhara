# Contributing to Jhara

Thanks for looking at this. Jhara is early-stage and contributions are welcome at every level, whether that's fixing a typo in the docs, adding a new ecosystem signature, or working on the Rust core.

A few things to know before you start.


## Where things stand right now

Check the [roadmap](ROADMAP.md) before picking something to work on. The project has a clear phase structure and some phases aren't ready for contributions yet because they depend on earlier work that isn't done.

The current focus is Phase 1 (jhara-core scanner) and Phase 10 (web dashboard). If you want to contribute Rust, that's the right place. If you're more TypeScript, the web app needs a lot of work.

Phase 8 onwards isn't open for contributions yet, mostly because the design isn't stable enough.


## Getting set up

### What you need

- Rust 1.77 or later (`rustup` is the easiest way)
- Node.js 22 or later
- pnpm 9 or later
- macOS 14 with Xcode 16, if you're working on the macOS app
- Docker, if you're working on the web dashboard (for the local database)

### Clone and build

```bash
git clone https://github.com/hmyousuf/jhara.git
cd jhara

# Rust core — this is where most of the interesting work is
cargo build -p jhara-core
cargo test -p jhara-core

# JavaScript packages
pnpm install
pnpm build

# Web dashboard (needs Docker running)
cd packages/db
docker compose up -d
pnpm db:push
```

### macOS app specifically

The macOS app needs the Rust static library built first. Run this before opening Xcode:

```bash
bash scripts/build_universal.sh
```

This builds `jhara-macos-ffi` for both Apple Silicon and Intel, runs `lipo`, and puts the universal binary at `apps/macos/lib/libjhara_core.a`. Then open `apps/macos/jhara.xcodeproj` in Xcode and it should build.

If `cargo` isn't on PATH in Xcode's Run Script phase, add `export PATH="$HOME/.cargo/bin:$PATH"` to the top of the script.


## What kinds of contributions are most useful right now

### Ecosystem signatures (great for first contributions)

The detection map in `crates/jhara-core/src/detector/signatures.rs` and `src/detector/frameworks.rs` covers 80+ project types but there are gaps. Adding a new ecosystem is self-contained, well-defined, and has a clear test pattern to follow.

What's needed for a new ecosystem entry:

1. A `ProjectSignature` in `signatures.rs` with the right descriptor filename, ecosystem variant, and artifact paths
2. At least one test in the `detector/mod.rs` test section that creates a temp directory, writes the descriptor file, calls `detector.observe()`, and asserts the right ecosystem and artifacts are detected
3. A rationale comment explaining why each artifact path has its safety tier

Look at any existing entry (Go, Zig, or Crystal are clean examples) to understand the pattern before writing your own.

### Safety classification changes

These need more scrutiny than new ecosystem entries. If you think something's classified wrong, open an issue first and explain the reasoning. Don't submit a PR that changes a `Safe` to `Caution` without a concrete argument for why. The classification directly affects what gets shown to users as one-click deletable.

### Performance improvements

If you have a before/after benchmark that shows meaningful improvement, that's worth a PR. Run `cargo bench -p jhara-core` (once the harness is built) to get numbers. "Meaningful" means at least 10% on a real dataset, not a synthetic microbenchmark.

### Bug reports

Use the bug report template. The most useful bug reports include the exact version, the OS, and a reproduction path. "It doesn't work" without context is hard to act on.

### Documentation

Corrections to the roadmap, README, or code comments are always welcome. If something's confusing or wrong, fix it.


## Rules that apply everywhere

**Tests are not optional.** New code needs tests. Changes to existing behavior need updated tests. If you're adding an ecosystem signature, there needs to be a test that actually exercises the detection path. The test doesn't have to be exhaustive, but it has to exist.

**No `unwrap()` in non-test code.** The Rust codebase uses `thiserror` for error handling. If you're writing production code that panics on failure, that's not acceptable. Tests can use `unwrap()`.

**Safety classifications need justification.** If you're adding artifact paths or changing safety tiers, add a comment explaining why. "Why is this Caution and not Safe?" should be answerable by reading the code.

**Performance-sensitive paths need measurements.** The scanner runs on millions of files. Anything that touches the hot path (traversal, inode tracking, batch processing) needs a benchmark before and after.

**Bengali is welcome.** Comments and commit messages in Bengali are fine. The project has roots in South Asian developer culture and that's intentional.


## How to submit a PR

1. Fork the repo and create a branch from `main`
2. Make your changes, write tests, make sure `cargo test -p jhara-core` passes
3. For JS changes, run `pnpm biome check` and fix any warnings
4. Open the PR and fill in the template

Don't open a PR for large changes without discussing it first. Open an issue or a discussion thread, describe what you want to do, and wait for a response. This saves everyone time.


## Commit style

We don't enforce a strict commit convention but a useful format is:

```
area: short description of what changed

Optional longer explanation if the why isn't obvious.
```

Where `area` is something like `scanner`, `detector`, `ffi`, `macos`, `web`, or `docs`.

Examples:
```
scanner: fix cross-device boundary check on Linux tmpfs
detector: add Crystal/Shards ecosystem signature
docs: clarify iCloudGuard pre-scan architecture
```


## Code of conduct

Be direct, be technical, don't be a jerk. That's basically it. This project covers a sensitive area (disk access and deletion), so discussions about safety classifications can get heated. Keep it about the technical argument, not the person.

If something's wrong, open an issue or email [hmyousuf@gmail.com](mailto:hmyousuf@gmail.com).


## License

By contributing, you agree your code is licensed under Apache 2.0. See [LICENSE](LICENSE).

---

*Questions? Open a discussion on GitHub or email directly.*
