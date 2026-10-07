# Domains: where a process spawns and where a path lives

Status: decided 2026-09-21 (roadmap step 9); built 2026-09-24 as
roadmap step 27 (see "Built" at the end for where the build departed);
ssh alone, as the roadmap's terminal track recommended, with wsl the
note after — decided 2026-10-07 with the domains' picker ("WSL, and
the picker", after "Built").
Systemic: it touches the io system, `fs`, the pty spawn, the LSP pool,
compile, the `$EDITOR` shim, sessions and the picker, so it is four
rounds rather than one (Build order). Each decision keeps the
alternative it beat. Companion to [mvp.md](mvp.md) Decision 3b (the
command socket, "the first door on the daemon's corridor") and the
roadmap's terminal track.

## The thesis

The cheap form exists today: `kawoosh.tool("box", { cmd = "ssh box" })`
is a terminal on a host. What it cannot do is everything that makes an
editor an editor there: `$EDITOR` from that shell opens the host's vim;
`:e` cannot name a file on it; `dir` cannot list it; the picker cannot
find in it; `:make` cannot run there. wezterm's multiplexer domain
answers the terminal half; vscode-remote answers the file half by
installing a server on the host. Kawoosh wants both halves without a
server on the host: the `ssh` binary the user has already configured
(hosts, keys, agent, jump hosts — all in `~/.ssh/config`), the SFTP
subsystem it ships for files, and a forwarded port for the shim to find
its way back.

A domain, then, is two things at once, and the design keeps them
together on purpose: *where a process spawns* (a pty, a tool, a
compile, a language server) and *where a path lives* (a buffer, a
listing, the cwd). A domain that is only the first is a tool; one that
is only the second is a mount.

## Decisions

### 1. A domain is a name in the settings

```lua
domains = {
  box = { ssh = "box" },          -- a Host in ~/.ssh/config, or user@host
  lab = { ssh = "me@lab.local" },
}
```

That is the whole declaration; ports, identities and jumps are the ssh
config's business. `kawoosh.domains()` reads the table back, as
`kawoosh.tools()` does. The local domain is always there and has no
name.

*Beat:* `kawoosh.domain("box", { … })` calls in `init.lua` — data over
calls, the way tools ended up as a `tools` table.

### 2. A path spells its domain: `box:/home/me/x.rs`

A name of two characters or more, then `:/` or `:~` — so `C:\` and
`C:/` on Windows never read as a domain (a drive is one letter, and a
domain name may not be). `box:~/proj` is the host's home. In the code
this is

```rust
pub struct Loc { pub domain: DomainId, pub path: PathBuf }
```

passed wherever a `PathBuf` goes today — a buffer's path, the cwd, an
`Io::open_file`, a `kawoosh.fs.*` argument, a session's entry — with
`Loc::local(p)` the free conversion and `Display` the spelling above.
`:e box:/etc/hosts`, `:cd box:~/proj`, `kawoosh.buf.path()` returning
`box:/…`, the tab strip showing `box: x.rs`.

*Beat:* `ssh://box/path` (unambiguous, and nobody types it);
`scp://box//path` (netrw's, with the double slash); a domain on the
buffer beside a plain path (then every place that joins a name with
the cwd needs the domain passed beside it, and the cwd is a `Loc` too).

### 3. The transport is OpenSSH's binary, one master per domain

`:domain connect box` (or the first use of `box:` while it is down)
opens a terminal pane running

```
ssh -M -S <runtime>/kawoosh-<pid>-box.ctl -o ControlPersist=yes box
```

so that a password, a passphrase or a second factor is asked where it
is answered, in a pane; once the master is up every later channel
goes through its control socket with no prompt: ptys (`ssh -t -S …
box -- cd DIR && exec $SHELL -l`, the environment on the command
line), processes (`-T`, for compile, tools and language servers), and
the SFTP subsystem (`ssh -s -S … box sftp`) for files.
`ControlPersist` lets the pane close without dropping the master;
`:domain disconnect box` is `ssh -O exit`. A dropped master is noticed
by the next channel failing, and that reopens the pane.

*Beat:* `russh` or `ssh2` in-process — their own config parsing, agent,
known hosts and jump hosts, when the user's `~/.ssh/config` is the
thing to reuse, and a prompt problem that is worse in-process. And a
kawoosh agent on the host (`ssh box kawoosh agent`, vscode's shape:
inotify, fast walks, one JSON channel): a binary to install per host
and per architecture, so it is the round after, if polling and SFTP
walks prove slow — and it plugs into Decision 4's trait, not into
anything else.

### 4. Files go through a trait, and SFTP is its second implementation

```rust
pub trait Fs {
    fn read(&self, p: &Path) -> io::Result<String>;
    fn write(&self, p: &Path, text: &str) -> io::Result<()>;
    fn stat(&self, p: &Path) -> io::Result<Stat>;
    fn list(&self, p: &Path) -> io::Result<Vec<Entry>>;
    fn walk(&self, root: &Path, max: usize) -> io::Result<Vec<String>>;
    fn rename, remove, create, copy, exists, canonicalize …
}
```

`kawoosh_systems::fs`'s functions become `Local`; `Sftp` is a blocking
SFTP v3 client over the subsystem's stdio (`openssh-sftp-client` if
its runtime sits on one thread; else a hand-rolled v3 — open, read,
write, stat, opendir, readdir, rename, remove, mkdir, ten packet
kinds), owned by the io thread, one channel per domain with its
requests serialised. `Io::open_file(loc)`, the save, `kawoosh.fs.*`
(the asynchronous forms — `kawoosh.fs.list(path, fn)` already is one —
for a remote loc) all go through the loc's `Fs`, so `dir` lists a host
without knowing it does. The settings files and `init.lua` stay local:
trust is local.

