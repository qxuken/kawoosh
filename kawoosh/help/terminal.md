# Terminals

A terminal pane runs your shell, or any program, inside kawoosh, beside your buffers. This page covers opening one, which keys reach the shell, scrollback and copy mode, opening paths from the output, shell integration, and `$EDITOR`.

## Opening a terminal

| how | what |
|---|---|
| `:terminal`, `:term` | a terminal running your shell, in a column of its own (`terminal.place`) |
| `:terminal CMD` | a terminal running CMD |
| `<C-w>!` | a terminal, in the working directory (`terminal.place`) |
| `<C-w>.`, `:terminal here [CMD]` | a terminal where the pane in front is: a file's directory, the directory a listing or `:du` is on, a terminal's shell's |
| `t` in the launcher | a new split (`<C-w>v`, `<C-w>s`) or tab opens on the launcher; `t` there makes it a terminal |
| `:!CMD` | CMD in a terminal below; `%` is the file, `%:h` its directory, `%:t` its name, quoted for the shell |

A `:!CMD` pane stays when CMD ends, how it ended printed under its output (`[finished]`, `[exited with 2]`) and `DONE` on the status line. With no program left to hear them its keys are normal mode's: `r` runs the line again in the same pane and directory (`:terminal again`), `q` closes the pane, `<C-S-x>` makes the output a buffer. A shell, or a `:terminal CMD`, that ends takes its pane with it.

A terminal opened from another terminal starts in that shell's directory; otherwise it starts in the working directory. `<C-w>.` starts it where you are instead: beside the file, in the listed directory. A plugin's pane says where it is with `here` in `kawoosh.view`'s options ([lua](lua.md)). Set `layout.new_pane = "terminal"` to make every bare split a terminal without asking.

The shell is `terminal.shell`, or `$SHELL` when that is empty (`/bin/sh` without one, `%ComSpec%` on Windows). When a session is restored, each shell pane is started again in the directory it was left in; a terminal started with `:terminal CMD` is not, and scrollback is not kept.

A terminal whose directory is on a remote host runs its shell there over ssh, with `$EDITOR` still reaching this window. See [remote](remote.md).

## Which keys are the shell's

Almost every key goes to the program in the terminal, `<C-w>`, `<C-l>`, `<C-r>` and `<Esc>` included. What kawoosh keeps:

| keys | what |
|---|---|
| `<C-\>` then keys | normal mode's keys, the which-key showing what can follow: `<C-\><C-w>l` a pane move, `<C-\><Space>f` the files, `<C-\>:` the command line, `<C-\><Space>mm` the memory |
| `<C-\><C-n>` | copy mode (below), as in vim |
| `<C-\><C-\>` | sends `<C-\>` itself to the program |
| `<C-\><Esc>` | nothing: the keys are the program's again |
| `<C-\>r` | raw on or off for the program in front (`:terminal raw`): every key but `<C-\>` {{mac:and ⌘ }}goes to it |
| `<C-S-h>` `<C-S-j>` `<C-S-k>` `<C-S-l>` | focus the pane left, below, above, right |
| `<A-S-h>` `<A-S-l>` `<A-S-j>` `<A-S-k>` | the pane narrower, wider, shorter, taller |
| {{mac:`⌘1`…`⌘9`, }}`<C-S-1>`…`<C-S-9>` | the Nth column (or pane) |
| `<C-Tab>` `<C-S-Tab>` | the next and previous tab |
| {{mac:`⌘=` `⌘-` `⌘0`}}{{pc:`<C-=>` `<C-->` `<C-0>`}} | font bigger, smaller, back to the setting |
| {{mac:`⌘v`, }}`<C-S-v>` | paste the clipboard, bracketed when the program asks for it |
| `F12` | the devtools |

