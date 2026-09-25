# kawoosh in a browser

This is kawoosh — the editor, its Lua, its tree-sitter grammars — built
for `wasm32-unknown-unknown` and drawn by kui on a `<canvas>` with
WebGPU. Nothing here is a web rewrite: it is the same code as the
desktop app, with what a page cannot have left out where it would start.

## Try

- `<Space>f` — the picker, on the files below; type to narrow, `<CR>` opens
- `:e docs/design/kui.md` — a markdown buffer, drawn as markdown
- `<C-w>v`, `<C-w>s` — splits; `<C-w>h j k l` moves between them
- `:lua kawoosh.notify("hello from " .. _VERSION)` — Lua 5.5, in the page
- `:w` — writes to the page's disk (it lasts until the page is closed)
- `:syntax_tree` — the tree-sitter tree of the buffer

The files are a few of kawoosh's own, at their places in the repo:
`editor/src/motions.rs`, `doc/src/paths.rs`, `kawoosh/lua/picker.lua`,
`docs/design/roadmap.md`, and the source of this page's entry point,
`web/src/lib.rs`.

## What is left out

A page starts no processes, opens no sockets and has no disk of its own:

- **terminals** — `:term` says so; the emulator is built in, the pty is not
- **language servers** — none start; the commands go nowhere
- **the command socket** — `$EDITOR` and the `kawoosh` CLI shim talk to it
- **the state database** — history and working memory are the session's
- **grammar libraries** — the built-in grammars are what a page has

## Build

    nu web/build.nu --serve

then open <http://localhost:8788>. `--release` for an optimized module:
33 MB, 6.7 MB gzipped — most of it the grammars' parse tables, as on
the desktop — beside four faces of Iosevka at 7 MB each.
It needs rustup's `wasm32-unknown-unknown` target, the `wasm-bindgen` CLI
at the version `Cargo.lock` pins, an LLVM with the WebAssembly backend,
and wasi-libc's sysroot — `brew install llvm wasi-libc` on macOS. The
script's head says the rest.

## How it fits

**The window** is kui's (its F87): winit's web backend is the loop,
`Launcher::run` hands the shell to the page and returns, and the
renderer is made in a task, since a page cannot block on the adapter.

**The C** — Lua, tree-sitter and two dozen grammars — is compiled
against wasi-libc's headers and linked with wasi-libc (`build.rs`), less
its allocator: tree-sitter's Rust side defines `malloc` and `free` on
Rust's allocator, so every C allocation is on one heap. Lua's errors
are `longjmp`, which clang lowers to wasm exceptions. The few WASI
calls libc makes — the clock, stdout — are answered by `www/wasi.js`.
Three crates are patched for the target (`lua-src/`,
`alacritty_terminal/`, and tree-sitter-language's build script,
overridden from `build.nu`); each says why at its head.

**The systems** are threads on the desktop. A page has one thread, so a
system's loop is a task the page runs once the event that sent it work
has returned (`kawoosh_systems::Service`) — the same loop body, the same
channels. The alarm is the page's timer; the file watcher polls on it.

**The disk** is `src/memfs.rs`, set as the process's local disk
(`kawoosh_doc::fs::set_local_disk`): every local path goes to it, as a
path on a remote domain goes to that domain.
