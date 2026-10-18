# Secrets: a private buffer, masks, and a register that forgets

Status: decided and built 2026-09-23 (roadmap step 19; "Built" at the
end says where it departed), with the user: both
readings — a vault scratch and masked values in ordinary files — and
Ansible's `vault.yml` beside them; the masks configurable; the
concealed pasteboard honoured, and a terminal's password prompt treated
the same way. The mask's shape was left to this note. Each decision
keeps the alternative it beat.

## The thesis

A secret in kawoosh today goes four places the user did not send it,
checked in the code: the buffer it is pasted into is a draft in the
store within a second (`history.rs`, `QUIET`), its undo tree carrying
the text both ways; once put, it is a `text` row in the store for a
week (`moments.rs`, `memory.text.keep_days`); a yank of it goes to the
system clipboard (`Effect::SetClipboard`); a session brings the scratch
back. And `ansible-vault edit`, `sops`, `pass edit` — anything that
decrypts to a temp file and runs `$EDITOR` — land in kawoosh as
`kawoosh-edit --wait` on a plaintext file whose history row keeps the
edits for good. The one guard there is: what the clipboard holds when
the window comes back is the register's and is not written until put
(`Took::Seen`).

So the work is mostly the engine's: a **private** buffer that every one
of those paths asks about, a register entry that is **put once and
forgotten**, **masks** drawn by the engine from rules, and the freed
text **zeroed**. The plugin — `secrets.lua` — is what decides which
buffers are private and adds the vault file. Hackable by the same
doors: a rule is data, a plugin can mark a buffer private and mask
ranges it computes itself.

## Decisions

### 1. A private buffer, asked about by every path that copies text

`Buffer::private` (doc). A private buffer:

