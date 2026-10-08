# Remote editing

How to edit on another machine over ssh, or in a WSL distro on Windows: naming a machine, the paths that point at it, what works there, and how `$EDITOR` in a terminal there opens files back in kawoosh. Nothing needs to be installed on the other side; kawoosh uses your own `ssh` and its configuration, or `wsl.exe`.

## The machines kawoosh knows

A remote machine is a *domain*. Kawoosh finds them on its own:

- every `Host` in `~/.ssh/config` (and the files it `Include`s) that is a name rather than a pattern: `Host box` makes `box` a domain;
- on Windows, `wsl` is your default WSL distro, and every other distro is a domain under its name in lower case (`debian`).

`<leader>wh` (`:domain pick`) lists them all, how each one stands, and where each was found. Pick one and kawoosh opens a new tab on it: its working directory your home there, listed in the file manager. It connects first when it has to. `<C-o>` in the picker connects without opening a tab.

## Naming a host

You can also name a machine yourself in your settings, which wins over a machine found with the same name.

```lua
-- settings.lua
return {
  domains = {
    box = { ssh = "box" },            -- a Host from ~/.ssh/config
    lab = { ssh = "me@lab.local" },   -- or user@host
    deb = { wsl = "Debian" },         -- a WSL distro, by its WSL name
  },
}
```

Ports, keys, jump hosts and the agent are all ssh's business: whatever `ssh box` does in a shell, kawoosh does too. `ssh.command` names another ssh program, and `ssh.poll_secs` sets how often the host's open files are checked for changes (5 seconds by default).

## Paths on a host

A path on a host is spelled with the domain's name, a colon, and the path there:

- `box:/etc/hosts` is an absolute path on `box`.
- `box:~/proj` is under your home folder on `box`.

The name is two characters or more (letters, digits, `_`, `-`, `.`), so a Windows drive such as `C:\` is never read as a host. Use these paths anywhere a path goes: `:e box:~/proj/main.rs`, `:cd box:~/proj`, the file manager, `kawoosh.open` and `kawoosh.fs.*` in Lua. The tab strip shows a host's file as `box: main.rs`.

## Connecting

The first time you use a path on a host that is not connected, kawoosh connects to it and then does what you asked. You can also connect ahead of time:

- `:domain connect box` connects.
- `:domain disconnect box` disconnects.
- `:domain` lists your domains, how each one stands, and how many of its files are open.
- `:domain tab box` opens a new tab on `box`, as the picker does.

Connecting opens a terminal in the dock running ssh, so a password, a passphrase or a second factor is asked for where you can answer it. Once ssh is in, the dock steps aside and every later connection to that host (files, terminals, processes) reuses it without asking again. Files travel over SFTP, which your ssh server almost certainly already provides. A host without an SFTP server, such as an OpenWrt router (dropbear and busybox), still connects: kawoosh reads, writes and lists its files with plain shell commands (`cat`, `ls`), which is slower but needs nothing installed.

If the connection drops, the next thing that uses the host connects again. Quitting kawoosh closes its connections.

## What works on a host

- **Files.** `:e`, `:w` and the rest work as they do locally. A file changed on the host is noticed on the next check and read again if you have no unsaved changes.
- **The file manager.** `-` and `:dir` list a host's folders, and renaming, creating and deleting work there. Press `<C-l>` in a listing to read it again.
- **Terminals.** With the working directory on a host (`:cd box:~/proj`), a new terminal is a shell on the host in that folder. Tools (`:tool`, `<leader>t`), `:compile` and processes a plugin starts run on the host in the same way.
- **Language servers.** A host's file gets its language server on the host, started in the project's root there. The server must be installed on the host.
- **Finding files.** The file picker (`<leader>f`) walks the host's folder over SFTP. The walk stops at 5000 files, skips hidden files, `target` and `node_modules`, and is kept for the rest of the session; kawoosh says so the first time.
- **Sessions.** A session brings back a host's files and terminals without connecting at startup. They wait, titled with `:domain connect`, until you connect or use the host.

The project search (`<leader>ss`) does not search a host's files yet.

Project settings and project `init.lua` files on a host are not read: trust and settings stay on your own machine.

## `$EDITOR` from a terminal on the host

When kawoosh connects, it copies a small `kawoosh` command (a bash script) and a `kawoosh-edit` beside it to `~/.cache/kawoosh` on the host. A terminal on the host starts with `EDITOR`, `VISUAL` and `GIT_EDITOR` set to `kawoosh-edit`, and with a port forwarded back to this kawoosh. So:

- `git commit` in that terminal opens the commit message here, in kawoosh, as a host file. Write it and close the buffer (`:wq`), and git goes on.
- `$EDITOR notes.txt` opens the host's `notes.txt` in kawoosh.
- `"$KAWOOSH_BIN" edit FILE` and `"$KAWOOSH_BIN" theme` work as the local `kawoosh edit` and `kawoosh theme` do.

This needs `bash` on the host, and the host's ssh server must allow port forwarding (`AllowTcpForwarding`). A host without bash (busybox, as on OpenWrt) still gets its terminals; their `$EDITOR` is the host's own then. The same goes when `~/.cache` on the host cannot be written. If your shell's own configuration sets `EDITOR` (nushell's `env.nu`, a `.bashrc`), yours wins; `"$KAWOOSH_BIN" edit --wait` still opens here.

## WSL distros

On Windows, `wsl:~/proj/main.rs` is a file in your default distro, with nothing in your settings. Everything above works there the same way, with a few differences:

- **No password, no pane.** Connecting starts the distro if it is stopped and asks it where your home is; that takes a second or two the first time.
- **Files** go through Windows' own share of the distro (`\\wsl.localhost\Ubuntu\…`). A file kawoosh saves keeps its permissions. The file picker walks the distro like a local folder: no limit, and `.gitignore` is read.
- **Programs** (terminals, tools, `:compile`, language servers) run through `wsl.exe` with the `PATH` your login shell sets up, so a language server you installed in the distro with brew, cargo or npm is found.
- **One file, one buffer.** `\\wsl.localhost\Ubuntu\home\me\x` opens as `wsl:/home/me/x`, and `wsl:/mnt/c/Users/me/x` opens as `C:\Users\me\x`.
- **Closing a terminal** ends everything it was running in the distro, as closing any terminal does.
- **`$EDITOR`** in a distro's terminal runs the Windows kawoosh through WSL's interop, so it works whatever WSL's networking mode is.

See also: [files](files.md), [terminal](terminal.md), [settings](settings.md).
