# Rules: a language's server, switched per project

Status: written 2026-09-26 from the user's ask — "toggleable rules for
specific lsp's (load all files in ts for example for a complete
diagnostics and stuff)". The calls below are taken here, each the
user's to overturn. Companion to [lists.md](lists.md), whose
Decision 2 left a TypeScript project's diagnostics at "the files you
opened" and whose "Not built" is where this came from.

## What there was

A language's server was a `ServerDef` — command, args, root markers,
the configuration it reads — from `ServerDef::builtin` or a
`kawoosh.lsp.server(language, t)` in Lua, and nothing about how it was
used could be said per language or per project:

- **A server spoke of what it was sent.** typescript-language-server
  answers for the documents it holds, so `:diagnostics` in a
  TypeScript project listed the errors of the files that happened to
  be open. rust-analyzer's check covers the crate; tsserver has
  nothing like it unasked, and `tsc --noEmit` through `:compile` was
  the only whole answer.
- **A server could not be turned off** but by redefining it to a
  command that is not there.
- **`lsp.inlay_hints` was one switch** for every language: rust's
  hints are worth reading, TypeScript's crowd a line.
- **The settings declared an `lsp` table** — "a language's server:
  `cmd`, `args`, `roots`, `settings`" (roadmap step 34) — that nothing
  read.

## Decisions

### 1. A rule is a setting under its server, named by its language

`lsp.<name>` is a language server, in the settings tree, named by the
language it is first for:

```lua
-- .kawoosh/settings.lua in a TypeScript project
return {
  lsp = {
    typescript = { load_all = true },   -- .ts, .tsx and .js
    rust = { inlay_hints = true },
    python = { enabled = false },
  },
}
```

The tree is layered already (kui.md D10): the user's file says what
holds everywhere, a project's `.kawoosh/settings.lua` what holds there,
and `:set` or `:lsp toggle` what holds for the session — a rule is
switched per project for free, and a `:cd` swaps the project's rules
with its layer. The shell folds the table over the definition of that
name (builtin or Lua's) into the one the pool runs, again on every
settings change.

**One server serves the languages one program reads**
(`ServerDef::languages`): `typescript` is typescript-language-server
for `typescript`, `tsx` and `javascript`, and `c` is clangd for `c` and
`cpp`. They were one process already — the pool keys by root and
command — but three definitions with three sets of rules, so a
TypeScript project needed `load_all` said twice (asked in review:
"typescript and tsx i think can be combined"). A rule is the server's
and reaches all its languages; each loaded file is still sent as its
own language. `lsp.tsx = { … }` without a `cmd` is no server: the
settings say once where its rules go (`lsp.tsx: tsx is served by
lsp.typescript`). With a `cmd` it is a server of its own, and a
language with a server of its own name is served there — `lsp.javascript
= { cmd = … }` takes javascript from typescript's. `languages = { … }`
under a name says the list outright.

Named by **language**, not by command: a buffer carries a language and
a user thinks "TypeScript", not `typescript-language-server`; the name
of the language a server is first for reads as that. `:lsp toggle`
from a `.tsx` buffer flips `lsp.typescript`'s rule.

Beaten: rule fields on `kawoosh.lsp.server` (Lua only, not per
project, not switched at runtime); rules keyed by command
(`lsp["typescript-language-server"]`); a definition per language with
each one's rules (as first built — the same rule twice for one
process); `lsp.tsx` falling back to `lsp.typescript`'s keys (two places
to look for one server's rule).

### 2. The table is the server too

`cmd`, `args`, `roots`, `languages` and `settings` under `lsp.<name>`
replace the definition's, as step 34 declared and nothing did. A new
`cmd`, `args`, `roots` or `languages` restarts the servers on its
command (the path `:lsp restart` takes: the buffers sent again, their
diagnostics cleared until the new server's land); new `settings` reach
a running server as `workspace/didChangeConfiguration`. `enabled =
false` takes the server out of the table the pool reads: it stops, its
languages' buffers are not sent, and `enabled = true` sends them again.
A `lsp.<name>` with a `cmd` and no definition of that name is a new
server: `lsp.zig = { cmd = "zls" }`.

### 3. `load_all`: the server holds every file of its languages

With `load_all` on, a server that starts — or one running when the
rule is switched on — is sent every file of its languages in its
workspace, read from disk: the walk the picker and `:grep` take
(`fs::walk`, `.gitignore` honoured), a language's files by its
extensions and names, at most `load_max` of them (2000), none over
1 MiB (a bundle is not source), and none a secrets rule's `files` names
— a private buffer's text never leaves the process (secrets.md
Decision 1), and a file read behind the buffers' backs is held to the
same rule. The server then speaks of the
project — `:diagnostics` lists every file's errors, as rust-analyzer's
check already does — and a reference or a rename reaches files no
buffer holds.