- has **no history row** — `draftable` is false, and a row it had is
  dropped the moment it becomes private — so nothing of it is in the
  store, not even a clean tree (a tree's edits are text);
- is **no one's moment**: no `file` row, no dwell, no location, so its
  name is not in the memory either;
- is **not in a session**: a private scratch does not come back, a
  private file comes back only as a path, reopened like any other
  closed file;
- is **not sent to a language server** — the one path by which its
  text leaves the process;
- yanks into the register as a **secret** (Decision 2), never to the
  system clipboard.

It keeps its grammar: the tree-sitter thread's copy shares the text's
blocks (a clone of the piece tree is a snapshot) and goes when they
do. What is private: a file a mask rule names (Decision 3 — its history
would carry what the mask hides), a vault scratch or vault file
(Decision 5), a `--wait` open of a file under the temp directory
(`secrets.private_temp`, on), what a plugin marks
(`kawoosh.buf.set_private(h)`, `open_scratch { private = true }`), and a
multibuffer while it shows a private buffer's lines (Decision 9).

*Beat:* a per-kind setting on each path (`history.skip`, `lsp.skip`, a
clipboard filter) — five settings to get right for every secret, and
one forgotten is a leak. One flag, asked everywhere, is the model.

### 2. A secret in the register is put once, then forgotten

A `Moment` has `secret: bool`. A yank or delete from a private buffer is
one, and so is anything put *into* a private buffer (a password pasted
from elsewhere is evidently one). A secret moment:

- is never a `text` row and never on the system clipboard;
- is taken out of the register by the put that uses it — `p` once, and
  the `"` register is what it was before the secret;
- is forgotten after `secrets.forget_secs` (30, the password managers'
  figure) if nothing puts it, an `Alarm` bringing the frame that does;
- is zeroed when it goes (Decision 6).

*Beat:* a count per moment (`uses = 3`) — nobody knows the count ahead
of time, and a second paste is a second `y` away.

*Amended 2026-09-27, from use:* "put once" read every put, so inside a
vault `yyp` spent the yank and a second `p` put whatever the secret had
covered — an older text nobody asked for, which read as "yanking does
not work here". Once is about a secret *leaving*: a put into a private
buffer spends nothing, since nothing leaves, and a text put there from
outside is a secret from then on (masked in a list, off the clipboard,
forgotten on the timer) rather than forgotten at once. A secret put
into any other buffer is still put once. And the register a secret
went from is *spent* until something is taken: `p` says the secret is
gone rather than putting the text under it; `[p` and the memory pane
still reach the older ones.

*Relaxed 2026-10-16 by Decision 7:* put once is gone; a secret is put
as often as wanted anywhere in kawoosh.

### 3. Masks are rules the engine draws, not a hook

`secrets.masks` is a table of named rules in the engine's settings
layer, a user's added or replaced by name as every other table is:

```lua
secrets = {
  masks = {
    env   = { files = { ".env", ".env.*", "*.env" },
              pattern = [[^\s*(?:export\s+)?[A-Za-z_][A-Za-z0-9_.]*\s*=\s*(.+)$]] },
    vault = { files = { "vault.yml", "vault.yaml", "*/vault.yml" },
              pattern = [[^\s*[A-Za-z_][A-Za-z0-9_]*\s*:\s*(.+)$]] },
    pem   = { from = [[-----BEGIN [A-Z ]*PRIVATE KEY-----]],
              to = [[-----END [A-Z ]*PRIVATE KEY-----]] },
    secret = { language = "secret", pattern = [[^[^:#]+:\s*(.+)$]] },
    token  = false,   -- a bundled rule switched off
  },
}
```

A rule applies by `files` (globs on the path's name, or on the whole
path when the glob has a `/`), by `language`, or everywhere when it
names neither. A line rule's `pattern` is a regex whose first group is
the masked part (the whole match without one); a block rule masks the
lines from a `from` match to the next `to` match. The engine scans a
buffer a rule applies to once per version, whole, and caches the ranges
— the texts rules apply to are small, and a scan per row would miss a
block's start above the screen. A file a `files` rule names is private
(Decision 1); a rule that applies everywhere (a PEM key pasted anywhere)
masks without making the buffer private.

A masked range is drawn as eight `•` whatever it hides (a secret's
length is not the screen's business either — changed from one per
character after use, 2026-09-24), through
the fold table the markdown buffer draws with (`Drawn::folded`): the
row's text never holds the secret, so kui never shapes it and its glyph
cache never keeps it. The caret moves through a mask as through a
fold. `zv` (`secret reveal`; vim's "open the folds to view the
cursor", and a mask is drawn as one) shows the mask under the caret —
that one, not the line — for `secrets.reveal_secs` (10); the caret
leaving it hides it again. Never the caret's line raw, as the markdown
buffer does: the point of a mask is a screen someone else can see.

A plugin gets `kawoosh.buf.mask(ranges[, buffer])` for ranges it
computes itself (replacing its earlier ones), and rules are settings,
so a plugin adds one with `kawoosh.set`.

*Beat:* a Lua function per edit returning ranges — a file buffer has
no edit hook for Lua (`on_change` is a scratch's), and a row drawn must
not wait on Lua. *Beat:* a per-row scan — misses a block above the
screen, and the ranges would be found again on every frame for nothing.

### 4. The pasteboard's marks and the terminal's password prompt

macOS password managers mark what they copy (`org.nspasteboard.
ConcealedType`, `TransientType`). kui reads the marks with a paste's
answer (kui F84): a concealed or transient text kawoosh only *looked
at* is not the register's newest at all; one explicitly pasted is put,
as a secret (Decision 2).

A terminal at a password prompt — `sudo`, `ssh`, `gpg` asking — has
turned the pty's echo off with the line discipline still canonical,
which the pty's termios says (`portable_pty`'s `get_termios`; iTerm2's
rule for its key icon). While a pane is at one, its title shows a key
and, while it has the keys, macOS's secure keyboard entry is on (kui
F85), so no other process reads the typing. A paste into a terminal is
already never remembered (`Terminal::paste` writes the pty).

### 5. The vault scratch and the vault file are `secrets.lua`'s

`:secret NAME` opens a scratch `*secret NAME*` in the `secret` language
(key: value lines, the values masked by the `secret` rule), private:
nothing of it is ever on disk, and it is gone when the window is.
`:secret` bare lists the open ones.

A file whose first line is `$ANSIBLE_VAULT;` is opened through
`kawoosh.on_open` (the way `dir.lua` takes a directory): `ansible-vault
view` fills a private scratch named for the file, in YAML, and its
`on_write` runs `ansible-vault encrypt --output PATH -` with the text on
stdin. The vault's password is `ANSIBLE_VAULT_PASSWORD_FILE` or
`secrets.vault_password_file`; without one the plugin says so and opens
the ciphertext as it is, since `kawoosh.spawn` gives the tool no tty to
ask on.

### 6. What "cleaned from RAM" can promise

The text blocks the piece tree is made of are zeroed when the last
thing holding them lets go (`text_buffer::Block`'s `Drop`, every block
and not only a private buffer's — a memset of what is freed, where the
allocation cost as much), which covers the undo states and the parser's
copy, since both share the blocks. A secret moment's text is zeroed
when it is forgotten. A mapped file's pages are the file's, on disk
already.

What it cannot promise, said plainly in `:help`-shaped words in the
plugin's header: a copy an allocator left behind when a `Vec` grew, a
`String` made on the way (a search's match, a `.` repeat's typed text,
a macro), the swap. The mask is why the text is never in kui.

### 7. A secret is kawoosh's alone, not put once

Asked 2026-10-16, from the todo: yanking in a secret buffer is "often
unusable". It was so by Decision 2 even as amended: a block yanked from
a vault went into a playbook once and was gone — the second `p`, a
`P` over a second selection, a `.` repeat all said *the secret is
gone* — and one not put within 30 seconds of the yank was forgotten,
while the user was still finding the file it was for. A key name or a
whole structure copied out of a `vault.yml` is a secret too, since the
yank cannot tell a value from its key, so the rule bit on the ordinary
edits of a secrets file, not only on passwords.

What Decision 2 guards against is a secret *leaving the process*: the
store's `text` rows, the system clipboard (every program, and any
clipboard manager), a plugin's Lua strings (never zeroed). A put into a
buffer is not that: the text lands where the user put it, and the
buffer it lands in is the user's choice — a private one keeps it as
private, an ordinary one has it as it has anything typed there. Put
once spent the register and bought nothing the put itself had not
already done. So:

