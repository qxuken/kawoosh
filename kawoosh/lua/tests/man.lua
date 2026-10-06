-- Manual pages (man.lua, docs/design/man.md): `:man PAGE` reads a
-- page into a read-only buffer of the `man` language, rendered to the
-- pane's width, its overstrikes read into paints; `K` follows a
-- reference, `<C-o>` comes back, `]]` walks the section heads, `q`
-- closes; `:man` alone takes the word under the caret, else the
-- picker. The reader is a script of the test's own, so the page is
-- known and no system's `man` is needed.
local man = kawoosh.man
local fs = kawoosh.fs

-- ------------------------------------------------------------ rendering

-- Overstrikes: `c\bc` bold, `_\bc` underline; the header and footer
-- name the page; a reference is a word and a section.
local raw = table.concat({
  "LS(1)        General Commands Manual        LS(1)",
  "",
  "N\bNA\bAM\bME\bE",
  "     l\bls\bs - list directory contents   ",
  "",
  "S\bSE\bEE\bE A\bAL\bLS\bSO\bO",
  "     chmod(1), _\bf_\bi_\bl_\be, and \27[1mbold\27[0m text",
  "",
  "macOS 14                                   LS(1)",
}, "\n") .. "\n"
local text, info = man.render(raw)
local lines = {}
for l in (text .. "\n"):gmatch("(.-)\n") do lines[#lines + 1] = l end
kawoosh.test.eq(lines[3], "NAME", "the overstrikes read off")
kawoosh.test.eq(lines[4], "     ls - list directory contents", "trailing spaces dropped")
kawoosh.test.eq(lines[7], "     chmod(1), file, and bold text", "the SGR read off")
kawoosh.test.ok(not text:find("\b", 1, true), "no backspace left")
kawoosh.test.ok(not text:find("\27", 1, true), "no escape left")
kawoosh.test.eq(#info.heads, 2, "two section heads")
kawoosh.test.eq(info.heads[1].line, 3)
kawoosh.test.eq(info.heads[2].line, 6)
kawoosh.test.eq(#info.refs, 1, "the header's and footer's names are no references")
kawoosh.test.eq(info.refs[1].page, "chmod")
kawoosh.test.eq(info.refs[1].section, "1")
kawoosh.test.ok(info.header ~= nil and info.footer ~= nil, "header and footer found")
-- The spans: NAME bold, ls bold, SEE ALSO bold, file underlined, bold bold.
local kinds = {}
for _, s in ipairs(info.spans) do kinds[#kinds + 1] = text:sub(s[1] + 1, s[2]) .. ":" .. s[3] end
kawoosh.test.eq(table.concat(kinds, "|"),
  "NAME:bold|ls:bold|SEE:bold|ALSO:bold|file:underline|bold:bold")

-- ------------------------------------------------------------- parsing

local function parsed(words)
  local p, s = man.parse(words)
  return p .. "/" .. tostring(s)
end
kawoosh.test.eq(parsed({ "ls" }), "ls/nil")
kawoosh.test.eq(parsed({ "3", "printf" }), "printf/3")
kawoosh.test.eq(parsed({ "printf(3)" }), "printf/3")
kawoosh.test.eq(parsed({ "printf.3" }), "printf/3")
kawoosh.test.eq(parsed({ "git-log(1)" }), "git-log/1")
kawoosh.test.eq(parsed("3p open"), "open/3p")
kawoosh.test.eq(man.parse({}), nil)

local line = "     chmod(1), file, and printf. Also foo.bar"
local function at(col)
  local p, s = man.reference_at(line, col)
  return tostring(p) .. "/" .. tostring(s)
end
kawoosh.test.eq(at(7), "chmod/1", "on the name")
kawoosh.test.eq(at(13), "chmod/1", "on the section")
kawoosh.test.eq(at(16), "file/nil", "a word")
kawoosh.test.eq(at(27), "printf/nil", "the sentence's dot dropped")
kawoosh.test.eq(at(4), "nil/nil", "a space")
kawoosh.test.eq(at(39), "foo.bar/nil", "a dotted name whole")

-- ------------------------------------------------------- a reader of ours

local dir = os.tmpname()
os.remove(dir)
dir = dir .. "-man"
fs.create(dir, true)
local script = fs.join(dir, "fakeman.sh")
fs.write(script, table.concat({
  "#!/bin/sh",
  "# A manual reader of the test's own: `-k .` lists, else a page.",
  'if [ "$1" = "-k" ]; then',
  "  printf 'ls(1)                    - list directory contents\\n'",
  "  printf 'chmod(1), fchmod(2)      - change file modes\\n'",
  "  exit 0",
  "fi",
  'sec=1; page=$1',
  'case "$1" in [0-9]*) sec=$1; page=$2;; esac',
  'if [ "$page" = "nope" ]; then echo "No manual entry for nope" >&2; exit 1; fi',
  'UP=$(printf %s "$page" | tr a-z A-Z)',
  "printf '%s(%s)   General Commands Manual   %s(%s)\\n\\n' \"$UP\" \"$sec\" \"$UP\" \"$sec\"",
  "printf 'N\\bNA\\bAM\\bME\\bE\\n     %s - a page at width %s under %s\\n\\n' \"$page\" \"$MANWIDTH\" \"$MANPAGER\"",
  "printf 'S\\bSE\\bEE\\bE A\\bAL\\bLS\\bSO\\bO\\n     chmod(1), _\\bf_\\bi_\\bl_\\be\\n\\n'",
  "printf 'test   %s(%s)\\n' \"$UP\" \"$sec\"",
  "",
}, "\n"))
kawoosh.opt("man.command", "sh " .. script)
kawoosh.frame()

-- `:man ls`: the page in the focused pane, read-only, its language
-- `man`, as wide as the pane less one.
kawoosh.cmd("man ls")
kawoosh.wait(function() return kawoosh.buf.name() == "*man ls(1)*" end, nil, "the ls page")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.language(), "man")
kawoosh.test.ok(kawoosh.holds("readonly") == true, "read-only")
kawoosh.test.eq(kawoosh.buf.line(3), "NAME")
local width = kawoosh.buf.line(4):match("at width (%d+)")
local size = kawoosh.pane_size(kawoosh.pane())
kawoosh.test.ok(size and size.cols > 40, "the pane's size is known: " .. tostring(size and size.cols))
kawoosh.test.eq(tonumber(width), size.cols - 1, "rendered to the pane's width less one")
kawoosh.test.ok(kawoosh.buf.line(4):find("under cat", 1, true), "no pager")
local pages = man.pages()
kawoosh.test.eq(pages["*man ls(1)*"].page, "ls")
kawoosh.test.eq(pages["*man ls(1)*"].section, "1")

-- `]]` `[[`: the section heads.
kawoosh.press("gg")
kawoosh.press("]]")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.cursor().line, 3, "the first head")
kawoosh.press("]]")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.cursor().line, 6, "the second")
kawoosh.press("[[")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.cursor().line, 3, "and back")

