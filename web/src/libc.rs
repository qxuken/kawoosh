//! wasi-libc's own names for the allocator, which its members call
//! (`locale_map` does) and which were defined beside the allocator the
//! module leaves out (web/build.rs): forwarded to the one it has,
//! tree-sitter's, on Rust's allocator.

use std::ffi::c_void;

unsafe extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    fn calloc(count: usize, size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
}

#[unsafe(no_mangle)]
unsafe extern "C" fn __libc_malloc(size: usize) -> *mut c_void {
    unsafe { malloc(size) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn __libc_calloc(count: usize, size: usize) -> *mut c_void {
    unsafe { calloc(count, size) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn __libc_free(ptr: *mut c_void) {
    unsafe { free(ptr) }
}
