# A database's pane: tables, rows, queries, a cell changed

Status: planned and built 2026-10-06 from the ask "How about we make
sqlite db inspector panel? i want to see tables, run queries. Maybe
edit fields." The calls below are taken here, each the user's to
overturn; where the build moved one, the section says so. Roadmap
step 91. Companion to [plugin-panes.md](plugin-panes.md) (how a Lua
pane is made), roadmap steps 88 and 89 (the bytes pane and the
picture's pane, the two viewers this one is shaped after) and
[memory.md](memory.md) (the one SQLite file kawoosh already keeps).

## What there is

SQLite is in the graph: `rusqlite`, bundled, is the store
(`systems/src/store.rs`, mvp.md Decision 7) and the grammar archive's
reader (`systems/src/grammars.rs`). Nothing opens a database a user
names.

A file that is not text opens in the bytes pane (`hex.binary`, step
88): a database today is sixteen bytes of `SQLite format 3` and then
hex. The bytes pane is the shape to copy — a Lua pane (`kawoosh.view`)
over a door in Rust, one `cells` grid for what is tabular, a cursor
that is a cell, the keys the editor's letters, a `state()` a test
reads, `t` back to the other view of the same file.

Work off the frame's thread goes through a message with a token:
`kawoosh.fs.list(path, fn)` and `fs.walk` push a `Msg`, the shell runs
the job on the io thread (`Io::run`) or inline in a test
(`jobs_inline`), the `IoMsg` back is answered to the callback by its
token (`Runtime::answer`). A query is one of those.

A one-line input in a pane is the engine's field (`ctx.field`,
plugin-panes.md): a real buffer's line with the editor's modes, keys
local to it by `{ view =, field = }`. The search panel's bar is four
of them.

Others: DB Browser for SQLite and TablePlus show a tree of tables on
the left, a grid of rows on the right, a query tab, a cell edited in
place. `sqlite3`'s shell shows `.tables`, `.schema`, and rows as
columns. vim-dadbod-ui is a tree of connections, tables and saved
queries; a query's result is a buffer. Zed and VS Code have extensions
of the same shape. The common grid is what is asked for.

## Decisions

### 1. One pane, two halves and a line: `:sqlite [PATH]`

`:sqlite [PATH]` — PATH's, or the focused buffer's file's — opens a
column of its own (pane-placement.md: a pane of its own, not of the
buffer). On the left the tables and views the file has, each with its
row count; on the right a query line over a grid of rows. `<Tab>`
moves the keys between the tables and the grid; `i` puts them in the
query line. The pane is `kawoosh/lua/sqlite.lua`, a Lua pane like
`:hex`, over the door `kawoosh.sqlite` (Decision 2).

A file whose first sixteen bytes say `SQLite format 3` opens here by
itself (`sqlite.open`, on by default) — before the bytes pane asks
whether it is binary, which it is: the plugin is listed before `hex`
in `plugins.rs`. `t` in the pane opens the file as bytes after all,
through `kawoosh.hex.open`, not through the openers, so the two do not
hand the file back and forth.

Beaten: **a table as a buffer** (dadbod's way, a query's rows as
text): rows do not fit lines, a wide table wraps or is cut, and a cell
edited in text has to be parsed back. **A tree of files and
connections**: kawoosh has one file at a time; a connection is a
path. **Panes for each half** (tables in one, the grid in another):
one pane keeps `<Tab>` cheap and the pane's state one table.

### 2. The door: a query is a job, its rows a table of tables

`kawoosh.sqlite` (`lua/src/lib.rs`, over `systems/src/sqlite.rs`):

| verb | what |
|---|---|
| `query(path, sql[, params][, opts], fn)` | runs the SQL on the io thread, `fn(result, err)` on a frame after: `result` is `{ columns = {…}, rows = { {…}, … }, truncated =, changes =, ms = }` |
| `schema(path, fn)` | `fn({ tables = { { name =, kind = "table" \| "view", rows = N, without_rowid =, columns = { { name =, type =, notnull =, pk =, default = } } } }, size = })` |
| `is(path)` | whether the file's head says SQLite, read without the rest |
| `null` | the value a NULL comes back as, and binds as — a table, one, compared by identity |

Several statements in one `sql` run in turn (rusqlite's `Batch`); the
rows are the last statement's that had columns, `changes` the sum of
every statement's. Rows are capped by `opts.cap` (the pane's
`sqlite.rows`, 1000, a page) and `truncated` says when there were
more. A value comes as what it is: an integer a Lua integer, a real a
float, text a string, NULL the `null` table, a blob `{ blob = bytes }`
— a Lua string holds bytes, but then text and a blob would look alike.
`params` bind by position (`?1`, `?2`), the same shapes back the other
way, so a string binds as text and SQLite's affinity makes `"42"` an
integer in an INTEGER column; `null` binds NULL.

The connection is opened for the job and closed with it — `READ_WRITE`
without `CREATE`, so a path that is not there is an error rather than
a new database, and a file the user cannot write opens read-only by
SQLite's own rule; a busy timeout of a second against another writer.
No connection is held across frames: nothing to leak, and what is
shown is what the file says now (a refresh re-reads).

