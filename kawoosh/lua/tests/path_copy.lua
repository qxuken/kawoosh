-- The path copies (roadmap step 23): `<leader>y*` puts the file's path
-- on the clipboard and in the register — relative to the working
-- directory, absolute, its directory both ways, its name, its stem —
-- and in a `dir` listing the entry's under the caret; in a plugin's
-- pane, what it shows (`kawoosh.view`'s `path`).
local root = os.tmpname()
os.remove(root)
root = root .. "-paths"
-- Not a `.rs`: an installed language server would start in the
-- directory and hold it (Windows will not remove it then).
kawoosh.fs.write(kawoosh.fs.join(kawoosh.fs.join(root, "src"), "lib.test.txt"), "x")
kawoosh.fs.write(kawoosh.fs.join(root, "pic.svg"),
  '<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>')
-- A path as the platform writes it: `src/x`, with a backslash on Windows.
local sep = kawoosh.fs.join("a", "b"):sub(2, 2)
local function native(p) return (p:gsub("/", sep)) end
kawoosh.cmd("cd " .. root)
kawoosh.frame()
-- The working directory as the editor spells it (the process's is
-- `/private/tmp`, the link resolved).
kawoosh.cmd("pwd")
kawoosh.frame()
local cwd = kawoosh.message()

kawoosh.cmd("e src/lib.test.txt")
kawoosh.frame()
local function copied(keys)
  kawoosh.press(keys)
  kawoosh.frame()
  return kawoosh.memory()[1].text
end
kawoosh.test.eq(copied("<leader>yp"), native("src/lib.test.txt"), "relative")
kawoosh.test.eq(kawoosh.message(), native("copied src/lib.test.txt"))
kawoosh.test.eq(copied("<leader>yP"), cwd .. native("/src/lib.test.txt"), "absolute")
kawoosh.test.eq(copied("<leader>yd"), "src", "the directory")
kawoosh.test.eq(copied("<leader>yD"), cwd .. native("/src"), "the directory, absolute")
kawoosh.test.eq(copied("<leader>yn"), "lib.test.txt", "the name")
kawoosh.test.eq(copied("<leader>yN"), "lib.test", "the stem")
kawoosh.test.eq(kawoosh.memory()[1].took, "yank")

-- In a listing, the entry under the caret; on `../`, the directory.
kawoosh.press("-")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.line(kawoosh.buf.cursor().line), "lib.test.txt")
kawoosh.test.eq(copied("<leader>yp"), native("src/lib.test.txt"), "the entry, relative")
kawoosh.test.eq(copied("<leader>yn"), "lib.test.txt", "the entry's name")
kawoosh.press("gg")
kawoosh.test.eq(copied("<leader>yp"), "src", "../ copies the listed directory")
kawoosh.test.eq(copied("<leader>yd"), ".", "its directory is the working one")

-- A plugin's pane: the file it shows, every form — the picture's, the
-- bytes' — and `:du`'s entry under its cursor.
kawoosh.cmd("image pic.svg")
kawoosh.frame(3)
kawoosh.test.eq(copied("<leader>yp"), "pic.svg", "the picture, relative")
kawoosh.test.eq(kawoosh.message(), "copied pic.svg")
kawoosh.test.eq(copied("<leader>yP"), cwd .. native("/pic.svg"), "the picture, absolute")
kawoosh.test.eq(copied("<leader>yD"), cwd, "the picture's directory, absolute")
kawoosh.test.eq(copied("<leader>yN"), "pic", "the picture's stem")
kawoosh.cmd("hex src/lib.test.txt")
kawoosh.frame(3)
kawoosh.test.eq(copied("<leader>yp"), native("src/lib.test.txt"), "the bytes' file")
kawoosh.test.eq(copied("<leader>yd"), "src", "the bytes' file's directory")
kawoosh.test.eq(copied("<leader>yn"), "lib.test.txt", "the bytes' file's name")
kawoosh.cmd("du")
kawoosh.wait(function()
  local du = kawoosh.du_pane.state()
  return du and du.cursor
end, 400, "the disk usage's cursor")
local entry = kawoosh.du_pane.state().cursor
kawoosh.test.eq(copied("<leader>yn"), entry, "the disk usage's entry")
kawoosh.test.eq(copied("<leader>yD"), cwd, "the entry's directory")

-- A scratch has no path to copy.
kawoosh.cmd("enew")
kawoosh.frame()
kawoosh.press("<leader>yp")
kawoosh.frame()
kawoosh.test.eq(kawoosh.message(), "no file for %")
kawoosh.fs.remove(root)