-- `K` on a reference: its page, in this pane; `<C-o>` the way back.
kawoosh.press("/chmod<CR>")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.cursor().line, 7)
kawoosh.press("K")
kawoosh.wait(function() return kawoosh.buf.name() == "*man chmod(1)*" end, nil, "the chmod page")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.line(1):sub(1, 8), "CHMOD(1)")
kawoosh.press("<C-o>")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.name(), "*man ls(1)*", "<C-o> comes back")

-- `:man 2 fchmod`, a section asked: the page named with it.
kawoosh.cmd("man 2 fchmod")
kawoosh.wait(function() return kawoosh.buf.name() == "*man fchmod(2)*" end, nil, "fchmod(2)")

-- `q` closes the page as `:bd` does.
kawoosh.press("q")
kawoosh.frame()
kawoosh.test.ok(kawoosh.buf.name() ~= "*man fchmod(2)*", "closed")
kawoosh.test.eq(man.pages()["*man fchmod(2)*"], nil, "forgotten")

-- A page there is none of: the reader's word.
kawoosh.cmd("man nope")
kawoosh.wait(function() return kawoosh.message():find("No manual entry", 1, true) ~= nil end, nil, "the error")

-- `:man` alone: the word under the caret.
local file = fs.join(dir, "notes.txt")
fs.write(file, "see chmod for modes\n\n")
kawoosh.cmd("e " .. file)
kawoosh.frame()
kawoosh.press("w")
kawoosh.cmd("man")
kawoosh.wait(function() return kawoosh.buf.name() == "*man chmod(1)*" end, nil, "the word's page")

-- And on nothing, the picker over every page: `<CR>` opens one.
kawoosh.cmd("e " .. file)
kawoosh.frame()
kawoosh.press("j")
kawoosh.cmd("man")
kawoosh.frame(2)
local st = assert(kawoosh.picker.state(), "the picker is open")
kawoosh.test.eq(st.source, "man")
kawoosh.wait(function() return kawoosh.picker.state().count == 3 end, nil, "three pages listed")
kawoosh.press("fch")
kawoosh.wait(function() return kawoosh.picker.state().count == 1 end, nil, "one match")
kawoosh.test.eq(kawoosh.picker.state().text, "fchmod(2)")
kawoosh.press("<CR>")
kawoosh.wait(function() return kawoosh.buf.name() == "*man fchmod(2)*" end, nil, "the pick's page")