Beaten: **a connection kept open per pane**: a handle to manage, a
lock held against the program that owns the file, and nothing gained
for a pane that reads a page at a time. **Rows as text already
formatted**: the pane aligns and colours by type, a plugin may want
the numbers.

### 3. The grid is a `cells` node, the cursor a cell

As the bytes pane: a terminal grid whose rows are the rows on show —
the header first, a row number at the left in the faint colour — and
whose columns are the result's columns, each as wide as its widest
shown value, bounded by `sqlite.cell_width` (40) with `…` past it,
`│` between them. Numbers are right-aligned and muted, text as it is,
NULL a faint `NULL`, a blob `x'…' 12 B`; a newline or a control
character in a cell is drawn as a space. The cursor is one cell, lit
in the accent colour (the warning colour while a change is pending);
`h` `j` `k` `l` walk, `0` `$` (helix's `gh` `gl` too) the row's ends, `gg` `G` the first and
last row, `<C-d>` `<C-u>` `<C-f>` `<C-b>` pages; a click puts it, the
wheel scrolls the rows. Columns scroll sideways by whole columns when
the cursor walks past the edge. The foot says `row 3 of 1,204 · name
TEXT` and the cursor's value whole, cut to the line.

A table browsed is read a page at a time: `SELECT rowid AS _rowid_, *
FROM "t" ORDER BY … LIMIT 1000 OFFSET n`, the next page fetched when
the cursor reaches the last row fetched, appended; the count is one
`count(*)` on browse. `_rowid_` is kept for Decision 5 and not shown. A
`WITHOUT ROWID` table or a view is read as `SELECT *` alone.

A click on the header, or `o` on a column, sorts the browsed table by
that column, again the other way; a query's result is as the query
ordered it.

### 4. A query is the line's, `<CR>` runs it; `:sqlite query SQL` too

The line is an engine field, so it edits as a buffer line does (`ciw`,
`<C-w>`, the registers). `<CR>` in it runs what it says and leaves the
keys on the grid; `<Esc>` is the field's normal mode, `<Esc>` again
the grid. The result replaces the grid's rows and the foot says
`N rows · 12 ms`, `N rows (the first 1000)`, or `3 changed` for a
statement without rows — in which case the tables' counts are read
again, since an UPDATE or a DROP may have moved them. An error is the
foot's, in the danger colour, with SQLite's words. `:sqlite query
SQL…` runs from the command line. The lines run are kept in the store
(`sqlite`, by the file's path, the last fifty) and `<C-p>` `<C-n>` in
the field walk them.

Beaten: **a multi-line query buffer**: a buffer's header
(`kawoosh.buf.header`) would give one, as the search panel's results
have, but a query that needs lines is a `.sql` file, and `:sqlite
query` from one is a command away. Not built.

### 5. A cell changed is one UPDATE, by rowid, at once

`c` (or `<CR>`) on a cell of a browsed table opens the query line
with the cell's value as text for editing; `<CR>` writes `UPDATE "t"
SET "col" = ?1 WHERE rowid = ?2` with the text bound — SQLite's affinity
types it as the column says — and the row is read again, the cell
shown as it is now. `x` sets the cell NULL. `u` takes the last change
back: the pane keeps `{ table, rowid, column, old }` for each change
this run, and `u` writes the old value the same way. A `WITHOUT ROWID`
table is keyed by its primary-key columns; a view, or a query's
result, refuses: `a query's rows: browse the table to change one`.

No confirm: the change is one row, one column, shown at once, and `u`
is there. No draft either, unlike the bytes pane — a database is
transactional and another program may be writing it, so a change
waiting in the pane would be a change against a row that moved.
A blob is not edited in the grid (`x` and a query are the ways).

### 6. Not built, on purpose

A connection's dialect other than SQLite; a schema editor (`CREATE`
and `ALTER` are a query); inserting and deleting rows from the grid
(a query); export (`y` copies a cell, `Y` a row as tab-separated,
which is a spreadsheet's paste); a session bringing the pane back
(`session = false`, as the other viewers); a query off the io thread
cancelled (a long one holds the thread, not the frame); syntax
colours in the query line (no SQL grammar is built in; kawoosh-grammars
carries one, and the field is a line); a host's (ssh) database (the
path is opened by SQLite, which reads the local disk).

## Built

2026-10-06, all of Decisions 1–5: `systems/src/sqlite.rs` (`query`,
`schema`, `is_sqlite`, `Value`, `Rows`), `kawoosh.sqlite` in
`lua/src/lib.rs` (`Msg::Sqlite`, `Msg::SqliteSchema`, `IoMsg::Sqlite`,
`IoMsg::SqliteSchema`, the shell's handlers in `scripting.rs` and
`app.rs`), `kawoosh/lua/sqlite.lua`, `kawoosh/tests/sqlite.rs`, a
`systems` unit test, help in `files.md`, the door in `lua.md`.
