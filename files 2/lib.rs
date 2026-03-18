/// jhara-macos-ffi
///
/// This crate is a thin re-export shim.  Its sole purpose is to produce a
/// `staticlib` crate-type that Xcode can link directly.
///
/// ## Why a separate crate?
///
/// `jhara-core` is a `rlib` (the default).  A `staticlib` build of the same
/// crate would include the Rust standard library twice when linked alongside
/// other Rust code.  By isolating the `staticlib` here, the workspace remains
/// composable: other Rust crates depend on `jhara-core` as an `rlib`, while
/// Xcode links only this shim.
///
/// ## What lives here
///
/// Nothing except `pub use` re-exports.  All logic stays in `jhara-core`.

// Re-export the entire public FFI surface so callers of this staticlib get
// the same symbols they would get from jhara-core directly.
pub use jhara_core::ffi::*;
pub use jhara_core::ffi::types::*;
