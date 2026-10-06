-- kawoosh's own project settings. `:compile` (`<leader>cc`) builds and
-- `]q` walks what failed; the tools — `run` among them — are in `<leader>t`.
-- The sweep a round ends on is `scripts/verify.nu`: `:tool verify` in
-- a terminal, or `:compile nu scripts/verify.nu` for its locations.
-- `:compile install` installs the app: `init.lua`'s, once trusted.
---@type kawoosh.Settings
return {
  compile = { default = "cargo build --workspace --all-targets" },
  tools = {
    run = { cmd = "cargo run -p kawoosh", cwd = "root" },
    verify = { cmd = "nu scripts/verify.nu", cwd = "root" },
    test = { cmd = "cargo test --workspace", cwd = "root" },
    clippy = { cmd = "cargo clippy --workspace --all-targets -- -D warnings", cwd = "root" },
  },
}
