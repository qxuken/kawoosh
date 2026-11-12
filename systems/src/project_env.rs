//! Project environments supplied by plugins. Providers print a JSON
//! object of strings; they run only on job/LSP threads, never while a
//! window is drawing. The cache is per provider, directory and domain.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub type Environment = Vec<(String, String)>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Provider {
    pub command: Vec<String>,
    /// Names relative to every ancestor of the working directory.
    pub watch: Vec<String>,
    /// With markers, the provider applies only inside such a project.
    pub markers: Vec<String>,
    /// The provider process's own environment (not the child's).
    pub env: Environment,
}

#[derive(Default, Debug)]
struct Cache {
    generation: u64,
    providers: BTreeMap<String, Provider>,
    entries: HashMap<(String, PathBuf), Arc<Mutex<Entry>>>,
}

#[derive(Default, Debug)]
struct Entry {
    at: Option<Instant>,
    fingerprint: u64,
    value: Environment,
}

/// One application's provider definitions and cache, shared with its workers.
#[derive(Default, Debug)]
pub struct Registry(Mutex<Cache>);

impl Registry {
    pub fn register(&self, name: String, provider: Option<Provider>) {
        let mut c = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if c.providers.get(&name) == provider.as_ref() {
            return;
        }
        c.generation = c.generation.wrapping_add(1);
        c.entries.retain(|(n, _), _| n != &name);
        match provider {
            Some(p) => {
                c.providers.insert(name, p);
            }
            None => {
                c.providers.remove(&name);
            }
        }
    }

    pub fn registered(&self) -> bool {
        !self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .providers
            .is_empty()
    }

    /// No provider or shell is run for this launch-time availability check.
    pub fn available(&self, dir: &Path) -> bool {
        let c = self.0.lock().unwrap_or_else(|e| e.into_inner());
        c.providers.values().any(|p| {
            crate::fs::domain_of(dir).is_some()
                || (applies(p, dir)
                    && p.command
                        .first()
                        .is_some_and(|p| crate::io::on_path(p) != Some(false)))
        })
    }

    /// Invalidation never waits for a provider running on another thread.
    pub fn invalidate(&self, name: &str, dir: Option<&Path>) {
        let mut cache = self.0.lock().unwrap_or_else(|e| e.into_inner());
        cache.generation = cache.generation.wrapping_add(1);
        cache
            .entries
            .retain(|(n, d), _| n != name || dir.is_some_and(|p| p != d));
    }

    /// A cheap token for application caches; no file or provider reads.
    pub fn generation(&self) -> u64 {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).generation
    }

    /// Resolve from a worker. Explicit child variables must be applied after
    /// this answer, so a caller's own override always wins.
    pub fn resolve(&self, dir: &Path) -> Environment {
        self.resolve_attempt(dir, 2)
    }

    fn resolve_attempt(&self, dir: &Path, retries: usize) -> Environment {
        let providers: Vec<_> = self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .providers
            .iter()
            .map(|(n, p)| (n.clone(), p.clone()))
            .collect();
        let mut result = BTreeMap::new();
        for (name, provider) in providers {
            if !applies(&provider, dir) {
                continue;
            }
            let key = (name, dir.to_path_buf());
            let entry = {
                let mut c = self.0.lock().unwrap_or_else(|e| e.into_inner());
                // Bounded even while browsing many projects. Old Arcs can
                // finish safely, but cannot repopulate an invalidated key.
                if c.entries.len() >= 64 && !c.entries.contains_key(&key) {
                    let remove = c.entries.keys().next().cloned();
                    if let Some(k) = remove {
                        c.entries.remove(&k);
                    }
                }
                c.entries.entry(key.clone()).or_default().clone()
            };
            let mut e = entry.lock().unwrap_or_else(|e| e.into_inner());
            let fingerprint = fingerprint(dir, &provider.watch);
            if e.at.is_none_or(|at| at.elapsed() >= Duration::from_secs(5))
                || e.fingerprint != fingerprint
            {
                e.value = ask(&provider, dir).unwrap_or_default();
                e.at = Some(Instant::now());
                e.fingerprint = fingerprint;
            }
            let value = e.value.clone();
            drop(e);
            let current = {
                let c = self.0.lock().unwrap_or_else(|e| e.into_inner());
                c.providers.get(&key.0) == Some(&provider)
                    && c.entries
                        .get(&key)
                        .is_some_and(|now| Arc::ptr_eq(now, &entry))
            };
            // A settings/file change may have invalidated this entry
            // during provider execution. Never install the old answer.
            if !current {
                return if retries > 0 {
                    self.resolve_attempt(dir, retries - 1)
                } else {
                    Vec::new()
                };
            }
            result.extend(value);
        }
        result.into_iter().collect()
    }
}

