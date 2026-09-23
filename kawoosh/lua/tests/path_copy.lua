-- The path copies (roadmap step 23): `<leader>y*` puts the file's path
-- on the clipboard and in the register — relative to the working
-- directory, absolute, its directory both ways, its name, its stem —
-- and in a `dir` listing the entry's under the caret.
local root = os.tmpname()
os.remove(root)
root = root .. "-paths"
assert(os.execute("mkdir -p '" .. root .. "/src'"))
local f = assert(io.open(root .. "/src/lib.test.rs", "w")) f:write("x") f:close()
kawoosh.cmd("cd " .. root)
kawoosh.frame()
-- The working directory as the editor spells it (the process's is
-- `/private/tmp`, the link resolved).
kawoosh.cmd("pwd")
kawoosh.frame()
local cwd = kawoosh.message()

kawoosh.cmd("e src/lib.test.rs")
kawoosh.frame()
local function copied(keys)
  kawoosh.press(keys)
  kawoosh.frame()
  return kawoosh.memory()[1].text
end
kawoosh.test.eq(copied("<leader>yp"), "src/lib.test.rs", "relative")
kawoosh.test.eq(kawoosh.message(), "copied src/lib.test.rs")
kawoosh.test.eq(copied("<leader>yP"), cwd .. "/src/lib.test.rs", "absolute")
kawoosh.test.eq(copied("<leader>yd"), "src", "the directory")
kawoosh.test.eq(copied("<leader>yD"), cwd .. "/src", "the directory, absolute")
kawoosh.test.eq(copied("<leader>yn"), "lib.test.rs", "the name")
kawoosh.test.eq(copied("<leader>yN"), "lib.test", "the stem")
kawoosh.test.eq(kawoosh.memory()[1].took, "yank")

-- In a listing, the entry under the caret; on `../`, the directory.
kawoosh.press("-")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.line(kawoosh.buf.cursor().line), "lib.test.rs")
kawoosh.test.eq(copied("<leader>yp"), "src/lib.test.rs", "the entry, relative")
kawoosh.test.eq(copied("<leader>yn"), "lib.test.rs", "the entry's name")
kawoosh.press("gg")
kawoosh.test.eq(copied("<leader>yp"), "src", "../ copies the listed directory")
kawoosh.test.eq(copied("<leader>yd"), ".", "its directory is the working one")

-- A scratch has no path to copy.
kawoosh.cmd("enew")
kawoosh.frame()
kawoosh.press("<leader>yp")
kawoosh.frame()
kawoosh.test.eq(kawoosh.message(), "no file for %")
assert(os.execute("rm -rf '" .. root .. "'"))