*Beat:* a mount (sshfs, macFUSE — a kernel extension, and a local path
that lies about where it is).

### 5. Watching is polling

SFTP cannot watch. The io thread stats every open remote buffer's loc
every `domains.poll_secs` (5) and feeds the same changed-paths list the
local watcher (`watch.rs`) feeds, so "the file changed on disk" is the
same message from both. A `dir` listing on a remote directory refreshes
on `<C-l>` only. An agent (Decision 3's after) would replace the poll
with inotify through the same list.

### 6. The `$EDITOR` handoff comes back over a forwarded port

The pty's ssh adds `-R 127.0.0.1:PORT:<KAWOOSH_SOCKET>` — OpenSSH
forwards a remote TCP port to a local unix socket — with PORT chosen by
kawoosh from the high range and retried on collision. At connect,
kawoosh writes the shim to the host through SFTP once
(`~/.cache/kawoosh/edit`, a bash script; bash's `/dev/tcp/127.0.0.1/PORT`
speaks the socket's one-line JSON with nothing installed:
`printf '%s\n' "$req" >&3; read -r reply <&3`) and puts
`EDITOR=~/.cache/kawoosh/edit --wait`, `VISUAL`, `GIT_EDITOR`,
`KAWOOSH_DOMAIN=box`, `TERM_PROGRAM=kawoosh`, `TERM_APPEARANCE` and
`KAWOOSH_BIN` on the pty's command line. `Request::Open` gains
`domain: Option<String>`; the shim fills it from `KAWOOSH_DOMAIN`, so
the path arrives as `box:/…`; `--wait` is the same held connection, and
`kawoosh theme` the shim's other verb.

*Beat:* forwarding a unix socket to a unix socket (the host then needs
`nc -U` or socat to talk to it; bash has `/dev/tcp` and no
`/dev/unix`); `SendEnv` (`AcceptEnv` is off on nearly every server).

### 7. A process spawns where its cwd is

```rust
impl Domain { fn command(&self, program, args, cwd: &Path) -> Command }
```

is the one door: local is `Command::new`; ssh is `ssh -S ctl -T box --
cd DIR && exec program args` (`-t` for a pty). The pty spawn (`:term`,
`:tool`), compile (`:make`) and the LSP pool spawn through it with the
cwd's domain, so `:cd box:~/proj` followed by `:tool git` is lazygit on
the host, and `:make` is `cargo build` there with its lines walked by
`]q` as `box:` locations.

A language server for a `box:` buffer is spawned through the domain and
given the buffer's host path as its `file://` URI: it sees its own
filesystem, its edits come back in its own paths, and kawoosh maps
`file:///home/…` ↔ `box:/home/…` in the one place URIs are made and
read (`systems/src/lsp.rs`). `ServerDef::builtin`'s commands must exist
on the host; a missing one is the message it is locally.

*Beat:* the LSP always local, with the files mirrored — vscode without
its server.

### 8. What the user sees

`:domain` lists the domains and their state (down, connecting, up, the
port); `:domain connect NAME`, `:domain disconnect NAME`. The status
line and the tab strip spell `box:` before a remote buffer's name;
`dir`'s header shows the loc. The picker's files source on a remote
cwd walks through SFTP with a cap and a cache for the session, and
says so once in the message line. Sessions keep remote locs and restore
their buffers lazily: the loc at once, the text when the domain
connects.

### Deliberately not

- **WSL.** The note after: `wsl.exe -d NAME` as the transport, `/mnt/c`
  ↔ `C:\` translated, `Domain::Wsl`; Windows only.
- **The daemon.** A domain is a client of a host, not a server of the
  editor; mvp.md's corridor is the same one — the io's messages are the
  boundary — and still not walked.
- **An agent on the host.** Decision 3's after.
- **mosh**, a remote clipboard beyond what OSC 52 already does in a
  pane, a domain's own settings layer.

## Build order

Four rounds, each a commit with its tests:

1. **`Loc` everywhere.** The type and its spelling; the `Fs` trait with
   `Local`; `Io`, the save, `kawoosh.fs.*`, `dir`, sessions on it. No
   behaviour changes: the whole corpus is the test, plus the spelling's
   unit tests (`box:/`, `box:~`, `C:\`, a one-letter name refused).
2. **ssh.** The `domains` table; the master in a pane; `Sftp`;
   `:e box:`, `:w`, `dir` on `box:`, the poll; `:domain`. The test
   fixture is a fake transport: `domains.ssh_command` (a setting for
   the binary, `ssh` by default) pointed at a python script that
   speaks SFTP v3 over its stdio and runs commands locally, in the
   pattern of `fake_lsp.py`, so the harness needs no host.
3. **Processes.** `Domain::command`; the pty with `-R` and the shim;
   compile and tools through it; `Request::Open`'s domain. Tests: the
   fake transport running the shim locally and the request arriving as
   `box:/…`.
4. **The LSP through the domain**, the picker's capped walk, reconnect
   after a dropped master, the session restore.

## Risks

- **`PathBuf` on a Windows host holding a unix path.** `Path::join` and
  the separator are the host's. Round 1 decides: `Loc.path` as a
  `String` with unix rules for a remote domain and a `PathBuf` for the
  local one, or the `typed-path` crate.
- **Latency.** A `:w` is a round trip (fine), a `dir` listing one
  (fine), a walk of forty thousand entries through `readdir` is
  seconds — the cap and the cache, then the agent if it hurts.
- **Forwarding refused.** `AllowTcpForwarding no` on the server means
  the shim cannot come back; `:domain` says so, and the pane's `$EDITOR`
  falls back to the host's.
- **The port.** A collision on `-R` fails the pty spawn; retry with
  another port, three times, then say so.
- **The master's pane.** `ControlPersist=yes` keeps the master past the
  pane; without it the user's closing the pane drops every channel.
  Say which in `:domain`.

## Built

**Round one, 2026-09-24: the spelling is the representation.** Risk 1
asked the round to decide what `Loc` is. Decided: no second type. A
path on a domain is spelled `box:/…` and carried in the `PathBuf` it
would have been in anyway — a buffer's path, the cwd, a listing's
directory, a session's entry — and `kawoosh_doc::paths::domain_of`
reads the domain back off it (a name of two characters or more, then
`:/` or `:~`). `expand` folds a host's path on the host's terms (its
`~` left for the host; `..` never climbs out of its root) and a
relative path against a cwd on a host is on that host; `is_absolute`,
`parent` and `basename` know a host's root is its own. What that beat:
a `Loc` struct replacing `PathBuf` at every buffer, listing, session
and argument — the same guarantee by type, at the cost of every path
in the code at once, where the invariant that matters is narrower:
*no disk operation runs on a path without asking its domain*.

That invariant lives in three places. `kawoosh_doc::fs` holds the
`Fs` trait (read, write through a sibling, stat, list, rename,
remove, create, canonicalize — the host's paths, no domain) and the
process's registry of connected domains, `remote(path)` answering
where a path's operations go or that its domain is not connected.
`kawoosh_systems::fs` asks it first in every operation, so `dir`, the
picker and every `kawoosh.fs.*` reach a host without knowing (a copy or
a rename between disks goes through read and write; a host's walk is
its listings, breadth first, hidden entries and `target` /
`node_modules` left out — no `.gitignore` is read over SFTP). And the
three places a buffer's own file is touched outside it — `Buffer::from_file`
and `Stamp::of` in `doc`, the save in the editor (`save_beside`), the io
thread's open — each ask it too. A language server is not started for
a host's file until round four runs it there: one spawned in a
directory that is not local would fail and mark its command failed for
the local files too. `kawoosh/tests/domains.rs` registers an in-process
file system mirroring a temporary directory: `:e box:/…`, `:w`, `-`
to the host's root, `<leader>yP` keeping the domain, an unconnected
domain refused.

**Round two, 2026-09-24: ssh.** `domains.NAME.ssh` in the settings as
Decision 1 has it; the first use of a path on a domain that is down —
`:e`, `:cd`, a `kawoosh.open` — connects it and does what was asked
once it is up (`Kawoosh::domain_gate`, the pending open or `cd`). The
master is a terminal in the dock running `ssh -M -S CTL -o
ControlPersist=yes HOST`, so a password is typed where it is asked; a
thread (`Io::connect_domain`) asks `ssh -S CTL -O check` every 200 ms
until the master answers — or its pane closes, or ten minutes pass —
then opens `ssh -S CTL -s HOST sftp` and registers the domain's files.
`kawoosh_systems::sftp` is the client: SFTP v3 by hand, blocking, one
request at a time under a lock, a write through a sibling renamed over
with `posix-rename@openssh.com` and the file's mode kept, `~` sent as
the directory the server started in. `:domain` (a `*domains*` pane:
each domain, its host, how it stands, its open files), `:domain
connect`, `:domain disconnect`; quitting tells every master to go,
since `ControlPersist` would keep it past kawoosh. The watch stats a
host's paths on a slower beat than the local ones (Decision 5), so a
clean buffer changed on the host reads again. A host's file is `box:
x.rs` in the tab strip and the title bar keeps the domain whole.
Departures: the binary is `ssh.command` and the beat `ssh.poll_secs`
(5), not `domains.ssh_command` and `domains.poll_secs`, where a domain
could be named either; a dropped master is an error on the next use
and `:domain connect` again, not a pane reopened by itself. The test
fixture is not a Python SFTP server but a stand-in `ssh`
(`kawoosh/tests/fixtures/fake_ssh.py`: the master a control file, `-O`
answered from it, `-s sftp` handed to OpenSSH's own `sftp-server`), so
the client is tested against the real server — `sftp.rs`'s own test
talks to `sftp-server` directly.

**Round three, 2026-09-24: processes.** Decision 7's door is
`Transport::remote_argv` beside the files' registry (a transport
registry in `kawoosh_systems::io`, filled at connect): the host's own
login shell is handed one line every shell reads alike — `sh -c 'eval
"$(echo B64 | base64 -d)"'` — with a POSIX script in base64 behind it
(`remote_script`: into the directory, its `~` the host's; the
environment exported; then `exec`), so neither this side's quoting nor
the host's reaches it. What that beat: `cd DIR && exec program args` as
the note wrote it, which a host whose login shell is nushell or fish
does not parse, and which this side's `$SHELL -lc` (nushell here)
would have quoted wrongly — so the master and every terminal on a host
are spawned by argv (`Terminal::spawn_argv`), no local shell in
between. `kawoosh.spawn`, a compile and a tool with a directory on a
host run there (`run_process_with` through the transport, the host's
`$SHELL -c`). A terminal on a host is `ssh -t -R 127.0.0.1:PORT:SOCKET`,
a port from the high range for each terminal, its shell started with
`TERM_PROGRAM`, `TERM_APPEARANCE`, `KAWOOSH_DOMAIN`, `KAWOOSH_PORT`,
`KAWOOSH_BIN` and `EDITOR` / `VISUAL` / `GIT_EDITOR` on its command
line; its OSC 7 reports are the host's paths whatever host they name
(`Terminal::set_domain`). At connect the host's CLI is written through
SFTP to `~/.cache/kawoosh`: `kawoosh` (bash — `edit [--wait] [+LINE]`,
`theme`, `pick` — speaking the socket's JSON over `/dev/tcp`) and
`kawoosh-edit` for `$EDITOR`, made executable (`Fs::set_mode`, SFTP's
SETSTAT); `Request::Open` carries the `domain` the shim fills from
`KAWOOSH_DOMAIN`, so the path arrives as `box:/…`. Once the master is
up the dock steps aside and the keys go back to the tab, where what
asked is done. Tests: io.rs's `a_remote_script_runs_as_written` and
`the_host_shim_speaks_the_socket` (bash against a listener), and
domains.rs's `processes_and_terminals_run_on_the_host` over the
stand-in, which forwards `-R` itself: a plugin's process on the host in
the tab's directory, a terminal whose `$EDITOR note.txt` opens the
host's file here, `:wq` answering it so the command after it runs.

**Round four, 2026-09-24: servers, the walk, drops, sessions.** A
language server for a host's file runs on the host through the
transport, started in the project's root there (`workspace_root` asks
the host for its markers); `uri_of` sends the host's own path, each
server knows its domain, and every event it causes is re-spelled on the
way out (`Pool::emit_from`: definitions, locations, symbols, workspace
edits, code actions) — so a definition lands in the host's buffer, not
a local file of the same path. A failed spawn is the domain's and the
command's, not the command's everywhere. A host's walk is capped at
`HOST_WALK_MAX` (5000 files), kept for the session, forgotten by any
change made on that host from here and by a disconnect, and explained
once in the message line. A dropped master is noticed before a call
finds out — the channel's process has exited (`Fs::is_alive`) — and
the next use connects again and does what it asked. A session brings a
host's file back as an open waiting on its connection (read only,
empty, titled `[box: :domain connect]`), its tab's directory as it was,
and its shell's pane kept for later; nothing asks for a password at
launch, and `:domain connect` or any use of the domain brings the
texts and the shells in. Tests: domains.rs's
`a_language_server_runs_on_the_host` (the fake language server on the
host: its diagnostic, its root spelled on the domain, `gd` in the host's
buffer and no local twin opened) and
`a_session_on_a_host_restores_lazily_and_a_drop_reconnects`.

