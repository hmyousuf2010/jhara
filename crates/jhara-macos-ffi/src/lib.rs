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

use std::ffi::{c_char, c_void};
pub use jhara_core::ffi::{JharaScanHandle, types::{ScanNodeBatchC, ScanNodeC}};
use jhara_core::ffi as ffi;

#[no_mangle]
pub unsafe extern "C" fn jhara_core_scan_start(
    roots:       *const *const c_char,
    root_count:  usize,
    skip_list:   *const *const c_char,
    skip_count:  usize,
    callback:    extern "C" fn(batch: ScanNodeBatchC, ctx: *mut c_void),
    ctx:         *mut c_void,
) -> *mut JharaScanHandle {
    ffi::jhara_core_scan_start(roots, root_count, skip_list, skip_count, callback, ctx)
}

#[no_mangle]
pub unsafe extern "C" fn jhara_core_scan_cancel(handle: *mut JharaScanHandle) {
    ffi::jhara_core_scan_cancel(handle);
}

#[no_mangle]
pub unsafe extern "C" fn jhara_core_scan_free(handle: *mut JharaScanHandle) {
    ffi::jhara_core_scan_free(handle);
}

#[no_mangle]
pub unsafe extern "C" fn jhara_core_tree_physical_size(
    handle: *mut JharaScanHandle,
    path:   *const c_char,
) -> i64 {
    ffi::jhara_core_tree_physical_size(handle, path)
}

#[no_mangle]
pub unsafe extern "C" fn jhara_core_delete_paths(
    paths: *const *const c_char,
    count: usize,
) -> i32 {
    ffi::jhara_core_delete_paths(paths, count)
}

#[no_mangle]
pub unsafe extern "C" fn jhara_core_project_classify(
    project_root: *const c_char,
) -> *mut c_char {
    ffi::jhara_core_project_classify(project_root)
}

#[no_mangle]
pub unsafe extern "C" fn jhara_core_string_free(s: *mut c_char) {
    ffi::jhara_core_string_free(s);
}

#[no_mangle]
pub unsafe extern "C" fn jhara_core_projects_results_json(
    handle: *const JharaScanHandle,
) -> *mut c_char {
    ffi::jhara_core_projects_results_json(handle)
}

#[no_mangle]
pub unsafe extern "C" fn jhara_core_global_caches_json(
    handle:   *const JharaScanHandle,
    home_dir: *const c_char,
) -> *mut c_char {
    ffi::jhara_core_global_caches_json(handle, home_dir)
}

#[no_mangle]
pub unsafe extern "C" fn jhara_core_orphan_scan_json(
    handle: *const JharaScanHandle,
) -> *mut c_char {
    ffi::jhara_core_orphan_scan_json(handle)
}
