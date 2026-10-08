#!/usr/bin/env python3
"""A stand-in for OpenSSH's `ssh`, for the domain tests (docs/design/domains.md).

It takes the arguments kawoosh gives `ssh` and does locally what the real
one would do on a host whose files are this machine's:

  -M -S CTL ... HOST       the master: says so, makes CTL, and stays until
                           CTL is gone (`-O exit`). A host named `refuse`
                           fails as a wrong password would; one named
                           `ask` asks for a password first (`secret`).
  -S CTL -O check HOST     0 while the master is up.
  -S CTL -O exit HOST      the master told to go.
  -S CTL -s HOST sftp      the SFTP subsystem: the real sftp-server, killed
                           when CTL goes, as a channel dies with its
                           master.
  -S CTL [-T|-t] [-R SPEC] HOST -- CMD
                           CMD run here, by sh. With `-R PORT:SOCKET`, TCP
                           on 127.0.0.1:PORT is forwarded to the unix
                           socket, as the real one forwards a host's port.

A HOST that is an absolute path is the host's home: the SFTP server starts
there and a command runs with it as `$HOME` — so a test's host writes
nothing into the real home. A command's `$SHELL` is `/bin/sh`, as on a
host whose login shell is sh.

A home holding a file `.fake-ssh-small` is a host as small as OpenWrt's
(dropbear and busybox): no SFTP subsystem (`-s sftp` fails as dropbear's
does), and a command's `PATH` without `base64`, `stat` or `bash` — a
directory of links to everything else on this machine's `PATH`.
"""

import os
import socket
import sys
import tempfile
import threading
import time

SFTP_SERVERS = ["/usr/libexec/sftp-server", "/usr/lib/openssh/sftp-server", "/usr/lib/ssh/sftp-server"]

# What a small host does without.
MISSING = {"base64", "stat", "bash", "sftp-server"}


def small_path(home):
    """A directory of links to every program on `PATH` but `MISSING`."""
    farm = os.path.join(tempfile.gettempdir(), "fake-ssh-small-%d" % os.getuid())
    if not os.path.isdir(farm):
        tmp = farm + ".%d" % os.getpid()
        os.makedirs(tmp, exist_ok=True)
        for d in os.environ.get("PATH", "/usr/bin:/bin").split(":"):
            if not os.path.isdir(d):
                continue
            for name in os.listdir(d):
                link = os.path.join(tmp, name)
                if name in MISSING or os.path.lexists(link):
                    continue
                try:
                    os.symlink(os.path.join(d, name), link)
                except OSError:
                    pass
        try:
            os.rename(tmp, farm)
        except OSError:
            pass
    return farm


def parse(argv):
    opts = {"M": False, "s": False, "S": None, "O": None, "R": [], "o": []}
    rest = []
    i = 0
    while i < len(argv):
        a = argv[i]
        if rest:
            rest.append(a)
        elif a in ("-M", "-s", "-T", "-t", "-q", "-N"):
            opts[a[1]] = True
        elif a in ("-S", "-O", "-R", "-o", "-p", "-l"):
            i += 1
            v = argv[i]
            if a == "-R":
                opts["R"].append(v)
            elif a == "-o":
                opts["o"].append(v)
            else:
                opts[a[1]] = v
        else:
            rest.append(a)
        i += 1
    host = rest[0] if rest else ""
    cmd = rest[1:]
    if cmd and cmd[0] == "--":
        cmd = cmd[1:]
    return opts, host, cmd


def forward(spec):
    """`-R [127.0.0.1:]PORT:SOCKET`: TCP on the port to the unix socket."""
    parts = spec.split(":")
    port, path = int(parts[-2]), parts[-1]
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("127.0.0.1", port))
    srv.listen(8)

    def pipe(a, b):
        try:
            while True:
                d = a.recv(65536)
                if not d:
                    break
                b.sendall(d)
        except OSError:
            pass
        finally:
            try:
                b.shutdown(socket.SHUT_WR)
            except OSError:
                pass

    def serve():
        while True:
            c, _ = srv.accept()
            u = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            u.connect(path)
            threading.Thread(target=pipe, args=(c, u), daemon=True).start()
            threading.Thread(target=pipe, args=(u, c), daemon=True).start()

    threading.Thread(target=serve, daemon=True).start()


def main():
    opts, host, cmd = parse(sys.argv[1:])
    ctl = opts["S"]
    if opts["O"] == "check":
        sys.exit(0 if ctl and os.path.exists(ctl) else 255)
    if opts["O"] == "exit":
        if ctl and os.path.exists(ctl):
            os.remove(ctl)
        sys.exit(0)
    if opts["M"]:
        print("fake ssh: connecting to %s" % host, flush=True)
        if host == "refuse":
            print("Permission denied (publickey,password).", flush=True)
            sys.exit(255)
        if host == "ask":
            sys.stdout.write("%s's password: " % host)
            sys.stdout.flush()
            if sys.stdin.readline().strip() != "secret":
                print("Permission denied.", flush=True)
                sys.exit(255)
        open(ctl, "w").close()
        print("fake ssh: master up for %s" % host, flush=True)
        while os.path.exists(ctl):
            time.sleep(0.05)
        sys.exit(0)
    # Every channel goes through the master.
    if not (ctl and os.path.exists(ctl)):
        sys.stderr.write("Control socket connect(%s): No such file or directory\n" % ctl)
        sys.exit(255)
    home = host if host.startswith("/") else os.environ.get("HOME", "/")
    env = dict(os.environ, HOME=home, SHELL="/bin/sh")
    os.chdir(home)
    small = os.path.exists(os.path.join(home, ".fake-ssh-small"))
    if small:
        env["PATH"] = small_path(home)
        if opts["s"]:
            sys.stderr.write("subsystem request failed on channel 0\n")
            sys.exit(255)
    if opts["s"]:
        server = next((p for p in SFTP_SERVERS if os.path.exists(p)), None)
        if not server:
            sys.exit(255)
        # A channel lives as long as its master: the server is killed
        # when the control file goes (a dropped connection).
        import subprocess
        child = subprocess.Popen([server], env=env)

        def watch():
            while os.path.exists(ctl) and child.poll() is None:
                time.sleep(0.05)
            if child.poll() is None:
                child.kill()

        threading.Thread(target=watch, daemon=True).start()
        sys.exit(child.wait())
    for spec in opts["R"]:
        forward(spec)
    line = " ".join(cmd) if cmd else "/bin/sh -l"
    if opts["R"]:
        # Stay while the forward is wanted: the command is a child.
        import subprocess
        sys.exit(subprocess.call(["/bin/sh", "-c", line], env=env))
    os.execve("/bin/sh", ["/bin/sh", "-c", line], env)


if __name__ == "__main__":
    main()
