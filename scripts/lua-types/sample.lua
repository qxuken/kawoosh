-- A plugin's worth of calls on the typed API, for `scripts/lua-types.nu`:
-- lua-language-server must find nothing here but the `-- expect:` lines.

kawoosh.map("n", "<leader>x", "write")
kawoosh.map(1, "<leader>x", "write") -- expect: param-type-mismatch

kawoosh.map("n", "<leader>y", function(ctx)
  local n = ctx.count + 1
  kawoosh.notify(tostring(n))
end, { language = "dir", when = { "!terminal" } })
kawoosh.map("x", "<leader>x", "write") -- expect: param-type-mismatch
kawoosh.unmap("n", "<leader>y", { language = "dir" })
kawoosh.buf.close(kawoosh.buf.current(), { force = true })
kawoosh.buf.close(1, { force = 1 }) -- expect: assign-type-mismatch