Not built, still: WSL (the note after, decided below); an agent on the host (Decision
3's after — the walk's cap and the poll are where it would pay); a
reconnect that reopens the master's pane by itself; the git
colours of a `dir` listing on a host, which run `git` there through
`kawoosh.spawn` and so work, at a round trip each.

## WSL, and the picker

Status: decided 2026-10-07, Windows; not built. The note after, with
what a picker over the domains needs: every machine within reach
offered without a line in the settings. Measured on Windows 11 /
WSL 2.6 / Ubuntu 24.04 (login shell nushell) before deciding: a stock
distro has no `sftp-server`; `\\wsl.localhost\DISTRO\…` reads and
lists with nothing installed, a walk ten times slower than a local
disk's (17,849 entries in 2.7 s against 0.24 s); `wsl.exe -e true` in
95 ms with the distro up; no change notification through the share,
made on either side; interop on (`WSLInterop-late`), the networking
NAT's by default (mirrored here, not to be counted on).

### W1. `wsl` is a second kind, and `wsl:` is built in

```lua
domains = {
  box = { ssh = "box" },
  deb = { wsl = "Debian" },   -- a distro by its WSL name
}
```

`wsl:/home/me/x.rs` is the default distro's with no settings at all,
on Windows with `wsl.exe` on it; `domains.wsl = { wsl = "Debian" }`
moves it. `kawoosh.domains()` gains the kind beside the target.