- **a put spends nothing**, into any buffer: `p`, `P`, a visual put, a
  count, `.` — as for any text;
- **the timer counts from the last use**: a secret is forgotten
  `secrets.forget_secs` after it was last yanked or put, not after the
  yank, and the default is 120 seconds, not 30 — the password managers'
  30 is for the system clipboard, where every program reads; this is
  the process's own register;
- **unchanged**: never a `text` row or a moment (not its hash, not the
  yank of its file), never on the system clipboard, withheld from Lua
  (`kawoosh.register` nil, `kawoosh.memory()` masked), masked in the
  memory pane and stepped over by `[p` `]p`, zeroed when it goes; a
  text put into a private buffer becomes a secret; a concealed paste
  (Decision 4) is a secret, put again by `p` until its time is up.

*Beat:* put once to a buffer that is not private, as Decision 2's
amendment had it — the put is the user's own act on their own buffer,
and spending the register is what made yanking "unusable". *Beat:* a
secret on the system clipboard marked concealed
(`org.nspasteboard.ConcealedType`, which managers skip) — still every
program's to read while it is there, and writing the mark is a kui
round of its own (F84 reads it); asked for, it is the next step. *Beat:*
no timer at all — a secret then lives as long as the register, through
a lunch break, for a put into the wrong buffer much later.

### 8. ⌘C is the one way a secret reaches the clipboard

