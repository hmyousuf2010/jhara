/// # jhara_core C FFI surface
///
/// All exported symbols follow the `jhara_` prefix convention.
/// This module is the **only** place `unsafe` code is permitted to cross the
/// FFI boundary; all other crates remain safe Rust.
///
/// ## Thread Safety
///
/// `jhara_scan_start` spawns a Rayon thread pool internally.  The `callback`
/// may be invoked from any of those threads **concurrently**.  Swift callers
/// must ensure their callback implementation is thread-safe (dispatch to an
/// actor or use a lock).
///
/// ## Ownership Rules
///
/// ```text
///  Rust owns                          Swift borrows (read-only)
///  ─────────────────────────────────  ──────────────────────────────────────
///  JharaScanHandle                    *mut JharaScanHandle (opaque handle)
///  ScanNodeC arena                    ScanNodeC* inside callback only
///  NUL-terminated strings             path/name pointers inside callback only
/// ```
///
/// Swift **must** call `jhara_scan_free` exactly once per handle.
/// Calling it on a handle returned by a failed `jhara_scan_start` (null)
/// is a no-op.
pub mod types;

use std::ffi::{c_char, c_void, CStr};
use std::ptr;

use types::{ScanNodeBatchC, ScanNodeC};

// ── Opaque handle ─────────────────────────────────────────────────────────────

/// Opaque handle to an in-progress or completed scan session.
///
/// Heap-allocated by `jhara_scan_start`, freed by `jhara_scan_free`.
/// Never dereference from Swift; treat as an opaque token.
pub struct JharaScanHandle {
    /// Roots passed to this scan (owned).
    roots: Vec<String>,
    /// Paths to skip (e.g. iCloud containers passed from Swift).
    skip_list: Vec<String>,
    /// Whether a cancellation has been requested.
    cancelled: std::sync::atomic::AtomicBool,
    /// Accumulated scan results, populated after the scan completes.
    results: std::sync::Mutex<Vec<ScanNodeOwned>>,
}

/// Owned mirror of `ScanNodeC` for storage inside the handle's arena.
#[derive(Debug)]
struct ScanNodeOwned {
    path:               String,
    name:               String,
    inode:              u64,
    physical_size:      i64,
    logical_size:       i64,
    modification_secs:  i64,
    modification_nanos: u32,
    link_count:         u16,
    kind:               u8,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Converts a C string pointer to an owned `String`.
///
/// Returns `None` if `ptr` is null or contains invalid UTF-8.
///
/// # Safety
/// Caller guarantees `ptr` is either null or points to a valid NUL-terminated
/// C string that will remain valid for the duration of this call.
unsafe fn c_str_to_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: caller upholds the pointer validity invariant.
    CStr::from_ptr(ptr).to_str().ok().map(String::from)
}

/// Converts a `**c_char` array of length `count` into a `Vec<String>`.
///
/// Null pointers and invalid UTF-8 entries are silently skipped.
///
/// # Safety
/// `ptr` must be null or point to a valid array of `count` C string pointers.
unsafe fn c_string_array(ptr: *const *const c_char, count: usize) -> Vec<String> {
    if ptr.is_null() || count == 0 {
        return Vec::new();
    }
    // SAFETY: caller guarantees `ptr` points to `count` valid elements.
    std::slice::from_raw_parts(ptr, count)
        .iter()
        .filter_map(|&p| c_str_to_string(p))
        .collect()
}

// ── Public FFI exports ────────────────────────────────────────────────────────

/// Starts an asynchronous filesystem scan.
///
/// ## Parameters
///
/// | Parameter    | Description                                             |
/// |--------------|---------------------------------------------------------|
/// | `roots`      | Array of NUL-terminated UTF-8 root paths to scan       |
/// | `root_count` | Length of `roots`                                       |
/// | `skip_list`  | Array of NUL-terminated paths to exclude (iCloud etc.) |
/// | `skip_count` | Length of `skip_list`                                   |
/// | `callback`   | Called with each batch of discovered nodes              |
/// | `ctx`        | Opaque context pointer forwarded to every callback      |
///
/// ## Returns
///
/// A non-null `*mut JharaScanHandle` on success, or **null** if:
/// - `roots` is null or `root_count` is zero
/// - Memory allocation fails
///
/// The caller must eventually call `jhara_scan_free` on the returned handle.
///
/// # Safety
/// All pointer parameters must satisfy their documented invariants.
/// The `callback` must be thread-safe (may be called from multiple threads).
#[no_mangle]
pub unsafe extern "C" fn jhara_scan_start(
    roots:       *const *const c_char,
    root_count:  usize,
    skip_list:   *const *const c_char,
    skip_count:  usize,
    callback:    extern "C" fn(batch: ScanNodeBatchC, ctx: *mut c_void),
    ctx:         *mut c_void,
) -> *mut JharaScanHandle {
    // Validate mandatory inputs.
    if roots.is_null() || root_count == 0 {
        return ptr::null_mut();
    }

    let root_strings = c_string_array(roots, root_count);
    if root_strings.is_empty() {
        return ptr::null_mut();
    }

    let skip_strings = c_string_array(skip_list, skip_count);

    let handle = Box::new(JharaScanHandle {
        roots:     root_strings.clone(),
        skip_list: skip_strings,
        cancelled: std::sync::atomic::AtomicBool::new(false),
        results:   std::sync::Mutex::new(Vec::new()),
    });

    let handle_ptr = Box::into_raw(handle);

    // TODO(phase-5): replace stub with real Rayon-backed scanner.
    // For now, emit a single empty batch so the Swift callback contract is
    // exercised end-to-end during integration testing.
    let empty_batch = ScanNodeBatchC {
        nodes: ptr::null(),
        count: 0,
    };
    callback(empty_batch, ctx);

    handle_ptr
}

