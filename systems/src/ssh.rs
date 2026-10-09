//! The ssh client in-process (docs/design/domains.md Decision 3, amended
//! 2026-10-09, "Built, our ssh"): one connection a domain, and every
//! channel the domain opens — its files' SFTP subsystem, its runners,
//! each process, each language server, each terminal and the forward
//! the `$EDITOR` shim comes back on — multiplexed on it. What OpenSSH's
//! master gave on unix and no ssh on Windows can give: Windows' own
//! makes no master, Git's MSYS one passes no session through one.
//!
//! russh speaks the protocol on a tokio runtime of its own; everything
//! here is used from ordinary threads, the channel's bytes crossing as
//! blocking readers and writers (`Remote`), so the SFTP client, the
//! runners, the process pumps and the terminal's reader are the ones
//! the OpenSSH path has. What OpenSSH did by itself is done here, from
//! `~/.ssh/config` (`ssh_config::resolve`): the host, user, port and
//! identity files; `ProxyJump`, each hop a connection through the one
//! before; the agent (`SSH_AUTH_SOCK`, Windows' OpenSSH pipe, Pageant),
//! then the identity files — a passphrase asked when one is encrypted —
//! then keyboard-interactive and the password, asked; `known_hosts`
//! checked, a host not in it asked about (accepted, it is written there
//! as `accept-new` would), a key that changed refused. What asks is the
//! caller's [`Asker`]: the window's confirm, with a field.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use russh::client::{self, Handle, Msg};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKey};
use russh::{Channel, ChannelMsg, Sig};
use tokio::sync::mpsc;

use crate::ssh_config::HostConfig;

/// The runtime every connection runs on: two threads, enough for the
/// protocol's work, which is copying.
fn rt() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("ssh")
            .enable_all()
            .build()
            .expect("the ssh runtime")
    })
}

/// What a connection asks the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    /// The question: `Passphrase for ~/.ssh/id_ed25519`.
    pub title: String,
    /// What it is about: a key's fingerprint, a server's instructions.
    pub lines: Vec<String>,
    /// How it is answered.
    pub kind: AskKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AskKind {
    /// Yes or no: `Some(_)` is yes.
    Confirm,
    /// A line, shown as typed.
    Text,
    /// A line, not shown.
    Secret,
}

/// Asks the user and waits for the answer — `None` when it was refused
/// or not given. Called from the connecting thread, never from the
/// window's.
pub type Asker = Arc<dyn Fn(Question) -> Option<String> + Send + Sync>;

/// Where a remote forward's connections go here.
#[derive(Clone, Debug)]
pub enum Local {
    /// A TCP port on this machine's loopback.
    Tcp(u16),
    /// A unix socket.
    #[cfg(unix)]
    Unix(PathBuf),
}

impl Local {
    /// The command socket at `path`: on unix the socket itself; on
    /// Windows its file holds the TCP port it listens on.
    pub fn of_socket(path: &Path) -> Option<Local> {
        #[cfg(unix)]
        {
            Some(Local::Unix(path.to_path_buf()))
        }
        #[cfg(not(unix))]
        {
            let port = std::fs::read_to_string(path).ok()?.trim().parse().ok()?;
            Some(Local::Tcp(port))
        }
    }
}

#[derive(Default)]
struct Shared {
    /// The remote ports forwarded back, and where each goes here.
    forwards: Mutex<HashMap<u32, Local>>,
}

/// The connection's side of russh: the server's key checked, a forward
/// opened from the host carried to where it goes.
struct H {
    host: String,
    port: u16,
    config: HostConfig,
    ask: Asker,
    shared: Arc<Shared>,
    /// Why the key was refused, for the error.
    refused: Arc<Mutex<Option<Failure>>>,
}

