-- A patch as `vcs show` draws it (docs/design/secrets.md Decision 9):
-- each file's section masked by its file's rules, its sides read whole;
-- the message, the stat and the headers kept; a hunk's lines counted.
local patch = table.concat({
  "commit 1234",
  "    TOKEN=in a message is no file's",
  "",
  " prod.env | 2 +-",
  "diff --git a/prod.env b/prod.env",
  "index 1..2 100644",
  "--- a/prod.env",
  "+++ b/prod.env",
  "@@ -1,4 +1,4 @@",
  " HOST=db",
  "--- x",
  "+++ y",
  "-TOKEN=old7",
  "+TOKEN=new8",
  " PORT=1",
  "diff --git a/gone.env b/gone.env",
  "deleted file mode 100644",
  "--- a/gone.env",
  "+++ /dev/null",
  "@@ -1 +0,0 @@",
  "-KEY=gone9",
  "diff --git a/notes.txt b/notes.txt",
  "--- a/notes.txt",
  "+++ b/notes.txt",
  "@@ -1 +1,4 @@",
  " a key:",
  "+-----BEGIN RSA PRIVATE KEY-----",
  "+MIIE",
  "+-----END RSA PRIVATE KEY-----",
  "\\ No newline at end of file",
  "TOKEN=after the hunks",
  "",
}, "\n")
local shown = kawoosh.vcs.shown_patch(patch, "/r")
local lines = {}
for l in shown:gmatch("([^\n]*)\n") do lines[#lines + 1] = l end
kawoosh.test.eq(lines[2], "    TOKEN=in a message is no file's", "the message kept")
kawoosh.test.eq(lines[7], "--- a/prod.env", "the headers kept")
kawoosh.test.eq(lines[10], " HOST=••••••••")
-- `--- x` and `+++ y` are lines of the hunk, which its counts say:
-- read as headers, they would have named the file `y`, no `.env`.
kawoosh.test.eq(lines[11], "--- x")
kawoosh.test.eq(lines[12], "+++ y")
kawoosh.test.eq(lines[13], "-TOKEN=••••••••", "the old side by its file's rules")
kawoosh.test.eq(lines[14], "+TOKEN=••••••••", "the new side")
kawoosh.test.eq(lines[21], "-KEY=••••••••", "a deleted file by its `---`")
kawoosh.test.eq(lines[26], " a key:")
kawoosh.test.eq(lines[27], "+••••••••", "a block across the new side's lines")
kawoosh.test.eq(lines[28], "+••••••••")
kawoosh.test.eq(lines[29], "+••••••••")
kawoosh.test.eq(lines[30], "\\ No newline at end of file")
kawoosh.test.eq(lines[31], "TOKEN=after the hunks", "past the hunks, no file's")
kawoosh.test.eq(#lines, 31, "every line kept")

-- fossil's headers: `Index:`, paths with no `a/` `b/`.
local fossil = "Index: conf/prod.env\n" .. string.rep("=", 10) .. "\n--- conf/prod.env\n+++ conf/prod.env\n"
  .. "@@ -1 +1 @@\n-TOKEN=old7\n+TOKEN=new8\n"
kawoosh.test.eq(kawoosh.vcs.shown_patch(fossil, "/r"),
  "Index: conf/prod.env\n" .. string.rep("=", 10) .. "\n--- conf/prod.env\n+++ conf/prod.env\n"
  .. "@@ -1 +1 @@\n-TOKEN=••••••••\n+TOKEN=••••••••\n")

-- A merge's combined diff: a column per parent.
local cc = "diff --cc prod.env\n--- a/prod.env\n+++ b/prod.env\n@@@ -1,1 -1,1 +1,1 @@@\n- TOKEN=one1\n -TOKEN=two2\n++TOKEN=both3\n"
kawoosh.test.eq(kawoosh.vcs.shown_patch(cc, "/r"),
  "diff --cc prod.env\n--- a/prod.env\n+++ b/prod.env\n@@@ -1,1 -1,1 +1,1 @@@\n- TOKEN=••••••••\n -TOKEN=••••••••\n++TOKEN=••••••••\n")

-- A rename: masked by the old name's rules as by the new's — git finds
-- renames, and `prod.env.bak` is no `.env` (a review, 2026-10-16).
local renamed = table.concat({
  "diff --git a/prod.env b/prod.env.bak",
  "similarity index 90%",
  "rename from prod.env",
  "rename to prod.env.bak",
  "--- a/prod.env",
  "+++ b/prod.env.bak",
  "@@ -1,2 +1,2 @@",
  " HOST=db",
  "-TOKEN=old7",
  "+TOKEN=new8",
  "",
}, "\n")
local r = kawoosh.vcs.shown_patch(renamed, "/r")
kawoosh.test.eq(r:find("old7", 1, true), nil, "the old side by the old name's rules")
kawoosh.test.eq(r:find("new8", 1, true), nil, "the new side too")
kawoosh.test.eq(r:find(" HOST=••••••••", 1, true) ~= nil, true, "the context too")

-- A name git C-quotes (a `"` in it): unquoted, its `a/` `b/` taken off.
local quoted = table.concat({
  'diff --git "a/we\\"ird.env" "b/we\\"ird.env"',
  '--- "a/we\\"ird.env"',
  '+++ "b/we\\"ird.env"',
  "@@ -1 +1 @@",
  "-TOKEN=old7",
  "+TOKEN=new8",
  "",
}, "\n")
local q = kawoosh.vcs.shown_patch(quoted, "/r")
kawoosh.test.eq(q:find("old7", 1, true), nil, "a quoted name's rules")
kawoosh.test.eq(q:find("new8", 1, true), nil, "a quoted name's rules, the new side")
