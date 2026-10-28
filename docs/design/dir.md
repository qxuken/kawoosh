# The file manager, after the listing

Status: written 2026-10-14 from the todo's "dir improvements: chmod,
chown, etc. remember sorting" and the ask "add `:du here`". The listing
itself — a directory as a buffer, edits planned by `:w` — is
[mvp.md](mvp.md) Decision 5b and the long comment heading
`kawoosh/lua/dir.lua`; this note holds what came after it. The calls
below are taken here, each the user's to overturn. Built the same day
(`kawoosh/lua/dir.lua`, `doc/src/perms.rs`, `systems/src/fs.rs`;
`a_listing_sets_modes_and_owners_on_its_entries` in
`kawoosh/tests/lua.rs`).

## Decisions

### 1. Modes and owners are commands on the entries under the carets

`:dir chmod MODE` and `:dir chown OWNER[:GROUP]` act on the entry
under each caret, or on every line a visual selection covers — the
user's choice of commands over a permissions column the write would
plan ("it will give flexibility"): several carets, `V` over a block,
`.` and macros make them bulk operations the way the listing's edits
are, with no column to keep in step with the disk.

- **The entry, not the text.** A line acts as the entry it tracks
  (`kawoosh.buf.tracked_lines`, the identity the plan reads): a line
  renamed and not written yet is still its file on disk, and a line
  pasted from another listing is that listing's entry. `../` is no
  entry; a line typed in has nothing on disk yet and is skipped, said
  in the answer ("not on disk until :w").
- **MODE** is `chmod(1)`'s: octal (`755`, `4755`) or symbolic clauses
  (`u+x`, `go-w`, `a=r`, `u=rwx,g=rx,o=`, `g=u`, `+X`, `+s`, `+t`).
  One difference, on purpose: a clause with no who is `a`'s and the
  umask is not asked — `+w` gives everyone write, as it reads. The
  umask is this process's and says nothing of a host the path may be
  on; a mode that meant one thing here and another there is worse than
  one that ignores a setting few remember they have. Parsed once in
  Rust (`kawoosh_doc::perms::ModeChange`), so every disk reads it alike;
  a mode that does not parse changes nothing and says why.