*Beat:* the settings only (ssh's way — but the default distro is
there on every machine that has WSL, and naming it is busywork); each
distro by its own name (`ubuntu-24.04:/…`, which nobody types — still
offered by discovery, W2, for the others).

### W2. Discovery: the machines within reach are domains already

A name the settings do not give is looked for, in order: a `Host` of
`~/.ssh/config` (its `Include`s followed; a pattern with `*`, `?` or
`!` is not a host) is `{ ssh = NAME }`; a WSL distro (`wsl.exe -l -v`,
`WSL_UTF8=1` for its output) is `{ wsl = DISTRO }` under its name in
lower case, the default one also `wsl`. So `:e cdvn1.qxuken.dev:/etc/hosts`
connects with no settings, as `ssh cdvn1.qxuken.dev` would. The
settings win a clash, then ssh, then WSL; a name the domain spelling
cannot carry (a space, a character past letters, digits, `_`, `-`,
`.`) is not discovered. The distros are asked once a session, on the
io thread, the first time the picker or a name wants them.

*Beat:* discovery for the picker only (then a picked host's path
would mean nothing typed again); `known_hosts` (hashed, mostly, and a
list of every host ever reached, not the ones named).

### W3. The picker opens a tab on the machine

`picker domains` (`<leader>wh`, a launcher section): each domain, its
kind and target, where it came from (settings, `~/.ssh/config`, WSL)
and how it stands. `<CR>` opens a new tab on `NAME:~` — its working
directory the machine's home, listed in `dir` — connecting first when
it is down: the tab is there at once, its listing when the domain is
up (the session's lazy open). `<C-o>` connects without a tab.

*Beat:* the pick making the current tab's directory the machine's (the
`dirs` picker's `<CR>`; a machine is a project of its own, the tab is
the gesture for one — workspaces.md Decision 6); a terminal tab on the
host (one `:term` away in the tab, and the listing is what every pane
the tab opens next starts from).

### W4. A distro's files are the share's

The files go through `\\wsl.localhost\DISTRO` (`\\wsl$\DISTRO` before
Windows 11) with the standard library — `WslFs`, the third `Fs`; the
host's `~` its home, learned at connect. An existing file is written
in place, so its mode, owner and links are the file's still (a
sibling renamed over loses the mode the share cannot set); a new one
through a sibling. `set_mode` is a `chmod` through the transport.
`Fs::local` gives a host path's local twin, and the walk — every
file under a directory on the domain, the picker's — goes through the
local walker on it: `.gitignore` read, no cap (`HOST_WALK_MAX` is
SFTP's), the rows spelled back on the domain.

*Beat:* SFTP through `wsl.exe -e sftp-server` (the ssh code whole, and
nothing installed to run it); the share's paths as local ones (a
buffer at `\\wsl.localhost\…` — no process would run there, no language
server would see its path).

