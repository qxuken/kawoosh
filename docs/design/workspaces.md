# Workspaces: which directory is "the" directory

Status: decided and built 2026-09-24 (roadmap step 26), in one round.
Asked 2026-09-23 as "the cwd per tab, or something else"; the
decisions below are the round's, each with the alternative it beat,
and the user's to overturn. Companion to [domains.md](domains.md),
whose rule 7 ("a process spawns where its cwd is") is the same change
seen from ssh, and to [memory.md](memory.md), whose workspace this
note redefines.

## What the cwd was

One thing that meant five, checked in the code on 2026-09-23:

1. **The process's own.** `set_cwd` called `set_current_dir`, so every
   child and every relative path agreed by inheritance.
2. **The project.** The project settings layer (`.kawoosh/settings.lua`
   above the cwd) and the trusted `.kawoosh/init.lua` were reloaded
   from it on every `:cd`.
3. **Where things start.** `:e`, the `files` walk, `:grep`, a tool, a
   terminal, a compile without a manifest.
4. **What the title bar shows.**
5. **The memory's workspace** — not the cwd but derived from it:
   `workspace_of`, the outermost directory at or above it with a
   `.kawoosh` in it, so a project with none had no workspace and every
   such project shared the one empty one, its pins included.

And beside them, two that were already something else: a terminal's
own directory (OSC 7, else its shell's, since step 15), and the
session, one per window (`"default"`) whatever the cwd.

The trouble with one global cwd is the daily driver's shape: a window
of tabs, each tab a piece of work, often in different projects. A
`:cd` in one moves all of them, and the only way to keep two projects
open was to keep `:cd`-ing between them — each time reloading the
other project's settings and asking its `init.lua` again.

## Decisions

### 1. The cwd is the tab's

Every tab has a directory (`Tab::cwd`). The focused tab's is *the*
cwd: what `:e` resolves against, where a terminal, a tool, a compile
and a picker's walk start, what the title bar shows, what `:pwd`
says. `:cd` moves the focused tab's; the other tabs stay where they
are. A new tab starts where the tab it was made from is. The dock has
no directory of its own — it is visible from every tab — and a
command in it uses the focused tab's.

Switching tabs moves the cwd with it. When the new cwd is in another
project — the project files above it are not the same list — the
project settings layer and the trusted `init.lua` are read again, as a
`:cd` into it would; within one project nothing is reloaded.

*Beat:* **a named workspace** — a root, its session, its memory scope,
its tabs — switched from a picker the directory jumps feed. It is the
larger model and the one that would cost a new concept to learn and a
place to keep the list; the tab is already the unit the hands move
between (`gt`, `<C-1>`…), and a tab with a directory is most of what a
workspace was for. **Vim's three levels** (`:cd`, `:tcd`, `:lcd`) —
a global cwd with overrides is two answers to "where am I", and the
pane-local one is the one nobody can see. **The two together** (a tab
opened *on* a workspace) is left open by this: Decision 4 keeps the
workspace a derived name, so a later note can make it an object
without undoing the tab's directory.

### 2. The process cwd is not moved

`set_current_dir` is gone: the process stays where it was started, and
nothing depends on it. Every spawn passes its directory — the pty, a
Lua `kawoosh.spawn` (its `cwd`, else the tab's), a compile (its
manifest's root, else the tab's), a language server (its root) — and
every relative path is resolved against the tab's cwd before it
reaches the disk. Lua's `kawoosh.fs.cwd()`, `fs.expand`, `fs.form`
and every `fs.*` argument read the editor's cwd from the snapshot, not
the process's. What a plugin reaches past the API — Lua's own
`io.open("relative")` — resolves against the process's, and is the one
thing this does not cover; `kawoosh.fs.*` is the door.

*Beat:* `set_current_dir` on every tab switch — correct only while
nothing runs on another thread, and the io, ts and lsp threads all
resolve paths.

### 3. `kawoosh.on_cwd(fn(path, how))` says why

`how` is `"cd"` (a `:cd`, `<leader>cd`, `kawoosh.fs.chdir`, a pick) or
`"tab"` (the focused tab changed to one in another directory). The
directory jumps count `"cd"` alone as a visit: a tab switch is not a
place gone to.

### 4. The memory's workspace is the project's root

`workspace_of(dir)`: the outermost directory at or above `dir` holding
a `.kawoosh`, as before; else the nearest holding a repository
(`.git`, `.jj`, `.hg`); else none. A project without a `.kawoosh` —
most of them — has a workspace of its own now, its pins and its
ranking with it, where every such project had shared the empty one.
It follows the focused tab's cwd as it followed the global one.

What was kept under the empty workspace stays there: rows are not
moved, since which project an old pin belonged to is not on record.
A pin made before in a project with a repository is under
`:memory all`, and `m` pins it again where it is.

*Beat:* the root a language server would pick (its markers per
language) — one workspace per language in a polyglot repo.

### 5. The session keeps each tab's directory

`TabData::cwd`; a restore puts each tab back in its directory when it
still is one, and in the launch directory when it is not or the
session is from before (no `cwd`). The session stays one per window.

*Beat:* **a session per workspace** — a bare launch in `~/p/foo`
restoring foo's tabs. Tempting, and it is Decision 1's beaten
alternative through the back door: which session a window is becomes
a question the user answers by where they launched from. Left for the
named-workspace note, if one is written.

### 6. What the user sees

The title bar shows the focused tab's cwd, as it did. When the tabs
are not all in one directory, each tab's label leads with its
directory's name (`kui · lib.rs`), so the strip says which project
each tab is. A directory jump's `<C-t>` opens a new tab *on* the
directory — its cwd, and the directory listed — which is the "tab a
project" gesture.

*Amended 2026-09-27 (roadmap step 50):* that rule is `tabs.directory =
"auto"`, the default; `always` leads every label with its directory,
`never` none. A tab whose focused pane is a terminal is where its shell
says it is (OSC 7), not where the tab was opened. `kawoosh.tab_title(fn)`
writes the labels outright, wezterm's `format-tab-title`: `fn(tab)` is
handed the label kawoosh would draw and its parts (`index`, `active`,
`dir`, `cwd`, `kind`, `name`, `path`, `modified`, `bell`, `panes`) and
returns the label, or nil for kawoosh's.

### 7. A tab lists its own buffers

*Added 2026-09-25 (roadmap step 30), asked as "workspaces should have
a separate buffer list by default and an option to see all".* With
the cwd the tab's, the lists are too: under `buffers.scope = "tab"`
(the default) the buffers picker, `:ls`, `]b` `[b` `<leader>bn` and
the launcher's *buffers* show the focused tab's — a listed file under
its directory, or a buffer it has shown (`Tab::seen`), which takes in a
scratch typed there and a file opened there from another project —
and the dock's. `:ls` numbers them as `:b N` counts every buffer and
says how many the other tabs have; `<C-a>` in the buffers picker flips
to every tab's for the session; `buffers.scope = "all"` is the old
list. The engine does not know tabs: the shell keeps
`Editor::tab_buffers` in step with the facts, and Lua reads it as
`kawoosh.buf.list { tab = true }`. What a tab has shown is not kept by
a session.

*Beat:* a buffer owned by one tab, moved out of the others' reach —
a file open in two tabs is both tabs', and `:b NAME` still reaches
any buffer, since hiding is a list's business, not the buffer's.
(Still beaten after the 2026-10-07 amendment below, which counts the
tabs instead of choosing one.)

*Amended 2026-09-29:* a listed file under the tab's directory is not
the tab's when it is in another open workspace nested there — a
repository inside the project with a tab of its own, a project under
a tab at `~` — so the outer tab's lists leave the inner's files out.
And `:bdo` closes the tab's list (`buffers.scope`) less anything
another open workspace has: what a tab in another workspace counts as
its own, even a file this tab showed too, and what a dock pane of one
shows. That holds under `buffers.scope = "all"` too: there it closes
every buffer but those. A closed workspace's buffers are nobody's in
particular, so they are the tab's again wherever the lists say so.

*Amended 2026-09-29, the same day:* closing a tab — `:tabclose`, its
button, its last pane closed or moved to the dock — closes the
buffers it had (what it showed, a listed file under its directory)
that no tab has now: none claims it and no pane shows it. An unsaved
one is kept and becomes the tab in front's, so its lists reach it, and
the message says so (`1 buffer(s) closed with the tab, 1 unsaved kept
here`). Before, a closed tab's buffers stayed loaded and in no tab's
list.

*Amended 2026-10-07:* **a buffer counts the tabs that hold it.** The
user's complaint: a `:bdo` in one tab closed what another tab had open.
Two tabs in one project are one workspace, so neither spared the
other's, and a tab's list took in every listed file under its
directory, so it reached files only the other tab had opened. Asked
first as "a tab is a workspace, every buffer under it exclusively
owned by it", then narrowed by the user to "refcount the buffer".

- A tab holds a buffer once a pane of it shows it (`Tab::holds`, which
  was `seen`), wherever the file is, until the tab lets it go. A
  buffer's count is the tabs holding it, and the dock while a pane of it
  shows it; it is open while the count is above zero.
- The path claim is gone: a file under a tab's directory is in its
  lists only if the tab opened it. The nested-workspace exception and
  `other_workspaces_buffers` went with it.
- `:bd` lets go from the tab in front: its panes on the buffer go
  where they came from, else to another of the tab's, else a new
  scratch. If another tab still holds the buffer it stays open there,
  unsaved changes and all, so there is nothing to refuse or discard;
  the message says so. The last `:bd` closes it as before, unsaved
  changes refused without `!`. Another tab's panes are never touched.
  `leave_in_other_tabs` is gone.
- `:bdo` lets go of the tab's list but the current buffer. Those whose
  count reaches zero close; the rest are counted in the message
  (`1 buffer(s) deleted, 1 left to other tabs`). Under
  `buffers.scope = "all"` the list is every buffer, but another tab's
  are its own, so they stay too.
- Closing a tab lets go of what it held; what nobody holds then closes,
  unsaved ones kept and held by the tab in front, as before.
- A listed buffer nobody holds goes, on the next sync, to the tab whose
  directory has its file (the deepest), else the tab in front. That
  covers a session's unsaved buffer put back hidden, or one a plugin
  loaded without a pane. So every open buffer is in some tab's lists.

Help, man, `*compile*` and other tool buffers are counted like files.
Under a count that only changes when the last holder lets go, so each
tab can `:bd` its help page without closing another tab's.

*Beat:* **exclusive ownership**: one owner per buffer, and reaching
another tab's file switches to that tab. Asked, and set aside by the
user for the count, which fixes the complaint (`:bdo` closing another
tab's buffer) without moving anyone between tabs. The workspace stays
the project root for the memory and the dock (Decisions 4, 8 to 10).

## Round two: a lifecycle, and the dock

*Decided and built 2026-09-25 (roadmap step 32), from three asks the
same day: the dock per workspace — "but then we should know when a
workspace is closed to kill the dock" — or global, "like running
tasks, maybe I want to see them all the time"; and recent workspaces
in the launcher, opening a listing or restoring something. The calls
are the round's, each with what it beat, and the user's to overturn.*

### 8. A workspace opens and closes with its tabs

A workspace is the directory the memory already names (Decision 4: the
outermost `.kawoosh`, else the repository's root), or the tab's own
directory when it is in neither. It is open while a tab's directory is
in it, and closes on the frame none is — its last tab closed, or
`:cd`'d out of it (`Kawoosh::sync_dock`). What is kept between is what
the memory keeps per workspace already: its files at their lines, its
pins, its ring. Nothing else is saved on close.

*Beat:* a session per workspace, kept on close and restored on return
— the objection Decision 5 had is answered by an explicit pick, but a
second session format for what the last file mostly gives is more than
the ask; it stays the next step if the last file is not enough.

### 9. The dock is the window's; its panes are a project's

One dock, visible from every tab, as it was — running tasks are what
one wants in sight whichever project is in front. Each dock pane is
stamped with the workspace in front when it appeared
(`Layout::dock_owner`), and a pane of another project than the one in
front leads its title with that project's name (`alpha · terminal`).
A domain's master (domains.md) is nobody's: it is the window's
connection to a host, not a project's task, and no workspace closing
ends it — which the domain tests caught when the first cut did.

*Beat:* a dock per workspace, swapped with the tab — it hides the task
one wanted to watch; and a dock with no owners — a closing project
could not find its tasks.

### 10. A workspace closing ends its tasks

When a workspace closes, its dock panes go with it: a terminal whose
process exited or whose shell sits at an empty prompt (the OSC 133
marks) closes at once; if any is running something — or is a shell
that marks nothing, so there is no telling — one confirm names them,
*End them* (`:dock end DIR`) or *Keep them*. Kept tasks stay, still
the closed project's, until closed by hand.

*Beat:* ending them unasked — a build or a server killed by a `:cd`;
keeping them hidden for a tab that may come back — the detachable
daemon mvp.md keeps out.

### 11. Recent workspaces are a picker source and a launcher section

`picker workspaces` (`<leader>sw`), `launcher = true` so the launcher
lists it too: every workspace the memory has files under, newest
first, the one in front left out, each with the file last attended. A
pick moves the tab's directory there and opens that file at its line,
or lists the root when it has none — so a new tab's launcher, then a
pick, is "open that project where I left it".

*Amended 2026-10-09 (Decision 13):* the picker's pick (`<leader>ww`,
the key since keymap-regroup.md) opens a new tab on the workspace, or
goes to the tab on it there is; moving the tab in front is `<C-o>`'s.
The launcher's row still fills its pane in place.

### 12. The dock as a strip, the experiment

`layout.dock = "scroll"` (`tree` the default): the dock's panes as
columns on a ribbon — a split beside is a column after the focused
one, a split below stays in the column, `<C-S-h>` `<C-S-l>` walk the
columns by index, the focused one revealed — and the project in
front's columns first, reordered when it changes. Flipping the setting
converts the dock both ways with its panes.

*Not decided:* narrowing the dock to the project in front
(`layout.dock_scope`) — the ordering and the titles may be enough;
and *levels* — strips stacked vertically, the dock one of them — which
the experiment is there to argue for or against.

### 13. A workspace picked is a tab of its own

*Decided and built 2026-10-09, asked as "i don't like workspace
switching. let's `<leader>ww` pick open new tab".* A pick in Decision
11's picker moved the tab in front: its directory went to the other
project, and the file the tab showed was replaced by the one last
attended there. The tab was the piece of work the user was in, and the
pick took it away to make the other — the switching Decision 1 had
argued the tab out of.

`<CR>` in `picker workspaces` now opens a new tab *on* the workspace:
its directory the workspace, the file last attended open at its line
(or the root listed when the memory has no file there), the tab it was
asked from left as it was. It is Decision 6's "tab a project" gesture
(`<C-t>` in the directory jumps) reached from the memory, and is made
the same way: `kawoosh.open(file, { split = "tab", line = })`, then
`kawoosh.fs.chdir(root)`, which moves the new tab's directory, as
`:domain tab NAME` makes a tab on a machine and then moves its
directory there.

When a tab is on the workspace already — its directory in it, not the
tab in front, which is left out of the list — the pick goes to that tab
(`:tab goto N`, the first such in the strip) instead of making a second.
The tab is the workspace (Decision 1); a second tab on it would be a
second piece of work in one project, which a user makes on purpose
(`:tabnew` there), not as a side effect of asking for the project. It
is `:tool NAME`'s rule, which finds the tool's pane in the tab before
starting another. The tab is shown as it was left, not at the memory's
last file: its panes are where the user was in it.

`<C-o>` keeps the old pick — the tab in front moved there, the file
opened in its pane — for whoever wants it, the secondary key the
domains picker uses for its own other way (`<C-o>` connects without a
tab, `<leader>wh`), named in the placeholder as the buffers picker
names `<C-x>` and the domains picker `<C-o>`.

The launcher's section stays as it was. A launcher is a bare pane
asking what it is for, and what is picked fills it in place: from a
`:tabnew`'s launcher a pick that opened yet another tab would leave the
asking one empty behind it, and one that went to the tab on the
workspace would too. Its row runs the source's `launch`
(`kawoosh.picker.source`'s, new, what a row does in the launcher when
it differs from the picker's pick), which is the old pick: the tab the
launcher is in moved there, the file in the launcher's pane. So "a new
tab's launcher, then a pick" is still "open that project where I left
it".

For it the engine says two things it did not: `kawoosh.tabs()`, every
tab's directory in the strip's order with which is in front, and `:tab
goto N`, the Nth tab from 1.

*Beat:* **always a new tab**, a second tab on a project open already —
two tabs that are one workspace share its buffers (Decision 7's count)
and its dock, so the second is the first's clutter; **a confirm**
asking "go to it, or another tab?" — a question every time for the
answer that is nearly always the same, where `:tabnew` then `<C-o>` is
the rare other; **the launcher's row opening a tab too**, for
symmetry with the picker — it leaves the launcher's own tab empty;
**`<C-t>` for the new tab**, the picker's split key, with `<CR>` left
moving the tab — the ask was that the plain pick open the tab.

## Build order

One round: the tab's `cwd` and the sync on the frame (`Kawoosh::sync_cwd`,
the project reloaded only across projects); `set_current_dir` gone and
every reader of the process's cwd moved to the tab's (compile,
`kawoosh.fs.*`, a socket's relative path, the `cwd` of a plugin's
root); `on_cwd`'s `how`; `workspace_of`'s repository fallback; the
session's `cwd`; the tab labels; `dirs`' `<C-t>`.

Tests: two tabs in two directories, `:cd` in one leaving the other; a
tab switch reloading the project layer across projects and not within
one; a terminal and a Lua spawn starting in the tab's directory; the
process's cwd unmoved by `:cd`; a session round trip of two tabs'
directories; `workspace_of` on a repository; the labels.

## Risks

- **A plugin that trusted the process cwd.** The bundled ones use
  `kawoosh.fs.*` and `kawoosh.spawn` — checked; a user's `io.open` of a
  relative path now reads from where kawoosh started.
- **`init.lua` run again on a tab switch across projects.** The same
  as a `:cd` there, which it already was; its side effects (a `map`
  replaced) are what they were on `:cd`.
- **The empty workspace's rows.** Not migrated (Decision 4); they age
  out on the memory's clock.