Asked 2026-10-16, from the todo: "i need a way to yank into clipboard
from secret buffer. now i need to yank, paste into normal buffer where
it is visible, then yank normally." Decision 7 left this as its next
step. The workaround was worse than any copy: the secret was drawn
unmasked in an ordinary buffer, kept in its history, and then went
onto the clipboard unmarked anyway.

`yank clipboard`, on ⌘C, in visual and normal mode: in visual mode
the selection yanked, as `y` does; in normal mode the newest yank.
When that is a secret it goes to the clipboard through kui's
`set_clipboard_secret` (F84), marked `org.nspasteboard.ConcealedType`
and `TransientType`, as a password manager marks what it copies, so
clipboard managers neither show nor keep it. An ordinary text goes on
as any yank does. Everything else in Decision 7 holds: a plain `y` and
every other path keep a secret inside kawoosh, and the register keeps
it a secret after the copy.

- **Asked each time, by a key of its own.** ⌘C already meant "copy to
  the clipboard" on a Mac, and nothing reaches the clipboard unless it
  is pressed. That keeps Decision 1's single flag: no setting lets
  every secret yank out.
- **No clear on a timer.** Password managers clear the clipboard after
  thirty seconds if it still holds what they put there. kui cannot say
  whether the pasteboard changed since (no change count), so a timed
  clear could wipe what the user copied in another program meanwhile.
  The marks keep it out of every manager's history; it stays on the
  clipboard until the next copy. A clear needs kui to report that
  count, which is a kui round of its own.

*Beat:* `"+y`. kawoosh names no registers but `"_` (the black hole),
and the clipboard is not a register here. *Beat:* a confirm on each
copy. The key is the confirmation.

### 9. A buffer's text is drawn as its pane draws it, wherever it is drawn

Audited 2026-10-16, after the picker's preview drew a vault's values
(Built, "From the todo"): every place outside a buffer's own pane that
draws its text drew it raw — the project search's results and a review
(multibuffers mirror their files' text, and drew it with their own
masks, which were none), the base's lines a review shows over a hunk,
the undo pane's hunk and its change snippets, `hunk preview`'s
`*hunk*` pane, `:memory jumps`' rows, the changed files' diff preview,
`*` on a masked value (its pattern in the message), a visual selection
seeding the project search's field, `:hex`'s text column, and a mark's
record (`:memory marks`, and the store). Each had made its own copy of
the lines, and the mask is the pane's. So the rule is one:

- **The text a buffer has now is drawn through its masks**, as its pane
  has them — a rule by its files, a rule a plugin asked for, ranges a
  plugin gave. Lua reads it so with `kawoosh.buf.shown_lines`, never
  `lines`, `text` or `slice` when it draws; the shell with
  `Kawoosh::shown_line`. A **multibuffer** is drawn with each excerpt's
  source's masks carried to where its lines are (`mirrored_masks`), and
  is **private while it shows a private buffer's lines**, so a yank from
  the results is a secret as one from the file is (Decision 1's list
  grows by one). An excerpt not synced yet, or whose file is gone, is
  masked whole, line by line.
