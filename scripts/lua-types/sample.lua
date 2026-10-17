-- A plugin's worth of calls on the typed API, for `scripts/lua-types.nu`:
-- lua-language-server must find nothing here but the `-- expect:` lines.

kawoosh.map("n", "<leader>x", "write")
kawoosh.map(1, "<leader>x", "write") -- expect: param-type-mismatch
