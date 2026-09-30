#!/usr/bin/env nu
# The sweep a round ends on: the format, clippy with warnings as errors,
# and the workspace's tests — the Lua acceptance scripts among them,
# since `kawoosh/tests/lua_harness.rs` runs `kawoosh/lua/tests/*.lua`
# from `cargo test`. `.kawoosh/settings.lua` names it: `:tool verify`
# in a terminal, `:compile nu scripts/verify.nu` into `*compile*` for
# `]q` over what failed.
#
# The tests run on cargo-nextest when it is installed — every test of
# every binary on one pool (`.config/nextest.toml`) — else on `cargo
# test`, a binary at a time. Either way in a temp directory of the
# run's own, removed after: what the tests leave in `$TMPDIR` had grown
# to tens of thousands of entries, and a test typing a path through it
# listed them all.
#
# A command that fails stops the script: nushell makes an external's
# non-zero exit an error.

# Check the workspace the way a round is checked before it merges.
def main [
  --fix  # format the tree instead of failing on it
] {
  cd ($env.FILE_PWD | path dirname)
  print "== fmt"
  if $fix { cargo fmt --all } else { cargo fmt --all --check }
  print "== clippy"
  cargo clippy --workspace --all-targets -- -D warnings
  # Every test binary, so one run names every failure.
  print "== test"
  let tmp = (mktemp -d)
  let passed = try {
    with-env { TMPDIR: $tmp, TMP: $tmp, TEMP: $tmp } {
      if (which cargo-nextest | is-not-empty) {
        cargo nextest run --workspace --no-fail-fast
      } else {
        cargo test --workspace --no-fail-fast
      }
    }
    true
  } catch { false }
  rm -rf $tmp
  if not $passed { error make { msg: "tests failed" } }
  print "== verified"
}