- **OWNER** is `chown(1)`'s: `OWNER`, `OWNER:GROUP`, `:GROUP`,
  `OWNER:` (the owner's login group), each a name or a number. There
  is no `chgrp`: `:GROUP` is it.
- **Links are followed**, as both commands follow a path given to
  them.
- **`!` recurses, and nothing else does.** `:dir chmod! go-rwx` changes
  everything under a directory too; a link met there is left alone,
  neither followed nor changed (`chmod -R`'s way), so a tree's change
  never reaches out of the tree. Never by default: a mode for a tree is
  a mode for files that were never on screen, an octal mode makes
  every file under a directory executable, and a recursive change has
  no undo. A directory is changed before it is read, so `u+rx` opens
  one it could not. The bang is how a `:dir` command asks for its
  stronger form already (`:dir!`, `:dir refresh!`).
- **One failure is one entry's.** Each path is tried; one refused
  (`EPERM` on a chown without the privilege, a host without owners)
  is in `:messages` and the first in an error toast — `1 of 3
  failed: …`, the path named — and the rest are changed all the same.
  Success is a corner line under `dir`, as a write's count is: one
  entry's new standing (`a.sh: rwxr-xr-x (755)`, `a.sh: me:staff`), or
  `chmod u+x: 12 changed`.
- **Off the frame.** The changes run on the io thread
  (`kawoosh.fs.chmod(paths, mode, { recursive }, fn)`, `fs.chown`),
  a tree of any size holding up nothing; the listings of the
  directories touched have version control's word asked again (`+x` is
  a change to git).

**Hosts.** Every disk that has modes and owners takes them; none that
lacks them pretends:

| where | mode | owner |
|---|---|---|
| this machine (unix) | read, changed, set | by name or number, looked up here (`getpwnam_r`) |
| a host over SFTP | `STAT`'s bits changed, set by `SETSTAT` | by number only: SETSTAT takes a uid and a gid and SFTP v3 has no word for a name, so a name is refused with "give OWNER:GROUP by number"; the half not given is the file's own |
| a host through its shell (`ShellFs`) | the host's `chmod`, the change spelled with its who named so the host's umask does not come in | the host's `chown`, its names the host's |
| a WSL distro | the distro's `chmod` | the distro's `chown` |
| Windows | refused: "modes and owners are unix's" | refused |

*The alternative*: a permissions column in the listing, edited and
planned by `:w` like a name. It makes every line wider for a change
made rarely, it needs the column read for every entry of every listing
(a stat a line, on a host a round trip), and a symbolic change (`+x`
over fifty files of different modes) is no edit of a column at all.

**What the listing shows.** The preview (`<C-p>`) says the entry's
mode as `ls -l` draws it and its owner (`rwxr-xr-x  me:staff`); the
notes past the names stay the size and the time. A host's shell says
no mode, so its preview says none.

**Keys.** None bound: no key in a listing is both free and obvious.
Proposed: `=` (ranger's, which opens `:chmod `) for a prompt on `:dir
chmod ` — it shadows the indent operator, meaningless in a listing,
and wants a way for a key to open the prompt with text in it, which
kawoosh has not.

**Left.** A mode column as a setting (`dir.columns`), for a listing
that is about permissions; a host's shell saying the mode (`ls -ln` in
its stat); a name for SFTP's numbers through the domain's transport
(`id -u NAME`); `touch` and links (`ln -s`) are other commands.

### 2. The sort is the directory's, else the one chosen last, and it outlasts a restart

The decided behaviour: a directory's order (`ma` `ms` `mm` `me`, the
capitals reversed; `:dir sort KEY[!]`) is remembered **per
directory**, and a directory with no order of its own is listed in
**the order chosen last anywhere** — by name until one is chosen. Both
persist across restarts.

- **Precedence.** The directory's own order, else the last chosen,
  else by name. Choosing an order makes it the directory's and the last
  chosen at once; a directory that has its own keeps it whatever is
  chosen elsewhere. The last chosen reaches a listing when it is next
  read (opened, `<C-l>`, its watch); a listing on show elsewhere is not
  read again for it.
- **Where: the plugin's store**, `kawoosh.store("dir")` in `state.db`
  — the key `sort:<dir>` per directory and `sort` for the last chosen,
  each spelled `size` or `size!` for the reverse. Read once a session
  (`dir.sorts[d]`, `false` for none; `dir.last_sort`), written when an
  order is chosen; a listing asks the disk nothing to be sorted.
- **Not the memory.** memory.md's rows are what was attended, with
  signals, ranked, aged out past 90 days and shown in `:memory`; an
  order is the user's word about a directory, like a setting, and
  memory.md Decision 9 says it outright: a plugin that wants a table
  wants `kawoosh.store`. Nor the session: a session is a window's
  layout, and an order chosen for a directory holds in any window.
- **Bounded by the asking.** A row is written only for a directory an
  order was chosen in, so the store grows by a few short rows a
  directory the user sorted by hand; nothing ages them out.
- **Cost.** Sorting is as it was: each entry's key taken once, then
  `table.sort`; 16k entries re-sorted by `mm` `ma` `mM` `mA` in 96–104
  ms a key in the dev build (reading the directory, filling the buffer,
  the notes, a frame), the same before and after, with a state db
  attached or not.

*The alternative*: a `dir.sort` setting for the default and the
per-directory orders in the session. A setting is the user's file to
edit, not what a key chose a minute ago, and the session would forget
an order in every other window.

**Left.** A way to forget a directory's order (it is replaced, never
dropped), if one is missed.

### 3. `:du here` reads the same "here" as `<C-w>.` and `<C-w>/`

`:du here` (`<C-w>u`) opens the disk-usage pane on the directory in
front: a file's directory, the directory a listing lists, a terminal's
shell's, the directory a `:du` pane is on — `Kawoosh::here_dir`, the
one resolution `:terminal here` and `:dir here` read, so the three
cannot disagree. Like `:dir here` it is an engine command beside the
Lua one (`terminals.rs`): it asks `here_dir` and runs `:du PATH`, whose
pane opens or is taken as `:du` has it. Not `picker.here()`, which
knows neither a terminal's directory nor a Lua pane's.

- From a `:du` pane it walks the directory the pane is in, as the new
  root: the way to look at a subtree alone.
- A directory literally named `here` is `:du ./here`, as with `:dir`.
- **The key**: `<C-w>u`, `u` for usage as `<leader>wu` is the working
  directory's — the "here" sibling of a key that exists. `.` and `/`
  are taken by the terminal and the listing; `<C-w>u` was free (vim
  has no `CTRL-W u`).

### 4. "Here" that is gone is its closest parent that is there

*Added 2026-10-16, from the todo:* "if `:dir here` buffer dir does not
exist (anymore, or not yet) let's notify user but open closest
existing parent." A file whose directory was deleted under it, or a
new file (`:e new/dir/f.txt`) whose directories are not made yet, gave
`<C-w>/` a listing of nothing, and `<C-w>.` a shell that failed to
start in it.

`Kawoosh::here_dir_there` climbs from `here_dir` (Decision 3) to the
closest directory that is there and says so in a note — `~/a/b is not
there: ~/a instead`. All three `here` commands — `:dir here`, `:du
here`, `:terminal here` — and a tool's terminal with no `cwd` read it,
so they still cannot disagree. A host's path is taken as it is:
finding out would wait on the host on the UI thread, and its listing
says what it found.

`a_directory_here_that_is_gone_lists_its_closest_parent` in
`kawoosh/tests/lua.rs`.

### 5. `gX` shows the entry in the OS's file manager

*Added 2026-10-24, asked:* "add open finder/explorer command to dir."
`:dir reveal`, or `gX` in a listing, hands the entry under the caret
to the OS's file manager, selected among the listed directory's
entries; on `../`, or in an empty listing, the listed directory is
opened as itself. The engine does the asking (`kawoosh.reveal(path,
{ inside = })`, `kawoosh/src/reveal.rs`), so a plugin pane can show
its file the same way.

- **How each OS is asked**, a program and its arguments, never a
  shell, as `gx` opens a URL: macOS `open -R PATH`, and `open -a
  Finder DIR` inside (a bundle is browsed, not run); Windows
  `explorer.exe /select,"PATH"`, written raw since Explorer reads its
  own command line and std would quote the whole argument; Linux the
  freedesktop `org.freedesktop.FileManager1` `ShowItems` /
  `ShowFolders` call through `dbus-send`, which Nautilus, Dolphin,
  Thunar and Nemo answer, and `xdg-open` of the directory — nothing
  selected — when no file manager answers it.
- **What is refused**, on the message line: a host's path (no file
  manager here can show it) and a path that is not there — a line
  typed in or renamed and not written yet.
- **The key**: `gX`, beside `gx`, which hands a link to the OS and
  stays that in a listing. `gO` was free too, but is the outline in
  neovim.

`a_listing_shows_its_entry_in_the_file_manager` in
`kawoosh/tests/lua.rs`, the host's refusal in
`a_hosts_file_is_opened_written_and_listed_through_its_domain`
(`kawoosh/tests/domains.rs`), the arguments per OS in
`reveal::tests`.

*Amended 2026-10-27*: the asking is kui's, `show_in_folder` for an
entry and `open_external` for a directory's own entries (kui F203, and
F222, which brought them to what `reveal.rs` did: the Linux selection
over `FileManager1.ShowItems`, a folder in Finder on macOS, every tool
started through the host's spawner — kawoosh's spawn lock, set in
`App::setup`). `reveal.rs` is gone, and with it Linux's `ShowFolders`
for a directory's own entries: `xdg-open` on the directory does the
same. `gx`'s URLs go through `open_external` too (`links::open_in_os`),
Windows' through `explorer "URL"` where kawoosh called `ShellExecuteW`.
The arguments per OS are kui's `system::tests`.