/// Requests cancellation of an in-progress scan.
///
/// This is advisory — the scan may emit one more callback batch after this
/// call returns.  Safe to call multiple times; safe to call on a completed
/// scan (no-op).
///
/// # Safety
/// `handle` must be a valid pointer returned by `jhara_scan_start` that has
/// not yet been freed via `jhara_scan_free`.
#[no_mangle]
pub unsafe extern "C" fn jhara_scan_cancel(handle: *mut JharaScanHandle) {
    if handle.is_null() {
        return;
    }
    (*handle)
        .cancelled
        .store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Frees all resources associated with a scan handle.
///
/// After this call `handle` is a dangling pointer; the caller must not use it.
/// Passing a null pointer is a safe no-op.
///
/// # Safety
/// `handle` must be either null or a valid pointer returned by
/// `jhara_scan_start` that has not previously been freed.
#[no_mangle]
pub unsafe extern "C" fn jhara_scan_free(handle: *mut JharaScanHandle) {
    if handle.is_null() {
        return;
    }
    // Re-box and immediately drop, freeing the allocation and its contents.
    drop(Box::from_raw(handle));
}

/// Returns the total physical size (bytes) of the subtree rooted at `path`.
///
/// ## Returns
///
/// - `>= 0` — cumulative `physical_size` of all nodes under `path`
/// - `-1`   — `handle` or `path` is null, path not found, or scan incomplete
///
/// # Safety
/// `handle` must be a valid, non-freed handle.
/// `path` must be a valid NUL-terminated C string or null.
#[no_mangle]
pub unsafe extern "C" fn jhara_tree_physical_size(
    handle: *mut JharaScanHandle,
    path:   *const c_char,
) -> i64 {
    if handle.is_null() || path.is_null() {
        return -1;
    }

    let query = match c_str_to_string(path) {
        Some(s) => s,
        None    => return -1,
    };

    let results = match (*handle).results.lock() {
        Ok(guard) => guard,
        Err(_)    => return -1,
    };

    let total: i64 = results
        .iter()
        .filter(|node| node.path.starts_with(&query))
        .filter_map(|node| {
            if node.physical_size >= 0 {
                Some(node.physical_size)
            } else {
                None
            }
        })
        .sum();

    total
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    extern "C" fn count_batches(batch: ScanNodeBatchC, ctx: *mut c_void) {
        let counter = unsafe { &*(ctx as *const AtomicUsize) };
        // Even empty batches increment the counter — we want to confirm the
        // callback was invoked at all.
        counter.fetch_add(1, Ordering::Relaxed);
        let _ = batch; // suppress unused warning
    }

    #[test]
    fn null_roots_returns_null_handle() {
        let handle = unsafe {
            jhara_scan_start(
                ptr::null(),
                0,
                ptr::null(),
                0,
                count_batches,
                ptr::null_mut(),
            )
        };
        assert!(handle.is_null());
    }

    #[test]
    fn valid_root_returns_non_null_handle_and_invokes_callback() {
        let root = CString::new("/tmp").unwrap();
        let root_ptr: *const c_char = root.as_ptr();
        let roots_arr: [*const c_char; 1] = [root_ptr];

        let counter = Arc::new(AtomicUsize::new(0));
        let ctx = Arc::as_ptr(&counter) as *mut c_void;

        let handle = unsafe {
            jhara_scan_start(
                roots_arr.as_ptr(),
                1,
                ptr::null(),
                0,
                count_batches,
                ctx,
            )
        };

        assert!(!handle.is_null());
        assert!(counter.load(Ordering::Relaxed) >= 1, "callback was not invoked");

        unsafe { jhara_scan_free(handle) };
    }

    #[test]
    fn cancel_on_null_handle_is_safe() {
        unsafe { jhara_scan_cancel(ptr::null_mut()) };
    }

    #[test]
    fn free_on_null_handle_is_safe() {
        unsafe { jhara_scan_free(ptr::null_mut()) };
    }

    #[test]
    fn tree_physical_size_null_guard() {
        assert_eq!(unsafe { jhara_tree_physical_size(ptr::null_mut(), ptr::null()) }, -1);
    }
}