A loaded file is the **pool's document**, not a buffer's. A buffer
opened on it takes the document over: the text is the disk's, so
nothing is sent unless it differs (a `didChange`, not a second
`didOpen`). A buffer closed hands it back: the disk's text again, a
`didChange` if the buffer had left it different. Switched off, the
loaded files are closed and their diagnostics dropped. More files than
`load_max` load the first by path and say so once.

The documents are counted apart — `:lsp info` shows `loaded N` beside
`docs`, and the title bar's count stays the buffers'.

### 4. `inlay_hints` per server

`lsp.<name>.inlay_hints` decides for its languages' buffers; unset,
`lsp.inlay_hints` does. `<leader>cI` stays the global switch.

### 5. `:lsp toggle RULE [LANGUAGE]`

Flips a rule that is on or off (`enabled`, `load_all`, `inlay_hints`)
for the server of the caret buffer's language — or `LANGUAGE`'s — in
the session layer, and says where it now stands. The session layer is what `:set`
writes, so the settings tab shows it as the session's and `:set
lsp.typescript.load_all -` takes it back.

### 6. A plugin's rules, set where the shell's are

Asked 2026-10-03, from "Not built" below: a rule that is not the
shell's — a plugin that organizes imports on save, runs a server's
fix-all, or turns a code lens on, per server and per project.

`kawoosh.lsp.rule(name, { doc =, default = })` declares one. It is set
exactly as the shell's rules are: `lsp.NAME.RULE` in the settings'
layers — the user's file, a project's `.kawoosh/settings.lua`, the
session — for the server `NAME`, and, like `inlay_hints`, `lsp.RULE`
for every server that does not say; then `default` (`false` unless
said; any value a setting holds). On or off, it is a switch: `:lsp
toggle RULE [LANGUAGE]` flips it for the caret's server in the session,
as it flips `load_all`. `:lsp info` lists it where it is set, with its
origin (`organize (project: /repo/.kawoosh/settings.lua)`).

`kawoosh.lsp.rules(where)` reads them for a buffer (a handle, the
current one by default) or a language (a name): `{ server = NAME,
enabled =, load_all =, load_max =, inlay_hints = }` and every rule
plugins declared, each resolved as above. `server` is the `lsp.NAME`
the rules go under — a `.tsx` buffer's are `typescript`'s (Decision
1), which the shell tells Lua whenever its table moves
(`Runtime::set_lsp_names`), so the plugin need not know which server
serves what.

The pool never reads a plugin's rule: the plugin does, when it acts
(on a write, on a key), so a rule switched mid-session holds from its
next read and nothing restarts. A rule's name is lowercase, digits and
`_`, and none of `lsp.NAME`'s own keys or `lsp`'s (`cmd`, `settings`,
`load_all`, `languages` …) — `lsp.RULE` would read as one — and a
table under `lsp.RULE` is not taken for a server. The same name again
replaces the rule.

**A rule's name is no server's, and a server's no rule's.** Found in
review, 2026-10-03: a rule named like a user's server (`lsp.zed = {
cmd = … }`) took `lsp.zed` out of the table, the server silently gone;
one named like a builtin (`rust`) made `kawoosh.lsp.rules` return that
server's table as the rule's value. Now `kawoosh.lsp.rule` raises on a
server's name — a builtin's, `servers.lua`'s, one `kawoosh.lsp.server`
defined (in this runtime too, before the shell has heard), or an
`lsp.NAME` that defines one (a key of `cmd` `args` `roots` `languages`
`when` `install` `settings` `answers`, `kawoosh_lua::defines_server`)
— and `kawoosh.lsp.server` raises on a rule's. The shell tells Lua
every server's name with the languages' (`Runtime::set_lsp_names`). A
table that defines a server and is said after a rule of its name (the
settings move later) is the server still, said once on the message
line, and `kawoosh.lsp.rules` does not read it as that rule for every
server; only a table under a rule's name that defines nothing is left
out of the servers. Declaring a rule makes the table again at once
(`rules_seen` reset), so what it leaves out is out from then.

**A rule goes with its runtime and its switch with its kind.** A new
Lua runtime (`attach_lua`) forgets the last one's rules and their
`:lsp toggle`s (`forget_lsp_rules`); a rule declared again as no
switch loses its toggle (`Editor::undeclare`).

Beaten: **rules in `kawoosh.lsp.server`'s table**, beside a server's
data: a rule is a plugin's, not a server's, and reaches every server —
a format-on-save plugin's switch is the same for rust-analyzer and
gopls. **`kawoosh.setting` per server** (`lsp.rust.organize` declared
for each name): the names are the table's and move with it; one
declaration under `lsp.*` is what a rule is. **A rule `enabled` by
the shell for the plugin** (the pool skipping a server, say): a rule
the shell acts on is the shell's to add; a plugin's means what the
plugin does with it.

### 7. What another program changes on disk reaches the server

Written 2026-10-03, from "Not built" below: a file `load_all` sent was
the server's as the walk read it, whatever a checkout, a generator or
a `git pull` did to it after; and a server was told nothing of files
it holds no document for — rust-analyzer's `Cargo.toml`, a file made
in the project. The protocol's way is `workspace/didChangeWatchedFiles`,
which the server asks for with the globs it cares about.

**A watch on the workspace, on the platform's events.** The pool
watches the root of each server here that hears of files — it
registered watches, or `load_all` holds files for it — and any folder
outside the root its watches name (a path dependency's crate):
`systems/src/tree_watch.rs`, on `notify` 8, the one new dependency.
ReadDirectoryChangesW and FSEvents watch a tree in one handle; on
inotify, whose recursive watch is a watch per folder anyway, the
folders are walked as the picker's walk reads `.gitignore` and watched
one by one, a folder made later as it comes. The buffers' watch
(`watch.rs`, roadmap step 12) is not it and stays: it polls a few
named paths, and a workspace is thousands — polled, a walk at every
beat — with the files made in it on no list.

**What is no news is cut before it counts**: a path in `.git` (`.hg`,
`.svn`, `.jj`), `target` or `node_modules`, or one a `.gitignore` names
in a repository, read as the walk reads them. A build or an `npm
install` stirs nothing a server hears.

**Batched**: what changed is gathered until the tree has been still
for 150 ms, or for a second since the first, and each path is said once,
as the disk has it then — made and gone again is nothing, gone and made
again (a save by rename) is a change, a folder's own stir is said by
its files. Where the platform dropped events (its buffer ran over),
every file loaded under that root is read again.

*Lost on Windows* (found in the review, 2026-10-03): inotify says it
ran over (`Q_OVERFLOW`) and FSEvents says a tree must be scanned, both
notify's `need_rescan`; notify 8.2's ReadDirectoryChangesW backend reads
16 KiB at a time — a hundred and some paths — ignores the byte count of
an overflowed read and says nothing, so a `git checkout` during a
`cargo build` lost changes silently. A probe (a slow handler, 3000
files written) heard 2 events and no error. So on Windows a **burst** —
[`BURST`] (128) events heard under a root in one batch's time, ignored
ones (`target`) included, since they fill the same buffer — counts as
possibly lost, and once nothing has been heard for 150 ms every file
loaded under it is read again, once per burst, not per batch: a build
costs one read of the loaded files after it, the text compared and only
a difference sent. What it cannot catch is an overflow whose whole
burst came while notify's thread was starved, fewer than [`BURST`] of
it heard; a buffer of our own (a ReadDirectoryChangesW of kawoosh's)
would be the cure, and is not built.

*Roots that come and go*: each root is looked at every 2 s — a stat —
and one not there (not made yet, or deleted: Windows' watch ends with
its folder) waits, and is watched again when it is there, or when
another folder stands in its place (its inode; on Windows its creation
time — file tunnelling may keep that for a folder made again within
15 s of its deletion and a check, which is not caught). Watched again,
what is in it is said as made and its loaded files read again. On
inotify a folder deleted or moved away lets go of its watches and of
those under it (they died with it), so one made again in its place is
watched as new and churn does not use up the 50 000 folders.
"No repository" is believed for 10 s, or until a `.git` is heard of
(`git init`); a folder a folder-only line names (`/gen/`) is read as
ignored when it is gone too.

*"Made" on FSEvents* (found 2026-10-05, the watch's tests failing on a
Mac): an FSEvents event carries all that happened to its path of late,
so a file made a moment ago is "made" again in each event after — when
it is written, when it is removed. The first word of a path being what
is kept, a change was said as a creation and a removal as nothing
("made and gone"). There (`Tree::stale_made`) a made file is new only
if it was not said to be there already (`Tree::known`, the files said
made or changed and not deleted since, 16 384 of them at most) and was
born under the watch (its birth time against when its root was
watched); else it is changed. One heard of as made and not found is
said deleted — it may have been made and gone within the batch, but a
deletion of what nobody heard of costs nothing, and one unsaid leaves
its file believed in. And a root's real path (`/private/var/…` for
`/var/…`) is asked when the root is watched, not when it is named: a
root not made yet had none, and nothing under it was heard once made.

**What a server is sent**, path by path:

- *A file `load_all` sent it* (the pool's document) is read again: its
  new text a `didChange`, gone — or grown past a MiB — a `didClose`,
  its diagnostics dropped as a rule switched off drops them; a folder
  gone takes its files (looked up in the loaded files in path order,
  not each file against each folder). A file of a loaded language made
  since, or a folder of them, is sent as the walk sends: not hidden,
  not private, under `load_max` — asked before a made folder is walked,
  a folder walked once a batch whichever servers want it, one inside
  another made folder not walked again. A `load_all` walk that ends
  after files made meanwhile were sent counts them toward `load_max`.
- *A file a buffer holds* is the buffer's, and nothing of the disk's is
  said: the protocol has the client own an open document's text and
  the server not read it from disk. The buffer's own watch reloads a
  clean buffer, and the reload reaches the server as the buffer's
  `didChange`; a modified one is asked, and meanwhile the server keeps
  the buffer's text. A save from kawoosh is the same: news to no
  server holding that buffer, but to one that holds no document for it
  — `Cargo.toml` saved, to rust-analyzer — it is said.
- *The rest*: `workspace/didChangeWatchedFiles`, one notification a
  batch, the paths each server's watches ask for.

**Registration.** A server here is offered
`workspace.didChangeWatchedFiles` with `dynamicRegistration` and
`relativePatternSupport`; its `client/registerCapability` for the method
is kept by registration, and `client/unregisterCapability` drops one —
under the protocol's spelling `unregisterations` or the word it meant.
A glob is LSP's (`*` within a segment, `**`, `{a,b}`, `[…]`): a
`RelativePattern` is matched on the path under its folder, a string
from a disk's root on the whole path, any other — as the protocol reads
it, relative to the workspace folders — under the server's root or a
folder its other watches name, never on the whole path (clangd in `/a`
asking for `**/compile_commands.json` was told of `/b`'s); `kind` masks
created, changed and deleted. On Windows a
`\` in a glob is a separator — rust-analyzer writes its root into a
glob string for a client without relative patterns — paths and globs
are matched case aside, and paths go out as every URI here does,
`file:///C:/…`. Offered this, rust-analyzer turns its own watcher off:
kawoosh's is its watcher now.

