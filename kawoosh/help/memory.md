# Memory, undo and secrets

Kawoosh remembers every text you yank, delete or paste in, every file you attend to, and every state a buffer has been through. This page covers the working memory and its pane, putting and the yank-pop, the system clipboard, the undo tree, and how secrets are kept out of all of it.

## The working memory

Every yank, delete, change and clipboard paste is a *text* the memory keeps, newest first, with where it came from and when. There are no numbered or lettered registers to manage: the register is simply the newest text, and everything before it is still there to put again.

To delete or change without keeping the text, name the black hole first: `"_dw`, `"_cc`, `"_x`, or `"_d` on a selection. What it takes goes nowhere: not into the memory, not onto the clipboard, and the register stays what it was. `"_` lasts one command, and `.` repeats it.

Texts survive a restart, for `memory.text.keep_days` days (7) and up to `memory.text.max_mb` megabytes (8). Set `memory.text.max_mb = 0` to keep them for the session only; nothing you copy is then written to disk. A text over 1 MiB is never written.

## Putting

| keys | what |
|---|---|
| `p` `P` | put the newest text after / before the caret (below / above for whole lines) |
| `p` on a selection | replace the selection; what it replaced becomes the newest text |
| `P` on a selection | replace the selection, keeping the register as it was |
| `]p` `[p` | right after a put: swap what was put for the next newer / older text, COUNT steps |
| `⌘v`, `<C-S-v>` (insert mode) | put the system clipboard at the caret |

The yank-pop (`[p` `]p`) walks the memory from the put you just made. The text you settle on becomes the register, so `p` puts it again, and one `u` takes the whole put back. Any edit ends the walk.

## The system clipboard

Every yank also goes onto the system clipboard. With `clipboard.system` on (the default), the other direction works too: when the window comes back to the front, or the keys come back to an editor pane from a terminal or another pane, whatever another program (or a terminal selection) put on the clipboard becomes the newest text, so a plain `p` puts it. Set `clipboard.system = false` to keep `p` to kawoosh's own texts; `⌘v` still pastes the clipboard.

Text that a password manager marks as concealed or transient is not picked up this way; pasted explicitly, it is treated as a [secret](#secrets).

## The memory pane

`:memory` (`<leader>mm`) opens the memory in a pane beside the buffer. It has views; `<Tab>` and `<S-Tab>` move between them, and `:memory VIEW` opens on one.

| view | rows |
|---|---|
| `texts` | what was yanked, deleted or pasted in, with the text under the rows |
| `files` | files and scratches attended, with visits, time spent, edits, and any unsaved draft (`:oldfiles` and `:browse` open this view) |
| `recent` | everything attended, in order, newest first: "where was I" (`<leader>ml`) |
| `jumps` | this tab's jumps, newest first, with how many `<C-o>` (`‹`) or `<C-i>` (`›`) away each is; `<CR>` goes there, `x` drops one (`<leader>mj`, `:jumps`; [editing](editing.md#jumps)) |
| `commands`, `searches` | command lines and searches |
| `pins` | pinned files (`<leader>mp`) |
| `marks` | marks |
| `all` | every kind of row |

The views show this workspace's memory (the focused tab's project) or the global one, every workspace's: `<C-a>` in the pane switches between them for the session, as does a click on **@workspace** or **@global** at the left of the pane's strip (the one on is filled), the head says which (`in ~/projects/foo` or `every workspace`), `:memory scope @global` sets one, and `:memory workspace` or `:memory global` opens the pane on one. `memory.scope` (`"workspace"`) is where it starts. `texts` are the same either way, since a yank is a yank anywhere, and `jumps` are always the tab's.

