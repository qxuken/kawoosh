#!/usr/bin/env nu
# kawoosh in a browser (web/README.md): the `kawoosh-web` module for
# wasm32-unknown-unknown, bound for the page by wasm-bindgen, and the page
# beside it — index.html, the WASI shim, the fonts — in target/web/, which
# any static server serves:
#
#   nu web/build.nu --serve        # then open http://localhost:8788
#
# What it needs, besides rustup's wasm32-unknown-unknown target:
#   wasm-bindgen    the CLI at the version Cargo.lock pins
#                   (cargo install wasm-bindgen-cli --version <it>)
#   clang, llvm-ar  an LLVM with the WebAssembly backend (Homebrew's llvm)
#   wasi-libc       its sysroot (Homebrew's wasi-libc): the C library Lua
#                   and the grammars are compiled against and linked with
#
# A command that fails stops the script: nushell makes an external's
# non-zero exit an error.

# Build the module and the page into target/web.
def main [
  --release   # optimized (slow to build; the debug module is ~40 MB)
  --serve     # then serve target/web on http://localhost:8788
] {
  let root = $env.FILE_PWD | path dirname
  let llvm = $env.LLVM_BIN? | default "/opt/homebrew/opt/llvm/bin"
  let sysroot = $env.WASI_SYSROOT? | default "/opt/homebrew/opt/wasi-libc/share/wasi-sysroot"
  for need in [$"($llvm)/clang" $"($llvm)/llvm-ar" $"($sysroot)/lib/wasm32-wasip1/libc.a"] {
    if not ($need | path exists) {
      error make { msg: $"missing ($need) — see the head of web/build.nu" }
    }
  }
  let profile = if $release { "release" } else { "debug" }
  let out = $root | path join target web

  # Every C file of the module — Lua, tree-sitter, the grammars — is
  # compiled against wasi-libc's headers. tree-sitter's own libc subset is
  # turned off (`-U`), since wasi-libc is the module's C library and its
  # `snprintf` formats floats, which Lua needs and tree-sitter's does not.
  # tree-sitter-language's build script is overridden: the grammars that
  # compile its wasm sources get empty ones (web/wasm-src), and its
  # headers are kept (web/wasm-include) for the grammars that include
  # them.
  let env_c = {
    WASI_SYSROOT: $sysroot
    CC_wasm32_unknown_unknown: $"($llvm)/clang"
    AR_wasm32_unknown_unknown: $"($llvm)/llvm-ar"
    CFLAGS_wasm32_unknown_unknown: $"--target=wasm32-wasip1 --sysroot=($sysroot) -UTREE_SITTER_WASM_STDLIB"
  }
  let ts = "target.wasm32-unknown-unknown.tree-sitter-language"
  let args = [
    build -p kawoosh-web --target wasm32-unknown-unknown
    --config $'($ts).wasm-headers="($root)/web/wasm-include"'
    --config $'($ts).wasm-src="($root)/web/wasm-src"'
  ] | append (if $release { [--release] } else { [] })
  with-env $env_c { ^cargo ...$args }

  mkdir ($out | path join fonts)
  ^wasm-bindgen --target web --out-dir ($out | path join pkg) ($root | path join target wasm32-unknown-unknown $profile kawoosh_web.wasm)
  cp ...(glob ($root | path join web www *)) $out
  for face in [Regular Bold Italic BoldItalic] {
    cp ($root | path join assets fonts IosevkaNavcon $"IosevkaNavcon-($face).ttf") ($out | path join fonts)
  }
  print $"built ($out)"

  if $serve {
    ^python3 -m http.server 8788 --bind 127.0.0.1 --directory $out
  }
}
