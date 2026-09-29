-- The files picker (picker.lua), on the harness: the files git sees
-- under the working directory, the open buffer's first; the query
-- narrows; `<CR>` opens the cursor's file where the keys came from.
local dir = os.tmpname()
os.remove(dir)
dir = dir .. "-picker"
local function write(p, s) kawoosh.fs.write(kawoosh.fs.join(dir, p), s) end
kawoosh.fs.create(kawoosh.fs.join(dir, ".git"), true)
-- A path as the platform writes it: `src/x`, with a backslash on Windows.
local sep = kawoosh.fs.join("a", "b"):sub(2, 2)
local function native(p) return (p:gsub("/", sep)) end
write("src/main.rs", "fn main() {}\nfn helper() {}\n")
write("src/lib.rs", "pub fn lib() {}\n")
write("README.md", "# notes\nalpha\nbeta\n")
write("target/out.o", "x")
write(".gitignore", "target/\n")

kawoosh.cmd("cd " .. dir)
kawoosh.cmd("e README.md")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.name(), "README.md", "the file is open")

kawoosh.press("<leader>f")
kawoosh.frame(2)
local st = assert(kawoosh.picker.state(), "the picker is open")
kawoosh.test.eq(st.source, "files")
kawoosh.test.eq(st.count, 3, "the ignored and hidden files left out")
kawoosh.test.eq(st.text, "README.md", "the open buffer's file first")
kawoosh.test.eq(kawoosh.mode(), "insert", "the query has the keys")

kawoosh.press("sl")
kawoosh.frame()
st = kawoosh.picker.state()
kawoosh.test.eq(st.query, "sl")
kawoosh.test.eq(st.text, native("src/lib.rs"), "narrowed to the best match")

-- A path typed whole, from the root: matched as the rows spell it.
kawoosh.field_set("picker", "q", kawoosh.fs.join(kawoosh.fs.cwd(), "src", "main.rs"))
kawoosh.frame()
st = kawoosh.picker.state()
kawoosh.test.eq(st.count, 1, "the one file")
kawoosh.test.eq(st.text, native("src/main.rs"))
-- Not under the root: as typed, which matches nothing here.
kawoosh.field_set("picker", "q", "/elsewhere/src/main.rs")
kawoosh.frame()
kawoosh.test.eq(kawoosh.picker.state().count, 0, "a path elsewhere is no row's")

kawoosh.field_set("picker", "q", "sl")
kawoosh.frame()
kawoosh.press("<CR>")
kawoosh.frame()
kawoosh.test.eq(kawoosh.picker.state(), nil, "gone on a pick")
-- By its tail: a file's path is canonical, and `/tmp` is a link on macOS.
local path = kawoosh.buf.path() or ""
local tail = native("-picker/src/lib.rs")
kawoosh.test.eq(path:sub(-#tail), tail, "the pick is open: " .. path)
kawoosh.test.eq(kawoosh.buf.line(1), "pub fn lib() {}")
-- `pcall`: a language server started on `lib.rs` holds the directory as
-- its cwd, and Windows will not remove it then.
pcall(kawoosh.fs.remove, dir)
