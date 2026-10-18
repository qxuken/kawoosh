-- The user's snippets from files (docs/design/completion.md Decision
-- 8): `kawoosh.snippets.load_json` and the `snippets.paths` setting,
-- over the JSON door (`kawoosh.fs.read_json`) and the store
-- (`kawoosh.snippets.add` / `_set`, kawoosh/src/user_snippets.rs).
--
-- A file is VS Code's snippet JSON — `rust.json` holds rust's, a
-- `*.code-snippets` file every language's (each snippet's `scope` may
-- name some) — or a package laid out as friendly-snippets is: a
-- `package.json` whose `contributes.snippets` lists `{ language =,
-- path = }`, `language` a name or a list. Every read is off the frame;
-- what a file put is replaced as a whole when it is read again.
--
-- `snippets.paths` names folders, packages and files; a language's are
-- read the first time a buffer of it asks for a completion (the editor
-- calls `kawoosh._snippets_want`), and every path and file read is
-- watched: an edit reads it again. What goes wrong goes to the log.

local S = kawoosh.snippets
local fs = kawoosh.fs

-- kawoosh.snippets.add(lang, snippets): VS Code's snippet JSON as
-- Lua — `{ ["name"] = { prefix = "for" | { … }, body = "…" | { lines },
-- description = …, scope = "rust,go" } }` — offered by prefix in a
-- buffer of `lang` ("all" for every language), a body the LSP grammar
-- (`$1`, `${1:x}`, `${1|a,b|}`, `$0`, `$TM_FILENAME`). A name added
-- again replaces it. Answers how many were taken and, when some were
-- not, why each (a list).
---@param lang string a language's name, "all" for every one
---@param snippets table<string, kawoosh.snippets.Snippet> by name
---@return integer count the snippets taken
---@return string[]? why each one passed over was
function S.add(lang, snippets) return S._add(lang, snippets) end

---A snippet as VS Code's JSON spells it.
---@class kawoosh.snippets.Snippet
---@field prefix string|string[] what is typed to be offered it
---@field body string|string[] the text, the LSP snippet grammar: `$1`, `${1:x}`, `${1|a,b|}`, `$0`, `$TM_FILENAME`; a list its lines
---@field description? string|string[] a list its lines
---@field scope? string the languages it is for, `"rust,go"`: in a file of every language's

-- kawoosh.snippets.names(lang): the names of the snippets `lang` has,
-- added or read (as their language was written, lowercased).
---@param lang string
---@return string[]
function S.names(lang) return S._names(lang) end

local function say(text)
  kawoosh.notify(text, { level = "warn", source = "snippets", show = "log" })
end

-- Said once a session: a package's entry refused each time it is read.
local said = {}
local function say_once(text)
  if said[text] then return end
  said[text] = true
  say(text)
end

