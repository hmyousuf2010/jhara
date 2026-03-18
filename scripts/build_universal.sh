#!/usr/bin/env bash
# build_universal.sh
#
# Xcode "Run Script" build phase for jhara-macos-ffi.
#
# Builds jhara-macos-ffi for both Apple Silicon (aarch64) and Intel (x86_64),
# then merges them into a single universal static library with `lipo`.
#
# ## Prerequisites
# - Rust toolchains: aarch64-apple-darwin, x86_64-apple-darwin
#   Install with:  rustup target add aarch64-apple-darwin x86_64-apple-darwin
# - cargo must be on PATH (source ~/.cargo/env if needed in Xcode)
#
# ## Xcode integration
# 1. Select the app target → Build Phases → + → New Run Script Phase
# 2. Paste the full path to this script (or copy its body).
# 3. Add `$(SRCROOT)/../rust/swift/lib/libjhara_universal.a` to
#    "Link Binary With Libraries".
# 4. Set "Input Files":
#      $(SRCROOT)/../rust/crates/jhara-macos-ffi/src/lib.rs
# 5. Set "Output Files":
#      $(SRCROOT)/../rust/swift/lib/libjhara_universal.a
#
# Setting input/output files enables Xcode's incremental build skipping.

set -euo pipefail

# ── Locate workspace root relative to this script ────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
CRATE_DIR="${WORKSPACE_ROOT}/crates/jhara-macos-ffi"
OUTPUT_DIR="${WORKSPACE_ROOT}/apps/macos/lib"

# ── Build profile ─────────────────────────────────────────────────────────────
# Respect Xcode's CONFIGURATION variable: Debug → dev, anything else → release.
if [[ "${CONFIGURATION:-Release}" == "Debug" ]]; then
    CARGO_PROFILE="dev"
    CARGO_PROFILE_DIR="debug"
else
    CARGO_PROFILE="release"
    CARGO_PROFILE_DIR="release"
fi

CARGO_FLAGS=(
    "--manifest-path" "${CRATE_DIR}/Cargo.toml"
    "--profile" "${CARGO_PROFILE}"
)

# ── Ensure targets are installed ──────────────────────────────────────────────
rustup target add aarch64-apple-darwin x86_64-apple-darwin 2>/dev/null || true

# ── Build for Apple Silicon ───────────────────────────────────────────────────
echo "jhara: building aarch64-apple-darwin (${CARGO_PROFILE})…"
cargo build "${CARGO_FLAGS[@]}" --target aarch64-apple-darwin

ARM_LIB="${WORKSPACE_ROOT}/target/aarch64-apple-darwin/${CARGO_PROFILE_DIR}/libjhara_macos_ffi.a"

# ── Build for Intel ───────────────────────────────────────────────────────────
echo "jhara: building x86_64-apple-darwin (${CARGO_PROFILE})…"
cargo build "${CARGO_FLAGS[@]}" --target x86_64-apple-darwin

X86_LIB="${WORKSPACE_ROOT}/target/x86_64-apple-darwin/${CARGO_PROFILE_DIR}/libjhara_macos_ffi.a"

# ── Merge with lipo ───────────────────────────────────────────────────────────
mkdir -p "${OUTPUT_DIR}"
UNIVERSAL_LIB="${OUTPUT_DIR}/libjhara_universal.a"

echo "jhara: creating universal binary at ${UNIVERSAL_LIB}"
lipo -create \
    "${ARM_LIB}" \
    "${X86_LIB}" \
    -output "${UNIVERSAL_LIB}"

# ── Verify ────────────────────────────────────────────────────────────────────
echo "jhara: lipo info for ${UNIVERSAL_LIB}:"
lipo -info "${UNIVERSAL_LIB}"

# Confirm both required architectures are present.
lipo -info "${UNIVERSAL_LIB}" | grep -q "arm64"  || { echo "ERROR: arm64 slice missing";  exit 1; }
lipo -info "${UNIVERSAL_LIB}" | grep -q "x86_64" || { echo "ERROR: x86_64 slice missing"; exit 1; }

echo "jhara: universal library built successfully."
