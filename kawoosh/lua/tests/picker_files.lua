-- The files picker (picker.lua), on the harness: the files git sees
-- under the working directory, the open buffer's first; the query
-- narrows; `<CR>` opens the cursor's file where the keys came from.
local dir = os.tmpname()
os.remove(dir)
dir = dir .. "-picker"
local function sh(cmd) assert(os.execute(cmd), cmd) end
local function write(p, s) local f = assert(io.open(p, "w")) f:write(s) f:close() end
sh("mkdir -p '" .. dir .. "/src' '" .. dir .. "/target' '" .. dir .. "/.git'")
write(dir .. "/src/main.rs", "fn main() {}\nfn helper() {}\n")
write(dir .. "/src/lib.rs", "pub fn lib() {}\n")
write(dir .. "/README.md", "# notes\nalpha\nbeta\n")
write(dir .. "/target/out.o", "x")
write(dir .. "/.gitignore", "target/\n")

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
kawoosh.test.eq(st.text, "src/lib.rs", "narrowed to the best match")

kawoosh.press("<CR>")
kawoosh.frame()
kawoosh.test.eq(kawoosh.picker.state(), nil, "gone on a pick")
-- By its tail: a file's path is canonical, and `/tmp` is a link on macOS.
local path = kawoosh.buf.path() or ""
kawoosh.test.eq(path:sub(-#"-picker/src/lib.rs"), "-picker/src/lib.rs", "the pick is open: " .. path)
kawoosh.test.eq(kawoosh.buf.line(1), "pub fn lib() {}")
sh("rm -rf '" .. dir .. "'")
