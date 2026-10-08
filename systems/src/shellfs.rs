//! A host's files through its shell, for a host with no SFTP server
//! (docs/design/domains.md, "Built, small hosts"): OpenWrt's dropbear
//! ships none, nor does a bare container's sshd. Each operation is one
//! POSIX script run through the domain's transport — `cat` to read,
//! `cat >` a sibling then copied over to write, the shell's own tests
//! and a glob to list — with nothing asked of the host past what
//! busybox's defaults have: no `stat` (OpenWrt's busybox has none; the
//! size is `ls -ln`'s and the time `date -r`'s then), no `base64`, no
//! bash.
//!
//! Slower than SFTP — a channel and a shell for every call — and a
//! name with a newline in it does not list; but the domain connects,
//! and `:e`, `:w`, `dir` and the picker's walk work.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

use kawoosh_doc::fs::{Entry, Fs, Stat};
use kawoosh_doc::paths::{host_join, host_parent};

use crate::io::{Transport, shell_quote};

type Runner = Box<dyn Fn(&str) -> Command + Send + Sync>;

/// A host's files over the shell its transport reaches.
pub struct ShellFs {
    run: Runner,
    transport: Option<Transport>,
    dead: AtomicBool,
}

/// What every script has before it: `ex` (there, a dangling link too),
/// `sz` (size and seconds: GNU's or busybox's `stat` where there is
/// one, else `ls -ln` and `date -r`), `kind` (`f`/`d`/`o`, whether a
/// link, then `sz`'s two).
const HELPERS: &str = r#"ex() { [ -e "$1" ] || [ -L "$1" ]; }
S=; stat -L -c %s / >/dev/null 2>&1 && S=1
sz() {
  if [ -n "$S" ] && s=$(stat -L -c '%s %Y' "$1" 2>/dev/null); then printf '%s\n' "$s"; return; fi
  set -f; set -- "$1" $(ls -ldLn "$1" 2>/dev/null); set +f
  m=$(date -r "$1" +%s 2>/dev/null) || m=-
  printf '%s %s\n' "${6:-0}" "${m:--}"
}
kind() { k=o; [ -f "$1" ] && k=f; [ -d "$1" ] && k=d; l=0; [ -L "$1" ] && l=1; printf '%s %s %s\n' "$k" "$l" "$(sz "$1")"; }
"#;

/// A script's exit codes for the errors a caller tells apart.
const NOT_FOUND: i32 = 44;
const EXISTS: i32 = 45;
const WRONG_KIND: i32 = 46;

/// A host path in the script: `~` the host's `$HOME`, a relative one
/// under it (as SFTP has it), the rest single-quoted.
fn q(p: &Path) -> String {
    let s = p.to_string_lossy();
    match s.strip_prefix('~') {
        Some("") => "\"$HOME\"".into(),
        Some(rest) if rest.starts_with('/') => format!("\"$HOME\"{}", shell_quote(rest)),
        _ if s.starts_with('/') => shell_quote(&s),
        _ if s.is_empty() => "\"$HOME\"".into(),
        _ => format!("\"$HOME\"/{}", shell_quote(&s)),
    }
}

/// `/d/.x.kawoosh~` of `/d/x`, on the host's `/`.
fn sibling(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    let name = s.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    let dir = host_parent(path).unwrap_or(Path::new(""));
    host_join(dir, Path::new(&format!(".{name}.kawoosh~")))
}

/// `k l size secs` (`kind`'s line) as a stat; `-` for seconds unknown.
fn parse_kind(fields: &[&str]) -> Option<Stat> {
    let [k, l, size, secs] = fields else {
        return None;
    };
    Some(Stat {
        is_dir: *k == "d",
        is_file: *k == "f",
        is_symlink: *l == "1",
        size: size.parse().unwrap_or(0),
        modified: secs.parse().ok(),
    })
}

impl ShellFs {
    /// The files of a domain whose processes `t` starts.
    pub fn over(t: Transport) -> ShellFs {
        let runner = t.clone();
        ShellFs {
            run: Box::new(move |script| runner.remote_command(script)),
            transport: Some(t),
            dead: AtomicBool::new(false),
        }
    }

    /// Over a command of the caller's making: `run(script)` is a
    /// command that runs `script` with POSIX sh (the tests' `sh -c`).
    pub fn with(run: impl Fn(&str) -> Command + Send + Sync + 'static) -> ShellFs {
        ShellFs {
            run: Box::new(run),
            transport: None,
            dead: AtomicBool::new(false),
        }
    }