**Not watched**: a server on a host ([domains.md](domains.md)). SFTP
has no watch and a walk there is a round trip a folder, so such a
server is not offered the capability, and rust-analyzer and tsserver
go on watching on their host themselves; its `load_all` files are, as
before, what the walk read until a buffer opens one, the rule is
switched or it restarts. Nor a disk's root, the home folder or one
above it (`/home`, `C:/Users`) as a root: a loose file's server starts
in its file's folder, and a home's caches stir all day.

Beaten: the buffers' polling watch handed every loaded file (no file
made is on it, and two thousand stats each half second — on Windows a
stat opens a handle); the workspace polled with the walk; a recursive
inotify watch (a watch on every folder of `target` and `node_modules`);
`didChangeWatchedFiles` for an open buffer's file as well, as VS Code
sends it — a server takes a document it holds from the client, not the
disk, and the buffer has said its text.

## Beside it

`didOpen`'s `languageId` is the document's own language now, where it
was the language that started the server: a `.tsx` opened on a server
started by a `.ts` was sent as `typescript` and read without JSX. `tsx`
is sent as `typescriptreact`, LSP's name for it.

## Built

2026-09-26, as decided; one server for several languages the same
day, from the first look at it. `:lsp info` lists what each server
serves and its rules. The shell's half is `kawoosh/src/lsp_rules.rs`
(`lsp_table` folds the settings over the definitions, `sync_lsp_rules`
diffs the table and restarts, stops or closes; `:lsp toggle enabled`,
`load_all`, `inlay_hints`); the pool's is `systems/src/lsp.rs` — a
`Document` whose `buffer` is `None` is a loaded file, `reconcile_loads`
walks on a thread (`load`) when a rule is switched on or a server comes
up, `Cmd::Stop` for a command no language uses now, and a server's new
`settings` sent as `workspace/didChangeConfiguration` where a restart
is not needed. `kawoosh/tests/lsp.rs`'s
`rules_load_all_hints_and_enabled`, `the_settings_table_is_the_server` and `one_server_serves_typescript_tsx_and_javascript`
run it against the fake server, and a throwaway probe against
typescript-language-server 6.0.1 on TypeScript 5.9.3 (configured by
`lsp.typescript` alone): a `.ts` and a `.tsx` never opened had their
errors listed within seconds of `load_all`. npm's `typescript` is 7
now, the native port, with no `tsserver.js`; typescript-language-server
on it answers nothing, so it wants `typescript@5` beside it.