impl client::Handler for H {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKey) -> Result<bool, Self::Error> {
        let key = key.clone();
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        let files = &self.config.known_hosts;
        for f in files {
            match russh::keys::check_known_hosts_path(&self.host, self.port, &key, f) {
                Ok(true) => return Ok(true),
                Ok(false) => {}
                Err(russh::keys::Error::KeyChanged { line }) => {
                    *self.refused.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some(Failure::Final(format!(
                            "THE HOST KEY OF {} CHANGED (line {line} of {} has another): refused — \
                         it may be someone in the middle; if the host was reinstalled, remove \
                         that line",
                            self.host,
                            f.display()
                        )));
                    return Ok(false);
                }
                Err(_) => {}
            }
        }
        let strict = self.config.strict.as_str();
        if strict == "no" || strict == "off" {
            return Ok(true);
        }
        if strict == "yes" {
            *self.refused.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(Failure::Final(format!(
                    "{} is not in known_hosts and StrictHostKeyChecking is yes",
                    self.host
                )));
            return Ok(false);
        }
        let accepted = strict == "accept-new" || {
            let ask = self.ask.clone();
            let q = Question {
                title: format!("{} is a host not seen before. Trust it?", self.host),
                lines: vec![
                    format!("{} key {fingerprint}", key.algorithm().as_str()),
                    format!(
                        "Trusted, it is written to {}",
                        files
                            .first()
                            .map(|f| f.display().to_string())
                            .unwrap_or_default()
                    ),
                ],
                kind: AskKind::Confirm,
            };
            tokio::task::spawn_blocking(move || ask(q).is_some())
                .await
                .unwrap_or(false)
        };
        if !accepted {
            *self.refused.lock().unwrap_or_else(|e| e.into_inner()) = Some(Failure::Declined(
                format!("{}'s key not trusted", self.host),
            ));
            return Ok(false);
        }
        if let Some(f) = files.first() {
            if let Some(dir) = f.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) =
                russh::keys::known_hosts::learn_known_hosts_path(&self.host, self.port, &key, f)
            {
                log::warn!("{}: writing {}: {e}", self.host, f.display());
            }
        }
        Ok(true)
    }

    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<Msg>,
        _connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        let to = self
            .shared
            .forwards
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&connected_port)
            .cloned();
        let Some(to) = to else {
            return Ok(());
        };
        tokio::spawn(async move {
            let mut ch = channel.into_stream();
            let r = match to {
                Local::Tcp(port) => match tokio::net::TcpStream::connect(("127.0.0.1", port)).await
                {
                    Ok(mut s) => tokio::io::copy_bidirectional(&mut ch, &mut s)
                        .await
                        .map(drop),
                    Err(e) => Err(e),
                },
                #[cfg(unix)]
                Local::Unix(p) => match tokio::net::UnixStream::connect(p).await {
                    Ok(mut s) => tokio::io::copy_bidirectional(&mut ch, &mut s)
                        .await
                        .map(drop),
                    Err(e) => Err(e),
                },
            };
            if let Err(e) = r {
                log::debug!("a forwarded connection: {e}");
            }
        });
        Ok(())
    }
}

/// One domain's connection.
pub struct Client {
    handle: Handle<H>,
    shared: Arc<Shared>,
    /// The connections before it, for a `ProxyJump`: kept for as long as
    /// this one goes through them.
    _jumps: Vec<Handle<H>>,
    pub host: String,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ssh::Client({})", self.host)
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let h = &self.handle;
        let _ = rt().block_on(async {
            tokio::time::timeout(
                std::time::Duration::from_millis(500),
                h.disconnect(russh::Disconnect::ByApplication, "", "en"),
            )
            .await
        });
    }
}

fn err(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

/// Why a connection was not made — and so whether OpenSSH's client might
/// make it (`ssh.client = "auto"`, docs/design/domains.md "Built, the
/// fallback and :ssh").
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The network: the host not reached, its name not known. Another
    /// client would fare no better.
    Network(String),
    /// A refusal that stands: a host key that changed, a host strictly
    /// unknown, a password wrong when asked.
    Final(String),
    /// The user refused a question: a host key not trusted, a
    /// passphrase or a password not given. Not asked again behind them.
    Declined(String),
    /// What this client does not do — a `ProxyCommand`, a `Match`, a
    /// certificate, an RSA key, an algorithm the server and it share
    /// none of, or no way in that asks nothing: OpenSSH's may.
    Unsupported(String),
}

impl Failure {
    pub fn message(&self) -> &str {
        match self {
            Failure::Network(m) | Failure::Final(m) | Failure::Declined(m) => m,
            Failure::Unsupported(m) => m,
        }
    }

