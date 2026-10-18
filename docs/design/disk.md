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

### 3. A sync the file system does not keep does not fail the save

Asked 2026-10-14: two files on a NAS (`/Volumes/appdata`,
`/Volumes/Media`, both `smbfs`) would not save — `Operation not
supported (os error 45)` — where vim wrote them. A regression of
2026-09-16 (b09e757): the save through a sibling renamed over came
with `sync_all()` on the sibling, before it a plain `std::fs::write`
that asked for no sync. On macOS the standard library's `sync_all`
(and `sync_data`) is `fcntl(F_FULLFSYNC)`, and smbfs refuses it:
measured read-only on both mounts, `F_FULLFSYNC` → `ENOTSUP` (45),
`fsync` → 0. vim's `vim_fsync` takes `F_FULLFSYNC` and falls back to
`fsync` when it fails.

A sync is a promise about bytes already written, not the write, so
it steps down as far as the file system goes (`kawoosh_doc::durable`):
`F_FULLFSYNC`, then `fsync`, then none, each step taken only when the
last answered *not supported* (`ENOTSUP`, `EOPNOTSUPP`, `EINVAL`,
`ENOSYS`). A sync that was tried and failed — `EIO`, a full disk, a
quota, which NFS may report only at the sync — still fails the save:
the bytes may not be there, and the rename over the file is not made.
Copying the file's mode onto the sibling is held to the same rule: a
mode the file system cannot hold does not stop the save. The bytes
pane's write in place (`kawoosh_systems::fs::write_runs`) syncs the
same way.

Nothing is said: what a save gets is what the mount gives every
program writing there, and the step down would be said again at every
`:w` (quiet-eviction's rule). *Beat:* the step down in the log — a
line a save on that mount would add each time, and nothing to do
about it.

### 4. A save the rename cannot make is written in place

Asked 2026-10-16: `/Volumes/appdata/prometheus/etc/prometheus.yml`
(the same `smbfs` NAS as Decision 3) would not save — `Permission
denied (os error 13)` — where neovim wrote it without a word. Measured
by its metadata alone (the file holds tokens): the file is
`-rwx------` this user's, `access(W_OK)` yes; its directory is
`drwx------` this user's too, yet `access(W_OK)` no — the share's ACL,
which the server answers for, lets the file be written and nothing be
made in the directory. The save's sibling (`.prometheus.yml.kawoosh~`)
is a new name there: refused. vim with `backupcopy=auto` copies the
file aside when it can and writes the file itself either way, so it
never needed one.

The rename is kept as the way: a failed write leaves the old file, and
a big file's buffer reads its mapping, which a write in place would
change under it. It steps down to a write in place
(`kawoosh_doc::durable::write_in_place`) when:

- the sibling or the rename is refused for permission (`EACCES`,
  `EPERM`) and the file is there — the NAS's case, and a directory a
  user may not write in holding a file they may;
- the rename would not keep the file as it was
  (`durable::kept_in_place`): one owned by someone else would become
  this user's, one with hard links would leave them the old text. vim's
  `backupcopy=auto` asks the same two. On Windows (std's link count is
  unstable) both are read through a handle: `nNumberOfLinks`, and an
  owner other than the token's default owner — what a file this
  process makes is owned by. An elevated process's is Administrators,
  so there the user's own files are written in place too; a share
  whose owners this machine does not know reads every file as
  another's. A file whose permissions may not be read is another's.

In place is the file opened for writing as it stands (never made, so a
name that is not there stays the caller's error), the text written
over from its start, the file cut to what was written, and synced as
Decision 3 does — never empty between, and its mode, owner, links and
extended attributes untouched. What it gives up is the rename's
atomicity: a write that fails part way leaves the new bytes over the
start of the old. The buffer still has the text whole, and `:w` again
writes it; vim takes the same risk where it cannot back up. A buffer
read from the file's mapping (`Buffer::mapped`, a file past 64 MB
opened on the io thread) is never written in place: its text and its
undo states would read the bytes being written — that save fails with
the reason, and `:w PATH` writes elsewhere.

*Beat:* a backup copy elsewhere first (vim's `backupdir`, `~/tmp`) —
the buffer is the copy that matters, and a second copy of a file of
tokens in the home is a leak of its own. *Beat:* in place always — the
rename's guarantee is worth keeping wherever it can be had.

### 5. A file the user may not write is offered as an administrator

When the file itself refuses the write too — `0400`, root's, a share's
ACL saying no — the save fails as permission and `:w` puts up a
confirm (`Effect::WriteRefused`): *Write as administrator* or *Cancel*.
Never done unasked: a password dialog that appears on a save is a
surprise, and an elevated write is a decision about that file, not a
setting. Answered, `:file admin PATH` runs `save.as_admin` on a
thread, the frame not waiting on the dialog:

- the program and its words are data, `{text}` a file holding the
  text and `{file}` the file. macOS's default is `osascript` with
  `do shell script "/bin/cat " & quoted form of item 1 of argv & " > "
  & … with administrator privileges`, the paths handed in as `argv` so
  nothing is spliced into the script; Linux's is `pkexec /bin/sh -c 'cat
  "$1" > "$2"'`, polkit's dialog; Windows has none, and nothing is
  offered. An empty list switches the offer off.
- `cat >` writes the file where it stands, as Decision 4 does: its
  owner and mode are its own. A file not there yet is made root's.
- the text goes through a file in a directory made `0700` under the
  temp directory, the file `0600`, removed when the program is done;
  the copy in memory is zeroed (a private buffer's text, secrets.md
  Decision 6). The file system may keep its blocks — the same promise
  as the file being written.
- the answer saves the buffer if it is still the version written, and
  takes the file's stamp either way, so the write is not asked about
  as a change on disk. A refusal — a cancelled dialog says `User
  canceled.` — is a warning with what the program said.

Not for a host's file (its domain has its own permissions, and an
administrator here is not one there), nor for a mapped buffer, nor
from `:wa`, which lists what it could not write as before.

*Beat:* vim's `:w !sudo tee %` — `sudo` wants a terminal to ask on,
and kawoosh's spawns have none (secrets.md, a session of their own);
`sudo -A` needs an askpass program the user would have to have.
*Beat:* the text on the program's stdin — `do shell script` gives its
command none of osascript's.

## Not built

- **Following the tail in the view**: a buffer followed keeps its
  scroll; a `tail -f` that keeps the end in sight when the caret is
  on the last line is a view's choice, not a buffer's, and is not
  made. `G` after a follow lands on the new end.
- **A modified buffer whose file keeps growing** is asked again at
  every change — one toast at a time, each replacing the last — as
  roadmap step 12 built it; a log a user edits while it is written to
  has not come up.
- **A Windows file's own ACL through a rename** (Decision 4): the
  renamed sibling has the entries its directory hands down, so entries
  given to the file itself, or carried from a directory it was moved
  out of, are lost, and so are its alternate data streams and its
  hidden and system attributes (the read-only one is kept, as the mode
  is). Copying them onto the sibling first (what `ReplaceFileW` does)
  has not come up.
