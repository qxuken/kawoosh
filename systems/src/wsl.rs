//! A WSL distro as a domain (docs/design/domains.md, "WSL, and the
//! picker"): its processes through `wsl.exe -e`, its files through the
//! share Windows serves them on (`\\wsl.localhost\DISTRO`), and the two
//! spellings of one file — the distro's `/mnt/c/x`, the share's
//! `\\wsl.localhost\DISTRO\x` — put back to one.
//!
//! Nothing here is Windows-only to compile; [`exe`] is `None` elsewhere,
//! and with it every distro.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use kawoosh_doc::fs::{Entry, Fs, Stat};

/// `wsl.exe`, where Windows keeps it.
pub fn exe() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let p = Path::new(&root).join("System32").join("wsl.exe");
    p.is_file().then_some(p)
}

/// What the probe learned of a distro at connect (W5).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Probe {
    /// The distro's own name, `Ubuntu-24.04` — the default's too.
    pub distro: String,
    pub home: String,
    /// Where the drives are, `/mnt/` (`/mnt/c` is `C:\`).
    pub mount: String,
    /// The login shell's `PATH`, exported to every process.
    pub path: Option<String>,
}

/// The probe's script: one `key=value` a line; a login shell that
/// prints a banner is read past.
const PROBE: &str = r#"echo "distro=$WSL_DISTRO_NAME"
echo "home=$HOME"
echo "mnt=$(wslpath -u 'C:\' 2>/dev/null)"
"${SHELL:-/bin/sh}" -l -c env </dev/null 2>/dev/null | grep '^PATH='
"#;

impl Probe {
    fn parse(out: &str) -> Option<Probe> {
        let mut p = Probe::default();
        for line in out.lines() {
            let line = line.trim_end_matches('\r');
            if let Some(v) = line.strip_prefix("distro=") {
                p.distro = v.to_string();
            } else if let Some(v) = line.strip_prefix("home=") {
                p.home = v.to_string();
            } else if let Some(v) = line.strip_prefix("mnt=") {
                // `/mnt/c/` → `/mnt/`.
                let v = v.trim_end_matches('/');
                if let Some(root) = v.strip_suffix("/c") {
                    p.mount = format!("{root}/");
                }
            } else if let Some(v) = line.strip_prefix("PATH=") {
                p.path = Some(v.to_string()).filter(|v| !v.is_empty());
            }
        }
        if p.mount.is_empty() {
            p.mount = "/mnt/".into();
        }
        (!p.distro.is_empty() && p.home.starts_with('/')).then_some(p)
    }
}

/// How a distro's processes are started: `wsl.exe [-d DISTRO] -e …`.
/// `distro` is the one the settings name, or none for the default;
/// `probe` what connecting learned.
#[derive(Clone, Debug)]
pub struct Wsl {
    pub exe: PathBuf,
    pub distro: Option<String>,
    pub probe: Option<Probe>,
}

impl Wsl {
    pub fn new(distro: Option<String>) -> Option<Wsl> {
        Some(Wsl {
            exe: exe()?,
            distro,
            probe: None,
        })
    }

    /// `wsl.exe [-d DISTRO] -e ARGS…`, as a command to run; its own
    /// messages in UTF-8 rather than UTF-16.
    pub fn command(&self, args: &[&str]) -> Command {
        let mut c = crate::io::command(&self.exe);
        c.env("WSL_UTF8", "1");
        if let Some(d) = &self.distro {
            c.arg("-d").arg(d);
        }
        c.arg("-e").args(args);
        c
    }

    /// The argv that runs `script` — POSIX sh — in the distro: `-e`, so
    /// no login shell reads it (Windows's quoting reaches one mangled,
    /// W6), the script in base64 as on ssh, the login shell's `PATH`
    /// exported first.
    pub fn remote_argv(&self, script: &str) -> Vec<String> {
        let mut v = vec![self.exe.display().to_string()];
        if let Some(d) = &self.distro {
            v.push("-d".into());
            v.push(d.clone());
        }
        let script = match self.probe.as_ref().and_then(|p| p.path.as_deref()) {
            Some(path) => format!("export PATH={}\n{script}", crate::io::shell_quote(path)),
            None => script.to_string(),
        };
        v.extend(["-e".into(), "sh".into(), "-c".into()]);
        v.push(format!(
            "eval \"$(echo {} | base64 -d)\"",
            crate::io::base64(script.as_bytes())
        ));
        v
    }