Found on the way: `:lsp restart` said each buffer the server held is
sent again, and one no pane showed was not; a reset now keeps them in
`also_sync`.

`:lsp logs [LANGUAGE]` came with it, asked while configuring clangd:
what a server said — stderr included, which the notification log drops
as a trace — kept per server and shown live (`kawoosh/src/lsp_logs.rs`).

### 8. A change is sent as its span

*Added 2026-10-07*, from the roadmap's "incremental sync from the
journal — open; measure before doing it" ("then do the lsp ones").
Measured first (release build, a character typed mid-file): the sync
copied the buffer's whole text on the UI thread — 0.7 ms at 1 MB, 4.7
ms at 10 MB, 23 ms at 50 MB — and the pool encoded it whole as JSON —
0.9, 5.1 and 46 ms — at every keystroke a server was attached. A
minified bundle with tsls on it (format-of-a-minified-bundle) is the
case that pays it.

- **The shell sends what moved.** After the open, a sync is the span
  the journal says changed since the version last sent
  (`Journal::changed_since`): the edits folded into one replacement —
  from the first byte any touched to the last, the head and tail no
  edit reached as they were — and only its bytes copied
  (`SyncText::Span`). The whole goes the first time, and when the
  journal cannot say (pruned past it, or `set_text` reset it). 150 ns
  at 10 MB and at 50 MB, where it was 4.7 and 23 ms.
