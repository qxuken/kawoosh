-- Task metadata becomes compile offers without executing tasks. A
-- directory/domain owns its cache; old asynchronous answers cannot win.
local mise = kawoosh.mise
local rows = mise.rows({
  { name = "build", description = "Build the app", source = "/work/mise.toml", dir = "/work/out", run = { "cargo build" } },
  { name = "test:one", source = "/work/mise.toml", usage = 'arg "<file>"' },
  { name = "hidden", hide = true },
  { name = "global", global = true },
}, "box:/work")
kawoosh.test.eq(#rows, 2)
kawoosh.test.eq(rows[1].cmd, "mise run build")
kawoosh.test.eq(rows[1].cwd, "box:/work")
kawoosh.test.eq(rows[1].file, "box:/work/mise.toml")
kawoosh.test.eq(rows[1].detail[1], "runs in /work/out")
kawoosh.test.eq(rows[2].needs, true)

local spawn, held, calls = kawoosh.spawn, {}, 0
kawoosh.spawn = function(argv, opts)
  kawoosh.test.eq(table.concat(argv, " "), "mise tasks ls --json")
  kawoosh.test.eq(opts.env.MISE_YES, "0")
  kawoosh.test.eq(opts.env.MISE_PARANOID, "1")
  calls = calls + 1
  held[#held + 1] = opts
  return 100000 + calls
end
local root = "box:/work"
kawoosh.test.eq(#mise.commands({ dir = root }), 0)
kawoosh.test.eq(#mise.commands({ dir = root }), 0)
kawoosh.test.eq(calls, 1, "one discovery per directory in flight")
held[1].on_done('[{"name":"build","source":"/work/mise.toml"}]', 0)
kawoosh.test.eq(mise.commands({ dir = root })[1].cmd, "mise run build")
kawoosh.test.eq(calls, 1, "cached discovery is immediate")
kawoosh._watched("mise:" .. root, { root .. "/mise.toml" })
kawoosh.test.eq(mise.commands({ dir = root })[1].cmd, "mise run build", "stale rows stay visible")
kawoosh.test.eq(calls, 2)
kawoosh._watched("mise:" .. root, { root .. "/mise.toml" })
held[2].on_done('[{"name":"old"}]', 0)
kawoosh.test.eq(mise.commands({ dir = root })[1].cmd, "mise run build", "superseded answer ignored")
kawoosh.test.eq(calls, 3)
held[3].on_done('[{"name":"fresh"}]', 0)
kawoosh.test.eq(mise.commands({ dir = root })[1].cmd, "mise run fresh")
kawoosh.test.eq(#mise.commands({ dir = "other:/work" }), 0, "another domain has its own rows")
kawoosh.test.eq(calls, 4)
kawoosh.spawn = spawn
kawoosh.fs.watch("mise:" .. root, nil)