- **A buffer's text that is not its text now** — a base, an undo state,
  a revision, the file on disk — is masked by the rules that apply to
  the buffer (or, for a file not open, to its path), scanned over the
  text shown (`Rules::scan_for`, `Kawoosh::shown_lines_of`,
  `kawoosh.secrets.mask_text`) — scanned whole where the whole is at
  hand, so a block is found though its first line is far from the
  lines drawn: an undo state is masked over the state's text
  (`Kawoosh::shown_state_lines`), a hunk's sides each with its context
  (a key's body replaced under an unchanged `BEGIN`), a commit's
  section by every path its headers name, the old and the new (a
  renamed `prod.env` keeps its rules). Every line is kept — a block's lines
  each a stand-in, as the pane draws them — so a hunk's line numbers
  still index the masked lines, and a diff is taken over the real texts
  and drawn from the masked ones (a value that changed shows as a
  changed line, `-KEY=••••••••` over `+KEY=••••••••`). Ranges a plugin
  gave are the one thing no rule finds again in another text: a private
  buffer with any is masked whole, line by line, there.
- **What cannot be masked is not drawn.** A change snippet in the undo
  pane is no line a rule could read: in a private buffer, or one with
  masks, it is a stand-in. A byte's cell has no room for one: `:hex` refuses a private
  file (`:mask private off` first). A pattern `*` took from a private
  buffer, or from under a mask in any buffer (a key in a plain file),
  is the text: the search is marked secret (`Search::secret`) and every
  message shows `/••••••••`, the one a search walked on a thread too. A
  selection that touches a mask, in any buffer, does not seed the
  project search. A mark records its line's text in a
  moment: a private buffer keeps no marks, and one made before the file
  was private keeps the text it had.

*Beat:* each place masking for itself, as the picker had — it is how
the eleven were missed; a new view that draws a buffer reaches for the
one door. *Beat:* refusing a private buffer everywhere outside its pane
(no search result, no undo pane) — the search over a project with a
`.env` in it is when the user needs to see its keys most, and the
masks already say what to hide. *Beat:* a yank from a multibuffer a
secret only over a private excerpt's lines — the register would have to
ask where every range came from, and Decision 1 is one flag asked
everywhere; the results of a search that found a `.env` keep yanks off
the clipboard until it is closed, the safe side.

*Reviewed 2026-10-16*, after the merge: the hunk preview's old side
was scanned without its context, the undo pane's sides without theirs,
a renamed file's old side by the new name's rules, a colour the user's
config asked for hid `vcs show`'s headers (`git show` runs with
`--no-color --no-ext-diff --no-textconv` now), a C-quoted name was read
with its quotes, the search walked off the frame said its pattern, and
`*` and the seed asked whether the buffer was private rather than
whether the text was masked. Each closed, with a test. A multibuffer's
carried masks are kept by its version, its sources' versions, the
settings' and a generation the secrets move (`MirrorKey`), not worked
out each frame.

## Deliberately not

- **Not encryption at rest.** A vault scratch is never written; a
  vault file is the tool's to encrypt.
- **Not a password manager.** No generation, no sync; `:secret` is a
  place to hold what you are about to paste.
- **Not a lock screen.** A revealed mask is shown; the screen is the
  user's.

## Build order

1. The private buffer and the register (Decisions 1, 2, 6): the flag,
   every path's guard, `Moment::secret`, the put-once and the timer,
   blocks zeroed, `secrets.private_temp`.
2. Masks (Decision 3): the rules, the scan and its cache, the fold, the
   reveal, `kawoosh.buf.mask`.
3. `secrets.lua` (Decision 5): `:secret`, the vault file.
4. kui F84 and F85, then Decision 4 in kawoosh: the concealed paste,
   the terminal's prompt.

## Risks

- **A path forgotten.** A new way text leaves the buffer (a future
  plugin door, a new system) must ask `private`. The test for each of
  Decision 1's paths pins them; a new one adds its line.
- **A rule too broad.** `env` masks every value in a `.env`, including
  a `PORT=3000`; that is the safe side, and a user narrows it by name.
- **The regex crate on a big file.** A rule that applies everywhere
  scans every buffer once per version; `pem` is anchored and cheap. A
  rule without `files` or `language` on a forty-megabyte log would be
  slow, which is why the scan skips a buffer past a size
  (`secrets.scan_max_kb`, 1024).

## Built

2026-09-23, in four commits and two kui rounds, in the build order's
shape. Where it departed:

- **The rules live in the editor crate** (`editor/src/masks.rs`), not
  the shell: `kawoosh.secrets.mask_text` answers from Lua without a
  round trip, and the shell and Lua read the same table the same way.
