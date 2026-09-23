# The memory: what passed through the attention, kept as data

Status: decided 2026-09-21; built the same day as roadmap step 10, its
three rounds in one commit (`systems/src/store.rs`'s `moments` and
`recent`, `kawoosh/src/moments.rs`, the pane in `memory.rs`,
`kawoosh/lua/memory.lua`; `kawoosh/tests/memory.rs`). What the build
changed against the text below: the working memory's texts are read
back whole at launch rather than as headers with the bytes on demand
(Decision 3) — the store's caps bound them to what the RAM held before,
a hundred rows under `memory.text.max_mb`, so the footprint claim
holds by the cap and not by the lazier read, which stays for a round
that needs it; dwell is counted while kui says the window has the
keyboard and a key or a click came within `memory.idle_secs`
(Decision 7), and a delete's text counts as a yank of its buffer
(every text taken from it, as the decision says); a text taken again
within a second is one visit, as any subject; the resume key is
`<leader>sl` (`<leader>sr` went to the picker's resume in step 4) and
the pins live under `<leader>e` as a prefix — `<leader>ee` the list,
`<leader>ea` pin — since a key cannot be both a binding and a prefix;
the undo root made with the first edit is not counted as one. Round
four (`tool` and `location` rows) and the notes after are not built.
*Corrected 2026-09-22*, after a soundness pass over the build: a
history is the *path's* where a moment is the path's under a root —
the migration at init twinned every workspace's `file` row with an
empty one under no workspace at each launch, and that twin, scoring
nothing, went first past a cap and took the real row's history with
it; now a history that has a moment under any workspace gets none,
and `forget_moment` drops a history only when no row of the path
remains. The keyboard coming back to a file from a picker, the dock or
the memory pane was a visit (the frame compared panes, not subjects);
`x` on the register's head and a recall counted a yank on the origin;
a text that may not be written (over 1 MiB, or `memory.text.max_mb` at
0) still got a row with its hash and origin — none now, which is what
"nothing I copied is on disk" has to mean. Decision 2's workspace was
written and never read: the pane's views but `all`, the pins
(`<leader>e1` is the workspace's first pin), `kawoosh.oldfiles` (an
`all` argument for every root's) and `memory.boosts` are the
workspace's now. `kawoosh.memory { … }` and `oldfiles` fold the deltas
not yet flushed in (the pending map is one cell the shell and the
runtime share), so a file opened a moment ago has its row; a delta's
`meta` merges over the row's (`json_patch`) instead of replacing it,
so the caret line and a plugin's keys keep out of each other's way.
*Round four built 2026-09-22*: a `location` row per `path:line`
jumped to — `]q` `[q` and `<CR>` on a listing's line, a server's
definition — with `meta.from` (the listing's name, `definition`) and
`meta.message` (the line that named it, two hundred characters at
most); a `tool` row per `:tool NAME` (run or focused again) and per
`kawoosh.compile` (`compile`, with `meta.cmd`); a terminal pane's
dwell to its tool's row while it is a tool's; `⏎` on either in the
pane opens the file at the line or runs the tool again; and a run's
row goes at thirty days whatever `memory.keep_days` says
(`keep_days_for`). *Filters, the same day*: `/` in the pane (Decision
10 gains it) is a field of the editor's in the pane, `memory/q`, whose
line narrows the view's rows as it is typed — fzy's scoring over a
row's text (a text's first line, its origin; a file's name and path;
a command's line; a location's line and message), best first, the
header counting `n of all` — `<CR>` taking the cursor's row, `<Esc>`
twice handing the keys back with the filter kept, `<Esc>` in the pane
or closing it clearing it; `:memory filter QUERY` from the prompt.
*Round five built 2026-09-24* (roadmap step 25), the yank-pop: `[p`
`]p` (`put older`, `put newer`) replace the last put with the text one
older or newer in the memory — the `]x` / `[x` family's keys, since
Decision 1 of the roadmap had given `<C-n>` to select next and a key
meaning two things by what came before it was the worse trade. The
walk's order is the memory's as it stood at the first put, held by
each moment's id (`Memory::id`, `position`), so the text chosen is
recalled — the register's head, `p` puts it next — without the walk
losing its place; a secret, or a text forgotten meanwhile, is stepped
over, and a count steps further. The put is undone and made again, so
one `u` takes all of it back (the texts tried stay as branches of the
undo tree); an edit since ends the walk. A recall by the engine is an
effect (`Effect::Recalled`) the shell counts as the pane's recall is —
attended, not a yank. Left: co-occurrence.
options and taken: the unit is a subject row *plus a bounded ring of
recent transitions* (Decision 1); the histories lose their bookkeeping
and keep their blob (Decision 6); eviction is a fixed score in Rust
(Decision 4); texts persist, with the RAM footprint kept low and their
days and size as settings (Decisions 3, 4). Each decision keeps the
alternative it beat. Grows the
working memory of 2026-09-20 (`Editor::memory`, `:memory`,
`kawoosh.memory()`) into the one place the editor remembers, and
retires `oldfiles`, the histories' bookkeeping, and the prompt
histories into it. Companion to [roadmap.md](roadmap.md), whose
"working memory, round two", "pinned files", and half of "the picker"
land here. Inspired by Scott Jenson's desktop talks (Ubuntu Summit
2025, GUADEC 2024): the desktop as an extension of the brain, with a
working memory and metadata that give context, and two proposals that
map onto an editor almost one to one — the clipboard as an editable,
persistent cache tied to documents, and a history compressed from weak
signals (time on a page, scrolling, clipboard events) into a timeline,
with no assistant in the loop.

## The thesis

Kawoosh already remembers, in four places that do not know each other:

- `Editor::memory` — every yank, delete, change and clipboard paste as
  a `Moment` with its origin, in RAM, a hundred at most, gone at quit.
- `oldfiles` in the store — a path, when it was opened, the caret's
  line; twenty of them read back for `:oldfiles`.
- `histories` in the store — a buffer's undo tree and draft, with a
  `touched` time that decides its aging, written by `touch_history`
  from the same places `touch_oldfile` writes the same fact.
- `cmd_history` and `search_history` in the editor — the prompt's
  `<Up>`, in RAM, gone at quit.

Each is a memory of one kind of subject, with its own cap, its own
notion of "recent", and no way to ask across them. What Jenson asks of
a desktop is what a modal editor with panes can actually do, because
its "apps" are panes it owns: keep a small, typed record of what the
user attended to — texts, files, command lines, searches, runs — with
the weak signals that came with it, bounded and visible and editable,
and let everything that ranks or suggests read it. The first consumer
is the picker, which needs exactly this to order files, buffers and
commands. The second is "where was I", which the `:memory` pane
becomes. Nothing here needs a model; the point is that the data,
quietly kept, is what an assistant would have had to guess.

The three parts that are not trivial, and that the decisions below are
for: **what the unit is** (a memory of events is a log, and a log is
not what anyone wants back), **how it reaches the store** (two windows,
a crash mid-write, a RAM copy that must not wipe what the disk has
gained), and **how a limit keeps what matters** (a burst of two
thousand files walked by `]q` must not push out the file opened every
morning).

## Decisions

### 1. The unit is a subject with signals, and a ring keeps the recent sequence

A moment is one row per `(kind, subject, workspace)`. Its kinds are a
closed set the engine writes — `text`, `file`, `scratch`, `command`,
`search`, `tool`, `location` — plus what plugins add under their own
names (Decision 9). Its subject is the thing's identity: an absolute
path, `scratch:<n>`, the command line as typed, the pattern, the tool's
name, a location's `path:line`; for a text, a hash of its bytes, the
bytes themselves in the row. Its signals are counters the engine adds
to: `visits`, `dwell` (Decision 7), `edits` (undo checkpoints made in
it), `yanks` (texts taken from it); and two times, `first` and `last`.
`meta` is a JSON column for what a kind carries besides — a file's
caret line, a text's `took`, `linewise` and origin, a location's
message — and `pinned` is the user's word (Decision 5).

The rule that bounds it is the working memory's own, widened: a text
taken again while it is the head takes the newer origin rather than a
second place. Now *every* subject attended again extends its row
rather than adding one — `last` moves, the counters grow. A file
focused fifty times in an hour is one row with fifty visits and an
hour's dwell, not fifty rows. That compression is what Jenson's
browser proposal asks for, and it is what makes the limits (Decision
4) count subjects, which are few, instead of events, which are not.

The sequence within a day (foo at 9, bar at 10, foo at 14) is kept
separately and bounded by construction: **the ring**, `recent`, the
last `RECENT_MAX` (1000) transitions as `(at, kind, subject,
workspace)`, one row per visit (Decision 7's meaning of a visit),
oldest dropped past the cap. It is the timeline — "where was I this
morning" reads it, `<leader>sr` opens on it (Decision 10) — and it is
what the subject rows cannot answer: which file came *after* this one.
A thousand transitions is a few days of work at most, which is what a
timeline is for; the subject rows are what lasts. A subject forgotten
(`x`, eviction, aging) takes its ring rows with it, so the ring never
names what the memory no longer knows.

*The alternative*: an event journal alone (`kind, subject, at`) with
aggregates as queries. It keeps the day's sequence and it is the log
the design says not to build: unbounded by nature, so the cap is on
the wrong axis, and a table that grows by the keystroke is what the
perf tab and the logger are for. The ring takes the one thing the
journal had, the sequence, at a fixed size. The `.`/macro recorder of
roadmap step 3 is the one place a command *stream* is wanted, with a
lifetime of one edit; it taps the same `Editor::run` and is a different
thing.

### 2. One table in the store; oldfiles and the prompt histories retire into it

`moments` in `state.db`, beside `session` and `histories`:

```sql
CREATE TABLE moments (
    kind      TEXT    NOT NULL,
    subject   TEXT    NOT NULL,
    workspace TEXT    NOT NULL DEFAULT '',   -- outermost .kawoosh root, or ''
    first_at  INTEGER NOT NULL,
    last_at   INTEGER NOT NULL,
    visits    INTEGER NOT NULL DEFAULT 0,
    dwell_ms  INTEGER NOT NULL DEFAULT 0,
    edits     INTEGER NOT NULL DEFAULT 0,
    yanks     INTEGER NOT NULL DEFAULT 0,
    pinned    INTEGER NOT NULL DEFAULT 0,
    meta      TEXT    NOT NULL DEFAULT '{}',
    text      BLOB,                          -- text moments only
    PRIMARY KEY (kind, subject, workspace));
CREATE INDEX moments_last ON moments (kind, last_at);
CREATE TABLE recent (                        -- the ring, Decision 1
    at        INTEGER NOT NULL,
    kind      TEXT    NOT NULL,
    subject   TEXT    NOT NULL,
    workspace TEXT    NOT NULL DEFAULT '');
CREATE INDEX recent_at ON recent (at);
```

`oldfiles` migrates at init into `file` rows (one visit, `last_at` its
`opened_at`, the line in `meta`) and the table is dropped, the way
`drafts` was renamed. `:oldfiles` becomes `:memory files`. The prompt
histories become `command` and `search` rows: `<Up>` at the prompt
walks them by `last_at`, which is what `remember`'s retain-and-push
did, and they survive a restart, which they do not today. The `"`
register stays the newest `text` row, held in RAM (Decision 3).

Workspace is a column, not a table: a moment made under a `.kawoosh`
root carries it, one made outside carries none. The picker asks for
the workspace's rows; `:memory` shows the workspace's by default and
`:memory all` everything. A path is one subject across workspaces
only if it is the same path, which is the right answer for a file
shared by two roots.

### 3. The store is the truth; RAM holds deltas, headers and one text

This is the clobbering question, and the answer is that nothing ever
writes a moment *whole*. The engine keeps, in RAM:

- per kind, the newest rows' *headers* read at launch — kind, subject,
  times, counters, `meta`, and for a text its first line and byte
  count, never its bytes — a read cache the pane and `<Up>` use. The
  one text held whole is the head, the `"` register, because `p` must
  not wait on a read; every other text's bytes are read from the store
  when the pane's cursor lands on it or `recall` asks, a row read that
  is microseconds. This is what keeps the RAM footprint low with texts
  persisted: `Editor::memory` today holds a hundred texts' bytes, and
  with `memory.text.max_mb` of them on disk it would hold megabytes;
  now it holds one text and a hundred lines.
- a delta map `(kind, subject, workspace) → Delta { visits, dwell,
  edits, yanks, last, meta, text }` of what happened since the last
  flush, and the pending ring rows `(at, kind, subject, workspace)` in
  order.

A flush turns each delta into one upsert whose counters are
*increments*:

```sql
INSERT INTO moments (kind, subject, workspace, first_at, last_at,
                     visits, dwell_ms, edits, yanks, meta, text)
VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
ON CONFLICT (kind, subject, workspace) DO UPDATE SET
    last_at  = max(last_at, excluded.last_at),
    visits   = visits + excluded.visits,
    dwell_ms = dwell_ms + excluded.dwell_ms,
    edits    = edits + excluded.edits,
    yanks    = yanks + excluded.yanks,
    meta     = excluded.meta,
    text     = coalesce(excluded.text, text);
```

Increments commute, so two windows on one `state.db` — the case the
histories never had to face, since a buffer is open in one — add their
halves and neither loses the other's. What is *set* rather than added
(`meta`, `pinned`) is user intent or the latest caret, where last
writer wins is the right answer. The RAM cache is never written back;
after a flush the cache rows are re-read for the subjects flushed, so
what another window added shows up. A flush that cannot take the write
lock (`busy_timeout` of 250 ms, then `SQLITE_BUSY`) keeps its deltas
for the next tick; nothing is dropped and nothing is doubled, because
a delta is cleared only after its upsert returned. The ring's pending
rows go in the same transaction, followed by the delete of whatever
is past `RECENT_MAX`. WAL makes each flush whole or nothing, as it does
a history's row.

Cadence: the histories' `QUIET` (one second still) and `LAG` (ten
seconds while active), one timer for both since the memory's flush and
a draft's write happen for the same reason. A `text` moment is flushed
at once — a yank is one row and one row is microseconds — so the
register is on disk before the next key, and a crash between keeps it.
The session save flushes, as it flushes drafts.

*The alternative*: RAM as the truth, written whole at quit and on a
timer, as `Editor::memory` is now. It is what "clobber" means: the
second window's save erases the first's, and a crash loses the day.

### 4. Limits on three axes per kind, and the whole store on one

A memory that grows is a liability; one that forgets the wrong thing
is useless. Limits are per kind, because a text row weighs kilobytes
and a file row weighs its path, and because a search pattern goes
stale in a week where a file matters for a year:

| kind       | rows      | bytes             | keep (days) |
|------------|-----------|-------------------|-------------|
| `text`     | 100       | `memory.text.max_mb` for all of them (8; 0 keeps none); a text over 1 MiB is RAM only for the session | `memory.text.keep_days` (7) |
| `file`     | 5000      | —                 | 90          |
| `scratch`  | with its history | —          | with its history |
| `command`  | 500       | 4 KiB             | 90          |
| `search`   | 200       | 4 KiB             | 90          |
| `tool`, `location` | 500 | 4 KiB          | 30          |
| a plugin's | 500       | 4 KiB             | 90          |
| the ring   | 1000      | —                 | —           |

Settings: `memory.keep_days` (90; 0 never), `memory.text.keep_days`
(7; 0 never), `memory.text.max_mb` (8; 0 writes no text at all, for a
machine where a yanked secret must not touch the disk — the register
still works for the session), `memory.max_mb` (64; 0 none),
`memory.idle_secs` (60, Decision 7). Texts get their own days and size
because they are the one kind whose rows weigh something and whose
value is gone in a week; the two settings are the user's dial between
"my yanks survive a restart" and "nothing I copied is on disk". The
rows-per-kind caps and `RECENT_MAX` are constants, as `MEMORY_MAX` is,
until someone needs them otherwise. `history.keep_days` and
`history.max_mb` go; Decision 6 says why they are the same numbers.

**Eviction is by score, and there are holds.** Past a kind's rows, or
past `memory.max_mb` for the store as a whole (the histories' blobs
counted, Decision 6), rows go lowest score first, computed in Rust
over the kind's rows after a flush, as `history.rs` walks its rows
today:

```
score = (visits + 3·edits + 2·yanks + min(dwell_min, 60)) · 0.5^(days_since_last / 7)
```

A half-life of a week: a file with a hundred visits last month scores
what one with six scores today, and either outlasts the two thousand
files `]q` walked once this morning, each with one visit and a
second's dwell. The score is the engine's, fixed, and is *not* the
picker's ranking (Decision 8) — eviction must not change under a
user's Lua, and it must not need Lua to run at all. A row is never
evicted while it is **held**: `pinned` (Decision 5); its subject open
in a buffer; a history with unsaved text hanging off it (Decision 6);
the `"` register's text. Aging by `keep_days` runs at the first frame
of a launch, as the histories' does, and respects the same holds.

Bytes: a `text` over 1 MiB is remembered for the session and not
written, with the pane saying so in its row, as a draft past
`MAX_TEXT` has no row and a corner line says so. Past
`memory.text.max_mb` for the text rows together, the lowest-scored
unheld text goes, as any kind's rows do past `memory.max_mb`. A
`command` line past 4 KiB is not remembered at all.

### 5. Pinned is a flag on a moment, and harpoon is the memory made explicit

The roadmap's pinned files (`<leader>e`, `<leader>e1`…`9`, `<A-1>`…`9`)
were to be a `kawoosh.store` table per workspace. They are a `file`
moment with `pinned = 1`: exempt from eviction, ranked first by the
picker, listed by `:memory pins` in pin order (`meta.pin` an ordinal),
and set from the pane with `m` on the cursor's row, from a buffer with
`memory pin` (`<leader>ea`), and from Lua (`kawoosh.pin`). Jenson's
word for the clipboard cache was *editable*; a pin is the user editing
the memory's ranking by hand, and a `x` in the pane is the other half.
Nothing to store elsewhere, nothing to keep in sync.

### 6. The histories keep their table and lose their bookkeeping

The question was whether the histories retire into the memory too.
The blob does not; the rest does.

A history is a buffer's undo tree and draft: state, mutable, up to
megabytes, one per buffer, rebuilt whole from its text. That is not a
moment, and folding blobs into a table of small keyed rows would make
every cap read wrong (a hundred texts and sixty-four megabytes are not
the same kind of limit). `histories` stays, keyed as it is
(`file:<path>`, `scratch:<n>`), holding `text`, `meta` (the tree), the
hash and `clean`.

What retires is everything the histories do *around* the blob, which
is a memory of files in disguise:

- **`touched` and aging.** `touch_history` writes what `touch_oldfile`
  writes: this file was attended now. Both become the `file` (or
  `scratch`) moment's `last_at`, and `saved_at` goes back to meaning
  only when the row was written. `history.keep_days` and
  `history.max_mb` are `memory.keep_days` and `memory.max_mb`: **a
  history lives exactly as long as its subject's moment**, and when a
  moment is evicted or aged out, its history goes with it (one `DELETE`
  by key in the same transaction). The hold runs the other way too: a
  history with unsaved text holds its moment (Decision 4), which is
  today's "a row held by a buffer with unsaved changes never goes",
  stated once.
- **The pane and the commands.** `:history` (the pane of every row
  with size, touched, held, present, and the cursor's row as a diff
  against the disk) becomes the `:memory files` view: a file row shows
  whether a draft hangs off it and how big, and the diff is the row's
  detail, as a text row's lines are. `:history list`, `:history drop
  KEY`, `:history clear[!]` become `:memory files`, `:memory forget`,
  `:memory clear[!]` (the clear vacuums). `history.rs` keeps the
  write-and-restore machinery (`save_history`, `load_history`,
  `install_history`, the drafts' cadence and caps); `history_pane.rs`
  goes, its tests moving to the memory pane's.
- **Scratch identity.** A scratch buffer's number is the history's key
  and becomes the `scratch` moment's subject; `seed_scratch` seeds the
  memory rows as it seeds the histories'. Scratches are session-bound
  today and stay so: a `scratch` row is held while the session holds
  the buffer and goes with its history when the buffer is discarded.

Migration at init: every `histories` row gets a `file` or `scratch`
moment with `last_at` from its `saved_at`, if it has none yet; the
`histories.saved_at` column keeps its meaning. `history.*` settings
are read once as the memory's defaults if `memory.*` are unset, so a
tuned `settings.lua` does not lose its numbers silently; the settings
pane's listing names the new keys.

*The alternative*: leave the histories whole and let the memory
duplicate their touch. That is where the code is today, and two rows
recording one fact is how the histories and oldfiles came to disagree
about what "recent" means.

### 7. Dwell is counted while the pane has the keyboard, not while the window is open

The one weak signal that distinguishes the file you worked in from the
one you glanced at. The shell knows which pane has the keyboard
(`Layout::focused`) and whether the window does (kui's focus). Dwell
accrues to the focused view's subject per frame while both hold, and
stops after `memory.idle_secs` without a key or a click (as the
histories' `QUIET` stops a draft's writes), so a lunch break does not
give one file an hour. It is capped at sixty minutes per visit in the
score (Decision 4) so a file left open overnight does not become the
most important thing ever seen. A terminal pane's dwell goes to its
`tool` subject when it has one and nowhere otherwise.

Edits: one per undo checkpoint (`Editor::run`'s boundary), attributed
to the view's buffer. Yanks: one per `text` moment taken from a
buffer, attributed to the origin's buffer. Visits: one when a view's
buffer changes or a pane is focused onto a buffer it was not showing,
and each visit is one ring row (Decision 1); a visit that comes within
a second of the last to the same subject — a `]q` walk landing twice
in one file — is one visit and one ring row.
`location` moments come from `]q`/`[q` and `<CR>` on a location
(compile, grep, diagnostics — D5c's table), `tool` from `:tool` and
`kawoosh.compile`.

### 8. Two scores: the engine's for eviction, the user's for ranking

Decision 4's score is fixed, in Rust. The picker's ranking is Lua's,
in a bundled `memory.lua` that ships `memory.rank(row, now)` with the
same formula as its default and is replaceable, since which file
should come first for *you* is exactly what "hackable by design" says
a user rewrites. The split keeps the fast path fast: `kawoosh.fuzzy`
scores forty thousand paths in Rust, and the memory's boost reranks
the top two hundred in Lua, where the rows are a table lookup. A
pinned row ranks above any boost. `:e`'s path completion and the
`<Up>` walk use `last_at` only; frecency is for lists, not for the
next key.

### 9. Lua: read everything, add signals, remember under your own name

`kawoosh.memory()` keeps its meaning — the texts, newest first, the
fields it has (`text`, `linewise`, `took`, `from`, `buffer` while
open, `age`) — and takes a query:

```lua
kawoosh.memory { kind = "file", workspace = true, limit = 50 }
kawoosh.memory { kind = "command", since = 3600 }
kawoosh.memory { kind = "file", subject = path }        -- one row or nil
kawoosh.memory { recent = true, limit = 100 }           -- the ring, newest first: at, kind, subject, workspace
-- a row: kind, subject, workspace, first, last, age, visits, dwell,
-- edits, yanks, pinned, meta (a table), text (text kind), buffer
-- (while open)
kawoosh.remember { kind = "file", subject = path, signals = { edits = 1 } }
kawoosh.remember { kind = "dir.rename", subject = path, meta = { from = old } }
kawoosh.pin(kind, subject, true)
kawoosh.forget(kind, subject)
kawoosh.recall(i)                       -- texts, as today
```

`remember` on an engine kind attaches signals to the subject — the
`dir` plugin marking a file it renamed, compile marking the file a
failing test lives in — and never creates a `text` (the register is
the engine's). A plugin's own kind is `<plugin>.<kind>`, capped as
Decision 4's last row, and is not a database: a plugin that wants a
table wants `kawoosh.store(ns)`, which exists. The memory is what
plugins *share* — the same rows the picker ranks by — and the
namespace rule is what keeps one plugin's noise out of another's
ranking. Every write goes through the delta map (Decision 3), so a
plugin cannot clobber either.

### 10. The pane: `:memory`, with kinds

`:memory` (`<leader>p`) opens as it does, on texts. `:memory files`,
`:memory commands`, `:memory searches`, `:memory pins`, `:memory all`
filter the subject rows; `:memory recent` is the ring, newest at the
top, one row per transition with the time it happened, so a morning
reads as a list — a subject that appears five times appears five
times. A key in the pane cycles (`<Tab>`). The rows are what they are
now — newest at the top with how it came, from where and when — plus
the kind's signals in the annotation column (`12 visits · 40 min · 3
edits`, `draft 4 KiB`), and `pinned` drawn as the accent. The keys
stay: `⏎` puts a text or opens a file at its line, `y` recalls, `o`
goes to the origin (a text's, carried through the edits since; a
file's line; a location), `x` forgets, `m` pins, `q` closes. The
cursor's row's detail is a text's lines, a file's draft diff against
the disk (the `:history` pane's), a command's line. `<leader>sr`
(resume) is `:memory recent` with the cursor on the first row: the
last thing attended, then the one before it.

### Deliberately not

- **Not a log, not telemetry.** Nothing leaves the machine; nothing
  counts keystrokes; the pane shows every row there is and `x` and
  `clear` are real deletes with a vacuum. `memory.text.max_mb = 0`
  exists for the case where a yanked secret must not touch the disk at
  all, and `memory.text.keep_days` for the case where a week is too
  long.
- **Not a model.** No summarization, no suggestion engine; the memory
  is data and the ranking is a formula the user can read and rewrite.
- **Not `.` and macros.** Roadmap step 3 records a command stream to
  replay; it is not a moment and does not persist.
- **Not co-occurrence, yet.** "Files that were open beside this one"
  is a real suggestion source no picker has, and the session layouts
  in the store already hold the data. A round after the picker, with
  its own note, once there is a memory to attach it to.
- **Not a yank-pop, yet.** `<C-p>`/`<C-n>` after `p` cycling the text
  put is a pane-less recall and rides on the same rows; the polish
  round after this one. *Built 2026-09-24 on `[p` `]p` (round five,
  above).*

## Build order

Each a round: one commit with its tests, a paragraph in roadmap.md
struck through when it lands.

1. **The table, the ring, and files.** `moments` and `recent` in the
   store with the migration from `oldfiles`; the delta map, the
   pending ring rows and the flush on the histories' timer; `file` and
   `scratch` rows with visits, edits, yanks, dwell and the idle guard;
   eviction by score with the holds, the ring trimmed with it;
   `:memory files` and `:memory recent` in the pane with `o` and `x`,
   `<leader>sr` on the ring; `:oldfiles` retired.
   Histories' `touched` and aging moved onto the moment, `:history`'s
   pane and commands folded in, `history.*` settings renamed with the
   fallback. Tests: two windows on one db each adding a visit and both
   counted; a flush that meets `SQLITE_BUSY` keeping its delta; a burst
   of two thousand single-visit files not evicting a daily one; a
   draft's moment held past the cap; a history going with its evicted
   moment; dwell stopping at idle; the ring at its cap dropping the
   oldest and a forgotten subject's rows going with it; the migration
   of an `oldfiles` row and of a `histories` row.
2. **Texts and the prompts.** `text` rows persisted under
   `memory.text.max_mb` and `memory.text.keep_days`; `Editor::memory`
   reduced to headers and the head, the pane reading a text's bytes on
   demand; the register read back at launch; `command` and `search`
   rows behind `<Up>`; the existing memory tests kept green against
   the store. Tests: a yank on disk before the next key; a text over
   1 MiB remembered for the session only; the text rows trimmed past
   `max_mb` lowest score first with the register held; `max_mb = 0`
   writing nothing and the register still putting; a text aged out at
   `keep_days`; the RAM footprint after a hundred large yanks (the perf
   tab's number) staying at one text's; `<Up>` after a restart.
3. **Pins and the picker's boost.** `pinned`, `m`, `<leader>e`, the
   ordinal; `memory.lua` with `rank`; the picker (roadmap step 4)
   reranking its top rows by it, files, buffers and commands. Tests:
   a pin outranking any score; a user's `rank` replacing the default.
4. **Runs.** `tool` and `location` rows from `:tool`, compile and the
   location walks; a terminal pane's dwell.
5. **The notes after**: co-occurrence from sessions; the yank-pop.

## Risks

- **Two processes, one db.** The store is one connection on the main
  thread with no busy handling today, because nothing needed it.
  Decision 3's retry is the whole answer, but it is new ground and
  the test with two `Kawoosh` instances on one `state.db` is the one
  to write first.
- **Migration on a live db.** The `oldfiles` drop and the histories'
  moments are one `init`, tolerant as the `drafts` rename is; a db
  from before must open, and one from after must open in a build from
  before (it will: the new table is ignored, `oldfiles` comes back
  empty). The settings fallback is the part to get right so a tuned
  `history.max_mb` does not silently become 64.
- **Dwell is a guess.** Focus is a proxy for attention, and a window
  left focused with someone reading is dwell that the idle guard does
  not catch. The score caps it; the pane shows it; nothing depends on
  it being exact.
- **The ranking's feel.** A half-life and four weights are a guess
  until the picker exists to feel them through. They are constants in
  one place and a Lua function in the other, which is what a guess
  should be.