Any Ctrl+Shift or Alt+Shift chord{{mac:, and any ⌘ chord,}} runs its normal-mode binding instead of reaching the shell. {{mac:A ⌘ chord bound to nothing does nothing — but for ⌘⌫ ⌘⌦ ⌘← ⌘→, which a shell gets as `<C-u>` `<C-k>` `<C-a>` `<C-e>` (delete to the line's start, its end, go to its start, its end), and unless the program speaks kitty's keyboard protocol (below). ⌥⌫ is Alt+Backspace, every shell's delete-word. }}Your own normal-mode maps on such chords work from terminals too.

Programs that speak kitty's keyboard protocol — neovim, helix, kakoune, fish 4, nushell, yazi — are sent every key as it is: `<C-i>` apart from Tab, `<C-S-l>` apart from `<C-l>`, Esc on its own, the keypad apart from the main keys, and, when they ask, key releases and a lone Shift. {{mac:A ⌘ chord kawoosh does not bind reaches them as a Super chord (`<D-j>` in neovim). }}The keys kawoosh keeps above are still its own, and so is {{mac:⌘-click}}{{pc:Ctrl-click}} on a link, even in a program that reads the mouse.

**Raw.** In a raw pane kawoosh keeps only `<C-\>`{{mac: and the ⌘ chords it binds}}; everything else — `<C-S-h>`, `<C-Tab>`, `<S-PageUp>`, F12 — goes to the program, and the status line says `RAW`. `<C-\>r` turns it on or off for the program in front: raw set at the shell's prompt stays through the commands you run from it, and the next full-screen program starts as `terminal.raw` says. `terminal.raw = { "nvim", "hx" }` makes a pane raw whenever one of those programs is in front. (On Windows only `<C-\>r` does it: the program in front is not known there.)

Mouse: the wheel scrolls through history. Dragging selects text (a double click a word, a triple click a line){{mac:, and `⌘c` copies it}}. The middle button pastes the clipboard, as {{mac:`⌘v`}}{{pc:`<C-S-v>`}} does, and the right one opens a menu with Copy, Paste and Select All. When a full-screen program asks for the mouse it gets the clicks and drags of every button, the right one included (tmux's and htop's menus, a file manager's); hold Shift to select anyway.

## Scrollback

A terminal keeps `terminal.scrollback` lines of history (10 000 by default; a smaller number drops the rest at once).

| keys | what |
|---|---|
| `<S-PageUp>` `<S-PageDown>` | a page back into history, a page toward the prompt |
| `<S-Home>` `<S-End>` | the top of history, back at the prompt |
| {{mac:`⌘↑` `⌘↓`, }}`<C-S-Up>` `<C-S-Down>` | the previous prompt, the next one (needs [shell integration](#shell-integration)) |
| `<C-S-o>` | the last command's output onto the clipboard (needs shell integration) |

The `Shift` page keys go to the program instead while it has the whole screen (an editor, `less`, `htop`). Scrolled away from the prompt, the pane shows a scrollbar you can drag and a badge with the lines below it; click the badge to go back. Typing also jumps back to the prompt.

## Copy mode

`<C-S-x>` (or `:scrollback`) turns the terminal's whole scrollback into a buffer in the same pane, in the colours it was printed in, with the caret where the terminal's cursor was. The status line says `COPY`. Every editor motion, search, selection and yank works in it. `<C-\><C-n>` does the same, for vim hands.

To go back to the terminal: `q`, `<C-S-x>` again, or `<Esc>` once there is nothing left for it to clear (extra carets, a search's highlight). The shell keeps running meanwhile.

## Opening paths and links

Hold {{mac:`⌘`}}{{pc:Ctrl}} over the terminal: a URL, or a path that exists, is underlined under the pointer. Click it to open it: a URL in the browser, a file in an editor pane at the line and column it names (`src/main.rs:42:7`, `a.ts(3,5)`), a directory as a listing. A relative path is looked for in the terminal's directory first, then in the working directory. `gx` does the same in an editor pane.

Some programs print links on purpose, with text that need not be the address: `ls --hyperlink`, `gcc`, `delta`, `gh`, `rg --hyperlink-format`. Those come first. While one is under the pointer with {{mac:`⌘`}}{{pc:Ctrl}} held, its address shows at the bottom left of the terminal, so you see where a click goes before you click. A `file://` link opens in an editor pane, at the line its `#12` or `#L12` names; over ssh, a link names a file on that host. A link to a file on another machine says so and opens nothing.

## Images

Programs that speak kitty's graphics protocol draw images in a terminal pane: `timg -pk`, `chafa -f kitty`, yazi's previews, `viu`, plotting libraries with a kitty backend. An image sits at the cells it was drawn at and scrolls with them into the scrollback; clearing the screen or scrolling far enough takes it away. Over ssh an image travels in the output, so it works there too; a program that sends a file's name instead falls back to sending the image. Animation and tmux's way of passing images through are not supported yet.

## Shell integration

Two escape codes let the shell tell kawoosh more: OSC 7 says which directory it is in, and OSC 133 marks where each prompt, command and output begins. `:terminal integration` opens the lines to add for zsh, bash and nushell (nushell has both built in, behind two settings).

Without OSC 7 kawoosh asks the shell process for its directory, which works locally. Without OSC 133, prompt jumps and `<C-S-o>` say so and do nothing.

`<C-S-z>` in a terminal opens the directory jumps picker (zoxide's directories, or kawoosh's own memory of them without zoxide). Picking one types `cd 'PATH'` and Enter, when the shell sits at an empty prompt; that needs the OSC 133 marks. From a shell, `kawoosh pick dirs` prints the pick instead; the integration lines define a `zk` function around it.

## $EDITOR inside kawoosh

Every kawoosh terminal gets these environment variables:

| variable | value |
|---|---|
| `EDITOR`, `VISUAL`, `GIT_EDITOR` | `kawoosh-edit` |
| `KAWOOSH_SOCKET` | how the command-line tools reach this window |
| `KAWOOSH_BIN` | the kawoosh binary |
| `TERM_PROGRAM` | `kawoosh` (and `TERM_PROGRAM_VERSION`) |
| `TERM_APPEARANCE` | `dark` or `light`, as the window was when the shell started |

So `git commit`, `lazygit`'s `e`, `crontab -e` or anything else that runs `$EDITOR` opens the file in a real kawoosh pane, not a second editor nested in the terminal. `kawoosh-edit [+LINE] PATH…` waits until the buffer is closed: `:wq` (or `ZZ`) hands it back, `:q!` discards. A file opened this way from the temp directory is private (see [secrets](memory.md#secrets)).

The same socket serves the `kawoosh` command inside a terminal:

| command | what |
|---|---|
| `kawoosh edit [--wait] [+LINE] PATH…` | open the paths here; `--wait` (`-w`) returns when the buffer is closed |
| `kawoosh ex LINE` | run LINE as a `:` command |
| `kawoosh theme` | print `dark` or `light`, for a prompt hook to follow the theme |
| `kawoosh pick SOURCE [QUERY]` | the picker on SOURCE (`dirs`, `files`…); prints the pick, or exits 1 |

## Bells

`terminal.bell` says what a program's bell does: `sound` (the default) plays a short chime, at most one every quarter second; `visual` plays nothing; `off` ignores it. Unless it is `off`, a terminal that rings while its tab is not in front marks that tab until you visit it. `editor.bell` (off by default) makes the editor ring for its own failures, such as a search with no match.

## Password prompts

When a program in the terminal turns echo off to ask for a password (`sudo`, `ssh`, `gpg`), the pane's title starts with `password ·`, and while that pane has the keys kawoosh turns on macOS's secure keyboard entry, so no other program can read what you type. It goes off by itself when the prompt is answered. A paste into a terminal is never remembered in the [memory](memory.md).

## Settings

| setting | default | what |
|---|---|---|
| `terminal.shell` | `""` | the program a terminal runs (`nu`, `pwsh`, a path); empty for `$SHELL` |
| `terminal.scrollback` | `10000` | lines of history each terminal keeps |
| `terminal.bell` | `"sound"` | `sound`, `visual` or `off` |
| `terminal.escape` | `"<C-\\>"` | the key before normal mode's keys in a terminal; `""` for none, every key the program's |
| `terminal.place` | `"column"` | where `:terminal` and `:!` open: `column`, a column of its own, or `under` the focused pane in its column; `<C-w>s` then `t` is under either way |
| `terminal.raw` | `{}` | programs a pane is raw for while one is in front, such as `{ "nvim", "hx" }` |
| `editor.bell` | `false` | whether the editor rings for its own failures |
| `env.shell` | `""` | the shell whose `PATH` kawoosh borrows when started outside a terminal (from the Dock, Finder); read at startup |

See [settings](settings.md) for how to set them.
