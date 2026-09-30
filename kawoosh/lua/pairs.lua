-- Auto-closing brackets (docs/design/pairs.md): on by default
-- (`pairs.enabled = false` turns them off): `(` types `()` with the
-- caret between, `)` before a `)` steps over it, `<BS>` between a pair
-- deletes both, `<CR>` between `{` and `}` opens the block, and a
-- quote pairs only where a quote can open. Contested in modal editors
-- and cheap to get wrong with several carets, so a bundled plugin in
-- Lua a user can read and rewrite, not the engine's; `gsa(` wraps a
-- selection after the fact.
--
-- The rules are per language, a table merged over the defaults —
--
--   pairs = {
--     enabled = true,
--     rules = {
--       default = { { "(", ")" }, { "<", ">" } },  -- added to the six
--       rust = { ["'"] = false },                  -- lifetimes
--       lisp = { ["'"] = false, ["`"] = false },
--     },
--   }
--
-- — a language's list adds pairs and `[char] = false` removes one; the
-- defaults are `()` `[]` `{}` `""` `''` and backticks, and rust leaves
-- `'` out. Every caret is read on its own, and what each does is one
-- edit of one `kawoosh.buf.edits`, the carets put back after with
-- `set_selections`. `.` replays a pairing as it was typed, and an undo
-- takes it with the rest of the insert.
--
-- The keys are insert mode's, gated on the `pairs` fact (set while
-- `pairs.enabled` is) and off the prompt and a view's field — where
-- they are gated off, a key types as it would.

local M = {}
kawoosh.pairs = M

local DEFAULT = { { "(", ")" }, { "[", "]" }, { "{", "}" }, { '"', '"' }, { "'", "'" }, { "`", "`" } }
-- What a language changes by default: rust's `'` is a lifetime as often
-- as a char.
M.languages = { rust = { ["'"] = false } }

local WHEN = { "pairs", "!prompt", "!field" }

kawoosh.setting("pairs.rules", { type = "table", doc = "pairs by language, `default` for all: `{ [\"<\"] = \">\" }`, `false` to drop one" })

-- The pairs for `language`: the defaults, the settings' `default`, then
-- the language's — built-in, then the settings'.
function M.rules(language)
  local set, order = {}, {}