| keys in the pane | what |
|---|---|
| `<CR>`, `p`, a click | put the text in the pane you came from; open a file at its line; put a command on the command line |
| `y` | make the text the register, without putting it |
| `o` | go to where the row came from (a text's origin follows the edits since) |
| `x` | forget the row (a file's draft with it) |
| `m` | pin or unpin |
| `<C-a>` | this workspace's memory or every workspace's |
| `/` | filter the rows as you type; `<CR>` takes the row, `<Esc>` twice returns to the list with the filter kept, `<Esc>` in the list clears it |
| `j` `k` `gg` `G` `<C-d>` `<C-u>` | move |
| `q` | close; `<Esc>` hands the keys back to the editor pane |

From the command line: `:memory forget SUBJECT`, `:memory filter QUERY`, and `:memory clear` to forget everything (`:memory clear!` reverts the buffers holding drafts too). Pins: `<leader>ma` pins the buffer's file, `<A-1>`…`<A-9>` open a pin. `<leader>mf` lists the files attended, with their drafts.

How long the memory keeps things: `memory.keep_days` (90) for files and their histories, `memory.max_mb` (64) for the whole store, `memory.idle_secs` (60) for when time spent stops counting. `.` and macros are not part of the memory.

## Undo

Undo is a tree: an edit after an undo starts a branch, and the old branch is kept.

| keys | what |
|---|---|
| `u` | back one state |
| `<C-r>`, `U` | forward again, along the branch last taken |
| `g-` `g+` | the state made before / after this one in time, across branches |
| `<leader>u` | the undo pane (`:undo history`) |

The undo pane lists every state newest first, drawn as a graph, with the saved state named and the change each row made shown under the list. `<CR>` or a click puts that state back, whichever branch it is on. In the pane, `u` `<C-r>` `g-` `g+` step the buffer as they do in it, `q` closes, and `<Esc>` hands the keys back. The pane follows the editor pane that last had the keys.

A buffer's undo tree is kept across restarts, and so is unsaved text: `:q` keeps what is unsaved for next time, `:q!` discards it. The `files` view of the memory pane lists every draft.

## Secrets

Some text must not be remembered. A *private* buffer has no history or draft on disk, no row in the memory, is not restored with a session, is never sent to a language server, and never reaches the system clipboard.

These buffers are private:

- a file named by a mask rule's `files` (`.env`, `*.key`, `vault.yml`…, see below);
- a `:secret` scratch and a decrypted Ansible vault;
- a file opened through `$EDITOR` from the temp directory, while `secrets.private_temp` is on;
- any buffer after `:mask private on` (`:mask private` says whether it is; `off` undoes it).

**Put once.** A yank or delete from a private buffer is a secret: it is never on the clipboard, one put into another buffer uses it up, and it is forgotten after `secrets.forget_secs` seconds (30) if not put. After that `p` says the secret is gone rather than putting an older text; `[p` and the memory pane still reach the older ones. Puts inside private buffers use up nothing, and text put into a private buffer becomes a secret too. The memory pane masks a secret's text, and `[p` `]p` step over it.

**Masks.** In any buffer a rule applies to, the secret part of each line is drawn as eight `•`, whatever its length. `zv` shows the mask under the caret for `secrets.reveal_secs` seconds (10), until the caret leaves it. Masks also apply to the picker's previews and search results.

The rules are `secrets.masks`, named tables you can add to or switch off:

```lua
secrets = {
  masks = {
    token = { files = { "*.token" }, pattern = [[^(.+)$]] },
    env = false, -- switch off a shipped rule
  },
}
```

A rule applies by `files` (globs on the name, or the whole path when the glob has a `/`), by `language`, or everywhere. `pattern` is a regex whose first group is masked; `from` and `to` mask every line between two matches. Shipped rules: `env`, `vault`, `key`, `vault_pass`, `pem` (private keys, anywhere) and `secret`. A buffer larger than `secrets.scan_max_kb` is not scanned.

**`:secret NAME`** opens `*secret NAME*`, a private scratch of `key: value` lines with the values masked. It is never on disk and is gone when the window closes. `:secret` alone lists the open ones.

**Ansible vaults.** A file starting with `$ANSIBLE_VAULT;` opens decrypted in a private scratch, and `:w` encrypts it back. `ansible-vault` runs from the nearest directory with an `ansible.cfg`; the password file comes from there, `ANSIBLE_VAULT_PASSWORD_FILE`, or `secrets.vault_password_file`, and `secrets.vault_command` replaces the tool. If it cannot decrypt, the file opens as it is and a notice offers *Retry*; `:!ansible-vault view %` runs it in a [terminal](terminal.md), where it can ask for the password.

Kawoosh clears the text of closed buffers and forgotten secrets from memory, but cannot promise that no copy remains anywhere (a string made along the way, swap).