-- The loader's reads in flight, at most `MAX_READS`, the rest waiting
-- their turn: a package naming ten thousand files reads four at a time,
-- not ten thousand threads at once.
local MAX_READS = 4
local in_flight, waiting = 0, {}
local function read_json(path, cb)
  if in_flight >= MAX_READS then
    waiting[#waiting + 1] = { path, cb }
    return
  end
  in_flight = in_flight + 1
  fs.read_json(path, function(data, err)
    in_flight = in_flight - 1
    local next_read = table.remove(waiting, 1)
    if next_read then read_json(next_read[1], next_read[2]) end
    cb(data, err)
  end)
end

-- A snippet file's language by its name: `rust.json` is rust's, a
-- `*.code-snippets` file `all`'s; nil for any other file.
local function lang_of(path)
  local base = fs.basename(path)
  if base:match("%.code%-snippets$") then return "all" end
  local l = base:match("^(.+)%.jsonc?$")
  return l and l:lower()
end

-- What a file put is known by its path and the language it was read
-- for: a package may name one file for two.
local function origin_of(file, lang) return file .. "\n" .. lang end

-- The read under way for each origin, by number: one that lands after
-- a newer one began puts nothing.
local reading = {}

-- `file` read as `lang`'s snippets, in place of what it put before;
-- `done(count, why)`. A file gone takes its snippets with it; one that
-- does not read now (half-saved) leaves what it put until it does.
local function read_file(file, lang, done)
  local origin = origin_of(file, lang)
  local n = (reading[origin] or 0) + 1
  reading[origin] = n
  read_json(file, function(data, err)
    if reading[origin] ~= n then return done(0) end
    if data == nil then
      if not fs.exists(file) then S._set(origin, lang, nil) end
      return done(0, err)
    end
    if type(data) ~= "table" or getmetatable(data) == kawoosh.json.array then
      return done(0, fs.short(file) .. ": not an object of snippets")
    end
    local count, problems = S._set(origin, lang, data)
    for _, p in ipairs(problems or {}) do say(fs.short(file) .. ": " .. p) end
    done(count)
  end)
end

-- A package's languages and their files, as its `package.json` says:
-- `{ [lang] = { file, … } }`, each file once a language; nil and why
-- when it names none. A path is the package's own: one written absolute,
-- or reaching out of the package's folder with `..`, is refused and
-- said once (VS Code warns of it too) — a package is data, and its
-- entries do not name the rest of the disk.
local function package_manifest(pkg, data)
  local c = type(data) == "table" and data.contributes
  local list = type(c) == "table" and c.snippets
  if type(list) ~= "table" then return nil, "no contributes.snippets" end
  local dir = fs.parent(pkg)
  local m, seen = {}, {}
  for _, e in ipairs(list) do
    if type(e) == "table" and type(e.path) == "string" then
      local file = fs.expand(fs.join(dir, e.path))
      local inside = not e.path:match("^[~/\\]") and not e.path:match("^%a:")
        and fs.relative(file, dir) ~= nil
      if not inside then
        say_once(fs.short(pkg) .. ": " .. e.path .. " is outside the package; not read")
      else
        local langs = type(e.language) == "table" and e.language or { e.language }
        for _, l in ipairs(langs) do
          if type(l) == "string" and l ~= "" then
            l = l:lower()
            local key = l .. "\n" .. file
            if not seen[key] then
              seen[key] = true
              m[l] = m[l] or {}
              table.insert(m[l], file)
            end
          end
        end
      end
    end
  end
  return m
end

-- What `path` holds, by language, to `cb(manifest, why, pkg)`: a
-- package's (`pkg` its `package.json`), a folder's snippet files, or
-- one file — its language `lang`, else its name's.
local function manifest(path, lang, cb)
  local pkg
  if fs.is_dir(path) then
    local p = fs.join(path, "package.json")
    if not fs.is_file(p) then
      return fs.list(path, function(entries, err)
        if not entries then return cb(nil, err) end
        local m = {}
        for _, e in ipairs(entries) do
          local l = not e.is_dir and e.name ~= "package.json" and lang_of(e.name)
          if l then
            m[l] = m[l] or {}
            table.insert(m[l], fs.join(path, e.name))
          end
        end
        cb(m)
      end)
    end
    pkg = fs.expand(p)
  elseif fs.basename(path) == "package.json" then
    pkg = path
  end
  if pkg then
    return read_json(pkg, function(data, err)
      if data == nil then return cb(nil, err, pkg) end
      local m, why = package_manifest(pkg, data)
      if not m then return cb(nil, fs.short(pkg) .. ": " .. why, pkg) end
      cb(m, nil, pkg)
    end)
  end
  local l = lang or lang_of(path)
  if not l then
    return cb(nil, fs.short(path) .. ": no language in its name (load_json(path, lang) names one)")
  end
  cb({ [l:lower()] = { path } })
end

-- kawoosh.snippets.load_json(path[, lang][, fn]): the snippets of a
-- VS Code JSON file (`lang` its language, else its name's), of a
-- folder of them, or of a package (its folder or its `package.json`),
-- every language it has, read off the frame; `fn(count, why)` once
-- every file landed — `why` the first that did not read. Read again,
-- a file's replace what it put.
---@param path string a file, a folder, a package's folder or its `package.json`
---@param lang? string|fun(count: integer, why?: string) the language, else the file's name says; or `fn` in its place
---@param fn? fun(count: integer, why?: string) once every file landed; by default `why` goes to the log
---@return nil
function S.load_json(path, lang, fn)
  if type(lang) == "function" then lang, fn = nil, lang end
  fn = fn or function(_, why) if why then say("load_json: " .. why) end end
  manifest(fs.expand(path), lang, function(m, why)
    if not m then return fn(0, why) end
    local left, total, first = 0, 0, nil
    for _, files in pairs(m) do left = left + #files end
    if left == 0 then return fn(0, nil) end
    for l, files in pairs(m) do
      for _, f in ipairs(files) do
        read_file(f, l, function(n, err)
          total, first, left = total + n, first or err, left - 1
          if left == 0 then fn(total, first) end
        end)
      end
    end
  end)
end

-- ---------------------------------------------------------- snippets.paths

-- Each path as last read: `path`, `manifest` once read, `pkg` (a
-- package's `package.json`), `loaded` (origin → { file, lang }), `gen`
-- (its manifest's read under way); in the setting's order.
local roots, order = {}, {}
-- The languages a buffer asked for, and the names each is found under.
local wanted = {}
local watched_key

local on_change

-- Every path and every file read, on the watch.
local function rewatch()
  local list, seen = {}, {}
  local function add(p)
    if p and not seen[p] then
      seen[p] = true
      list[#list + 1] = p
    end
  end
  for _, p in ipairs(order) do
    local r = roots[p]
    add(r.path)
    add(r.pkg)
    local files = {}
    for _, o in pairs(r.loaded) do files[#files + 1] = o.file end
    table.sort(files)
    for _, f in ipairs(files) do add(f) end
  end
  local key = table.concat(list, "\n")
  if key == watched_key then return end
  watched_key = key
  fs.watch("snippets", #list > 0 and list or nil, on_change)
end

-- `names`' files of root `r` read.
local function load_lang(r, names)
  for _, name in ipairs(names) do
    for _, file in ipairs(r.manifest[name] or {}) do
      r.loaded[origin_of(file, name)] = { file = file, lang = name }
      read_file(file, name, function(_, why) if why then say(why) end end)
    end
  end
end

-- Root `r`'s manifest read again, what it no longer names dropped, and
-- every language asked for read again from it. A manifest that does not
-- read now — a typo, a `package.json` caught half-saved or between its
-- removal and its rename back, a listing that failed — leaves what the
-- root put until it reads again; only a path gone takes all of it.
local function refresh(r)
  r.gen = r.gen + 1
  local g = r.gen
  r.reading = true
  manifest(r.path, nil, function(m, why, pkg)
    if roots[r.path] ~= r or r.gen ~= g then return end
    r.reading = false
    local gone = not fs.exists(r.path)
    if not gone and r.pkg and not pkg then
      m, why = nil, fs.short(r.pkg) .. ": gone, or not there yet"
    end
    if not m and not gone then
      say("snippets.paths: " .. why)
      r.manifest = r.manifest or {}
      rewatch()
      return
    end
    m = m or {}
    r.manifest, r.pkg = m, pkg
    local keep = {}
    for l, files in pairs(m) do
      for _, f in ipairs(files) do keep[origin_of(f, l)] = true end
    end
    for origin, o in pairs(r.loaded) do
      if not keep[origin] then
        reading[origin] = (reading[origin] or 0) + 1
        S._set(origin, o.lang, nil)
        r.loaded[origin] = nil
      end
    end
    for _, names in pairs(wanted) do load_lang(r, names) end
    rewatch()
  end)
end

on_change = function(changed)
  for _, p in ipairs(changed) do
    for _, path in ipairs(order) do
      local r = roots[path]
      if p == r.path or p == r.pkg then
        -- With no language asked for yet, read when one is.
        if next(wanted) then refresh(r) else r.manifest = nil end
      else
        for _, o in pairs(r.loaded) do
          if o.file == p then
            read_file(o.file, o.lang, function(_, why) if why then say(why) end end)
          end
        end
      end
    end
  end
end

-- Called by the editor the first time a buffer of `lang` asks for a
-- completion: its snippets, under each of `names`, read from every
-- path.
---@param lang string
---@param names string[] the names its snippets are found under
---@return nil
function kawoosh._snippets_want(lang, names)

  wanted[lang] = names
  for _, path in ipairs(order) do
    local r = roots[path]
    if r.manifest and not r.reading then
      load_lang(r, names)
    elseif not r.reading then
      refresh(r)
    end
  end
  rewatch()
end

-- A path as the setting has it: `~`, absolute, or a host's as written;
-- else under the config directory.
local function root_path(p)
  if not (p:match("^[~/\\]") or p:match("^%a:[/\\]") or p:match("^[%w_%.%-]+:[/~]")) then
    local c = fs.config()
    if c then p = fs.join(c, p) end
  end
  return fs.expand(p)
end

local paths_key
kawoosh.on_settings(function()
  local set = kawoosh.opt("snippets.paths")
  if type(set) == "string" then set = { set } end
  local list = {}
  for _, p in ipairs(type(set) == "table" and set or {}) do
    if type(p) == "string" and p ~= "" then list[#list + 1] = root_path(p) end
  end
  local key = table.concat(list, "\n")
  if key == paths_key then return end
  paths_key = key
  local new = {}
  for _, p in ipairs(list) do new[p] = roots[p] or { path = p, loaded = {}, gen = 0 } end
  for p, r in pairs(roots) do
    if not new[p] then
      r.gen = r.gen + 1
      for origin, o in pairs(r.loaded) do
        reading[origin] = (reading[origin] or 0) + 1
        S._set(origin, o.lang, nil)
      end
    end
  end
  roots, order = new, {}
  local seen = {}
  for _, p in ipairs(list) do
    if not seen[p] then
      seen[p] = true
      order[#order + 1] = p
    end
  end
  for _, p in ipairs(order) do
    local r = roots[p]
    if not r.manifest and not r.reading and next(wanted) then refresh(r) end
  end
  rewatch()
end)
