# contributing to jhara

thanks for looking into contributing. jhara is a solo project for now, but the goal is to build the most trusted and fastest developer disk cleaner. help with ecosystem rules or fixing swift ui bugs is always welcome.

## philosophy

we have one rule: be conservative. if you're not 100% sure a file is safe to delete, it shouldn't be in the safe tier. it's better to leave a stray log file than to delete a terraform state file by mistake. 

performance is the other thing. the scanner has to stay fast even with a million files. if a change slows down the core traversal, i'll probably ask you to rethink it.

## technical architecture

jhara is a monorepo with a few moving parts:

* **jhara-core (rust):** the engine itself. it handles directory traversal, project detection, and safety classification.
* **jhara-macos-ffi (rust):** a thin wrapper that exposes core logic to swift using a c-style ffi.
* **apps/macos (swift):** the native mac app. it handles platform-specific things like icloud detection and the menu bar ui.
* **apps/tauri:** the linux and windows version using next.js and tauri.

### adding a new ecosystem

if you want to add support for a new runtime or framework, you'll mostly be working in `crates/jhara-core/src/detector`.

1. **signatures:** add the project signature file (like `package.json` or `mix.exs`) to `signatures.rs`.
2. **artifacts:** define which directories are safe/riskly to delete in `classifier/mod.rs`.
3. **tests:** add a test case in `detector/mod.rs` to make sure your signature is correctly identified. 

## development workflow

you'll need rust, bun, and xcode (for mac) installed. 

### setup

```bash
git clone https://github.com/hmyousuf/jhara.git
cd jhara
bun install
```

### rust core development

don't just run `cargo build`. run tests for the core package to make sure you haven't broken the scanning logic:

```bash
cargo test -p jhara-core
```

### macOS development

if you change the rust ffi interface, you'll need to rebuild the static library for xcode to see it. anyway, the readme has the lipo commands for that. 

## pull requests

1. **open an issue:** even for small things, it helps to have a record.
2. **no em-dashes:** i've got a weird thing about em-dashes in documentation. use commas or colons instead.
3. **benchmarks:** if you change the scanner, show me a before and after on a large directory (like your home folder).

if you're stuck on the ffi logic or swift setup, just ping me in the pr itself. we'll figure it out.

---

*Author: H.M. Yousuf*
*Last updated: March 2026*
