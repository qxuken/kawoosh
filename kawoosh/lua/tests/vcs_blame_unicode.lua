-- Valid UTF-8 author names must reach the blame column intact, including
-- names whose thirteenth byte falls in the middle of a character.
local dir = os.tmpname()
os.remove(dir)
dir = dir .. "-blame-unicode"
kawoosh.fs.create(dir, true)
local file = kawoosh.fs.join(dir, "a.txt")
kawoosh.fs.write(file, "one\n")

local author
kawoosh.vcs.register("unicode", {
  probe = function(d, done) done(kawoosh.fs.is_file(kawoosh.fs.join(d, "a.txt")) and d or nil) end,
  base = function(_, _, _, done) done("one\n") end,
  blame = function(_, _, _, done)
    done { { line = 1, count = 1, rev = string.rep("a", 40),
      author = author, time = os.time(), summary = "Изменение" } }
  end,
})
kawoosh.opt("vcs.backends", { "unicode" })
kawoosh.frame()
kawoosh.cmd("e " .. file)
kawoosh.frame(2)
for _, case in ipairs {
  { "Ann Author", "Ann Author" },
  { "abcdefghijklmn", "abcdefghijklmn" },
  { "abcdefghijklmno", "abcdefghijklm…" },
  { "Александр", "Александр" },
  { "Александр Иван", "Александр Иван" },
  { "Александр Иванов", "Александр Ива…" },
  { "山田太郎", "山田太郎" },
  { string.rep("山", 15), string.rep("山", 13) .. "…" },
  { "😀😀😀😀", "😀😀😀😀" },
  { string.rep("😀", 15), string.rep("😀", 13) .. "…" },
} do
  author = case[1]
  kawoosh.vcs.blame_on(kawoosh.buf.current())
  kawoosh.frame()
  local row = kawoosh.buf.blame_at(1)
  kawoosh.test.eq(row.label, case[2] .. " · now", author)
  kawoosh.test.eq(row.summary, "Изменение")
end
kawoosh.fs.remove(dir)
