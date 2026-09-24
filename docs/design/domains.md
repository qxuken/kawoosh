# Domains: where a process spawns and where a path lives

Status: decided 2026-09-21 (roadmap step 9); built 2026-09-24 as
roadmap step 27 (see "Built" at the end for where the build departed);
ssh alone, as the roadmap's terminal track recommended, with wsl the
note after.
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