    /// Whether OpenSSH's client is worth trying in its place.
    pub fn falls_back(&self) -> bool {
        matches!(self, Failure::Unsupported(_))
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

/// Connects to `target` as `~/.ssh/config` says (`ssh_config::resolve`),
/// through its jumps, authenticated, asking `ask` what has to be asked.
/// With `fall_back` (OpenSSH there to take over), what this client does
/// not do is said before anything is asked: a config it cannot follow,
/// an RSA key it cannot use. Blocks; not on the window's thread.
pub fn connect(target: &str, ask: Asker, fall_back: bool) -> Result<Client, Failure> {
    let config = crate::ssh_config::resolve(target);
    if fall_back {
        let hops = config
            .proxy_jump
            .iter()
            .map(|h| crate::ssh_config::resolve(h));
        for hc in std::iter::once(config.clone()).chain(hops) {
            if let Some(why) = unsupported(&hc) {
                return Err(Failure::Unsupported(why));
            }
        }
    }
    rt().block_on(connect_async(config, ask, fall_back))
}

/// What in `hc` this client cannot follow, before it tries.
pub fn unsupported(hc: &HostConfig) -> Option<String> {
    if hc.proxy_command.is_some() {
        return Some("ProxyCommand isn't supported by the built-in client".into());
    }
    if !hc.certificate_files.is_empty() {
        return Some("CertificateFile isn't supported by the built-in client".into());
    }
    if hc.has_match {
        return Some("the built-in client doesn't follow Match blocks".into());
    }
    // Every key it has is RSA, and no agent to offer another.
    let files = if hc.identity_files.is_empty() {
        default_identities()
    } else {
        hc.identity_files.clone()
    };
    let there: Vec<&PathBuf> = files.iter().filter(|f| f.is_file()).collect();
    if !there.is_empty() && there.iter().all(|f| is_rsa(f)) && !agent_there() {
        return Some(format!(
            "{} is an RSA key, which the built-in client can't use",
            there[0].display()
        ));
    }
    None
}

/// Whether the key at `f` is RSA: its public half says so, or its
/// private one is PEM's RSA.
fn is_rsa(f: &Path) -> bool {
    let public = PathBuf::from(format!("{}.pub", f.display()));
    if let Ok(p) = std::fs::read_to_string(&public) {
        return p.trim_start().starts_with("ssh-rsa");
    }
    std::fs::read_to_string(f).is_ok_and(|t| t.contains("BEGIN RSA PRIVATE KEY"))
}

/// Whether an agent may answer: `SSH_AUTH_SOCK`, or on Windows OpenSSH's
/// pipe (Pageant cannot be asked without asking it).
fn agent_there() -> bool {
    if std::env::var_os("SSH_AUTH_SOCK").is_some() {
        return true;
    }
    cfg!(windows) && Path::new(r"\\.\pipe\openssh-ssh-agent").exists()
}

async fn connect_async(config: HostConfig, ask: Asker, fall_back: bool) -> Result<Client, Failure> {
    let mut jumps: Vec<Handle<H>> = Vec::new();
    for hop in &config.proxy_jump {
        let hc = crate::ssh_config::resolve(hop);
        let h = match jumps.last() {
            None => open(&hc, &ask, None, fall_back).await?,
            Some(prev) => open(&hc, &ask, Some(prev), fall_back).await?,
        };
        jumps.push(h.0);
    }
    let (handle, shared) = open(&config, &ask, jumps.last(), fall_back).await?;
    Ok(Client {
        handle,
        shared,
        _jumps: jumps,
        host: config.alias.clone(),
    })
}

/// One connection, made and authenticated: over TCP, or through `via`'s
/// direct-tcpip channel for a jump.
async fn open(
    hc: &HostConfig,
    ask: &Asker,
    via: Option<&Handle<H>>,
    fall_back: bool,
) -> Result<(Handle<H>, Arc<Shared>), Failure> {
    let shared = Arc::new(Shared::default());
    let refused = Arc::new(Mutex::new(None));
    let handler = H {
        host: hc.host_name.clone(),
        port: hc.port,
        config: hc.clone(),
        ask: ask.clone(),
        shared: shared.clone(),
        refused: refused.clone(),
    };
    let config = Arc::new(client::Config {
        keepalive_interval: Some(std::time::Duration::from_secs(30)),
        keepalive_max: 3,
        nodelay: true,
        ..Default::default()
    });
    // A failure past the TCP connection and before authentication is the
    // handshake's — an algorithm not shared, a server that hung up on it
    // — unless the host key was refused.
    let said = |e: russh::Error| {
        refused
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .unwrap_or_else(|| {
                Failure::Unsupported(format!("the built-in client's handshake failed: {e}"))
            })
    };
    let mut handle = match via {
        None => {
            let addr = (hc.host_name.as_str(), hc.port);
            let tcp = tokio::time::timeout(
                std::time::Duration::from_secs(20),
                tokio::net::TcpStream::connect(addr),
            )
            .await
            .map_err(|_| {
                Failure::Network(format!("{}:{}: no answer in 20 s", hc.host_name, hc.port))
            })?
            .map_err(|e| Failure::Network(format!("{}:{}: {e}", hc.host_name, hc.port)))?;
            let _ = tcp.set_nodelay(true);
            client::connect_stream(config, tcp, handler)
                .await
                .map_err(said)?
        }
        Some(prev) => {
            let ch = prev
                .channel_open_direct_tcpip(hc.host_name.clone(), hc.port as u32, "127.0.0.1", 0)
                .await
                .map_err(|e| {
                    Failure::Network(format!("through the jump to {}: {e}", hc.host_name))
                })?;
            client::connect_stream(config, ch.into_stream(), handler)
                .await
                .map_err(said)?
        }
    };
    authenticate(&mut handle, hc, ask, fall_back).await?;
    Ok((handle, shared))
}

/// The identity files OpenSSH tries when the config names none.
fn default_identities() -> Vec<PathBuf> {
    let home = crate::fs::home().unwrap_or_default().join(".ssh");
    ["id_ed25519", "id_ecdsa", "id_rsa"]
        .iter()
        .map(|n| home.join(n))
        .collect()
}

/// `ask(q)` off the runtime's threads.
async fn asking(ask: &Asker, q: Question) -> Option<String> {
    let ask = ask.clone();
    tokio::task::spawn_blocking(move || ask(q))
        .await
        .ok()
        .flatten()
}

/// The ways in the server takes, tried as OpenSSH tries them: the
/// agent's keys, the identity files (a passphrase asked), then
/// keyboard-interactive and the password, asked — each only where the
/// server offers it, so a host that takes keys alone is not asked for a
/// password it would refuse. A question refused ends it there
/// (`Declined`); with nothing asked and nothing taken it is this
/// client's failure (`Unsupported`, OpenSSH may have a way in), else the
/// host's (`Final`).
async fn authenticate(
    handle: &mut Handle<H>,
    hc: &HostConfig,
    ask: &Asker,
    fall_back: bool,
) -> Result<(), Failure> {
    use russh::MethodKind;
    let user = hc.user.clone();
    let who = format!("{}@{}", user, hc.host_name);
    let methods: Vec<MethodKind> = match handle.authenticate_none(user.clone()).await {
        Ok(r) if r.success() => return Ok(()),
        Ok(russh::client::AuthResult::Failure {
            remaining_methods, ..
        }) => remaining_methods.to_vec(),
        Ok(_) => Vec::new(),
        Err(e) => return Err(Failure::Unsupported(format!("{who}: {e}"))),
    };
    let takes = |m: MethodKind| methods.contains(&m);
    let hash = handle
        .best_supported_rsa_hash()
        .await
        .ok()
        .flatten()
        .flatten();
    let mut asked = false;
    // Keys this client could not use, for the failure's word.
    let mut unusable: Vec<String> = Vec::new();
    if takes(MethodKind::PublicKey) {
        let files = if hc.identity_files.is_empty() {
            default_identities()
        } else {
            hc.identity_files.clone()
        };
        // The public halves of the files, for `IdentitiesOnly`.
        let wanted: Vec<PublicKey> = files
            .iter()
            .filter_map(|f| {
                let p = PathBuf::from(format!("{}.pub", f.display()));
                russh::keys::ssh_key::PublicKey::read_openssh_file(&p).ok()
            })
            .collect();
        let mut tried = Vec::new();
        // The agent's keys first, as OpenSSH tries them.
        if agent_auth(handle, &user, hash, hc.identities_only, &wanted, &mut tried).await {
            return Ok(());
        }
        for f in files.iter().filter(|f| f.is_file()) {
            let key = match russh::keys::load_secret_key(f, None) {
                Ok(k) => Some(k),
                Err(russh::keys::Error::KeyIsEncrypted) => {
                    let mut got = None;
                    for _ in 0..3 {
                        asked = true;
                        let q = Question {
                            title: format!("Passphrase for {}", f.display()),
                            lines: vec![who.clone()],
                            kind: AskKind::Secret,
                        };
                        let Some(pass) = asking(ask, q).await else {
                            return Err(Failure::Declined(format!(
                                "{who}: the passphrase for {} not given",
                                f.display()
                            )));
                        };
                        if let Ok(k) = russh::keys::load_secret_key(f, Some(&pass)) {
                            got = Some(k);
                            break;
                        }
                    }
                    got
                }
                Err(e) => {
                    log::info!("{}: {e}", f.display());
                    unusable.push(f.display().to_string());
                    None
                }
            };
            let Some(key) = key else { continue };
            if tried.contains(key.public_key()) {
                continue;
            }
            let r = handle
                .authenticate_publickey(
                    user.clone(),
                    PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                )
                .await
                .map_err(|e| Failure::Unsupported(format!("{who}: {e}")))?;
            if r.success() {
                return Ok(());
            }
        }
    }
    // A key here this client could not read (RSA, a format it lacks):
    // OpenSSH's may take it, before anything is asked.
    if fall_back && let Some(f) = unusable.first() {
        return Err(Failure::Unsupported(format!(
            "{f} is a key the built-in client can't use"
        )));
    }
    // Keyboard-interactive: each prompt asked.
    if takes(MethodKind::KeyboardInteractive)
        && let Ok(mut r) = handle
            .authenticate_keyboard_interactive_start(user.clone(), None)
            .await
    {
        for _ in 0..5 {
            use russh::client::KeyboardInteractiveAuthResponse as K;
            match r {
                K::Success => return Ok(()),
                K::Failure { .. } => break,
                K::InfoRequest {
                    name,
                    instructions,
                    prompts,
                } => {
                    let mut answers = Vec::new();
                    for p in prompts {
                        asked = true;
                        let q = Question {
                            title: if p.prompt.trim().is_empty() {
                                who.clone()
                            } else {
                                format!("{who}: {}", p.prompt.trim())
                            },
                            lines: [name.clone(), instructions.clone()]
                                .into_iter()
                                .filter(|l| !l.trim().is_empty())
                                .collect(),
                            kind: if p.echo {
                                AskKind::Text
                            } else {
                                AskKind::Secret
                            },
                        };
                        match asking(ask, q).await {
                            Some(a) => answers.push(a),
                            None => {
                                return Err(Failure::Declined(format!("{who}: not answered")));
                            }
                        }
                    }
                    r = match handle
                        .authenticate_keyboard_interactive_respond(answers)
                        .await
                    {
                        Ok(r) => r,
                        Err(_) => break,
                    };
                }
            }
        }
    }
    // The password, asked.
    if takes(MethodKind::Password) {
        for _ in 0..3 {
            asked = true;
            let q = Question {
                title: format!("Password for {who}"),
                lines: Vec::new(),
                kind: AskKind::Secret,
            };
            let Some(pass) = asking(ask, q).await else {
                return Err(Failure::Declined(format!("{who}: the password not given")));
            };
            match handle.authenticate_password(user.clone(), pass).await {
                Ok(r) if r.success() => return Ok(()),
                Ok(_) => continue,
                Err(e) => return Err(Failure::Final(format!("{who}: {e}"))),
            }
        }
    }
    let why = format!(
        "{who}: not authenticated (no key the agent or the identity files hold was taken{})",
        if asked { ", nor what was typed" } else { "" }
    );
    Err(if asked || !fall_back {
        Failure::Final(why)
    } else {
        Failure::Unsupported(format!(
            "the built-in client found no way in to {who} that asks nothing"
        ))
    })
}

/// The agent's keys tried, each recorded in `tried`: true once one is
/// taken. The agent is `SSH_AUTH_SOCK`'s; on Windows OpenSSH's pipe,
/// then Pageant.
async fn agent_auth(
    handle: &mut Handle<H>,
    user: &str,
    hash: Option<HashAlg>,
    only: bool,
    wanted: &[PublicKey],
    tried: &mut Vec<PublicKey>,
) -> bool {
    use russh::keys::agent::client::AgentClient;
    #[cfg(unix)]
    {
        if let Ok(a) = AgentClient::connect_env().await
            && with_agent(handle, user, hash, only, wanted, tried, a).await
        {
            return true;
        }
    }
    #[cfg(windows)]
    {
        if let Ok(path) = std::env::var("SSH_AUTH_SOCK")
            && path.starts_with(r"\\.\pipe\")
            && let Ok(a) = AgentClient::connect_named_pipe(&path).await
            && with_agent(handle, user, hash, only, wanted, tried, a).await
        {
            return true;
        }
        if let Ok(a) = AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await
            && with_agent(handle, user, hash, only, wanted, tried, a).await
        {
            return true;
        }
        if let Ok(a) = AgentClient::connect_pageant().await
            && with_agent(handle, user, hash, only, wanted, tried, a).await
        {
            return true;
        }
    }
    false
}

async fn with_agent<S>(
    handle: &mut Handle<H>,
    user: &str,
    hash: Option<HashAlg>,
    only: bool,
    wanted: &[PublicKey],
    tried: &mut Vec<PublicKey>,
    mut agent: russh::keys::agent::client::AgentClient<S>,
) -> bool
where
    S: russh::keys::agent::client::AgentStream + Unpin + Send + 'static,
{
    let Ok(ids) = agent.request_identities().await else {
        return false;
    };
    for id in ids {
        let key = id;
        if only && !wanted.iter().any(|w| w.key_data() == key.key_data()) {
            continue;
        }
        tried.push(key.clone());
        let hash = if key.algorithm().is_rsa() { hash } else { None };
        if let Ok(r) = handle
            .authenticate_publickey_with(user.to_string(), key, hash, &mut agent)
            .await
            && r.success()
        {
            return true;
        }
    }
    false
}

// ------------------------------------------------------------ channels

enum Ctl {
    Data(Vec<u8>),
    Eof,
    Resize(u32, u32),
    Kill,
}

/// How a channel ended: the exit code, `None` for a signal or a channel
/// closed without one.
#[derive(Default)]
pub struct Done {
    state: Mutex<Option<Option<i32>>>,
    cv: Condvar,
}

impl Done {
    fn set(&self, code: Option<i32>) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.is_none() {
            *s = Some(code);
        }
        self.cv.notify_all();
    }

    /// The end, when it has come.
    pub fn try_get(&self) -> Option<Option<i32>> {
        *self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The end, waited for.
    pub fn wait(&self) -> Option<i32> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(c) = *s {
                return c;
            }
            s = self.cv.wait(s).unwrap_or_else(|e| e.into_inner());
        }
    }
}

/// A channel's output as a blocking reader.
pub struct RemoteReader {
    rx: mpsc::Receiver<Vec<u8>>,
    buf: Vec<u8>,
    at: usize,
}

impl Read for RemoteReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.at >= self.buf.len() {
            match self.rx.blocking_recv() {
                Some(b) => {
                    self.buf = b;
                    self.at = 0;
                }
                None => return Ok(0),
            }
        }
        let n = out.len().min(self.buf.len() - self.at);
        out[..n].copy_from_slice(&self.buf[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

/// A channel's input as a blocking writer: dropped, the channel's input
/// ends (EOF).
pub struct RemoteWriter {
    ctl: mpsc::UnboundedSender<Ctl>,
}

impl Write for RemoteWriter {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.ctl
            .send(Ctl::Data(b.to_vec()))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "the channel closed"))?;
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for RemoteWriter {
    fn drop(&mut self) {
        let _ = self.ctl.send(Ctl::Eof);
    }
}

/// A process on the host, over a channel of the domain's connection:
/// what a `std::process::Child` with piped stdio is to a local one.
pub struct Remote {
    pub stdin: Option<RemoteWriter>,
    pub stdout: Option<RemoteReader>,
    pub stderr: Option<RemoteReader>,
    pub done: Arc<Done>,
    ctl: mpsc::UnboundedSender<Ctl>,
}

impl Remote {
    /// The process ended: a KILL signal, then the channel closed (which
    /// hangs a server's process up where it takes no signals).
    pub fn kill(&self) {
        let _ = self.ctl.send(Ctl::Kill);
    }

    /// Its terminal's new size (a pty's).
    pub fn resize(&self, cols: u16, rows: u16) {
        let _ = self.ctl.send(Ctl::Resize(cols as u32, rows as u32));
    }

    pub fn killer(&self) -> RemoteKiller {
        RemoteKiller {
            ctl: self.ctl.clone(),
        }
    }

    /// Its end, apart from its streams: what an SFTP session or a runner
    /// keeps to know it has gone and to end it.
    pub fn end(&self) -> ChannelEnd {
        ChannelEnd {
            done: self.done.clone(),
            killer: self.killer(),
        }
    }
}

/// A channel's end, kept beside its streams.
pub struct ChannelEnd {
    done: Arc<Done>,
    killer: RemoteKiller,
}

impl crate::sftp::Behind for ChannelEnd {
    fn ended(&mut self) -> bool {
        self.done.try_get().is_some()
    }
    fn end(&mut self) {
        self.killer.kill();
    }
}

/// Ends a [`Remote`] from anywhere.
#[derive(Clone, Debug)]
pub struct RemoteKiller {
    ctl: mpsc::UnboundedSender<Ctl>,
}

impl RemoteKiller {
    pub fn kill(&self) {
        let _ = self.ctl.send(Ctl::Kill);
    }
}

impl std::fmt::Debug for Ctl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Ctl")
    }
}