    pub fn remote_command(&self, script: &str) -> Command {
        let argv = self.remote_argv(script);
        let mut c = crate::io::command(&argv[0]);
        c.args(&argv[1..]).env("WSL_UTF8", "1");
        c
    }

    /// Starts the distro if it is not running and asks it what it is.
    pub fn probe(&self) -> Result<Probe, String> {
        let out = crate::spawn::output(&mut self.remote_command(PROBE))
            .map_err(|e| format!("wsl.exe: {e}"))?;
        let text = String::from_utf8_lossy(&out.stdout);
        Probe::parse(&text).ok_or_else(|| {
            let err = String::from_utf8_lossy(&out.stderr);
            let err = err.trim();
            match self.distro.as_deref() {
                _ if !err.is_empty() => err.lines().next().unwrap_or(err).to_string(),
                Some(d) => format!("{d}: the distro did not answer"),
                None => "the default distro did not answer".into(),
            }
        })
    }

    /// The probe, its distro's share, the files there: what a domain on
    /// it registers.
    pub fn connect(mut self) -> Result<(Wsl, WslFs), String> {
        let probe = self.probe()?;
        let root = share(&probe.distro)
            .ok_or_else(|| format!("\\\\wsl.localhost\\{}: not there", probe.distro))?;
        self.probe = Some(probe.clone());
        let fs = WslFs {
            root,
            home: probe.home.clone(),
            wsl: self.clone(),
        };
        Ok((self, fs))
    }
}

/// The share a distro's files are served on: `\\wsl.localhost\DISTRO`,
/// or `\\wsl$\DISTRO` before Windows 11.
pub fn share(distro: &str) -> Option<PathBuf> {
    ["wsl.localhost", "wsl$"]
        .iter()
        .map(|h| PathBuf::from(format!("\\\\{h}\\{distro}\\")))
        .find(|p| p.is_dir())
}

/// A path on the share split into its distro and the distro's own path:
/// `\\wsl.localhost\Ubuntu\home\me` (or `\\wsl$\…`, `\\?\UNC\…`, with
/// `/` for `\`) is `("Ubuntu", "/home/me")`.
pub fn on_share(path: &Path) -> Option<(String, String)> {
    let s = path.to_str()?.replace('/', "\\");
    let rest = s
        .strip_prefix("\\\\?\\UNC\\")
        .or_else(|| s.strip_prefix("\\\\"))?;
    let (host, rest) = rest.split_once('\\')?;
    if !host.eq_ignore_ascii_case("wsl.localhost") && !host.eq_ignore_ascii_case("wsl$") {
        return None;
    }
    let (distro, rest) = rest.split_once('\\').unwrap_or((rest, ""));
    if distro.is_empty() {
        return None;
    }
    let rest = rest.trim_end_matches('\\').replace('\\', "/");
    Some((distro.to_string(), format!("/{rest}")))
}

/// A distro's path on a drive as the local one: `/mnt/c/x` → `C:\x`
/// (`mount` `/mnt/`). None for a path off the drives.
pub fn local_of(path: &str, mount: &str) -> Option<PathBuf> {
    let rest = path.strip_prefix(mount)?;
    let (drive, rest) = rest.split_once('/').unwrap_or((rest, ""));
    let mut d = drive.chars();
    let letter = d
        .next()
        .filter(|c| c.is_ascii_alphabetic() && d.next().is_none())?;
    let rest = rest.trim_end_matches('/').replace('/', "\\");
    Some(PathBuf::from(format!(
        "{}:\\{rest}",
        letter.to_ascii_uppercase()
    )))
}

/// A local path as a distro sees it: `C:\x\y` → `/mnt/c/x/y`. None for
/// one off a drive (a share, a device).
pub fn mounted(path: &Path, mount: &str) -> Option<String> {
    let s = path.to_str()?;
    let s = s.strip_prefix("\\\\?\\").unwrap_or(s);
    let mut c = s.chars();
    let letter = c.next().filter(char::is_ascii_alphabetic)?;
    if c.next() != Some(':') {
        return None;
    }
    let rest = c.as_str().replace('\\', "/");
    let rest = rest.trim_matches('/');
    let mut out = format!("{mount}{}", letter.to_ascii_lowercase());
    if !rest.is_empty() {
        out.push('/');
        out.push_str(rest);
    }
    Some(out)
}

