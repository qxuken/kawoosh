//! The C library the browser module links (web/README.md).
//!
//! Lua and the grammars are compiled against wasi-libc's headers, and the
//! module links wasi-libc for what they call — less two members: its
//! allocator (`dlmalloc`) and `abort`. tree-sitter's Rust side defines
//! `malloc`, `calloc`, `realloc`, `free` and `abort` for
//! wasm32-unknown-unknown, on Rust's allocator, and a symbol cannot be
//! defined twice; so every C allocation in the module, Lua's included,
//! is Rust's, on one heap. The archive is wasi-libc's `libc.a` with those
//! two members taken out, made here; `libsetjmp` (Lua's errors, through
//! wasm exceptions) and the signal emulation Lua's `os` library names
//! come as they are.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=WASI_SYSROOT");
    if std::env::var("TARGET").as_deref() != Ok("wasm32-unknown-unknown") {
        return;
    }
    let sysroot = std::env::var("WASI_SYSROOT")
        .expect("WASI_SYSROOT names a wasi-libc sysroot (web/build.nu sets it)");
    let lib = PathBuf::from(&sysroot).join("lib/wasm32-wasip1");
    let ar = std::env::var("AR_wasm32_unknown_unknown").unwrap_or_else(|_| "llvm-ar".into());
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let libc = out.join("libc.a");
    std::fs::copy(lib.join("libc.a"), &libc).expect("copying wasi-libc's libc.a");
    let status = Command::new(&ar)
        .arg("d")
        .arg(&libc)
        .args(["dlmalloc.c.obj", "abort.c.obj"])
        .status()
        .unwrap_or_else(|e| panic!("running {ar}: {e}"));
    assert!(status.success(), "{ar} d failed on {}", libc.display());
    println!("cargo::rustc-link-search=native={}", out.display());
    println!("cargo::rustc-link-search=native={}", lib.display());
    println!("cargo::rustc-link-lib=static=c");
    println!("cargo::rustc-link-lib=static=setjmp");
    println!("cargo::rustc-link-lib=static=wasi-emulated-signal");
}