/// What a channel is opened for.
pub enum Open<'a> {
    /// `line` run by the host's login shell, with a terminal of
    /// `cols`×`rows` when `pty`.
    Exec {
        line: &'a str,
        pty: Option<(u16, u16)>,
    },
    /// A subsystem: `sftp`.
    Subsystem(&'a str),
}

impl Client {
    /// Whether the connection is still there.
    pub fn is_alive(&self) -> bool {
        !self.handle.is_closed()
    }

    /// A channel opened for `what`, its ends as blocking readers and a
    /// writer, driven on the runtime until it closes.
    pub fn open(&self, what: Open<'_>) -> io::Result<Remote> {
        let (line, pty, sub) = match what {
            Open::Exec { line, pty } => (Some(line.to_string()), pty, None),
            Open::Subsystem(s) => (None, None, Some(s.to_string())),
        };
        let handle = &self.handle;
        let channel = rt().block_on(async {
            let ch = handle.channel_open_session().await.map_err(err)?;
            if let Some((cols, rows)) = pty {
                ch.request_pty(false, "xterm-256color", cols as u32, rows as u32, 0, 0, &[])
                    .await
                    .map_err(err)?;
            }
            match (&line, &sub) {
                (Some(l), _) => ch.exec(true, l.as_bytes()).await.map_err(err)?,
                (_, Some(s)) => ch.request_subsystem(true, s.as_str()).await.map_err(err)?,
                _ => {}
            }
            Ok::<_, io::Error>(ch)
        })?;
        Ok(drive(channel, pty.is_none()))
    }