    /// Whether the host's shell answers through the transport, and what
    /// it said when it does not.
    pub fn check(&self) -> Result<(), String> {
        match self.call("printf 'kawoosh:%s\\n' up", None) {
            Ok(out) if String::from_utf8_lossy(&out).contains("kawoosh:up") => Ok(()),
            Ok(out) => Err(format!(
                "the host's shell answered something else: {}",
                String::from_utf8_lossy(&out).trim()
            )),
            Err(e) => Err(format!("the host's shell did not answer: {e}")),
        }
    }

    /// `script` run on the host, `stdin` written to it: its output, or
    /// its exit as an error of the kind the code says.
    fn call(&self, script: &str, stdin: Option<&[u8]>) -> io::Result<Vec<u8>> {
        if self.dead.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "the connection closed",
            ));
        }
        let full = format!("{HELPERS}{script}");
        // Through the domain's runner where it keeps them (ssh with no
        // master, a distro): a round trip, not a connection a call.
        let out = match &self.transport {
            Some(t) if crate::runner::wanted(t) => crate::io::run_script(t, &full, stdin)?,
            _ => {
                let mut c = (self.run)(&full);
                c.stdin(if stdin.is_some() {
                    Stdio::piped()
                } else {
                    Stdio::null()
                })
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
                let mut child = crate::spawn::spawn(&mut c)?;
                // Written on a thread of its own, so a host that answers
                // before it has read everything does not leave both
                // sides waiting.
                let writer = stdin.zip(child.stdin.take()).map(|(bytes, mut pipe)| {
                    let bytes = bytes.to_vec();
                    std::thread::spawn(move || {
                        let _ = pipe.write_all(&bytes);
                    })
                });
                let out = child.wait_with_output()?;
                if let Some(w) = writer {
                    let _ = w.join();
                }
                crate::runner::Output {
                    code: out.status.code().unwrap_or(-1),
                    stdout: out.stdout,
                    stderr: out.stderr,
                }
            }
        };
        let code = out.code;
        if code == 0 {
            return Ok(out.stdout);
        }
        let said = String::from_utf8_lossy(&out.stderr);
        let said = said
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim();
        let kind = match code {
            NOT_FOUND => io::ErrorKind::NotFound,
            EXISTS => io::ErrorKind::AlreadyExists,
            255 if matches!(&self.transport, Some(Transport::Ssh(s)) if !s.is_up()) => {
                self.dead.store(true, Ordering::Relaxed);
                io::ErrorKind::NotConnected
            }
            _ => io::ErrorKind::Other,
        };
        let msg = match (code, said) {
            (NOT_FOUND, _) => "no such file or directory".to_string(),
            (EXISTS, _) => "already exists".to_string(),
            (WRONG_KIND, _) => "not the kind of file asked for (a directory, or not one)".into(),
            (_, "") => format!("the host's shell exited with {code}"),
            (_, s) => s.to_string(),
        };
        Err(io::Error::new(kind, msg))
    }

    /// `call`, its error naming the path.
    fn on(&self, path: &Path, script: &str, stdin: Option<&[u8]>) -> io::Result<Vec<u8>> {
        self.call(script, stdin)
            .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))
    }

    fn text(&self, path: &Path, script: &str) -> io::Result<String> {
        let out = self.on(path, script, None)?;
        Ok(String::from_utf8_lossy(&out).into_owned())
    }
}