- **A rule can be asked for by name** (`kawoosh.buf.mask_with`), which
  the note did not have: a decrypted vault is a scratch with no path,
  so no `files` glob could name it.
- **The picker masks too**: a grep row, a file's preview and a private
  buffer's lines go through `mask_text`, since a list of a `.env`'s
  lines drew them raw.
- **`mask private`** (`:mask private [on|off]`) and **`mask reveal`**
  (`zv`) are the engine's commands; `:secret` is the plugin's.
- **The register's secrets are withheld from Lua** — `kawoosh.register`
  is nil for one, `kawoosh.memory()` shows it as the pane does — since
  a Lua string is never zeroed.
- **`kawoosh.spawn` grew `stdin`** and **`kawoosh.fs` grew `head`**: the
  vault's plaintext goes to `ansible-vault encrypt -` on stdin, and the
  opener, asked about every path, reads fifteen bytes.
- **A long setting's value is cut** in the settings tab: the mask
  patterns were the first values wider than the tab, and the table's
  aligned columns wrapped every path a letter at a time.
- **kui F84** answers a paste with the pasteboard's `concealed` and
  `transient` marks (`InputEvent::Paste`), read on macOS and Windows,
  not yet on Linux; **kui F85** is `Ui::secure_input(on)`, declared per
  frame, balanced by the runner. The secure input's effect could not be
  seen on the machine it was built on (another process held it on
  throughout); the calls link and run.

### After a day of use, 2026-09-24

- **Two rules more**: `key` (`*.key`, every line but a comment — a
  `master.key`) and `vault_pass` (`.vault_pass`, whole).
- **One stand-in length**, eight `•`, where it had been a `•` per
  character up to twelve.
- **The caret on a mask** covers the stand-in (`secrets::mask_at`): a
  byte inside a mask maps to where the stand-in starts, and the block
  caret had been drawn zero wide there — invisible but at a line's end.
- **A vault that hung at `decrypting…`**: `ansible-vault` ran from
  kawoosh's working directory, found no `ansible.cfg`, and asked for a
  password on the terminal kawoosh was started from. The tool now runs
  where the nearest `ansible.cfg` above the vault is, and every process
  a plugin spawns is in a session of its own (`spawn::session`:
  `posix_spawn` with `POSIX_SPAWN_SETSID`), with no controlling terminal
  to ask on, so a prompt fails at once. What it
  said is a toast that stays, with *Retry*; the ciphertext opens in
  the pane. The hang had a second half in the engine: `kawoosh.open`
  asked the openers twice, so the fallback's own open was taken by the
  vault's opener again, for ever (`Kawoosh::open_file`).
- **`:!CMD`** runs the line in a terminal below, `%` (`%:h`, `%:t`)
  the file quoted for the shell — and in the decrypted scratch the
  vault, which the scratch stands for (`Buffer::about`, `open_scratch
  { about = }`): `:!ansible-vault view %` asks for the password where
  it can be answered.

### `*.secret.json`, 2026-10-08

- **One rule more**, `json`: `*.secret.json`, `*.secrets.json` and
  `secrets.json` are private and masked by value — a pair's, an
  array's bare one, an inline array or object whole — with the keys
  kept, so the file's shape still reads. A line that opens a nested
  `{` or `[` is not a value. The cost was the rule: a `files` rule is
  what makes a file private.

### From the todo, 2026-10-16

- **A vault written was left empty.** `:w` in a decrypted vault
  encrypted the text over the file — the file was right — and then
  refilled the scratch with it from the tool's exit, through an upvalue
  `write_back` had set to `nil` the moment the tool started (to let the
  string go): the refill was `open_scratch { text = nil }`, an empty
  buffer, clean. The refill was the wrong model too: it replaced the
  text, so an edit typed while the tool ran was lost with the undo
  history. Now the exit only says the write landed —
  `kawoosh.buf.saved(text[, buffer])`, a door for any hook whose write
  ends after its `on_write` answered: saved if the buffer still holds
  what was written, line by line, else left modified by what was typed
  meanwhile; its text is never touched.