    /// The host's loopback `port` forwarded back to `to` here, for as long
    /// as the connection lasts. The port the host gave, or an error (a
    /// port taken, forwarding refused by the server).
    pub fn forward(&self, port: u16, to: Local) -> io::Result<u16> {
        let handle = &self.handle;
        let got = rt()
            .block_on(handle.tcpip_forward("127.0.0.1", port as u32))
            .map_err(err)?;
        let got = if got == 0 { port as u32 } else { got };
        self.shared
            .forwards
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(got, to);
        Ok(got as u16)
    }
}

/// `channel` driven on the runtime: its data out to the readers (stderr
/// apart where `split_err`, a pty having none), what the writer sends in,
/// resizes and kills, its end into `done`.
fn drive(mut channel: Channel<Msg>, split_err: bool) -> Remote {
    let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>(64);
    let (err_tx, err_rx) = mpsc::channel::<Vec<u8>>(64);
    let (ctl_tx, mut ctl_rx) = mpsc::unbounded_channel::<Ctl>();
    let done = Arc::new(Done::default());
    let d = done.clone();
    rt().spawn(async move {
        let mut code = None;
        let mut ctl_open = true;
        loop {
            tokio::select! {
                m = channel.wait() => match m {
                    Some(ChannelMsg::Data { data }) => {
                        let _ = out_tx.send(data.to_vec()).await;
                    }
                    Some(ChannelMsg::ExtendedData { data, ext: 1 }) => {
                        let tx = if split_err { &err_tx } else { &out_tx };
                        let _ = tx.send(data.to_vec()).await;
                    }
                    Some(ChannelMsg::ExitStatus { exit_status }) => code = Some(exit_status as i32),
                    Some(ChannelMsg::Failure) => {
                        // A request refused (a subsystem not there): the
                        // channel goes, its readers see their end.
                        let _ = channel.close().await;
                    }
                    Some(ChannelMsg::Close) | None => break,
                    _ => {}
                },
                c = ctl_rx.recv(), if ctl_open => match c {
                    Some(Ctl::Data(b)) => {
                        if channel.data(&b[..]).await.is_err() {
                            break;
                        }
                    }
                    Some(Ctl::Eof) => {
                        let _ = channel.eof().await;
                    }
                    Some(Ctl::Resize(c, r)) => {
                        let _ = channel.window_change(c, r, 0, 0).await;
                    }
                    Some(Ctl::Kill) => {
                        let _ = channel.signal(Sig::KILL).await;
                        let _ = channel.close().await;
                    }
                    None => ctl_open = false,
                },
            }
        }
        drop(out_tx);
        drop(err_tx);
        d.set(code);
    });
    Remote {
        stdin: Some(RemoteWriter {
            ctl: ctl_tx.clone(),
        }),
        stdout: Some(RemoteReader {
            rx: out_rx,
            buf: Vec::new(),
            at: 0,
        }),
        stderr: Some(RemoteReader {
            rx: err_rx,
            buf: Vec::new(),
            at: 0,
        }),
        done,
        ctl: ctl_tx,
    }
}

