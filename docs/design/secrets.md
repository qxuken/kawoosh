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
(`secrets.private_temp`, on), and what a plugin marks
(`kawoosh.buf.set_private(h)`, `open_scratch { private = true }`).

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

A masked range is drawn as `•`, one per character up to twelve, through
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
