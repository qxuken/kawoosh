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
thing to reuse, and a prompt problem that is worse in-process.
*Amended 2026-10-09* ("Built, our ssh"): on Windows no ssh can keep a
master — Windows' own makes none, Git's MSYS one passes no session
through one — so there every channel was a connection of its own and a
password-only host could not be reached at all. The in-process client
(russh) is the transport there now, by default, and anywhere by
`ssh.client = "builtin"`: one connection a domain, every channel on
it, `~/.ssh/config`, the agent, `known_hosts` and the prompts done by
kawoosh, the questions asked in the window's confirm rather than a
pane. OpenSSH's binary stays the default elsewhere and the way to what
the client does not do (an RSA key, `Match`, a `ProxyCommand`). And a
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

Status: decided 2026-10-07, Windows; built the same day (see "Built,
WSL" at the end). The note after, with
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

`picker domains` (`<leader>wh`): each domain, its
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

## Built, WSL

**2026-10-07, all four rounds of W's build order in a day.** As
decided, with these departures. The default distro is the built-in
`wsl` only; the others are their names in lower case, and a share's
path is put back on the settings' name for its distro first, `wsl`
for the default, else that lower-case name (`Kawoosh::one_spelling`,
called by `resolve`, which every open, `cd` and request goes
through). `expand` was found joining a drive's or a share's path onto
a cwd on a host — `:e C:\x` in an ssh tab went to `box:/…/C:\x` — and
keeps it local now, on Windows, where only a prefix makes a path
absolute. The picker is not a launcher section: a module's rows are
asked without `domain pick` having published them. `Fs::local` is
the trait's ninth method, and `files_named` (the walk for a marker
file) takes it too. In-place writes are the share's, so W4's "through
a sibling" for a new file is just the write. Tests, against the real
distro and skipped without one: `wsl.rs`'s
`a_distro_is_reached_through_its_share` (a script written, made
executable, written again and run with its mode kept),
`ssh_config.rs`'s parse, and domains.rs's `a_wsl_distro_is_a_domain`
(open, `:w`, both spellings one buffer, a process in the tab's
directory, a terminal's `kawoosh edit --wait` answered),
`the_domains_picker_opens_a_tab_on_a_machine` and
`a_language_server_runs_in_the_distro`. The registry of connected
domains is the process's, so each test that connects takes a name of
its own.

Found while testing: a login shell whose own config sets `EDITOR`
(nushell's `env.nu` here) wins over the one the terminal exports, on
ssh as in WSL; `$KAWOOSH_BIN edit --wait` is kawoosh's either way.


## Built, small hosts and the kind in the name

**2026-10-09, four reports from use.** "In OpenWrt I couldn't even open
a terminal"; "WSL should be marked always, not just a random ubuntu";
a Homebrew lock on openssl in a WSL terminal; "the host is up, but what
does it mean and how can I down it?"

*A small host.* Measured on OpenWrt 24.10's rootfs (the
`openwrt/rootfs:x86-64-openwrt-24.10` image, dropbear on its port,
reached with OpenSSH): busybox 1.36.1's ash is root's shell, `$SHELL`
is `/bin/ash` under dropbear; there is no `sftp-server`, no `base64`,
no `stat` and no bash; `printf` (with octal escapes, `\045` printed as
`%`), `date -r FILE`, `ls -ln`, `readlink -f`, `mktemp` and `nc` are
there. Three steps assumed what is not: the SFTP channel (its failure
was the domain's, so nothing — no file, no terminal — was ever
started), `base64 -d` in the line every process and terminal is
started by (round three's `sh -c 'eval "$(echo B64 | base64 -d)"'`
evaluated nothing, and the terminal exited at once), and the CLI's
bash (`$EDITOR` pointing at a script whose `#!/usr/bin/env bash` is
not there). Each degrades now:

- The line is `sh -c 'eval "$(printf "\143\144…")"'`: the script's
  bytes as `printf`'s octal escapes, letters, digits and `_ ./:,=+@-`
  left as they are (`io::printf_octal`). What is left holds no quote,
  no `\\` or `\'` (fish's single-quote escapes), no `$` or backquote
  (`sh`'s double quotes) and no `%`, so bash, zsh, fish, nushell and
  ash read the line alike, and nothing past `sh`'s own `printf` is
  asked of the host. *Beat:* base64 with a probe for it at connect (a
  round trip per connect, and two lines to keep); `printf '%b'` (its
  octal is `\0NNN`, which busybox reads with a digit fewer after a
  `\0` than bash does). WSL keeps base64: `wsl.exe -e` reaches the
  distro's own `sh` with no login shell between, and every distro has
  coreutils' or busybox's `base64`.
- A host whose `-s sftp` fails still connects, its files through its
  shell (`systems::shellfs::ShellFs`, the `Fs` trait's fourth): one
  POSIX script a call through the transport — `cat` to read; to write,
  `cat >` a sibling, its length checked (a cut channel ends `cat` as
  an end of input would), then copied over the file, which keeps the
  file's mode, owner and links; a glob and the shell's own tests to
  list; size and seconds from `stat -L -c` where there is one, else
  `ls -ln` and `date -r`. The connect asks the shell to answer first
  (`ShellFs::check`), and says both failures when it does not. Slower
  than SFTP — a channel and a shell a call — and a name with a newline
  in it does not list. `Fs::via` says how a domain's files travel, and
  `:domain` and the connect's message say it.
- A terminal's `KAWOOSH_BIN`, `EDITOR`, `VISUAL` and `GIT_EDITOR` are
  exported only when the CLI is there and can run (`[ -x … ]`, and
  `command -v bash` on ssh): a read-only `~/.cache` or a host without
  bash keeps the host's own `$EDITOR`, and the shell starts either way.

Tests: `shellfs.rs`'s against this machine's `sh` with and without
`stat` and `readlink` (functions that fail shadowing them), and
`a_containers_files_go_through_its_shell` against a real OpenWrt
container when `KAWOOSH_TEST_CONTAINER` names one (passed against the
image above: a write's mode kept, a dangling link and a link to a
directory listed as what they are); `io.rs`'s
`a_remote_line_needs_no_base64`; domains.rs's
`a_host_with_no_sftp_or_base64_still_connects` over the stand-in, whose
home holding `.fake-ssh-small` makes it such a host (no subsystem, a
`PATH` without `base64`, `stat` and bash), and
`a_real_small_host_connects` against a real one when
`KAWOOSH_TEST_SSH_HOST` names it — passed from a Debian container
(OpenSSH's client, its master in the pane) against the OpenWrt one:
the SFTP channel refused, the shell answering, a file opened, written
with `:w`, a process and a terminal run. The stand-in's tests are
unix's; on Windows they skip.

*The kind in the name.* A distro other than the default, found rather
than named in the settings, is a domain under `wsl-` and its name in
lower case — `wsl-debian`, `wsl-docker-desktop` — not the bare name.
The name is what the tab strip (`wsl-debian: x.rs`), the title, the
status line, a listing's header and every path show, so putting the
kind there marks it in each of them with no second vocabulary. The
default stays `wsl`; a name the settings give is the user's, as they
gave it. The bare name is still read (`domain_kind`), so a session's
`debian:/…` from the two days it was discovered so opens; a share's
path comes in as `wsl-debian:/…`. Wherever domains are listed — the
picker's rows, `:domain` — each says what it is with the kind first,
`ssh: box`, `wsl: Ubuntu-24.04 (default)` (`kawoosh.domains()`'s
`label`). *Beat:* the label alone, the name left bare (the tab strip
and the paths still a "random ubuntu", and a label that is not what is
typed); the label in the tab strip too (`wsl: ubuntu: x.rs`, read as a
path on `wsl`).

*What "up" is.* A state is said in words — `connected`, `not
connected`, `connecting…`, `failed: why` — and what it means is said
once: in `:domain`'s listing, under the rows (for ssh, a master
connection every file, terminal and process goes through, open until
it is disconnected or kawoosh quits; for WSL, the distro answered and
nothing is held open), and in the picker's preview for the row
(`kawoosh.domains()`'s `means`), with what to do about it. `<C-x>` in
the picker disconnects the row's domain, as it closes a buffer in the
buffers' picker; the picker stays, and its row is read again once the
state moves (`refresh_domain_pick`: a connect, a failure, a
disconnect republish the domains while the picker has them). The
messages say what happened: `connected, files over SFTP`,
`disconnected, its ssh master told to exit`, `disconnected, forgotten
here (the distro runs on, as WSL keeps it)`.

*The Homebrew lock was Homebrew's.* Closing a WSL terminal's pane kills
its `wsl.exe` (the job object, term's `job.rs`), and WSL hangs up the
distro's side: measured with Kawoosh's own terminal on Ubuntu-24.04
and nushell, an external `sleep` in the foreground of an interactive
`nu -l` (its own process group) and one under `nu -lc` were both gone
with the session within seconds of the pane closing. The connect's
probe runs the login shell's `env`, whose `env.nu` runs `brew --prefix`
— which takes no formula lock. The lock files left in
`/home/linuxbrew/.linuxbrew/var/homebrew/locks` show one brew run
locking `openssl@3` and `openssl@4` in the same instant, and an
`openssl@3.6` — an alias of `openssl@3` — after: a brew run that
reaches one formula under two names conflicts with itself, flock being
per open file. Not Kawoosh's.

*Found, not built: ssh from Windows.* Windows' own OpenSSH (9.5p2,
the `ssh` on `PATH`) cannot be a master: `ssh -M -S CTL` fails at once
with "getsockname failed: Not a socket", so the master's pane closes
and the domain fails, whatever the host. Git's ssh (10.5p1, MSYS)
makes a master, but a session through it fails passing descriptors
(`mux_client_request_session: read from master failed`), each channel
falling back to a connection of its own, and the master's own pane
exits as its session fails. And the forward a terminal asks for, `-R
PORT:SOCKET`, names the command socket's path, which on Windows is a
file holding a port. An ssh domain on Windows wants a mode of its own:
no master, each channel its own connection (a key or the agent, since
a file channel has no terminal to ask in), and `-R
PORT:127.0.0.1:LOCALPORT`.

## Built, speed

**2026-10-09, two reports from use.** "Very slow ssh experience. Vim
inside a terminal works faster, traverse faster"; "WSL speed
unacceptable: when I use WezTerm it performs even faster than native
Windows Terminal, but Kawoosh is like I work on a remote machine."

*Measured first.* Two benches. One in the tests
(`kawoosh/tests/remote_frames.rs`): a mirrored host whose every call
takes a while and notes the thread that asked, through a session of
opening, moving, typing, saving, `-`, `<CR>` and the files picker. The
other live: a real `sshd` in a container (Alpine, OpenSSH 9.7, a git
repository of 3,003 files, `tc netem` adding 30 ms) and the Kawoosh
window driven over its socket (`kawoosh ex`, whose own round trip is
about 77 ms, so a command's wait on the frame is what it takes past
that), the same for the WSL distro. What they showed:

- The frame waited on the host where nothing needed it to. Opening a
  file was eleven calls on it: the `:e` path's completion listing
  three directories, each opener's `is_dir` (four) and two of them
  reading the whole file for its head, then the stamp and the read
  itself. Opening the 2 MB file stopped the window 7.6 s; a small one
  680 ms. A listing asked `is_dir` of its directory three times; the
  picker's preview a stat and a read each frame.
- SFTP was one request at a time: a 2 MB read 62 round trips one after
  another (2,018 ms), and a stat asked on one thread waited behind a
  read on another (five of each: 10.3 s).
- The language-server pool looked for a workspace's markers up the
  parents at every sync — every edit — a stat each on the files'
  channel, which the frame's calls then queued behind.
- The picker's walk of a host was its listings, breadth first: 3,000
  files had not come in 180 s.
- ssh from Windows has no master ("Built, small hosts"): each channel
  was a connection, 420 to 500 ms at 30 ms of delay, 26 of them in
  the session — every listing's `git status`, every base, every walk —
  and the master's pane closing as its own session failed made the
  domain come up by a race (once in three runs).
- WSL's files were not the slow part on this machine (a listing on
  the share some 30 ms, the walk 0.5 s); its processes were: `wsl.exe`
  is a tenth of a second before it runs anything, and a listing's git
  colours are a process each. Its terminal is not: through the
  `OpenConsole` a release ships beside `kawoosh.exe`, a byte's echo
  from `wsl.exe -e cat` came back in 0.4 ms against 0.25 ms from a
  local one (`term/tests/pty_latency.rs`); Windows' own console host
  splits output 10 to 20 ms apart, which a development build, with
  nothing beside it, still has.

*What changed.*

- **SFTP pipelined.** Each request goes out with its id and a thread
  hands each answer to whoever waits on it, so nothing waits behind
  another thread's call; a read sends its chunks a window at a time, a
  write its chunks with the CLOSE behind them, a listing two READDIRs
  and its links' stats at once, a stat LSTAT and STAT together, a
  read's CLOSE unwaited. `Fs::read_at` reads a range. The bench: a
  listing 125 → 63 ms, the 2 MB read 2,018 → 231, its write 2,211 →
  333, the ten calls on two threads 10.3 s → 0.97.
- **The frame asks less.** A host's file is read on the io thread
  whatever its size, its stamp taken there; what a host says of a path
  holds for a moment (`fs::new_moment`, at each frame and event, a
  quarter second at most, ended by any change made there from here),
  so the openers' four `is_dir` are one stat and their heads one
  64 KB `read_at`; the picker's preview reads a host's file once while
  it is open; a save makes its directory only when the write says it
  is missing; the pool keeps a host's workspace roots. What the frame
  still waits for on a host: a stat for a listing, a stat and a head
  for a file opened (`a_host_is_asked_little_on_the_frame`).
- **The host walks itself.** One process: `git ls-files -co
  --exclude-standard` in a repository, else `find` with the SFTP
  walk's rules; the listings only where neither runs. A distro too,
  before the share's walk.
- **No master on Windows, and runners.** `ssh.master` (off on
  Windows): no pane, each channel its own connection in `BatchMode`
  where it has no terminal, a terminal's `-R` to the TCP port the
  socket's file names. What runs to its end — a process whose output
  is wanted whole (git's), the walk, `ShellFs`'s calls — goes through
  a runner (`systems::runner`): a POSIX sh loop kept open on the host,
  started as an ordinary process's one script, that reads a script and
  its input as lines of `printf` octal, runs it, and answers the code
  and both outputs by their lengths. Up to three a domain, one warmed
  at connect. On ssh without a master a script is 45 ms against
  494 ms for a connection of its own; through `wsl.exe`, a few
  milliseconds against a tenth of a second.

The live bench after: connecting and opening a file 4.1 s → 1.3; a
file opened stops the frame about 180 ms (680); the 2 MB file 0.2 s
(7.6); a listing about 45 ms (190 to 250); the picker's walk 215 ms
(unfinished at 180 s); 11 connections in the session (26). On WSL the
walk went from 0.5 s to below what the bench can tell.

*Beat:* a master kept by hand — a pane per channel that asks — and
no master with a connection a process (each git a connection: the 26).
`ssh -O` over Git's MSYS client fails on descriptor passing, which no
option turns off.

*An in-process client, measured, not built.* With no master on
Windows, Decision 3's beat was asked again: one connection a domain,
every channel on it, on every OS. A spike (russh 0.64.1 on `ring`, no
`aws-lc` and so no CMake or NASM, russh-sftp 3.0.1; 61 crates new to
the workspace, `pageant` among them; built from nothing in 28 s on
Windows) against the same container: connect and authenticate with an
ed25519 key 274 ms (OpenSSH's 450 to 500); the SFTP channel 196 ms; a
stat 31; a listing of 60 entries 124; a small file 154 (its client
waits on each step — ours, pipelined, is two round trips); the 2 MB
file 367 (ours 231); an exec channel 98 ms a process (three round
trips: a runner's script is one, 45 ms); `git ls-files` of 3,003
files 113 ms (a runner 50); a pty's echo 31 ms, the round trip.
Against OpenWrt's dropbear (no delay added): connected in 94 ms, the
SFTP subsystem refused cleanly, exec and a pty under busybox worked.
What it would buy: one connection for files, runners, terminals and
language servers alike — a terminal or a server a channel of 100 ms
rather than a connection of 500 — and a password typed once on
Windows. What it would take, none of it in the spike: `~/.ssh/config`
read to the depth OpenSSH does (`Include`, `ProxyJump`, `Match`), the
agent (Windows' pipe, Pageant, `SSH_AUTH_SOCK`), `known_hosts` with an
accept-new prompt, a passphrase, password and keyboard-interactive
prompt in a pane, remote forwarding for the `$EDITOR` shim, and a
terminal over a channel where `kawoosh_term` takes a `portable-pty`
child. Each is a piece of what the user's `ssh` already does, which is
why the runner was built first: it keeps that `ssh`, and its scripts
are faster than the in-process exec. The in-process client stays the
candidate for terminals and servers on Windows, measured here for the
round that takes it up.

## Built, our ssh

**2026-10-09, the round that took it up.** "Let's continue with our
ssh", measured in the real world: the user's OpenWrt router on the LAN
(aarch64, busybox, dropbear with OpenSSH's `sftp-server`), the same
container `sshd` behind 30 ms of `netem`, and OpenWrt's rootfs in a
container for a dropbear with no SFTP at all.

*What it is.* `kawoosh_systems::ssh`: russh 0.58 on `ring` (no CMake,
no NASM; not later, whose ML-KEM's `kem` cannot sit beside the
pre-release `age` holds; and no `rsa`, whose release candidates no
longer build together) on a tokio runtime of two threads, used from
ordinary threads through blocking readers and a writer, so everything
above it is the code the OpenSSH path has. `ssh.client` says which
carries a host: `builtin`, the default on Windows, or `openssh`, the
default elsewhere. One connection a domain carries:

- the SFTP subsystem — the same pipelined client (`Sftp::on_channel`),
  `ShellFs` through the runners when the host refuses it;
- the runners, each an exec channel running the loop;
- a process (`kawoosh.spawn`, a compile, a tool) an exec channel, its
  outputs pumped as a process's (`pump_outputs`), killed by a KILL
  signal and the channel closed;
- a language server an exec channel (`ServerProc::Channel`);
- a terminal a pty channel, `portable-pty`'s master and child traits
  over it (`ChannelPty`, `ChannelChild`), which `Terminal::spawn_on`
  takes as it takes a local pseudo console — a resize a window-change;
- the `$EDITOR` shim's way back: `tcpip-forward` of the terminal's port,
  each connection the host opens on it carried to the command socket
  (on Windows the TCP port its file names).

What OpenSSH did by itself is done here. `~/.ssh/config`
(`ssh_config::resolve`): `HostName` (`%h`), `User`, `Port`,
`IdentityFile` (every one, `~` and `%d %u %r %h`), `IdentitiesOnly`,
`ProxyJump` (each hop a connection through the one before, over a
direct-tcpip channel), `UserKnownHostsFile`, `StrictHostKeyChecking`,
first value winning, `Host` patterns and `!` negations, `Include` where
it stands; `Match` is passed over. Authentication: the agent's keys
(`SSH_AUTH_SOCK`; on Windows `SSH_AUTH_SOCK` when it names a pipe,
OpenSSH's `\\.\pipe\openssh-ssh-agent`, then Pageant), then the
identity files — the configured, else `id_ed25519`, `id_ecdsa`,
`id_rsa` — an encrypted one's passphrase asked, three tries; then
keyboard-interactive, each prompt asked; then the password, asked.
`known_hosts`: a host whose key is there goes on; one not there is
asked about with the key's SHA-256 fingerprint and, trusted, written
to the first `UserKnownHostsFile` (OpenSSH's `accept-new`, asked);
`StrictHostKeyChecking yes` refuses it, `no` takes it; a key that
changed is refused in capitals with the line it differs from, nothing
asked. A connection that drops (keepalives every 30 s, three missed)
is noticed by its files' channel ending, and the next use connects
again — the poll no longer says the open files were deleted when it
could not ask (`watch.rs`: only a host's not-found is gone).

*What asks.* The window's confirm, given a field (`confirm::Asking`):
the connecting thread sends `IoMsg::DomainAsk` and waits; a host key
is *Trust it* or *Refuse*, a passphrase or a password is typed into a
field drawn as dots, `<CR>` answering and `<Esc>` refusing. A confirm
put up over it answers it with none. *Beat:* a pane running a prompt
of the CLI's (a process and a socket request for each question); the
picker's query (shown as typed).

*Found on the way, on either client.* A host's git root was made a
local path: `git rev-parse --show-toplevel` prints the host's `/p`,
which `fs.expand` made `C:\p` on Windows, so every `git` the listing
and the status line asked for started in a directory that is not there
— and the status line asked for the head at every frame, its failure
not kept: 500 spawns in three seconds. The root is spelled on the
domain now, and a head that could not be read stays unread until the
repository moves.

*Measured* (the live Kawoosh driven over its socket, each step until
its buffer is there, the CLI's own round trip about 77 ms of each; and
the client's own timings in `ssh.rs`'s and `pty_latency.rs`'s ignored
tests):

| | router, LAN, builtin | router, openssh | container 30 ms, builtin | container 30 ms, openssh |
|---|---|---|---|---|
| connect (the client alone) | 85 ms | 215 ms a connection | 231 ms | ~490 ms a connection |
| connect + first file, live | 261 ms | 343 ms | 962 ms | 1,229 ms |
| a file opened, live | ~160 ms | ~155 ms | ~340 ms (its frame 90) | ~410 ms (its frame 140) |
| the 2 MB file, live | — | — | 590 ms | 729 ms |
| a listing, live | ~155 ms | ~155 ms | 185–219 ms | 201–233 ms |
| the picker's walk, live | 157 ms (143) | 155 ms | 157 ms (3,003) | 265 ms |
| `:w`, its round trip | 92 ms | 93 ms | 232 ms | 279 ms |
| a process (an exec) | 15 ms | 215 ms (a connection) | 135 ms | ~490 ms |
| a script through a runner | 14 ms | — | 34 ms | 45 ms |
| a terminal's echo | 2.4 ms | 2.4 ms | 31 ms | 31 ms |

On the LAN every live step is at what the bench can tell; the client
is faster to connect and to open a channel — a terminal, a language
server, a process that streams — by a connection's handshake each, and
over 30 ms its files' calls are cheaper than through a process's pipes.
And it reaches what the OpenSSH path on Windows cannot: a host that
takes a password, or a key with a passphrase and no agent running.

*Measured on macOS* (2026-10-09, Apple Silicon, Docker Desktop: a
Debian bookworm `sshd` behind 30 ms of `netem`, OpenWrt 24.10.8's
rootfs for dropbear; the client's own ignored tests, three runs each,
russh 0.58 and 0.64.1 alike within their noise). The container's own
round trip is 0.4 ms, so Docker's port proxy adds nothing measurable.

| | builtin, 30 ms | openssh, 30 ms | builtin, 0 ms | dropbear, 0 ms |
|---|---|---|---|---|
| connect | 313–324 ms | 336 ms (a master up), ~510 (`ssh true`) | 50 ms | 19 ms |
| a process | 144–150 ms, the first | 70–77 ms through a master, ~510 without | 49 ms, the first | 1.9 ms |
| the SFTP channel | 103–108 ms | 601 ms (a connection) | 2.1 ms | refused cleanly |
| a script through a runner | 35–40 ms | 37–50 ms | 2.0 ms | 2.3 ms |
| `git ls-files`, 3,004 files | — | 47 ms (runner), 574 (a process) | — | — |
| a terminal's echo | 34–35 ms | — | 0.4 ms | 0.3 ms |

The round trip here is about 34 ms, against Windows' 31, and every
channel step is Windows' number plus that difference. Connect is
not: 313 against 231 — 50 ms of it is spent with no delay at all, as
OpenSSH's is (~72 ms), so it is this sshd's own cost, not the client's.
So is a session's first process: 49 ms with no delay, against dropbear's
1.9 — OpenSSH's first through a fresh master took 56, its later ones 12.

*The two clients side by side on macOS* (the same day, the same
containers; one scratch bench running the same steps over each in one
process — the SFTP client, the runner, the echo loop the same code —
three runs, the range shown). OpenSSH is macOS's own, `/usr/bin/ssh`
(OpenSSH 10.3p1, LibreSSL), the way kawoosh runs it here: one master
(`-M`, `ControlPersist`), every channel through it, and so no runners —
a script is a process through the master.

| | russh, 30 ms | OpenSSH + master, 30 ms | russh, 0 ms | OpenSSH + master, 0 ms |
|---|---|---|---|---|
| connect (the master up) | 301–329 ms | 357–372 ms | 50–53 ms | 63–69 ms |
| a process, the session's first | 144–150 | 157–167 | 49–51 | 52–55 |
| a process, later | 72–73 | 77–85 | 2.2–2.4 | 10–11 |
| the SFTP channel | 106–111 | 146–152 | 1.7–2.0 | 9–10 |
| a stat | 34–37 | 36–37 | 0.3–0.4 | 0.3 |
| a listing (6) | 68–69 | 70–74 | 0.7–0.8 | 0.6–0.7 |
| read 2.8 MB | 279–286 | 286–292 | 81–82 | 81–82 |
| write 2.8 MB | 184–197 | 187–249 | 14–15 | 12–16 |
| a script (a runner; a process) | 36–39 | 80–86 | 1.2–1.3 | 8.9–9.3 |
| `git ls-files`, 3,004 files | 37–42 | 72–83 | 2.0–2.3 | 9–11 |
| a terminal's echo | 35 | 41–42 | 0.3 | 20–22 |

Over the wire the two are one protocol and cost the same: a stat, a
listing, a read and a write are the round trips either way, and a
process is two (the earlier note here, that the builtin exec took three
against OpenSSH's two, set the session's first against later ones — it
was wrong). Where they part is around the wire. Each OpenSSH channel is
a process of its own (`ssh -S`), 8 to 10 ms of start and mux before a
byte moves, so a script costs one round trip through a runner and two
through the master — the runner's whole point, which a master rules out.
And OpenSSH's terminal echo waits on `ObscureKeystrokeTiming` (on since
OpenSSH 9.5): keystrokes leave on a 20 ms tick, hiding their timing from
whoever watches the traffic — 20 ms a key on a fast link, 6 to 8 at
30 ms. With `-o ObscureKeystrokeTiming=no` given to the master (a mux
client's own is ignored) it is 0.3 ms, as russh's; russh has no such
obfuscation.
`builtin_ssh.rs` passes against both containers (the dropbear's root
given a password, else the `none` probe gets in and no key is tried).

Tests: `ssh_config.rs`'s resolve; `ssh.rs`'s
`one_connection_carries_everything` (ignored, against any host: an
exec's code and outputs, SFTP, a runner, a pty's echo, a forward back);
`builtin_ssh.rs` against the container `sshd` when `KAWOOSH_TEST_SSHD`
names it (the host key asked and written down, a password typed and not
drawn, a file, a process, a terminal's shell answering, a passphrase,
a changed key refused) and an OpenWrt dropbear when
`KAWOOSH_TEST_DROPBEAR` does (no SFTP: a new file written and read
back through the shell).

Not built: `Match`, `ProxyCommand`, certificates, an RSA key (each the
`openssh` client's still), GSSAPI; the hosts that refuse a key in
`BatchMode` (`cdvn1`, `gfs1` here) were not connected to — they want a
passphrase or a password typed, which the window now asks for.

## Built, the fallback and `:ssh`

**2026-10-09, two asks.** "Can you silently fall back to another
client?" — then "but better with a notification probably"; and "is
there something like `:ssh qxuken@somehost`?"

### F1. `auto` falls back to OpenSSH's client where ours cannot serve

`ssh.client` is `auto` by default: the in-process client on Windows,
OpenSSH's taking over where it cannot serve a host; OpenSSH's
elsewhere, where its master works and nothing needs falling back from.
`builtin` and `openssh`, set, pin one and never fall back. What the
in-process client cannot serve is said in its failure's kind
(`ssh::Failure`): `Unsupported` falls back, `Network`, `Final` and
`Declined` do not.

- *Before anything is asked*, from the host's resolved config and its
  jumps': a `ProxyCommand`, a `CertificateFile`, a `Match` block
  anywhere in the files read (what it would apply cannot be known
  without evaluating it), every identity file RSA with no agent there
  (`SSH_AUTH_SOCK`, Windows' OpenSSH pipe).
- *On the way*: a handshake that fails past the TCP connection (an
  algorithm not shared, a server that hangs up on our offer), a key
  file it cannot read (RSA, a format it lacks) before a password would
  be asked, and authentication run out with nothing asked — the
  server's ways in are learned first (`none`), so a host that takes
  keys alone is not asked for a password.
- *Never*: a host key that changed (`Final`, loud as before), a
  question answered `<Esc>` (`Declined`: not asked again behind the
  user's back), the network (`Network`: a host not reached, a name not
  known — another client would fare no better), a wrong password typed.

The fallback is OpenSSH's client with no master (`ssh.master` off:
Windows has none to keep), in the connecting thread, which goes on as
if it had been asked for. One note a domain a session says so, with
the reason in a few words — `box: using OpenSSH — ProxyCommand isn't
supported by the built-in client` — nothing to answer; the log has it
at info; `:domain` adds `· over OpenSSH: why` to the host's row and the
picker's preview says it. *Beat:* silent (asked, and taken back: a
host that went slower, or asked nothing where it used to ask, with no
word of why); a dialog offering the switch (a question for something
the user can do nothing about but say yes); falling back on every
failure (a wrong password typed twice, once to each client).

### S1. `:ssh [user@]host[:port] [path]` makes the domain on the spot

`:ssh qxuken@somehost`, `:ssh 192.168.50.1 /etc`, `:ssh
ssh://root@box:2222/srv`: a new tab on the machine, its home or the
path its directory and listed, as `:domain tab` makes one — connected
first. The domain is the one the settings or `~/.ssh/config` name when
the target is just that host (the user and port the config's own);
else one made for it, named after the host as written (an alias is a
host), the user before it when it is not the one the host is reached
as anyway (the config's `User`, else this machine's: `qxuken-somehost`)
and the port after it when it is not its own (`somehost-2222`) — so two
users on one host are two domains, and each name reads as what it is.
A settings domain of the same name reaching elsewhere keeps the name;
this one is `ssh-NAME`. The domain reaches `user@host`, or
`ssh://user@host:port` with a port — the destination OpenSSH takes, and
`ssh_config::resolve` reads the same — so either client carries it.

It is remembered in the store (namespace `domains`: the name, what it
reaches, when it was last used), as trust's records are: the picker
offers it, the last used first, from `:ssh`; `domain_kind` knows it, so
a session's files and tabs on it come back as any domain's do. `<Tab>`
completes a host (`ArgKind::Host`): `~/.ssh/config`'s, what `:ssh`
reached before, every domain's name.

*Beat:* `user@host` in a path (`me@box:/etc`) — a domain name cannot
hold `@`, and a path's domain would be two things at once; `:domain add
NAME TARGET`, then `:domain tab NAME` (two commands and a name to make
up for what `ssh` takes in one); writing the domain into
`settings.lua` (the user's file written by a command, and a host tried
once kept there for good); the moments' memory (`kawoosh.remember`, as
the workspaces picker's) — ranked and pruned for what is attended, where
a domain is a name to keep until it is replaced.

Tests (`builtin_ssh.rs`, against the container `sshd`, the OpenWrt
dropbear and the Windows OpenSSH client, each run only where the
environment names it): a `Match` block falls back with the note said
once across a reconnect and the listing saying why, nothing asked; an
RSA key falls back and OpenSSH's client takes it; a password refused
with `<Esc>`, an unreachable port and a pinned `builtin` do not; a
dropbear that takes no password and a key it does not know falls back
and says OpenSSH's own failure; `:ssh me@HOST:PORT /home/me/proj` opens
a tab on `me-HOST-PORT`, a second window on the same store knows it and
`ssh://me@HOST:PORT/home/me` reaches it again. `ssh_cmd.rs`'s parse and
naming. Live: `:ssh 192.168.50.1 /www` and `:ssh
ssh://root@192.168.50.1/etc` on the router both reach its `~/.ssh/config`
domain, `root` being the config's user.
