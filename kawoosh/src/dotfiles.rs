//! qd, the dotfiles manager, linked as a library (`qdot`, lib `qd`;
//! docs/design/lsp-installs.md Decision 5): what `kawoosh._qd(op,
//! args, fn)` asks, done on a thread of its own, one at a time — a status reads every
//! module's files, a pull may decrypt — and answered to the Lua callback
//! in the frame after, as JSON-shaped data: the shapes `qd status
//! --json` prints, so the pane reads either door alike.
//!
//! The library and the `qd` on the PATH share `state.toml`, the journal
//! and the trash; the pane uses this door only while `qd::VERSION` is
//! the binary's version, or there is no binary, and runs the binary
//! otherwise (`kawoosh/lua/qd.lua`).

use std::sync::mpsc::{Receiver, Sender, channel};

use kawoosh_systems::WakeHandle;
use qd::plan::Direction;
use serde_json::{Value, json};

/// An answer: its token, and the data or why not.
pub type Answer = (u64, Result<Value, String>);

/// A question for the worker: its token, the operation, its arguments.
type Ask = (u64, String, Vec<String>);

pub struct Dotfiles {
    tx: Sender<Answer>,
    rx: Receiver<Answer>,
    /// The worker's queue, started on the first question.
    asks: Option<Sender<Ask>>,
    wake: WakeHandle,
}

impl Dotfiles {
    pub fn new(wake: WakeHandle) -> Self {
        let (tx, rx) = channel();
        Self {
            tx,
            rx,
            asks: None,
            wake,
        }
    }

    /// The one thread qd's operations run on, in the order asked: a
    /// push and a pull asked together must not write the same files and
    /// `state.toml` at once, and a status asked after a pull reads what
    /// the pull left.
    fn worker(&mut self) -> Option<&Sender<Ask>> {
        if self.asks.is_none() {
            let (asks, queue) = channel::<Ask>();
            let (tx, wake) = (self.tx.clone(), self.wake.clone());
            std::thread::Builder::new()
                .name("qd".into())
                .spawn(move || {
                    for (token, op, args) in queue {
                        let r = run(&op, &args).map_err(|e| format!("{e:#}"));
                        let _ = tx.send((token, r));
                        wake.wake();
                    }
                })
                .ok()?;
            self.asks = Some(asks);
        }
        self.asks.as_ref()
    }

    /// `op` with `args`, answered to `token` on [`Dotfiles::drain`]:
    /// `version` (`{ version }`, at once), `status` (`{ repo, push, pull
    /// }`, each a list of plans), `sync` (`args`: `push` or `pull`, then
    /// modules; `{ run, applied, compiled }`), `add` (`args`: name, path,
    /// ignore globs…; the module written and pulled in, `{ file, run,
    /// applied }`).
    pub fn ask(&mut self, token: u64, op: String, args: Vec<String>) {
        if op == "version" {
            let _ = self.tx.send((token, Ok(json!({ "version": qd::VERSION }))));
            self.wake.wake();
            return;
        }
        let sent = self
            .worker()
            .is_some_and(|w| w.send((token, op, args)).is_ok());
        if !sent {
            let _ = self.tx.send((token, Err("qd: no thread to run on".into())));
            self.wake.wake();
        }
    }

    /// The answers come since the last call.
    pub fn drain(&self) -> Vec<Answer> {
        self.rx.try_iter().collect()
    }
}

fn direction(s: Option<&String>) -> anyhow::Result<Direction> {
    match s.map(String::as_str) {
        Some("push") => Ok(Direction::Push),
        Some("pull") => Ok(Direction::Pull),
        other => anyhow::bail!("push or pull, not {other:?}"),
    }
}

fn run(op: &str, args: &[String]) -> anyhow::Result<Value> {
    let mut s = qd::Session::open(None)?;
    Ok(match op {
        "status" => json!({
            "repo": s.root()?,
            "push": s.status(&[], Direction::Push)?,
            "pull": s.status(&[], Direction::Pull)?,
        }),
        "sync" => {
            let d = direction(args.first())?;
            serde_json::to_value(s.sync(&args[1..], d, qd::SyncOpts::default())?)?
        }
        "add" => {
            let (Some(name), Some(path)) = (args.first(), args.get(1)) else {
                anyhow::bail!("add: a name and a path");
            };
            let path = std::fs::canonicalize(path).map_err(|e| anyhow::anyhow!("{path}: {e}"))?;
            let file = s.add_module(name, &path, &args[2..])?;
            let done = s.sync(
                std::slice::from_ref(name),
                Direction::Pull,
                qd::SyncOpts::default(),
            )?;
            json!({ "file": file, "run": done.run, "applied": done.applied })
        }
        other => anyhow::bail!("no qd operation {other}"),
    })
}