impl Fs for ShellFs {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        let p = q(path);
        self.on(
            path,
            &format!("p={p}\nex \"$p\" || exit {NOT_FOUND}\n[ -d \"$p\" ] && exit {WRONG_KIND}\nexec cat \"$p\"\n"),
            None,
        )
    }

    /// Into a sibling first, its length checked (a cut connection ends
    /// `cat` as an end of input would), then copied over the file —
    /// which keeps the file's mode, owner and links — or moved where
    /// there was none.
    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let (p, t, n) = (q(path), q(&sibling(path)), bytes.len());
        let script = format!(
            "p={p}; t={t}\n[ -d \"$p\" ] && exit {WRONG_KIND}\n\
             cat > \"$t\" || {{ rm -f \"$t\"; exit 1; }}\n\
             set -- $(wc -c < \"$t\")\n\
             [ \"$1\" = {n} ] || {{ rm -f \"$t\"; echo \"the write was cut short: $1 of {n} bytes arrived\" >&2; exit 1; }}\n\
             if [ -f \"$p\" ]; then cat \"$t\" > \"$p\" || {{ rm -f \"$t\"; exit 1; }}; rm -f \"$t\"; else mv -f \"$t\" \"$p\"; fi\n"
        );
        self.on(path, &script, Some(bytes)).map(|_| ())
    }

    fn stat(&self, path: &Path) -> io::Result<Stat> {
        let out = self.text(
            path,
            &format!(
                "p={}\nex \"$p\" || exit {NOT_FOUND}\nkind \"$p\"\n",
                q(path)
            ),
        )?;
        let fields: Vec<&str> = out.split_whitespace().collect();
        parse_kind(&fields).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: the host's answer was not read: {out}", path.display()),
            )
        })
    }

    fn list(&self, dir: &Path) -> io::Result<Vec<Entry>> {
        let script = format!(
            "d={}\n[ -d \"$d\" ] || {{ ex \"$d\" && exit {WRONG_KIND}; exit {NOT_FOUND}; }}\n\
             cd \"$d\" || exit 1\n\
             for f in * .[!.]* ..?*; do ex \"$f\" || continue; printf '%s %s\\n' \"$(kind \"$f\")\" \"$f\"; done\n",
            q(dir)
        );
        let out = self.text(dir, &script)?;
        Ok(out
            .lines()
            .filter_map(|line| {
                let f: Vec<&str> = line.splitn(5, ' ').collect();
                let (st, name) = (parse_kind(f.get(..4)?)?, *f.get(4)?);
                Some(Entry {
                    name: name.to_string(),
                    is_dir: st.is_dir,
                    is_symlink: st.is_symlink,
                    size: st.size,
                    modified: st.modified,
                })
            })
            .collect())
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let mk = host_parent(to)
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| format!("mkdir -p {} && ", q(p)))
            .unwrap_or_default();
        let script = format!(
            "f={}; t={}\nex \"$f\" || exit {NOT_FOUND}\nex \"$t\" && exit {EXISTS}\n{mk}mv \"$f\" \"$t\"\n",
            q(from),
            q(to)
        );
        self.on(from, &script, None).map(|_| ())
    }

    fn remove(&self, path: &Path) -> io::Result<()> {
        let script = format!(
            "p={}\nex \"$p\" || exit {NOT_FOUND}\nrm -rf \"$p\"\n",
            q(path)
        );
        self.on(path, &script, None).map(|_| ())
    }

    fn create(&self, path: &Path, is_dir: bool) -> io::Result<()> {
        let script = if is_dir {
            format!("mkdir -p {}\n", q(path))
        } else {
            let mk = host_parent(path)
                .filter(|p| !p.as_os_str().is_empty())
                .map(|p| format!("mkdir -p {} && ", q(p)))
                .unwrap_or_default();
            format!(
                "p={}\nex \"$p\" && exit {EXISTS}\n{mk}: > \"$p\"\n",
                q(path)
            )
        };
        self.on(path, &script, None).map(|_| ())
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        let script = format!(
            "p={}\nex \"$p\" || exit {NOT_FOUND}\n\
             r=$(readlink -f \"$p\" 2>/dev/null) && [ -n \"$r\" ] && {{ printf '%s\\n' \"$r\"; exit 0; }}\n\
             if [ -d \"$p\" ]; then cd \"$p\" && pwd -P; exit; fi\n\
             cd \"${{p%/*}}/\" || exit 1; d=$(pwd -P); [ \"$d\" = / ] && d=\n\
             printf '%s/%s\\n' \"$d\" \"${{p##*/}}\"\n",
            q(path)
        );
        let out = self.text(path, &script)?;
        let line = out.lines().next().unwrap_or("").trim_end_matches('\r');
        if !line.starts_with('/') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{}: the host's answer was not a path: {out}",
                    path.display()
                ),
            ));
        }
        Ok(PathBuf::from(line))
    }

    fn set_mode(&self, path: &Path, mode: u32) -> io::Result<()> {
        self.on(
            path,
            &format!("chmod {:o} {}\n", mode & 0o7777, q(path)),
            None,
        )
        .map(|_| ())
    }

    fn is_alive(&self) -> bool {
        !self.dead.load(Ordering::Relaxed)
    }

    fn via(&self) -> &'static str {
        "shell commands (no SFTP on the host)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A POSIX `sh` here: `/bin/sh`, or Git's on Windows (on `PATH` in
    /// the test environment). None skips.
    fn sh() -> Option<PathBuf> {
        if cfg!(unix) {
            return Some(PathBuf::from("/bin/sh"));
        }
        crate::io::program_path("sh")
    }

    /// The host is this machine through `sh -c`, `$HOME` a directory of
    /// the test's; `shadow` names commands the host has none of (a
    /// function that fails, as a missing one does), so a busybox without
    /// `stat` or `readlink` is this machine too.
    fn host(home: &Path, shadow: &'static [&'static str]) -> Option<ShellFs> {
        let sh = sh()?;
        let home = home.to_path_buf();
        Some(ShellFs::with(move |script| {
            let mut pre = String::new();
            for c in shadow {
                pre.push_str(&format!(
                    "{c}() {{ echo \"{c}: not found\" >&2; return 127; }}\n"
                ));
            }
            let mut c = Command::new(&sh);
            c.arg("-c").arg(format!("{pre}{script}")).env("HOME", &home);
            c
        }))
    }

    fn root(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kawoosh-shellfs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The host's own spelling of a local directory: what its `sh`
    /// says `pwd` is (`/tmp/…` under Git's, the path itself on unix).
    fn host_path(sh: &Path, dir: &Path) -> String {
        let out = crate::spawn::output(Command::new(sh).arg("-c").arg("pwd -P").current_dir(dir))
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn session(tag: &str, shadow: &'static [&'static str]) {
        let dir = root(tag);
        let Some(fs) = host(&dir, shadow) else {
            eprintln!("no sh here: skipped");
            return;
        };
        let hd = host_path(&sh().unwrap(), &dir);
        let at = |p: &str| PathBuf::from(format!("{hd}/{p}"));
        assert!(fs.check().is_ok(), "{:?}", fs.check());
        // Made, written, read back whole — bytes no shell would carry in
        // a quoted word among them.
        let text: Vec<u8> = b"it's \"$HOME\" `x` \\ % \n\ttab\n"
            .iter()
            .copied()
            .chain(0..=255u8)
            .collect();
        fs.create(&at("deep/er"), true).unwrap();
        fs.write(&at("deep/er/a b.bin"), &text).unwrap();
        assert_eq!(fs.read(&at("deep/er/a b.bin")).unwrap(), text);
        assert_eq!(std::fs::read(dir.join("deep/er/a b.bin")).unwrap(), text);
        assert!(
            !dir.join("deep/er/.a b.bin.kawoosh~").exists(),
            "the sibling gone"
        );
        // Written again: in place, shorter.
        fs.write(&at("deep/er/a b.bin"), b"short").unwrap();
        assert_eq!(
            std::fs::read(dir.join("deep/er/a b.bin")).unwrap(),
            b"short"
        );
        let st = fs.stat(&at("deep/er/a b.bin")).unwrap();
        assert!(st.is_file && !st.is_dir && st.size == 5, "{st:?}");
        assert!(st.modified.is_some_and(|m| m > 1_600_000_000), "{st:?}");
        assert!(fs.stat(&at("deep")).unwrap().is_dir);
        assert_eq!(
            fs.stat(&at("nope")).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(
            fs.read(&at("nope")).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        // A listing: hidden entries too, each what it is.
        fs.create(&at("deep/.hid"), false).unwrap();
        assert_eq!(
            fs.create(&at("deep/.hid"), false).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        let mut l: Vec<(String, bool, u64)> = fs
            .list(&at("deep"))
            .unwrap()
            .into_iter()
            .map(|e| (e.name, e.is_dir, if e.is_dir { 0 } else { e.size }))
            .collect();
        l.sort();
        assert_eq!(
            l,
            [(".hid".to_string(), false, 0), ("er".to_string(), true, 0)]
        );
        assert!(fs.list(&at("empty-not-there")).is_err());
        // Renamed into a directory made for it, refused onto one there.
        fs.rename(&at("deep/er/a b.bin"), &at("moved/to/c.bin"))
            .unwrap();
        assert!(dir.join("moved/to/c.bin").is_file());
        assert_eq!(
            fs.rename(&at("deep/.hid"), &at("moved/to/c.bin"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        // The home: `~` is `$HOME`.
        assert_eq!(fs.canonicalize(Path::new("~")).unwrap(), PathBuf::from(&hd));
        assert_eq!(
            fs.canonicalize(Path::new("~/moved/to/c.bin")).unwrap(),
            at("moved/to/c.bin")
        );
        fs.write(Path::new("~/home.txt"), b"h").unwrap();
        assert_eq!(std::fs::read(dir.join("home.txt")).unwrap(), b"h");
        fs.remove(&at("moved")).unwrap();
        assert!(!dir.join("moved").exists());
        assert_eq!(
            fs.remove(&at("moved")).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every operation against a host's shell with its usual tools.
    #[test]
    fn a_hosts_files_go_through_its_shell() {
        session("full", &[]);
    }

    /// And on a host as small as OpenWrt's busybox: no `stat`, no
    /// `readlink`, no `base64`.
    #[test]
    fn a_small_hosts_files_go_through_its_shell() {
        session("small", &["stat", "readlink", "base64"]);
    }

    /// Against a real small host, when one is named: a container
    /// (`KAWOOSH_TEST_CONTAINER=NAME`, run as `docker exec -i NAME`) —
    /// OpenWrt's rootfs, busybox's ash with no `stat`, `base64` or bash.
    /// Each script reaches it as an ssh host's login shell would hand it
    /// over: the transport's one line, through `sh -c`.
    #[test]
    fn a_containers_files_go_through_its_shell() {
        let Ok(name) = std::env::var("KAWOOSH_TEST_CONTAINER") else {
            eprintln!("KAWOOSH_TEST_CONTAINER not set: skipped");
            return;
        };
        let line_of = |script: &str| {
            crate::io::Ssh {
                ssh: "ssh".into(),
                host: "h".into(),
                ctl: "/c".into(),
                master: true,
            }
            .remote_argv(script, false, None)
            .pop()
            .unwrap()
        };
        let dir = format!("/tmp/kawoosh-shellfs-{}", std::process::id());
        let (n, d) = (name.clone(), dir.clone());
        let fs = ShellFs::with(move |script| {
            let mut c = Command::new("docker");
            c.args(["exec", "-i", "-e", &format!("HOME={d}"), &n, "sh", "-c"])
                .arg(line_of(script));
            c
        });
        let at = |p: &str| PathBuf::from(format!("{dir}/{p}"));
        let sh = |s: &str| {
            let out =
                crate::spawn::output(Command::new("docker").args(["exec", &name, "sh", "-c", s]))
                    .unwrap();
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        sh(&format!("rm -rf {dir}; mkdir -p {dir}"));
        fs.check().unwrap();
        let text: Vec<u8> = b"it's \"$HOME\" `x` \\ % \n"
            .iter()
            .copied()
            .chain(0..=255u8)
            .collect();
        fs.write(&at("a b.bin"), &text).unwrap();
        assert_eq!(fs.read(&at("a b.bin")).unwrap(), text);
        sh(&format!("chmod 751 '{dir}/a b.bin'"));
        fs.write(&at("a b.bin"), b"again").unwrap();
        assert_eq!(
            sh(&format!("ls -l '{dir}/a b.bin' | cut -c1-10")).trim(),
            "-rwxr-x--x",
            "the mode kept"
        );
        let st = fs.stat(&at("a b.bin")).unwrap();
        assert!(
            st.is_file && st.size == 5 && st.modified.is_some(),
            "{st:?}"
        );
        fs.create(&at("sub/deeper"), true).unwrap();
        fs.create(&at(".hid"), false).unwrap();
        sh(&format!(
            "ln -s /nowhere {dir}/dangling; ln -s {dir}/sub {dir}/to-sub"
        ));
        let mut l: Vec<(String, bool, bool)> = fs
            .list(Path::new(&dir))
            .unwrap()
            .into_iter()
            .map(|e| (e.name, e.is_dir, e.is_symlink))
            .collect();
        l.sort();
        assert_eq!(
            l,
            [
                (".hid".into(), false, false),
                ("a b.bin".into(), false, false),
                ("dangling".into(), false, true),
                ("sub".into(), true, false),
                ("to-sub".into(), true, true),
            ]
        );
        assert_eq!(
            fs.canonicalize(Path::new("~")).unwrap(),
            PathBuf::from(&dir)
        );
        assert_eq!(fs.canonicalize(&at("to-sub")).unwrap(), at("sub"));
        fs.rename(&at("a b.bin"), &at("sub/x/c")).unwrap();
        fs.set_mode(&at("sub/x/c"), 0o700).unwrap();
        assert_eq!(
            sh(&format!("ls -l {dir}/sub/x/c | cut -c1-10")).trim(),
            "-rwx------"
        );
        fs.remove(&at("sub")).unwrap();
        assert_eq!(
            fs.stat(&at("sub")).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        sh(&format!("rm -rf {dir}"));
    }

    #[test]
    fn a_hosts_path_is_quoted_with_its_home() {
        assert_eq!(q(Path::new("~")), "\"$HOME\"");
        assert_eq!(q(Path::new("~/a b")), "\"$HOME\"'/a b'");
        assert_eq!(q(Path::new("/etc/x")), "/etc/x");
        assert_eq!(q(Path::new("it's")), "\"$HOME\"/'it'\\''s'");
        assert_eq!(sibling(Path::new("/d/x")), PathBuf::from("/d/.x.kawoosh~"));
        assert_eq!(sibling(Path::new("~/x")), PathBuf::from("~/.x.kawoosh~"));
    }
}