- **The pool holds the text.** Each synced buffer's text is kept in the
  pool (`Pool::texts`) at its version, and a span is applied to it there;
  a server opening the buffer later — eslint joining, one restarted — is
  sent the whole from that copy, not asked for it. A span that does not
  fit the copy (its version is not the one held, or its bytes are out
  of it) is answered `Event::SyncLost`, and the shell's next sync is the
  whole: the pool never guesses.
- **A server is told the way it takes it.** One whose `initialize` says
  `textDocumentSync.change` 2 (`Caps::incremental`) gets the span as a
  range and its text, the range's end worked out from its start over the
  bytes between (`position_after`); any other gets the whole document
  from the pool's copy — the encoding off the UI thread, where it was
  already, but no copy made on it.

A test compares the text a server holds with the buffer's after typing
past a character outside the BMP, a line opened and joined, one deleted
and an undo, with an incremental server and a full one
(`a_change_is_sent_as_its_span`). It found a bug on the way: an answer
held while typing was put over a newer one (fixed apart, 66104c2).

Beaten: the edits one by one as `contentChanges` (each change's own
text is gone once a later edit overwrites it — the journal keeps
lengths, not texts); a diff of the old and new texts in the pool (the
UI thread would still copy the whole to send it); the copy kept in each
server's document alone (a server joining has no text to be opened
with).

