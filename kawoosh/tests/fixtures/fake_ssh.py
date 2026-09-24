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
  -S CTL -s HOST sftp      the SFTP subsystem: the real sftp-server.
  -S CTL [-T|-t] [-R SPEC] HOST -- CMD
                           CMD run here, by sh. With `-R PORT:SOCKET`, TCP
                           on 127.0.0.1:PORT is forwarded to the unix
                           socket, as the real one forwards a host's port.
"""

import os
import socket
import sys
import threading
import time

SFTP_SERVERS = ["/usr/libexec/sftp-server", "/usr/lib/openssh/sftp-server", "/usr/lib/ssh/sftp-server"]


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
    if opts["s"]:
        server = next((p for p in SFTP_SERVERS if os.path.exists(p)), None)
        if not server:
            sys.exit(255)
        os.execv(server, [server])
    for spec in opts["R"]:
        forward(spec)
    line = " ".join(cmd) if cmd else os.environ.get("SHELL", "/bin/sh")
    if opts["R"]:
        # Stay while the forward is wanted: the command is a child.
        import subprocess
        sys.exit(subprocess.call(["/bin/sh", "-c", line]))
    os.execv("/bin/sh", ["/bin/sh", "-c", line])


if __name__ == "__main__":
    main()