### W5. Connecting is a probe; there is no master

`wsl.exe [-d DISTRO] -e sh -c SCRIPT` on the io thread prints the
distro's name, its home, the mount root (`wslpath -u 'C:\'`) and the
login shell's `PATH` (`"$SHELL" -l -c env`: bash, zsh, fish and
nushell alike); the domain is up when it answers. No pane: there is no
password to type. A distro stopped meanwhile is started again by the
next use of the share or of `wsl.exe`, so a WSL domain does not drop;
`:domain disconnect` forgets it, and stops nothing.

### W6. A process runs through `wsl.exe -e`, with the login `PATH`

`wsl.exe [-d DISTRO] -e sh -c 'eval "$(echo B64 | base64 -d)"'`, the
same `remote_script` as ssh's, with `PATH` the probe's exported first —
a language server installed through the login shell's profile
(linuxbrew, cargo, fnm) found as a terminal there finds it. A terminal
execs the login shell, as on ssh. `Transport` becomes an enum, `Ssh`
and `Wsl`, behind the same `remote_argv` / `remote_command`.

*Beat:* `wsl.exe -- CMD`, through the default shell as ssh does it —
Windows's quoting reaches that shell's `-c` mangled (seen: the line
arrived as one word); the `PATH` of `-e` alone (no profile read:
rust-analyzer from cargo not found).