## Not built

- ~~**A loaded file changed on disk by another program** is not read
  again; the server holds what the walk read until a buffer opens it,
  the rule is switched, or the server restarts. A
  `workspace/didChangeWatchedFiles` registration is the way.~~ Built
  2026-10-03 (Decision 7): `systems/src/tree_watch.rs` (the watch,
  batched and cut by the ignores), `systems/src/lsp.rs` (the
  registrations — `FileWatch`, `file_watches` — `rewatch`,
  `handle_files` and `files_changed`, `load_made`); tests
  `tree_watch`'s `changes_are_heard_recursively`,
  `changes_are_heard_a_folder_at_a_time`,
  `a_path_heard_of_twice_is_said_as_it_ended` and
  `gitignores_are_read_as_the_walk_reads_them`, `lsp`'s
  `watched_files_match_as_the_protocol_reads_them`, and
  `kawoosh/tests/lsp.rs`'s
  `files_changed_outside_reach_the_servers_that_watch_them` and
  `a_loaded_file_changed_on_disk_reaches_the_server` against the fake
  server's `--watch`, `--watch-rel` and `@unwatch`. A server on a host
  is still not told — looked at 2026-10-07 ("then do the lsp ones") and
  left for the agent: such a server is not offered
  `didChangeWatchedFiles` and watches on its own (rust-analyzer, gopls
  and tsserver fall back to their own watchers for a client that does
  not), so what another program changes there reaches it. What does not
  is a file `load_all` sent it from the host: the client holds it open,
  and a server reads an open document from the client, not the disk.
  Telling it means something watching on the host — `inotifywait` or
  `fswatch` where one is installed (neither is, on a stock host), or
  domains.md's agent — and SFTP's stat of up to `load_max` files a
  poll is the cost the agent is there to save. Until then, `load_all`
  on a host is a snapshot as of the walk, `:lsp restart` takes another. The review's fixes (2026-10-03: Windows' unsaid
  overflow, folders and roots made again, loose globs kept to their
  workspace, the made-file walk's cost, the home's ancestors, "no
  repository" forgotten, `load_max` kept with made files) are tested by
  `tree_watch`'s `a_burst_is_said_as_lost_once_it_is_over`,
  `a_folder_made_again_is_watched_again`, `a_root_made_later_is_watched`
  and `a_repository_made_later_and_a_folder_gone_are_read_as_ignored`,
  `lsp`'s `a_loose_glob_stays_in_its_workspace` and
  `a_folder_above_the_home_is_not_watched`, and a folder made and moved
  away in `a_loaded_file_changed_on_disk_reaches_the_server`.
- ~~**The pull model, workspace-wide** (`workspace/diagnostic`) —
  lists.md's; with it a server that answers would need no `load_all`.
  A document's own pull is built ([lsp-installs.md](lsp-installs.md)
  Decision 7).~~ Built 2026-10-07, [lists.md](lists.md) Decision 8.
- ~~**Rules a plugin defines**: the table is open, but only the shell
  reads its rules.~~ Built 2026-10-03 (Decision 6):
  `kawoosh.lsp.rule` and `kawoosh.lsp.rules` (`lua/src/lib.rs`),
  `add_lsp_rule` and `tell_lsp_names` with the plugin's rules in
  `:lsp info` and `:lsp toggle` (`kawoosh/src/lsp_rules.rs`). Test:
  `kawoosh/tests/lsp.rs`'s
  `a_plugins_rule_is_set_and_flipped_as_the_shells_are` (toggled from a
  default either way, the session's word, a project's, `lsp.RULE` for
  every server, `.tsx` read under `typescript`).