- **A key commented out stays a secret.** `# ansible_become_password:
  secret_password` was drawn raw while the same line uncommented was
  masked: a commented-out key is a value kept for later, as secret as
  it was. `env`, `vault` and `secret` take `#`s and spaces before the
  key (`^\s*(?:#+\s*)?…`). A comment that reads as a key — `# Note:
  rotate monthly` in a vault — is masked too, which is the safe side
  (Risks: a rule too broad). `key` already masked every line but a
  comment; JSON has none.
- **Yanking relaxed** (Decision 7): a secret is put as often as wanted
  into any buffer, and forgotten 120 seconds after its last use; still
  never on the clipboard, in the store or in Lua.
- **`-` from a vault's scratch listed the working directory.** The
  scratch is no file, so `dir`'s up read no path and fell back to the
  cwd. The scratch does stand for one (`about`, which `%` names), and
  `kawoosh.buf.file([buffer])` is now that door for Lua: the path, else
  what the buffer stands for. `-` lists the vault's directory with its
  line under the caret.
- **A secret onto the clipboard** by ⌘C, marked concealed (Decision 8).
- **A picker drew an open vault's values.** The buffers picker's
  preview and the lines picker's rows masked a private buffer by the
  rules its path and language name — a vault's scratch has no path, its
  language is YAML, and its rule is the one `secrets.lua` asked for by
  name (`mask_with`), which the lookup never saw. The values were drawn,
  and so were in kui. Now Lua reads an open buffer as its pane draws
  it: the shell puts every buffer's masks in the editor once a frame,
  at their version (`Editor::masks`, `Kawoosh::publish_masks`), the
  snapshot carries them, and `kawoosh.buf.shown_lines(from, to[,
  buffer])` gives the lines with what they hide as `••••••••` — a rule
  by files, a rule asked for, ranges a plugin gave (`kawoosh.buf.mask`),
  all as the pane has them. A private buffer whose masks are another
  version's (edited since this frame's) is masked whole, line by line.
  The picker reads every buffer through it.

### Every view audited, 2026-10-16