// ------------------------------------------------- a terminal's channel

/// A pty channel as `portable-pty`'s master, so `kawoosh_term`'s
/// terminal takes it as it takes a local pseudo console.
pub struct ChannelPty {
    reader: Mutex<Option<RemoteReader>>,
    writer: Mutex<Option<RemoteWriter>>,
    size: Mutex<portable_pty::PtySize>,
    ctl: mpsc::UnboundedSender<Ctl>,
}

/// The process behind a [`ChannelPty`], as `portable-pty`'s child.
#[derive(Clone)]
pub struct ChannelChild {
    done: Arc<Done>,
    ctl: mpsc::UnboundedSender<Ctl>,
}

impl std::fmt::Debug for ChannelChild {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ChannelChild")
    }
}

impl Client {
    /// A terminal on the host: `line` run in a pty of `cols`×`rows`.
    pub fn pty(&self, line: &str, cols: u16, rows: u16) -> io::Result<(ChannelPty, ChannelChild)> {
        let mut r = self.open(Open::Exec {
            line,
            pty: Some((cols, rows)),
        })?;
        let pty = ChannelPty {
            reader: Mutex::new(r.stdout.take()),
            writer: Mutex::new(r.stdin.take()),
            size: Mutex::new(portable_pty::PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            }),
            ctl: r.ctl.clone(),
        };
        let child = ChannelChild {
            done: r.done.clone(),
            ctl: r.ctl.clone(),
        };
        Ok((pty, child))
    }
}