fn applies(provider: &Provider, dir: &Path) -> bool {
    provider.markers.is_empty()
        || crate::fs::domain_of(dir).is_some()
        || dir
            .ancestors()
            .take(64)
            .any(|d| provider.markers.iter().any(|m| d.join(m).exists()))
}

fn fingerprint(dir: &Path, names: &[String]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    // Remote providers expire after five seconds; their plugin's file
    // watcher can invalidate sooner. Do not turn every launch into one
    // network stat per ancestor and per possible config filename.
    if let Some((name, _)) = crate::fs::domain_of(dir) {
        crate::io::transport_of(name).map(|t| t.key()).hash(&mut h);
        return h.finish();
    }
    for parent in dir.ancestors().take(64) {
        for name in names {
            let path = parent.join(name);
            path.hash(&mut h);
            if let Ok(m) = std::fs::metadata(&path) {
                m.modified().ok().hash(&mut h);
                m.len().hash(&mut h);
                // Config fragments and file tasks: directory entries'
                // stamps catch a rewrite as well as an add/remove.
                if m.is_dir()
                    && let Ok(entries) = std::fs::read_dir(path)
                {
                    let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).take(256).collect();
                    paths.sort();
                    for p in paths {
                        p.hash(&mut h);
                        if let Ok(m) = p.metadata() {
                            m.modified().ok().hash(&mut h);
                            m.len().hash(&mut h);
                        }
                    }
                }
            }
        }
    }
    h.finish()
}

const PATIENCE: Duration = Duration::from_secs(5);
const OUTPUT_MAX: u64 = 4 << 20;

fn ask(provider: &Provider, dir: &Path) -> Option<Environment> {
    ask_for(provider, dir, PATIENCE)
}