/// The distros WSL has, each with whether it is the default:
/// `wsl.exe -l -v`, read.
pub fn distros() -> io::Result<Vec<(String, bool)>> {
    let Some(exe) = exe() else {
        return Ok(Vec::new());
    };
    let mut c = crate::io::command(exe);
    c.env("WSL_UTF8", "1").args(["-l", "-v"]);
    let out = crate::spawn::output(&mut c)?;
    if !out.status.success() {
        // No distro installed is a failure of `-l`; nothing to offer.
        return Ok(Vec::new());
    }
    Ok(parse_list(&decode(&out.stdout)))
}

/// `wsl.exe`'s output: UTF-8 when it heeded `WSL_UTF8`, else UTF-16.
fn decode(bytes: &[u8]) -> String {
    let utf16 = bytes.len() >= 2 && bytes.len().is_multiple_of(2) && bytes[1] == 0;
    if utf16 {
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

/// `  NAME  STATE  VERSION` rows under a head, `*` before the default.
fn parse_list(text: &str) -> Vec<(String, bool)> {
    text.lines()
        .skip(1)
        .filter_map(|l| {
            let l = l.trim_start_matches('\u{feff}').trim();
            let (default, l) = match l.strip_prefix('*') {
                Some(rest) => (true, rest.trim_start()),
                None => (false, l),
            };
            let name = l.split_whitespace().next()?;
            Some((name.to_string(), default))
        })
        .collect()
}

/// The CLI in a distro (W7): the Windows kawoosh run through interop,
/// each path `edit` is handed made Windows's first — the share's for
/// the distro's own files, the drive's for `/mnt/c`. POSIX sh, so a
/// busybox distro runs it. `kawoosh-edit` beside it is `$EDITOR`.
pub const SHIM: &str = r#"#!/bin/sh
# kawoosh in WSL (kawoosh's docs/design/domains.md, W7): the Windows
# kawoosh, run through interop, every path edit is handed made Windows's.
exe=${KAWOOSH_EXE:?kawoosh: not in a kawoosh terminal}
win() {
  case $1 in /*) p=$1 ;; *) p=$PWD/$1 ;; esac
  d=$(dirname -- "$p") b=$(basename -- "$p")
  w=$(wslpath -w "$d") || return 1
  case $w in *\\) printf '%s%s' "$w" "$b" ;; *) printf '%s\\%s' "$w" "$b" ;; esac
}
verb=$1
[ "$verb" = edit ] || exec "$exe" "$@"
shift
n=$#
while [ "$n" -gt 0 ]; do
  a=$1; shift; n=$((n - 1))
  case $a in -*|+[0-9]*) ;; *) a=$(win "$a") || exit 1 ;; esac
  set -- "$@" "$a"
done
exec "$exe" edit "$@"
"#;

/// A distro's files through its share (W4). Paths in and out are the
/// distro's own — `/home/me/x`, `~/x` — never the share's.
pub struct WslFs {
    root: PathBuf,
    home: String,
    wsl: Wsl,
}

impl WslFs {
    /// The distro's path with its `~` its home and `.`/`..` folded.
    fn host(&self, path: &Path) -> String {
        let s = path.to_string_lossy().replace('\\', "/");
        let s = match s.strip_prefix('~') {
            Some(rest) if rest.is_empty() || rest.starts_with('/') => {
                format!("{}{rest}", self.home)
            }
            _ if s.starts_with('/') => s,
            _ => format!("{}/{s}", self.home),
        };
        let mut parts: Vec<&str> = Vec::new();
        for c in s.split('/') {
            match c {
                "" | "." => {}
                ".." => {
                    parts.pop();
                }
                c => parts.push(c),
            }
        }
        format!("/{}", parts.join("/"))
    }

    /// Where the share serves the distro's path.
    pub fn local(&self, path: &Path) -> PathBuf {
        let mut p = self.root.clone();
        for c in self.host(path).split('/').filter(|c| !c.is_empty()) {
            p.push(c);
        }
        p
    }
}

fn modified(m: &std::fs::Metadata) -> Option<u64> {
    m.modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

impl Fs for WslFs {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(self.local(path))
    }

    /// In place for a file that is there — its mode, owner and links
    /// the file's still, which a sibling renamed over through the share
    /// would lose — and created for one that is not.
    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        use std::io::Write;
        let p = self.local(path);
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&p)?;
        f.write_all(bytes)?;
        f.flush()
    }

    fn stat(&self, path: &Path) -> io::Result<Stat> {
        let p = self.local(path);
        let link = std::fs::symlink_metadata(&p)?;
        let m = std::fs::metadata(&p).unwrap_or_else(|_| link.clone());
        Ok(Stat {
            is_dir: m.is_dir(),
            is_file: m.is_file(),
            is_symlink: link.file_type().is_symlink(),
            size: m.len(),
            modified: modified(&m),
        })
    }

    fn list(&self, dir: &Path) -> io::Result<Vec<Entry>> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(self.local(dir))? {
            let e = e?;
            let name = e.file_name().to_string_lossy().into_owned();
            let is_symlink = e.file_type().is_ok_and(|t| t.is_symlink());
            let m = std::fs::metadata(e.path()).ok();
            out.push(Entry {
                name,
                is_dir: m.as_ref().is_some_and(|m| m.is_dir()),
                is_symlink,
                size: m.as_ref().map_or(0, |m| m.len()),
                modified: m.as_ref().and_then(modified),
            });
        }
        Ok(out)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let to = self.local(to);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(self.local(from), to)
    }

    fn remove(&self, path: &Path) -> io::Result<()> {
        let p = self.local(path);
        if std::fs::symlink_metadata(&p)?.is_dir() {
            std::fs::remove_dir_all(p)
        } else {
            std::fs::remove_file(p)
        }
    }

    fn create(&self, path: &Path, is_dir: bool) -> io::Result<()> {
        let p = self.local(path);
        if is_dir {
            return std::fs::create_dir_all(p);
        }
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(p)
            .map(drop)
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        let host = self.host(path);
        // Through the share, a link resolves as Windows reads it; where
        // that fails the path folded is the answer.
        let real = std::fs::canonicalize(self.local(Path::new(&host)))
            .ok()
            .and_then(|p| on_share(&p))
            .map(|(_, p)| p);
        if self.local(Path::new(&host)).exists() || real.is_some() {
            return Ok(PathBuf::from(real.unwrap_or(host)));
        }
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{host}: no such file or directory"),
        ))
    }

    fn set_mode(&self, path: &Path, mode: u32) -> io::Result<()> {
        let host = self.host(path);
        let status = crate::spawn::status(
            self.wsl
                .command(&["chmod", &format!("{mode:o}"), &host])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null()),
        )?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!("chmod {host}: failed")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_share_is_read_as_a_distro_and_its_path() {
        let d = |s: &str| on_share(Path::new(s));
        let ub = |p: &str| Some(("Ubuntu".to_string(), p.to_string()));
        assert_eq!(d(r"\\wsl.localhost\Ubuntu\home\me"), ub("/home/me"));
        assert_eq!(d(r"\\WSL$\Ubuntu\home\me\"), ub("/home/me"));
        assert_eq!(d(r"\\?\UNC\wsl.localhost\Ubuntu\etc"), ub("/etc"));
        assert_eq!(d("//wsl.localhost/Ubuntu/etc/hosts"), ub("/etc/hosts"));
        assert_eq!(d(r"\\wsl.localhost\Ubuntu"), ub("/"));
        assert_eq!(d(r"\\server\share\x"), None);
        assert_eq!(d(r"C:\Users"), None);
        assert_eq!(d("/home/me"), None);
    }

    #[test]
    fn a_drive_is_spelled_both_ways() {
        assert_eq!(
            local_of("/mnt/c/Users/me", "/mnt/"),
            Some(PathBuf::from(r"C:\Users\me"))
        );
        assert_eq!(local_of("/mnt/e", "/mnt/"), Some(PathBuf::from(r"E:\")));
        assert_eq!(local_of("/mnt/wsl/x", "/mnt/"), None);
        assert_eq!(local_of("/home/me", "/mnt/"), None);
        assert_eq!(local_of("/c/x", "/"), Some(PathBuf::from(r"C:\x")));
        assert_eq!(
            mounted(Path::new(r"C:\Program Files\k.exe"), "/mnt/").as_deref(),
            Some("/mnt/c/Program Files/k.exe")
        );
        assert_eq!(
            mounted(Path::new(r"\\?\E:\x"), "/mnt/").as_deref(),
            Some("/mnt/e/x")
        );
        assert_eq!(
            mounted(Path::new(r"E:\"), "/mnt/").as_deref(),
            Some("/mnt/e")
        );
        assert_eq!(mounted(Path::new(r"\\wsl.localhost\U\x"), "/mnt/"), None);
    }

    #[test]
    fn the_probe_and_the_list_are_read() {
        let p =
            Probe::parse("Welcome!\ndistro=Ubuntu-24.04\nhome=/home/me\nmnt=/mnt/c/\nPATH=/a:/b\n")
                .unwrap();
        assert_eq!(p.distro, "Ubuntu-24.04");
        assert_eq!(p.home, "/home/me");
        assert_eq!(p.mount, "/mnt/");
        assert_eq!(p.path.as_deref(), Some("/a:/b"));
        assert_eq!(Probe::parse("distro=\nhome=/x\n"), None);
        let list = "  NAME            STATE           VERSION\r\n\
                    * Ubuntu-24.04    Stopped         2\r\n  \
                    docker-desktop  Running         2\r\n";
        assert_eq!(
            parse_list(list),
            vec![
                ("Ubuntu-24.04".to_string(), true),
                ("docker-desktop".to_string(), false)
            ]
        );
        let wide: Vec<u8> = "  NAME\r\n* Deb  Stopped  2\r\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(parse_list(&decode(&wide)), vec![("Deb".to_string(), true)]);
    }

    #[test]
    fn a_script_is_handed_over_whole() {
        let w = Wsl {
            exe: PathBuf::from("wsl.exe"),
            distro: Some("Deb".into()),
            probe: Some(Probe {
                path: Some("/opt/x:/usr/bin".into()),
                ..Default::default()
            }),
        };
        let argv = w.remote_argv("echo hi\n");
        assert_eq!(&argv[..6], ["wsl.exe", "-d", "Deb", "-e", "sh", "-c"]);
        // The login shell's PATH first, then the script as it was.
        let script = crate::io::base64(b"export PATH=/opt/x:/usr/bin\necho hi\n");
        assert_eq!(argv[6], format!("eval \"$(echo {script} | base64 -d)\""));
        assert_eq!(argv.len(), 7);
    }

    /// Against the real distro, where there is one: connected, a file
    /// written, read, listed, its mode kept by a second write, renamed
    /// and removed through the share, and a process there in its
    /// directory with the login `PATH`.
    #[test]
    fn a_distro_is_reached_through_its_share() {
        let Some(w) = Wsl::new(None) else {
            eprintln!("skipped: no wsl.exe");
            return;
        };
        let (w, fs) = match w.connect() {
            Ok(x) => x,
            Err(e) => {
                eprintln!("skipped: {e}");
                return;
            }
        };
        let dir = format!("/tmp/kawoosh-wsl-test-{}", std::process::id());
        let d = Path::new(&dir);
        fs.create(d, true).unwrap();
        let f = PathBuf::from(format!("{dir}/a.sh"));
        fs.write(&f, b"echo one\n").unwrap();
        fs.set_mode(&f, 0o755).unwrap();
        fs.write(&f, b"echo two\n").unwrap();
        assert_eq!(fs.read(&f).unwrap(), b"echo two\n");
        assert!(fs.stat(&f).unwrap().is_file);
        let names: Vec<String> = fs.list(d).unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["a.sh"]);
        let script = crate::io::remote_script(d, &[], "exec ./a.sh", false);
        let out = crate::spawn::output(&mut w.remote_command(&script)).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            "two",
            "the mode kept"
        );
        let g = PathBuf::from(format!("{dir}/sub/b.sh"));
        fs.rename(&f, &g).unwrap();
        assert!(fs.stat(&f).is_err());
        assert_eq!(
            fs.canonicalize(Path::new(&format!("{dir}/sub/../sub/b.sh")))
                .unwrap(),
            g
        );
        fs.remove(d).unwrap();
        assert!(fs.stat(d).is_err());
    }
}