impl portable_pty::MasterPty for ChannelPty {
    fn resize(&self, size: portable_pty::PtySize) -> anyhow::Result<()> {
        *self.size.lock().unwrap_or_else(|e| e.into_inner()) = size;
        let _ = self
            .ctl
            .send(Ctl::Resize(size.cols as u32, size.rows as u32));
        Ok(())
    }
    fn get_size(&self) -> anyhow::Result<portable_pty::PtySize> {
        Ok(*self.size.lock().unwrap_or_else(|e| e.into_inner()))
    }
    fn try_clone_reader(&self) -> anyhow::Result<Box<dyn Read + Send>> {
        self.reader
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .map(|r| Box::new(r) as Box<dyn Read + Send>)
            .ok_or_else(|| anyhow::anyhow!("a channel's output is read once"))
    }
    fn take_writer(&self) -> anyhow::Result<Box<dyn Write + Send>> {
        self.writer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .map(|w| Box::new(w) as Box<dyn Write + Send>)
            .ok_or_else(|| anyhow::anyhow!("a channel's input is taken once"))
    }
    #[cfg(unix)]
    fn process_group_leader(&self) -> Option<libc::pid_t> {
        None
    }
    #[cfg(unix)]
    fn as_raw_fd(&self) -> Option<std::os::unix::io::RawFd> {
        None
    }
    #[cfg(unix)]
    fn tty_name(&self) -> Option<PathBuf> {
        None
    }
}

impl portable_pty::ChildKiller for ChannelChild {
    fn kill(&mut self) -> io::Result<()> {
        let _ = self.ctl.send(Ctl::Kill);
        Ok(())
    }
    fn clone_killer(&self) -> Box<dyn portable_pty::ChildKiller + Send + Sync> {
        Box::new(self.clone())
    }
}

fn status(code: Option<i32>) -> portable_pty::ExitStatus {
    match code {
        Some(c) => portable_pty::ExitStatus::with_exit_code(c as u32),
        None => portable_pty::ExitStatus::with_signal("HUP"),
    }
}