fn ask_for(provider: &Provider, dir: &Path, patience: Duration) -> Option<Environment> {
    let (program, args) = provider.command.split_first()?;
    let start = Instant::now();
    let process = if let Some((domain, path)) = crate::fs::domain_of(dir) {
        let transport = crate::io::transport_of(domain)?;
        let line = format!(
            "exec {}",
            provider
                .command
                .iter()
                .map(|s| crate::io::shell_quote(s))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let script = crate::io::remote_script(path, &provider.env, &line, false);
        // Own channel/process, never the shared runner whose serialized
        // socket another provider/LSP might otherwise wait behind.
        let script = deadline_script(&script, patience);
        match transport.client() {
            Some(client) => {
                let line = crate::io::ssh_line(&script);
                let remote = client
                    .open_timeout(
                        crate::ssh::Open::Exec {
                            line: &line,
                            pty: None,
                        },
                        patience,
                    )
                    .ok()?;
                Capture::Remote(remote)
            }
            None => Capture::local(transport.remote_command(&script))?,
        }
    } else {
        let mut command = crate::io::command(program);
        command
            .args(args)
            .current_dir(dir)
            .envs(provider.env.iter().cloned());
        Capture::local(command)?
    };
    let bytes = capture(process, start, patience)?;
    let object: BTreeMap<String, String> = serde_json::from_slice(&bytes).ok()?;
    Some(
        object
            .into_iter()
            .filter(|(k, v)| !k.is_empty() && !k.contains(['=', '\0']) && !v.contains('\0'))
            .collect(),
    )
}

/// The host also bounds its own job: closing a local wsl.exe/ssh pipe
/// alone need not stop a process it started on the host. Linux/WSL has
/// setsid; other POSIX hosts still kill the direct provider process.
fn deadline_script(script: &str, patience: Duration) -> String {
    let seconds = patience.as_secs().max(1);
    let quoted = crate::io::shell_quote(script);
    format!(
        "if command -v setsid >/dev/null 2>&1; then setsid sh -c {quoted} & grouped=1; else sh -c {quoted} & grouped=0; fi\njob=$!\n(sleep {seconds}; if [ \"$grouped\" = 1 ]; then kill -KILL -\"$job\"; else kill -KILL \"$job\"; fi) >/dev/null 2>&1 &\nguard=$!\nwait \"$job\"\nresult=$?\nkill \"$guard\" 2>/dev/null\nwait \"$guard\" 2>/dev/null\nexit \"$result\""
    )
}

enum Capture {
    Local {
        child: crate::spawn::Session,
        reaped: bool,
        #[cfg(windows)]
        tree: crate::job::Tree,
    },
    Remote(crate::ssh::Remote),
}

type OutputPipes = (Box<dyn std::io::Read + Send>, Box<dyn std::io::Read + Send>);

impl Capture {
    fn local(mut command: std::process::Command) -> Option<Self> {
        use std::process::Stdio;
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[allow(unused_mut)]
        let mut child = crate::spawn::session(&mut command, false).ok()?;
        #[cfg(windows)]
        let tree = crate::job::Tree::of(child.child_mut());
        Some(Self::Local {
            child,
            reaped: false,
            #[cfg(windows)]
            tree,
        })
    }

    fn pipes(&mut self) -> Option<OutputPipes> {
        match self {
            Self::Local { child, .. } => Some((child.take_stdout()?, child.take_stderr()?)),
            Self::Remote(remote) => {
                drop(remote.stdin.take());
                Some((
                    Box::new(remote.stdout.take()?),
                    Box::new(remote.stderr.take()?),
                ))
            }
        }
    }

    fn poll(&mut self) -> Option<Option<Option<i32>>> {
        match self {
            Self::Local { child, reaped, .. } => {
                let result = child.try_wait().ok()?;
                if result.is_some() {
                    *reaped = true;
                }
                Some(result)
            }
            Self::Remote(remote) => Some(remote.done.try_get()),
        }
    }

    fn kill(&mut self) {
        match self {
            Self::Local {
                child,
                reaped,
                #[cfg(windows)]
                tree,
            } => {
                if *reaped {
                    return;
                }
                #[cfg(windows)]
                tree.kill(child.child_mut());
                child.kill();
                // Reap without leaving the provider/LSP waiting on a
                // kernel that has not completed process teardown yet.
            }
            Self::Remote(remote) => remote.kill(),
        }
    }
}

fn capture(mut process: Capture, start: Instant, patience: Duration) -> Option<Vec<u8>> {
    use std::io::Read;
    let (stdout, stderr) = process.pipes()?;
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.take(OUTPUT_MAX + 1).read_to_end(&mut bytes);
        let _ = tx.send(bytes);
    });
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut stderr.take(OUTPUT_MAX), &mut std::io::sink());
    });
    let mut output = None;
    let mut ended = None;
    loop {
        if output.is_none() {
            output = rx.try_recv().ok();
            if output
                .as_ref()
                .is_some_and(|bytes| bytes.len() as u64 > OUTPUT_MAX)
            {
                process.kill();
                break;
            }
        }
        let status = match ended {
            Some(code) => Some(Some(code)),
            None => process.poll(),
        };
        if let Some(Some(code)) = status {
            ended = Some(code);
        }
        match status {
            Some(Some(Some(0))) if output.is_some() => return output,
            Some(Some(Some(0))) => {}
            Some(Some(_)) => break,
            None => {
                process.kill();
                break;
            }
            Some(None) if start.elapsed() < patience => {}
            Some(None) => {
                process.kill();
                break;
            }
        }
        if start.elapsed() >= patience {
            process.kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if let Capture::Local {
        mut child,
        reaped: false,
        ..
    } = process
    {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hung_remote_provider_is_cancelled_and_the_transport_can_answer_again() {
        let dir = std::env::temp_dir().join(format!("kawoosh-env-timeout-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join(if cfg!(windows) { "ssh.cmd" } else { "ssh" });
        let completed = dir.join("completed");
        let hanging = if cfg!(windows) {
            format!(
                "@echo off\r\nping -n 3 127.0.0.1 >nul\r\necho completed>\"{}\"\r\n",
                completed.display()
            )
        } else {
            format!(
                "#!/bin/sh\nsleep 1\nprintf completed > {}\n",
                crate::io::shell_quote(&completed.display().to_string())
            )
        };
        std::fs::write(&fake, hanging).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let name = "mise-provider-timeout-test";
        crate::io::register_transport(
            name,
            crate::io::Transport::Ssh(crate::io::Ssh {
                ssh: fake.display().to_string(),
                host: "unused".into(),
                ctl: dir.join("ctl"),
                master: true,
                builtin: false,
                client: None,
                fall_back: false,
            }),
        );
        let remote = PathBuf::from(format!("{name}:/work"));
        let provider = Provider {
            command: vec!["mise".into(), "env".into(), "--json".into()],
            ..Default::default()
        };
        let start = Instant::now();
        assert!(ask_for(&provider, &remote, Duration::from_millis(150)).is_none());
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "timeout held the caller: {:?}",
            start.elapsed()
        );
        std::thread::sleep(Duration::from_millis(if cfg!(windows) {
            2300
        } else {
            1200
        }));
        assert!(
            !completed.exists(),
            "the timed-out process continued running"
        );
        let recovered = if cfg!(windows) {
            "@echo off\r\necho {\"PROJECT_VALUE\":\"recovered\"}\r\n"
        } else {
            "#!/bin/sh\nprintf '%s' '{\"PROJECT_VALUE\":\"recovered\"}'\n"
        };
        std::fs::write(&fake, recovered).unwrap();
        assert_eq!(
            ask_for(&provider, &remote, Duration::from_secs(2)),
            Some(vec![("PROJECT_VALUE".into(), "recovered".into())])
        );
        crate::io::unregister_transport(name);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_provider_is_cached_per_directory_and_invalidated_by_its_files() {
        let dir = std::env::temp_dir().join(format!("kawoosh-project-env-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("env.json");
        std::fs::write(&file, r#"{"PROJECT_VALUE":"first"}"#).unwrap();
        let command = if cfg!(windows) {
            vec!["cmd.exe", "/d", "/c", "type env.json"]
        } else {
            vec!["sh", "-c", "cat env.json"]
        };
        let provider = Provider {
            command: command.into_iter().map(str::to_string).collect(),
            watch: vec!["env.json".into()],
            ..Default::default()
        };
        let registry = Registry::default();
        registry.register("test-project-env".into(), Some(provider));
        assert!(
            registry
                .resolve(&dir)
                .contains(&("PROJECT_VALUE".into(), "first".into()))
        );
        assert!(
            Registry::default().resolve(&dir).is_empty(),
            "another application owns its own providers"
        );
        std::fs::write(&file, r#"{"PROJECT_VALUE":"second-value"}"#).unwrap();
        assert!(
            registry
                .resolve(&dir)
                .contains(&("PROJECT_VALUE".into(), "second-value".into()))
        );
        // A different directory cannot inherit this one's environment.
        let other = dir.join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("env.json"), r#"{"PROJECT_VALUE":"other"}"#).unwrap();
        assert!(
            registry
                .resolve(&other)
                .contains(&("PROJECT_VALUE".into(), "other".into()))
        );
        registry.invalidate("test-project-env", Some(&dir));
        registry.register("test-project-env".into(), None);
        std::fs::remove_dir_all(dir).ok();
    }
}
