# A buffer against its file

The files under the buffers: roadmap step 12 built the watch, the
reload and the questions (2026-09-22); this note holds what came
after. `editor/src/disk.rs`, `kawoosh/src/disk.rs`, `systems/src/watch.rs`.

## What there is

Every open file is on a watch of its own, a thread that stats the set
twice a second. A moved stamp (length, mtime) is the cheap question
and the text the real one: a stamp that moved over the same bytes (a
`touch`, a checkout of what was there) is taken as the new stamp and
nothing is said. When the text differs, a clean buffer takes the
disk's text as one undo node and a corner line says so; a modified
one is asked once per change with a toast that stays — *Reload*,
*Keep mine*, *Diff*; a deleted file is said and the buffer keeps its
text. `:w` over a changed file asks, `:w!` writes over, `:file`
(`:checktime`) asks about every buffer now.

## Decisions

### 1. A file that grew is followed, not reloaded

Asked 2026-10-08: two Kafka consumers in `:!` panes wrote megabytes
into two log files that were open in buffers, and kawoosh lagged,
then froze for seconds on a tab switch; the consumers killed, it
recovered. Measured with `perf.rs`'s `tail_cost` (a 20 MB log, 64 KB
appended, ten times): the frame after each change was 183 ms, and
the process grew by 20 MB a change. Three things, each on its own
enough:

- the reload read the file into a fresh tree and `restore`d it. The
  journal's edit was the tail alone (`diff_trees` found the common
  prefix), but the undo node kept the old root, over its own 20 MB
  block, and a fresh block came with every reload — 1000 nodes deep.
  Two files, two changes a second: 80 MB a second kept, until the
  machine swapped. A hidden tab hid it, since nothing drew; coming
  back paged it all in;
- the question read the file whole too, a vector never the same size
  twice, so the allocator never reused one: the same 20 MB a change,
  kept;
- `conflicts_in` walked every line of the buffer at every version
  (vcs.md Decision 11), 150 ms for 150,000 lines, from the pane's
  draw — and 150 ms of a 20 MB file's first frame.

Now a file is read against the text through a 1 MB window
(`file_against`): the bytes up to the text's length compared piece
by piece, no tree built, and only what lies past them read. A file
that is the text and then more — a log written to — is **followed**:
the tail goes in as an append (`Buffer::replace` at the end), so the
text before it is the same pieces still, the layers over it stay, and
the undo node shares all of it. A file that differs within the text,
or shrank (a log rotated), is reloaded whole as before. A file that
is not UTF-8 falls back to the whole read, repaired, to compare. The
conflict scan is a byte search for `<<<<<<<` at a line start.

After: 2 ms the question, 2 ms the follow, 1.5 ms the frame; the
footprint flat; the open 25 ms where it was 186.

Beaten: reading the file on the io thread. The window keeps the
frame's share at the compare's cost — 20 MB at memory speed, a few
milliseconds — and the follow needs the text under it unchanged
meanwhile, which a frame has and a thread would have to check again.
Past `COMPARE_MAX` (64 MB) the question is still answered from the
stamp alone, and the reload's own compare is the one read.

### 2. A follow soon after the last is the log's

The corner line a reload earns, twice a second for two files, held
the corner for as long as the consumers ran. A follow within
`FOLLOW_QUIET` (10 s) of the buffer's last is logged and not shown
(`Show::Log`); the first, and the first after a quiet spell, says
`consumer.log: grew by 64.0 KB on disk, followed (u takes it back)`
in the corner. A whole reload says `reloaded, changed on disk` as it
did. (quiet-eviction's rule: housekeeping goes to the log and its
pane, not the corner.)

## Not built

- **Following the tail in the view**: a buffer followed keeps its
  scroll; a `tail -f` that keeps the end in sight when the caret is
  on the last line is a view's choice, not a buffer's, and is not
  made. `G` after a follow lands on the new end.
- **A modified buffer whose file keeps growing** is asked again at
  every change — one toast at a time, each replacing the last — as
  roadmap step 12 built it; a log a user edits while it is written to
  has not come up.
