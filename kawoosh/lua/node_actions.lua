-- Node actions (docs/design/node-actions.md): one key that does what the
-- syntax node under the caret means — a boolean flipped, an operator
-- mirrored, a list split onto its lines or joined back, a string's
-- quotes changed, a number's digits grouped. ts-node-action's idea on
-- `kawoosh.node`, and only on it: the registry, the dispatch and the
-- actions are this file.
--
-- An action is a name, the node types it takes (tokens too: `==` is a
-- type), the languages it is for, and `run(n, ctx)` — which answers the
-- node's new text, `{ text =, cursor = }` (the caret that many bytes
-- into it), or nil when this node is not one it changes. `run` only
-- reads; the dispatch makes the edits.
--
--   kawoosh.node.action("flip", {
--     types = { "true", "false" },
--     run = function(n) return n:text() == "true" and "false" or "true" end,
--   })
--
-- From each caret the dispatch starts at the leaf and climbs: the first
-- node, innermost first, that an action takes and answers is the one
-- changed. The climb stops at a node filling a `body` field, or at a
-- `block` (odin's procedures name none), so a caret in a closure never
-- splits the call around it. `node_actions.NAME = false` in the
-- settings turns one off.

local M = { list = {} }
kawoosh.node_actions = M

kawoosh.setting("node_actions", {
  type = "table",
  doc = "node actions by name, `false` to turn one off: `{ quotes = false }`",
})

-- kawoosh.node.action(name, def): an action added, or replaced by its
-- name (and then the newest), or removed with `def` nil — `def` is
-- `{ doc =, languages = "*" | { … }, types = { … } | fn(type), run =
-- fn(n, ctx) }`, `types` a function when what it takes can grow.
function kawoosh.node.action(name, def)
  for i, a in ipairs(M.list) do
    if a.name == name then
      table.remove(M.list, i)
      break
    end
  end
  if def == nil then return end
  assert(type(def.run) == "function", "node action " .. name .. ": run is a function")
  local types = def.types
  if type(types) ~= "function" then
    local set = {}
    for _, t in ipairs(types or {}) do set[t] = true end
    types = function(t) return set[t] == true end
  end
  M.list[#M.list + 1] = {
    name = name,
    doc = def.doc,
    languages = def.languages or "*",
    types = types,
    run = def.run,
  }
end

local function enabled(a)
  local off = kawoosh.opt("node_actions")
  return not (type(off) == "table" and off[a.name] == false)
end

local function for_language(a, language)
  if a.languages == "*" then return true end
  for _, l in ipairs(a.languages) do
    if l == language then return true end
  end
  return false
end

-- The context an action runs with: the leading whitespace of the node's
-- first line, one indent's text, the caret.
local function context(n, caret)
  local indent = kawoosh.buf.indent(n.buffer)
  return {
    buffer = n.buffer,
    language = n.language,
    caret = caret,
    indent = kawoosh.buf.line(n.line, n.buffer):match("^[ \t]*"),
    unit = indent and indent.unit or "    ",
  }
end

-- What would happen from `where` (an offset or a range): every action
-- that answers, on each node from the leaf up to a body, innermost
-- first and the newest action first — `{ action, node, text, cursor }`,
-- in `buffer` (the current one by default).
function M.answers(where, caret, buffer)
  local n, why = kawoosh.node.leaf(where, buffer)
  if not n then return nil, why end
  local out = {}
  while n do
    local ctx
    for i = #M.list, 1, -1 do
      local a = M.list[i]
      if a.types(n.type) and for_language(a, n.language) and enabled(a) then
        ctx = ctx or context(n, caret)
        local got = a.run(n, ctx)
        if type(got) == "string" then got = { text = got } end
        if type(got) == "table" and type(got.text) == "string" then
          out[#out + 1] = { action = a, node = n, text = got.text, cursor = got.cursor }
        end
      end
    end
    if n.field == "body" or n.type == "block" then break end
    n = n:parent()
  end
  return out
end

-- Runs, from every caret, the answer `pick` takes of its list (the
-- first, by default); one batch of edits, the carets after. `target`
-- is `{ buffer, sels, primary_only }` — the picker's, which acts on the
-- buffer and the caret it was opened from once its own field has the
-- keys — else the current buffer's selections, in its mode.
function M.run(pick, target)
  pick = pick or function(list) return list[1] end
  target = target or {}
  local h = target.buffer
  local visual = not target.buffer and kawoosh.mode() == "visual"
  local sels = target.sels or kawoosh.buf.selections()
  local chosen, why = {}, nil
  for i, s in ipairs(sels) do
    local where = s.head
    if visual then
      local a, b = math.min(s.anchor, s.head), math.max(s.anchor, s.head)
      where = { a, b + 1 }
    end
    local list, err
    if not target.primary_only or s.primary then
      list, err = M.answers(where, s.head, h)
    end
    why = why or err
    local c = list and pick(list)
    if c then chosen[#chosen + 1] = { caret = s.head, index = i, answer = c } end
  end
  if #chosen == 0 then
    return kawoosh.echo(why or "no node action here")
  end
  -- One node from two carets acts once; a node inside another's is
  -- skipped, its edit being inside text the outer one replaces.
  table.sort(chosen, function(x, y)
    local a, b = x.answer.node, y.answer.node
    if a.from ~= b.from then return a.from < b.from end
    return a.to > b.to
  end)
  local edits, kept, skipped, reach = {}, {}, 0, -1
  for _, c in ipairs(chosen) do
    local n = c.answer.node
    local last = kept[#kept]
    if last and last.answer.node == n then
      c.edit = last.edit
    elseif n.from < reach then
      skipped = skipped + 1
    else
      edits[#edits + 1] = { n.from, n.to, c.answer.text }
      c.edit = #edits
      kept[#kept + 1] = c
      reach = n.to
    end
  end
  -- The carets: an acting one at its distance into the node, or where
  -- the answer said; any other where the edits move it (kept its
  -- distance into a node rewritten under it).
  local placed = {}
  for _, c in ipairs(chosen) do
    if c.edit then
      local a = c.answer
      placed[c.index] = { edit = c.edit,
                          at = a.cursor or math.min(c.caret - a.node.from, math.max(#a.text - 1, 0)) }
    end
  end
  local carets = {}
  for i, s in ipairs(sels) do
    local c = placed[i] or { at = s.head }
    c.primary = s.primary
    carets[#carets + 1] = c
  end
  if visual then kawoosh.cmd("normal") end
  kawoosh.buf.edits(edits, h, { carets = carets })
  local first = kept[1].answer
  local msg = first.action.name .. " · " .. first.node.type
  if #edits > 1 then msg = msg .. " ×" .. #edits end
  if skipped > 0 then msg = msg .. " · " .. skipped .. " inside another skipped" end
  kawoosh.echo(msg)
end

local WHEN = { "editor", "!readonly" }

-- The pick of the Nth answer of the action named `name`.
local function nth_of(name, nth)
  return function(list)
    local seen = 0
    for _, c in ipairs(list) do
      if c.action.name == name then
        seen = seen + 1
        if seen == (nth or 1) then return c end
      end
    end
  end
end

kawoosh.command("node action", function(ctx)
  local name, nth = ctx.args[1], tonumber(ctx.args[2] or "")
  if not name then return M.run() end
  M.run(nth_of(name, nth))
end, {
  when = WHEN,
  args = { "text", "text" },
  doc = "the node action at every caret: the first there is (g.), or NAME's — on the Nth node up it takes",
})

kawoosh.command("node actions", function()
  local h, sels = kawoosh.buf.current(), kawoosh.buf.selections()
  local list, why = M.answers(nil, kawoosh.buf.cursor().offset)
  if not list or #list == 0 then return kawoosh.echo(why or "no node action here") end
  local target = { buffer = h, sels = sels, primary_only = true }
  local items, count = {}, {}
  for _, c in ipairs(list) do
    local name = c.action.name
    count[name] = (count[name] or 0) + 1
    local oneline = c.text:gsub("%s*\n%s*", " ")
    items[#items + 1] = {
      text = name .. " · " .. c.node.type,
      sub = #oneline > 60 and oneline:sub(1, 60) .. "…" or oneline,
      pick = (function(pick)
        return function() M.run(pick, target) end
      end)(nth_of(name, count[name])),
    }
  end
  kawoosh.picker.open({ title = "node actions", items = items })
end, { when = WHEN, doc = "every node action from the primary caret up, in a picker" })

kawoosh.map("n", "g.", "node action")
kawoosh.map("v", "g.", "node action")

-- ---- the actions

local FLIP = {
  ["true"] = "false", ["false"] = "true",
  True = "False", False = "True",
  TRUE = "FALSE", FALSE = "TRUE",
}

kawoosh.node.action("flip", {
  doc = "true ↔ false",
  types = { "true", "false", "boolean_scalar", "boolean" },
  run = function(n) return FLIP[n:text()] end,
})

-- Mirrored, as ts-node-action has them: a comparison turned round, an
-- equality negated, `and` for `or`. Lua spells not-equal `~=`.
M.operators = {
  ["=="] = "!=", ["!="] = "==", ["==="] = "!==", ["!=="] = "===",
  ["&&"] = "||", ["||"] = "&&", ["and"] = "or", ["or"] = "and",
  ["<"] = ">", [">"] = "<", ["<="] = ">=", [">="] = "<=",
}
M.operators_by_language = { lua = { ["=="] = "~=", ["~="] = "==" } }

do
  local types = { "~=" }
  for op in pairs(M.operators) do types[#types + 1] = op end
  kawoosh.node.action("operator", {
    doc = "== ↔ !=, && ↔ ||, < ↔ >, …",
    types = types,
    run = function(n, ctx)
      -- A token filling an operator's field: rust's `Vec<T>` is none.
      if n.field ~= "operator" and n.field ~= "operators" then return nil end
      local own = M.operators_by_language[ctx.language] or {}
      local op = n:text()
      return own[op] or M.operators[op]
    end,
  })
end

-- The lists `split` takes, per language: whether one split keeps a
-- trailing comma, whether one joined has spaces inside its brackets,
-- and whether a list of one keeps its comma — as each language's
-- formatter writes it.
local T = { trailing = true }
local TP = { trailing = true, pad = true }
local N = { trailing = false }
local JS = {
  arguments = T, formal_parameters = T, array = T, object = TP,
  named_imports = TP, object_pattern = TP, array_pattern = T,
}
M.lists = {
  rust = {
    arguments = T, parameters = T, array_expression = T, use_list = T,
    tuple_expression = { trailing = true, single = true },
    field_initializer_list = TP,
  },
  javascript = JS, typescript = JS, tsx = JS,
  python = {
    argument_list = T, parameters = T, list = T, dictionary = T, set = T,
    tuple = { trailing = true, single = true },
  },
  go = { argument_list = T, parameter_list = T, literal_value = T },
  lua = { arguments = N, parameters = N, table_constructor = TP },
  json = { array = N, object = { trailing = false, pad = true } },
  jsonc = { array = N, object = { trailing = false, pad = true } },
  c = { argument_list = N, parameter_list = N, initializer_list = T },
  cpp = { argument_list = N, parameter_list = N, initializer_list = T },
  toml = { array = T },
  -- A call, a literal and a declaration hold their brackets beside what
  -- comes before them: `f(…)`, `Point{…}`, `[3]int{…}`, `struct {…}`.
  odin = {
    parameters = T, call_expression = T, struct = T, map = T,
    struct_declaration = T, enum_declaration = T, union_declaration = T,
    overloaded_procedure_declaration = T,
  },
}

-- The opener of each closer.
local PAIR = { [")"] = "(", ["]"] = "[", ["}"] = "{" }

do
  kawoosh.node.action("split", {
    doc = "a list one item a line, or joined on one",
    -- Read from the tables as they are, so a list a user adds counts.
    types = function(t)
      for _, lists in pairs(M.lists) do
        if lists[t] then return true end
      end
      return false
    end,
    run = function(n, ctx)
      local spec = (M.lists[ctx.language] or {})[n.type]
      if not spec then return nil end
      local kids = n:children({ anonymous = true })
      -- The list is the node's last child, a closer, back to its first
      -- opener: the node's own first child, or after what heads it (a
      -- callee, a type, `struct`), which stays as it is.
      local close = kids[#kids]
      if #kids < 2 or close.named or not PAIR[close:text()] then return nil end
      local first
      for i = 1, #kids - 1 do
        if not kids[i].named and kids[i]:text() == PAIR[close:text()] then
          first = i
          break
        end
      end
      if not first then return nil end
      local open = kids[first]
      local head = kawoosh.buf.slice(n.from, open.from, n.buffer)
      -- The items: the text between the commas, whatever nodes it is
      -- (an attribute and its field, a `*`).
      local items, from, to, comment = {}, nil, nil, false
      local function done()
        if from then items[#items + 1] = kawoosh.buf.slice(from, to, n.buffer) end
        from, to = nil, nil
      end
      for i = first + 1, #kids - 1 do
        local k = kids[i]
        if not k.named and k.type == "," then
          done()
        else
          if k.type:find("comment") then comment = true end
          from = from or k.from
          to = k.to
        end
      end
      done()
      if #items == 0 then return nil end
      local o, c = open:text(), close:text()
      -- From the opener: what heads it may sit on lines above (odin's
      -- `@(private)` over a declaration).
      if open.line == n.end_line then
        local inner = ctx.indent .. ctx.unit
        return {
          text = head .. o .. "\n" .. inner .. table.concat(items, ",\n" .. inner)
            .. (spec.trailing and "," or "") .. "\n" .. ctx.indent .. c,
          cursor = #head,
        }
      end
      -- A line comment would swallow what followed it on one line.
      if comment then return nil end
      local pad = spec.pad and " " or ""
      local one = (#items == 1 and spec.single) and "," or ""
      return {
        text = head .. o .. pad .. table.concat(items, ", ") .. one .. pad .. c,
        cursor = #head,
      }
    end,
  })
end

local QUOTES = {
  javascript = { '"', "'", "`" }, typescript = { '"', "'", "`" }, tsx = { '"', "'", "`" },
  python = { '"', "'" }, lua = { '"', "'" },
  -- `'c'` is a rune; a backquote quotes a raw string.
  odin = { '"', "`" },
}

kawoosh.node.action("quotes", {
  doc = "\"…\" → '…' → `…`",
  languages = { "javascript", "typescript", "tsx", "python", "lua", "odin" },
  types = { "string", "template_string" },
  run = function(n, ctx)
    local cycle = QUOTES[ctx.language]
    local text = n:text()
    local prefix, q = text:match("^(%a*)([\"'`])")
    if not q or text:sub(-1) ~= q or #text < #prefix + 2 then return nil end
    -- Python's triple quotes, and anything this cannot say the same way.
    if text:sub(#prefix + 1, #prefix + 3) == q:rep(3) then return nil end
    local body = text:sub(#prefix + 2, -2)
    local at
    for i, c in ipairs(cycle) do
      if c == q then at = i end
    end
    if not at then return nil end
    local to = cycle[at % #cycle + 1]
    if body:find(to, 1, true) or body:find("\\", 1, true) or body:find("\n", 1, true)
        or (q == "`" and body:find("${", 1, true)) then
      return nil
    end
    return prefix .. to .. body .. to
  end,
})

kawoosh.node.action("digits", {
  doc = "1000000 ↔ 1_000_000",
  languages = { "rust", "python", "javascript", "typescript", "tsx", "go", "odin" },
  types = { "integer_literal", "float_literal", "integer", "float", "number", "int_literal" },
  run = function(n)
    local text = n:text()
    if text:match("^0[xXoObB]") then return nil end
    local num, rest = text:match("^([%d_]+)(.*)$")
    if not num then return nil end
    if num:find("_", 1, true) then return num:gsub("_", "") .. rest end
    if #num < 5 then return nil end
    local grouped = num:reverse():gsub("(%d%d%d)", "%1_"):reverse():gsub("^_", "")
    return grouped .. rest
  end,
})
