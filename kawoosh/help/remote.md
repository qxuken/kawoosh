# Remote editing

How to edit on another machine over ssh: naming a host, the paths that point at it, what works there, and how `$EDITOR` in a terminal on the host opens files back in kawoosh. Nothing needs to be installed on the host; kawoosh uses your own `ssh` and its configuration.

## Naming a host

A remote machine is a *domain*: a name you give it in your settings, and the ssh host it connects to.

```lua
-- settings.lua
return {
  domains = {
    box = { ssh = "box" },            -- a Host from ~/.ssh/config
    lab = { ssh = "me@lab.local" },   -- or user@host
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

Connecting opens a terminal in the dock running ssh, so a password, a passphrase or a second factor is asked for where you can answer it. Once ssh is in, the dock steps aside and every later connection to that host (files, terminals, processes) reuses it without asking again. Files travel over SFTP, which your ssh server almost certainly already provides.

If the connection drops, the next thing that uses the host connects again. Quitting kawoosh closes its connections.

## What works on a host

- **Files.** `:e`, `:w` and the rest work as they do locally. A file changed on the host is noticed on the next check and read again if you have no unsaved changes.
- **The file manager.** `-` and `:dir` list a host's folders, and renaming, creating and deleting work there. Press `<C-l>` in a listing to read it again.
- **Terminals.** With the working directory on a host (`:cd box:~/proj`), a new terminal is a shell on the host in that folder. Tools (`:tool`, `<leader>tt`), `:compile` and processes a plugin starts run on the host in the same way.
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

This needs `bash` on the host, and the host's ssh server must allow port forwarding (`AllowTcpForwarding`).

See also: [files](files.md), [terminal](terminal.md), [settings](settings.md).
