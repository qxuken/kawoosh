-- kawoosh's own project settings. `:compile` (`<leader>cc`) builds and
-- `]q` walks what failed; `run` and the tools are in `<leader>tt`.
-- The sweep a round ends on is `scripts/verify.nu`: `:tool verify` in
-- a terminal, or `:compile nu scripts/verify.nu` for its locations.
---@type kawoosh.Settings
return {
  compile = { command = "cargo build --workspace --all-targets" },
  run = { command = "cargo run -p kawoosh" },
  tools = {
    verify = { cmd = "nu scripts/verify.nu", cwd = "root" },
    test = { cmd = "cargo test --workspace", cwd = "root" },
    clippy = { cmd = "cargo clippy --workspace --all-targets -- -D warnings", cwd = "root" },
  },
}