### W7. `$EDITOR` in a distro is the Windows kawoosh

The shim written at connect (`~/.cache/kawoosh/kawoosh`, POSIX sh, so
a busybox distro runs it) turns each path into Windows's
(`wslpath -w` on its directory, the name after it) and runs
`kawoosh.exe` itself through interop — `KAWOOSH_EXE` and
`KAWOOSH_SOCKET` exported to the terminal, `WSLENV` naming the socket
for the way back. The Windows CLI speaks the socket as it does from
any local shell; `kawoosh-edit` beside it is `$EDITOR`.

*Beat:* the host shim's `/dev/tcp/127.0.0.1/PORT` (Windows's loopback
is WSL's only with mirrored networking); a listener on the WSL
adapter's address (the firewall asks, and the address moves).

### W8. Two spellings, one file

A distro's `/mnt/c/x` is the local `C:\x`, and the share's
`\\wsl.localhost\DISTRO\x` (or `\\wsl$\…`) is `NAME:/x`. Each is put
back the one way wherever a path comes in — `:e`, `:cd`, an open from
a shell, a terminal's OSC 7, a language server's location — so a file
is one buffer however it was named.

### W9. Watching is the poll

As on ssh (Decision 5): the share announces nothing. `ssh.poll_secs`
is the beat of both.

### Build order

1. **Kinds and files.** `Transport` an enum; `domains.NAME.wsl` and
   the built-in `wsl`; the probe; `WslFs`; W8's spellings; `:domain`
   showing the kind. Tests on Windows with WSL, skipped without it.
2. **Processes and terminals.** W6, W7; compile and tools there.
3. **Discovery and the picker.** W2, W3 — for ssh and WSL both.
4. **Servers, the walk, sessions, help.** The LSP through `wsl.exe`,
   `Fs::local` and the walk, a session's WSL tab, `help/remote.md`.