impl portable_pty::Child for ChannelChild {
    fn try_wait(&mut self) -> io::Result<Option<portable_pty::ExitStatus>> {
        Ok(self.done.try_get().map(status))
    }
    fn wait(&mut self) -> io::Result<portable_pty::ExitStatus> {
        Ok(status(self.done.wait()))
    }
    fn process_id(&self) -> Option<u32> {
        None
    }
    #[cfg(windows)]
    fn as_raw_handle(&self) -> Option<std::os::windows::io::RawHandle> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Against a real host, where `KAWOOSH_TEST_SSH` names one (a `Host`
    /// of the config `KAWOOSH_SSH_CONFIG` names, or of `~/.ssh/config`),
    /// a directory there in `KAWOOSH_TEST_SSH_DIR`: one connection, and
    /// on it an exec with its code and both outputs, SFTP (or its refusal,
    /// a host with none), a runner, a pty's echo and a forward back here
    /// — each timed. `cargo test -p kawoosh-systems ssh::tests --
    /// --ignored --nocapture`.
    #[test]
    #[ignore]
    fn one_connection_carries_everything() {
        let Ok(host) = std::env::var("KAWOOSH_TEST_SSH") else {
            eprintln!("no KAWOOSH_TEST_SSH: skipped");
            return;
        };
        let dir = std::env::var("KAWOOSH_TEST_SSH_DIR").unwrap_or("/tmp".into());
        let time = |what: &str, at: std::time::Instant| {
            eprintln!("{what:<24} {:>8.1} ms", at.elapsed().as_secs_f64() * 1000.0);
        };
        let ask: Asker = Arc::new(|q| {
            eprintln!("asked: {} {:?}", q.title, q.lines);
            // A host key is trusted only when the run says so: the
            // user's known_hosts is not written by a test unasked.
            (q.kind == AskKind::Confirm && std::env::var_os("KAWOOSH_TEST_SSH_TRUST").is_some())
                .then(|| "yes".into())
        });
        let t = std::time::Instant::now();
        let c = Arc::new(connect(&host, ask, false).expect("connected"));
        time("connect", t);
        let t = std::time::Instant::now();
        let out = crate::io::run_on_channel(
            c.open(Open::Exec {
                line: &crate::io::ssh_line("echo out; echo err >&2; exit 3\n"),
                pty: None,
            })
            .unwrap(),
            None,
        )
        .unwrap();
        time("exec", t);
        assert_eq!(
            (out.code, out.stdout.as_slice(), out.stderr.as_slice()),
            (3, &b"out\n"[..], &b"err\n"[..])
        );
        let t = std::time::Instant::now();
        let sftp = c
            .open(Open::Subsystem("sftp"))
            .and_then(crate::sftp::Sftp::on_channel);
        time("sftp channel", t);
        match &sftp {
            Ok(s) => {
                use kawoosh_doc::fs::Fs;
                let t = std::time::Instant::now();
                let n = s.list(Path::new(&dir)).unwrap().len();
                time(&format!("list ({n})"), t);
                let f = PathBuf::from(format!("{dir}/.kawoosh-ssh-test"));
                let t = std::time::Instant::now();
                s.write(&f, b"written\n").unwrap();
                assert_eq!(s.read(&f).unwrap(), b"written\n");
                time("write + read", t);
                s.remove(&f).unwrap();
            }
            Err(e) => eprintln!("no SFTP: {e}"),
        }
        let tr = crate::io::Transport::Ssh(crate::io::Ssh {
            ssh: "ssh".into(),
            host: host.clone(),
            ctl: "unused".into(),
            master: false,
            builtin: true,
            client: Some(c.clone()),
            fall_back: false,
        });
        let t = std::time::Instant::now();
        let first = crate::runner::run(&tr, "true\n", b"").unwrap();
        time("runner start", t);
        assert!(first.is_ok(), "{first:?}");
        let t = std::time::Instant::now();
        let r = crate::runner::run(&tr, "echo $((40 + 2))\n", b"")
            .unwrap()
            .unwrap();
        time("runner script", t);
        assert_eq!(r.stdout, b"42\n");
        // A pty: raw, echoing what it is sent.
        let (pty, _child) = c
            .pty(
                &crate::io::ssh_line("stty raw -echo; echo READY; cat\n"),
                80,
                24,
            )
            .unwrap();
        use portable_pty::MasterPty;
        let mut rd = pty.try_clone_reader().unwrap();
        let mut wr = pty.take_writer().unwrap();
        let mut seen = Vec::new();
        let mut buf = [0u8; 4096];
        while !String::from_utf8_lossy(&seen).contains("READY") {
            let n = rd.read(&mut buf).unwrap();
            assert!(n > 0, "the pty closed: {}", String::from_utf8_lossy(&seen));
            seen.extend_from_slice(&buf[..n]);
        }
        let mut times = Vec::new();
        for i in 0..10 {
            let mark = format!("k{i:02}");
            let t = std::time::Instant::now();
            wr.write_all(mark.as_bytes()).unwrap();
            let mut got = Vec::new();
            while !String::from_utf8_lossy(&got).contains(&mark) {
                let n = rd.read(&mut buf).unwrap();
                got.extend_from_slice(&buf[..n]);
            }
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        times.sort_by(f64::total_cmp);
        eprintln!("{:<24} {:>8.1} ms median", "pty echo", times[5]);
        // A forward: the host's port back to a listener here.
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let local = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = l.accept() {
                let _ = s.write_all(b"from here\n");
            }
        });
        let port = 40000 + (std::process::id() % 20000) as u16;
        match c.forward(port, Local::Tcp(local)) {
            Ok(p) => {
                let script = format!(
                    "if command -v bash >/dev/null; then bash -c 'exec 3<>/dev/tcp/127.0.0.1/{p} && head -n1 <&3'; \
                     else nc 127.0.0.1 {p} </dev/null | head -n1; fi\n"
                );
                let out = crate::io::run_on_channel(
                    c.open(Open::Exec {
                        line: &crate::io::ssh_line(&script),
                        pty: None,
                    })
                    .unwrap(),
                    None,
                )
                .unwrap();
                eprintln!(
                    "forward: {:?} {:?}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                );
            }
            Err(e) => eprintln!("forward refused: {e}"),
        }
    }
}