Decision 9, from an audit of every Lua view and plugin and the engine's
own views. Changed: the multibuffers (masks carried, privacy too), the
review's base lines (`vcs.layout`), the changed files' preview (which
also asked for its diff again for ever: a reload makes the items anew,
and the diff was kept on its item; now it is kept per picker by path),
the undo pane, `hunk preview`, `:memory jumps`, `*`, the project
search's seed, `:hex`, marks. `kawoosh.secrets.mask_text` keeps every
line, a block's too, where it had joined a block into one stand-in.
Checked and left: `pairs.lua`, `node_actions.lua`, `timed.lua`,
`man.lua`, `dir.lua` (read their own buffers' text to act on it, not to
draw it; `dir`'s preview already masked); `vcs.lua`'s blame (the text
goes to `git blame --contents -`, the column draws labels); the picker
(`shown_lines` already); the inspector (kinds and positions, no text);
breadcrumbs (symbol names — a YAML's keys, never its values); hover,
diagnostics, references and completion (a private buffer is never a
server's, Decision 1). `kawoosh/tests/secrets_drawn.rs` pins each.

`vcs show`'s `*show*` buffer, the backend's patch, is masked by its
files' headers (`vcs.shown_patch`): each file's section by the rules of
the file its `diff --git`/`diff --cc`/`Index:` and `+++` lines name (its
`---` for a file deleted), the section's old side (context and `-`
lines of all its hunks) and new side (context and `+`) each scanned
whole, so a block added to a file is found across its lines; a hunk's
lines are counted by its `@@` header, so a line taken away that reads
`--- …` is no header. The message and the stat are kept: a commit's
message is no file's. `git show` is run with `a/`/`b/` prefixes and
`core.quotePath=false` whatever the user's config, so the path is the
file's. The `vcs log` picker's preview is the commit's summary alone;
fossil's `diff --checkin` is the same `*show*` buffer.

Not done, and known: a hunk's sides are scanned alone, a `*show*`
section's sides a hunk's run together: a `pem` block whose `BEGIN` line
is outside the hunk and its context is not found in it (the `key` rule,
a line rule, masks a private key file whole either way).

### Line moves and a value over lines, 2026-10-18

From the todo: "`<A-j/k>` line moves inside dotenv files flicker both
lines", in a `.env` whose value is quoted over several lines
(`NFQWS2_OPT="` … `"`).

- **No frame drew a mask's value.** A buffer's rule masks are scanned
  whole for each version before its rows are drawn (`masks_of`, kept
  per version), so the frame that draws an edit draws it with that
  edit's masks: checked frame by frame across `<A-j>` `<A-k>`, `dd`
  `p` `P`, `u` `<C-r>` and a paste. The scan is ~8 µs a version for a
  `.env` the user's size, 6.7 ms at `scan_max_kb` (1 MiB): no reason
  to carry the rule masks through the journal instead.
- **The flicker was the colours**: the move rewrote both lines in one
  edit and their syntax runs went with it, both lines plain until the
  parser answered — every language, seen first in `.env` because its
  grammar had just been installed. An edit now says what it moved, and
  the runs go with the text (core.md Decision 1, `Move`).
- **What was drawn was the value's other lines.** `env` was a line
  rule: on `NFQWS2_OPT="` it masked the quote, and the lines inside the
  quotes — the value — matched no key and were drawn as they are, on
  every frame. `env` is a `quoted` rule now: its regex matches a key up
  to where its value starts, and the value is read by its quotes, not
  by a pattern — one opening with `"` runs to the first `"` no `\`
  escapes (an escaped newline, a shell's continuation, among them), one
  with `'` or `` ` `` to the next such quote, either over lines and then
  to the end of the closing line; any other to its line's end. A
  commented key (`# KEY="`) opens one as the key does. An opening quote
  never closed masks to the end of the file — the safe side.

*Beat:* rules of their own (`env_quoted`, built first) — they named the
same files, so `secrets.masks.env = false` no longer made a `.env` plain
or not private: one rule for one kind of file is what a user switches
off by name. *Beat:* a `from`/`to` block pair of regexes (built second,
taken back on review) — a regex cannot count backslashes: `KEY="a \`
never opened (`\.` takes no newline), and a closing `\\"` was missed,
the block running on to the next value's opening quote and leaving
that value's lines raw. A scanner of a few lines reads quotes as a
shell does.

- **A line read alone, from a review.** A list that shows a file's
  lines one at a time — the grep picker's rows, a review's old lines
  without the old text at hand (`old_shown`), `vcs show`'s hunks — drew
  the inside of a quoted value raw: read alone, `--hostlist=…` is no
  key's. Where the line's place is known and the file is on disk (a
  grep row: `rg` read the disk), the file is read, within
  `secrets.scan_max_kb`, kept per path by its length and time, and
  scanned whole (`mask_text(text, path, { line = N })`,
  `Rules::mask_line`): a line a mask from above runs over is a stand-in,
  any other masked as a line of its own. Where it is not — a hunk of a
  revision, the old side of a change — the text is a part
  (`{ part = true }`, `Rule::scan_part`): before its first key a
  `quoted` rule masks every line whole, since any may be inside a value
  opened above it, and a block whose `to` comes before any `from` is
  masked from the start. `vcs show` reads each section's sides whole,
  as before, and each hunk as a part; a line the whole side left as it
  is takes its hunk's. Only for a file a rule of its own spans lines in
  (`spans_lines`): a `.env` is read for its grep rows, a `.rs` never.
  The safe side: a comment before a hunk's first key is masked too.

