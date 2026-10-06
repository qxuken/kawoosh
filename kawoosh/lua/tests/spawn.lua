-- `kawoosh.spawn` with a list (docs/design/vcs.md Decision 5): the
-- program and its arguments, no shell between; `on_done` with stdout
-- whole, its trailing newline (or the lack of one) kept; `on_stderr`
-- apart from the lines. `printf` through `sh`: Windows has an `sh`
-- where Git is installed, and no `printf` of its own on the PATH.
local got
kawoosh.spawn({ "sh", "-c", "printf 'a\\nb'" }, {
  on_done = function(text, code) got = { text = text, code = code } end,
})
kawoosh.wait(function() return got ~= nil end, nil, "printf done")
kawoosh.test.eq(got.text, "a\nb", "whole, no newline added")
kawoosh.test.eq(got.code, 0)

got = nil
kawoosh.spawn({ "sh", "-c", "printf 'one\\ntwo\\n'" }, {
  on_done = function(text, code) got = { text = text, code = code } end,
})
kawoosh.wait(function() return got ~= nil end, nil, "printf done")
kawoosh.test.eq(got.text, "one\ntwo\n", "the trailing newline kept")

-- An argument with a space or a quote is one argument, as given: the
-- shell's `$@` is what the list gave it.
got = nil
kawoosh.spawn({ "sh", "-c", 'printf "%s|" "$@"', "sh", "it's a", "b" }, {
  on_done = function(text) got = text end,
})
kawoosh.wait(function() return got ~= nil end, nil, "printf done")
kawoosh.test.eq(got, "it's a|b|")

-- stderr apart, stdout in lines, the exit code.
local out, err, code = {}, {}, nil
kawoosh.spawn({ "sh", "-c", "echo out; echo err >&2; exit 3" }, {
  on_lines = function(ls) for _, l in ipairs(ls) do out[#out + 1] = l end end,
  on_stderr = function(ls) for _, l in ipairs(ls) do err[#err + 1] = l end end,
  on_exit = function(c) code = c end,
})
kawoosh.wait(function() return code ~= nil end, nil, "sh done")
kawoosh.test.eq(table.concat(out, "|"), "out")
kawoosh.test.eq(table.concat(err, "|"), "err")
kawoosh.test.eq(code, 3)

-- A program that is not there fails at once, the exit with no code.
local exited = false
kawoosh.spawn({ "kawoosh-no-such-program-here" }, {
  on_exit = function(c) exited = true; code = c end,
})
kawoosh.wait(function() return exited end, nil, "missing program")
kawoosh.test.eq(code, nil)
kawoosh.test.ok(kawoosh.message():find("spawn:", 1, true), kawoosh.message())

-- `env`: variables the process has over the inherited ones.
got = nil
kawoosh.spawn({ "sh", "-c", 'printf "%s|%s" "$KAWOOSH_SPAWN_A" "$KAWOOSH_SPAWN_B"' }, {
  env = { KAWOOSH_SPAWN_A = "one", KAWOOSH_SPAWN_B = 2 },
  on_done = function(text) got = text end,
})
kawoosh.wait(function() return got ~= nil end, nil, "sh done")
kawoosh.test.eq(got, "one|2", "the env reached the process")
