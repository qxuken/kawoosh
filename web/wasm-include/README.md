tree-sitter-language 0.1.8's `wasm/include`, the headers its build script
hands the grammars for wasm32-unknown-unknown (MIT, `LICENSE`). The
browser build overrides that build script (`web/build.nu`) and so names
them from here; `ctype.h` declares the classic set beside `isblank`,
which tree-sitter-md's scanner calls and wasi-libc defines.