local function apply(t)
    if type(t) ~= "table" then return end
    for _, p in ipairs(t) do
      if type(p) == "table" and type(p[1]) == "string" and type(p[2]) == "string" then
        if not set[p[1]] then order[#order + 1] = p[1] end
        set[p[1]] = p[2]
      end
    end
    for k, v in pairs(t) do
      if type(k) == "string" and v == false then set[k] = nil end
    end
  end
  apply(DEFAULT)
  local mine = kawoosh.opt("pairs.rules")
  mine = type(mine) == "table" and mine or {}
  apply(mine.default)
  apply(M.languages[language])
  apply(mine[language])
  local out = {}
  for _, o in ipairs(order) do
    if set[o] then out[#out + 1] = { o, set[o] } end
  end
  return out
end

local function enabled() return kawoosh.opt("pairs.enabled") == true end

-- A word character: a letter, a digit, `_`, or any byte of a non-ASCII
-- character (a word in most scripts).
local function wordy(ch)
  return ch ~= "" and (ch:match("^[%w_]") ~= nil or ch:byte(1) >= 0x80)
end

-- The character before and after `at`.
local function around(at)
  local before = at > 0 and kawoosh.buf.slice(at - 1, at) or ""
  local after = kawoosh.buf.slice(at, at + 1)
  return before, after
end

-- The carets, in insert mode points at their heads, in text order,
-- and which of them is the primary.
local function carets()
  local all = {}
  for _, s in ipairs(kawoosh.buf.selections()) do
    all[#all + 1] = { at = s.head, primary = s.primary }
  end
  table.sort(all, function(a, b) return a.at < b.at end)
  local heads, primary = {}, 1
  for i, c in ipairs(all) do
    heads[i] = c.at
    if c.primary then primary = i end
  end
  heads.primary = primary
  return heads
end

-- One action per caret, applied: `{ text =, del_before =, del_after =,
-- caret = }` — bytes deleted either side, text put in their place,
-- the caret that many bytes into it. Done as one step whatever they
-- are, the carets placed after, the primary kept. A caret's deletion
-- reaching into the one before it (`(|)|` and `<BS>`: the pair and
-- the `)` after it) starts where that one ends: edits are disjoint.
local function apply(heads, actions)
  local edits, sels, shift, reached = {}, {}, 0, 0
  for i, at in ipairs(heads) do
    local a = actions[i]
    local text = a.text or ""
    local from, to = at - (a.del_before or 0), at + (a.del_after or 0)
    from = math.max(from, reached)
    to = math.max(to, from)
    reached = to
    -- A step over edits nothing: the caret moves on.
    if to > from or text ~= "" then edits[#edits + 1] = { from, to, text } end
    local caret = from + shift + (a.caret or #text)
    sels[#sels + 1] = { caret, caret, primary = i == heads.primary }
    shift = shift + #text - (to - from)
  end
  if #edits > 0 then kawoosh.buf.edits(edits) end
  kawoosh.buf.set_selections(sels)
end

-- A typed character `ch` at each caret: pairs, steps over, or itself.
local function key(ch)
  local rules = M.rules(kawoosh.buf.language())
  local closer_of, opener_of = {}, {}
  for _, p in ipairs(rules) do
    closer_of[p[1]] = p[2]
    opener_of[p[2]] = p[1]
  end
  local heads = carets()
  local actions = {}
  for i, at in ipairs(heads) do
    local before, after = around(at)
    local close = closer_of[ch]
    local a
    if close and close == ch then
      -- A quote: over the same quote, else a pair where one can open.
      if after == ch then
        a = { text = "", caret = 1 }
      elseif not wordy(before) and not wordy(after) and before ~= "\\" and before ~= ch then
        a = { text = ch .. ch, caret = 1 }
      else
        a = { text = ch }
      end
    elseif close then
      -- An opener: a pair unless a word follows.
      a = wordy(after) and { text = ch } or { text = ch .. close, caret = 1 }
    elseif opener_of[ch] and after == ch then
      -- A closer before the same closer: stepped over.
      a = { text = "", caret = 1 }
    else
      a = { text = ch }
    end
    actions[i] = a
  end
  apply(heads, actions)
end

-- `<BS>`: between an opener and its closer, both; else the engine's.
local function backspace()
  local rules = M.rules(kawoosh.buf.language())
  local closer_of = {}
  for _, p in ipairs(rules) do closer_of[p[1]] = p[2] end
  local heads = carets()
  local any = false
  local actions = {}
  for i, at in ipairs(heads) do
    local before, after = around(at)
    if before ~= "" and closer_of[before] and closer_of[before] == after then
      actions[i] = { del_before = 1, del_after = 1, text = "" }
      any = true
    else
      actions[i] = false
    end
  end
  if not any then return kawoosh.cmd("delete char back") end
  for i, a in ipairs(actions) do
    if not a then
      -- A caret past a character of its own: that character goes.
      local at = heads[i]
      local before = around(at)
      actions[i] = { del_before = #before, text = "" }
    end
  end
  apply(heads, actions)
end

-- The brackets whose block the engine's `insert newline` opens itself.
local ENGINE = { ["("] = true, ["["] = true, ["{"] = true }

-- `<CR>` between a pair's opener and its closer, at every caret: the
-- block opened — the closer on a line of its own, the caret on an
-- indented line between. The engine's newline does that for `()` `[]`
-- `{}`; this for a pair a rule adds (`<>`). Else the engine's newline.
local function enter()
  local rules = M.rules(kawoosh.buf.language())
  local closer_of = {}
  for _, p in ipairs(rules) do
    if p[1] ~= p[2] and not ENGINE[p[1]] then closer_of[p[1]] = p[2] end
  end
  local heads = carets()
  local all = #heads > 0
  for _, at in ipairs(heads) do
    local before, after = around(at)
    if not (closer_of[before] and closer_of[before] == after) then all = false end
  end
  kawoosh.cmd("insert newline")
  if all then
    kawoosh.cmd("insert newline")
    kawoosh.cmd("move up")
    kawoosh.cmd("line end insert")
    kawoosh.cmd("insert tab")
  end
end

kawoosh.command("pairs key", function(ctx)
  local ch = string.char(tonumber(ctx.args[1]) or 0)
  key(ch)
end, { args = { "text" }, when = WHEN, doc = "a pair's character typed: paired, stepped over, or itself (pairs.lua)" })
kawoosh.command("pairs backspace", backspace,
  { when = WHEN, doc = "delete back, both of an empty pair (pairs.lua)" })
kawoosh.command("pairs enter", enter,
  { when = WHEN, doc = "a newline, the block opened between a pair (pairs.lua)" })

-- The keys: every opener and closer any rule names, bound once each,
-- by the byte (the binding's argument is a word, and `"` would not be).
local bound = {}
local function bind(ch)
  if bound[ch] or #ch ~= 1 then return end
  bound[ch] = true
  kawoosh.map("i", ch, "pairs key " .. ch:byte(), { when = WHEN })
end

local function sync()
  kawoosh.fact("pairs", enabled())
  local langs = { "" }
  for l in pairs(M.languages) do langs[#langs + 1] = l end
  local mine = kawoosh.opt("pairs.rules")
  if type(mine) == "table" then
    for l in pairs(mine) do if type(l) == "string" then langs[#langs + 1] = l end end
  end
  for _, l in ipairs(langs) do
    for _, p in ipairs(M.rules(l)) do
      bind(p[1])
      bind(p[2])
    end
  end
end

kawoosh.map("i", "<BS>", "pairs backspace", { when = WHEN })
kawoosh.map("i", "<CR>", "pairs enter", { when = WHEN })
kawoosh.on_settings(sync)
