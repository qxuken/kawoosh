//! The `log` crate's logger, with the notification log as its sink
//! (kui.md Decision 9): `log::warn!` anywhere in the process is a warn
//! toast, `log::info!` a corner line, `log::debug!` a line in
//! `:messages`, `log::trace!` nothing unless `RUST_LOG=trace` — and,
//! when stderr is a terminal (or `RUST_LOG` asks),
//! every entry the notification log takes is written there too, once
//! per frame, by the frame.
//!
//! The caller's cost is the design constraint: a systems thread logs
//! from its hot loop. So `log` cuts by level and target before it looks
//! at the message, borrows a literal message rather than formatting it,
//! sends one record down an unbounded channel, and wakes the loop only
//! for the first record of a burst — the flag the drain clears — so a
//! thousand lines cost a thousand sends and one wake. Nothing is
//! written on the caller's thread.

use std::sync::atomic::{AtomicBool, Ordering};

use crossbeam_channel::{Receiver, Sender, unbounded};
use kawoosh_systems::WakeHandle;
use log::{LevelFilter, Metadata, Record};
use smallvec::SmallVec;

use crate::notify::Level;

/// A record as it crosses to the frame: the level, the target it was
/// logged under (copied inline — a module path fits, no allocation),
/// and the message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rec {
    pub level: Level,
    target: SmallVec<[u8; 24]>,
    text: Text,
}

/// A message: a literal borrowed as it is, or one formatted into an
/// inline buffer — a line's worth before the heap is touched.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Text {
    Literal(&'static str),
    Formatted(SmallVec<[u8; 64]>),
}

impl std::fmt::Write for Text {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        if let Text::Formatted(buf) = self {
            buf.extend_from_slice(s.as_bytes());
        }
        Ok(())
    }
}

impl Rec {
    pub fn target(&self) -> &str {
        std::str::from_utf8(&self.target).unwrap_or("?")
    }

    pub fn text(&self) -> &str {
        match &self.text {
            Text::Literal(s) => s,
            // Only whole `&str`s were written: it is UTF-8.
            Text::Formatted(b) => std::str::from_utf8(b).unwrap_or("?"),
        }
    }

    /// Who logged it: the target's last segment for kawoosh's crates
    /// (`kawoosh::app` → `app`, `kawoosh_systems::lsp` → `lsp`), the
    /// crate for anyone else's (`wgpu_core::device` → `wgpu_core`).
    pub fn source(&self) -> &str {
        let target = self.target();
        if ours(target) {
            target.rsplit("::").next().unwrap_or(target)
        } else {
            target.split("::").next().unwrap_or(target)
        }
    }
}

pub struct Logger {
    tx: Sender<Rec>,
    /// Set by the first record since the last drain — the one that
    /// wakes; cleared by [`Logger::drain`].
    pending: AtomicBool,
    wake: WakeHandle,
}

/// The receiving end: what the frame drains.
pub struct Sink {
    rx: Receiver<Rec>,
    pending: &'static AtomicBool,
}

impl Sink {
    /// Everything logged since the last drain, and the burst flag
    /// cleared so the next record wakes again.
    pub fn drain(&self) -> impl Iterator<Item = Rec> + '_ {
        self.pending.store(false, Ordering::Release);
        self.rx.try_iter()
    }
}

/// Where kawoosh's own crates are named: a target under one of these is
/// "ours".
const OURS: &[&str] = &["kawoosh", "text_buffer"];

fn ours(target: &str) -> bool {
    OURS.iter().any(|p| {
        target
            .strip_prefix(p)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with("::") || rest.starts_with('_'))
    })
}

fn level_of(l: log::Level) -> Level {
    match l {
        log::Level::Error => Level::Error,
        log::Level::Warn => Level::Warn,
        log::Level::Info => Level::Info,
        log::Level::Debug => Level::Debug,
        log::Level::Trace => Level::Trace,
    }
}

impl Logger {
    /// A logger and its sink, not yet installed: what a test drives by
    /// calling `log` on it.
    pub fn new(wake: WakeHandle) -> (&'static Logger, Sink) {
        let (tx, rx) = unbounded();
        let logger: &'static Logger = Box::leak(Box::new(Logger {
            tx,
            pending: AtomicBool::new(false),
            wake,
        }));
        (
            logger,
            Sink {
                rx,
                pending: &logger.pending,
            },
        )
    }

    /// Installs it as the process's logger, taking records from `keep`
    /// up — the cut the macros make before calling in, so a trace
    /// nobody keeps costs a load. `None` when one is already installed
    /// — a second app in the same process, which is a test.
    pub fn install(wake: WakeHandle, keep: Level) -> Option<Sink> {
        let (logger, sink) = Logger::new(wake);
        log::set_logger(logger).ok()?;
        log::set_max_level(match keep {
            Level::Trace => LevelFilter::Trace,
            Level::Debug => LevelFilter::Debug,
            Level::Info => LevelFilter::Info,
            Level::Warn => LevelFilter::Warn,
            Level::Error => LevelFilter::Error,
        });
        Some(sink)
    }

    /// The cut: kawoosh's own crates from debug up, anyone else's from
    /// warn — before the message is formatted.
    fn wanted(level: log::Level, target: &str) -> bool {
        level <= log::Level::Warn || ours(target)
    }
}

impl log::Log for Logger {
    fn enabled(&self, m: &Metadata) -> bool {
        Self::wanted(m.level(), m.target())
    }

    fn log(&self, record: &Record) {
        if !Self::wanted(record.level(), record.target()) {
            return;
        }
        // A literal message is borrowed for free; one with arguments is
        // formatted once, here, into the record's own buffer.
        let text = match record.args().as_str() {
            Some(s) => Text::Literal(s),
            None => {
                let mut t = Text::Formatted(SmallVec::new());
                let _ = std::fmt::Write::write_fmt(&mut t, *record.args());
                t
            }
        };
        let rec = Rec {
            level: level_of(record.level()),
            target: SmallVec::from_slice(record.target().as_bytes()),
            text,
        };
        if self.tx.send(rec).is_err() {
            return;
        }
        // The first of a burst wakes; the rest ride along.
        if !self.pending.swap(true, Ordering::AcqRel) {
            self.wake.wake();
        }
    }

    fn flush(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use log::Log;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

    fn record(level: log::Level, target: &str, args: std::fmt::Arguments<'_>) -> Vec<Rec> {
        let wake = WakeHandle::new();
        let (logger, sink) = Logger::new(wake);
        logger.log(
            &Record::builder()
                .level(level)
                .target(target)
                .args(args)
                .build(),
        );
        sink.drain().collect()
    }

    /// Our crates from debug up, with the module as the source; other
    /// crates from warn, with the crate as the source.
    #[test]
    fn the_cut_and_the_source() {
        let got = record(log::Level::Debug, "kawoosh::app", format_args!("x {}", 1));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].source(), "app");
        assert_eq!(got[0].text(), "x 1");
        assert_eq!(got[0].level, Level::Debug);
        let got = record(log::Level::Warn, "kawoosh_systems::lsp", format_args!("y"));
        assert_eq!(got[0].source(), "lsp");
        assert!(
            matches!(got[0].text, Text::Literal(_)),
            "a literal is borrowed"
        );
        assert!(record(log::Level::Info, "wgpu_core::device", format_args!("z")).is_empty());
        let got = record(log::Level::Warn, "wgpu_core::device", format_args!("z"));
        assert_eq!(got[0].source(), "wgpu_core");
        assert!(record(log::Level::Debug, "kawooshish::x", format_args!("no")).is_empty());
    }

    /// A burst wakes once; after a drain, the next record wakes again.
    #[test]
    fn a_burst_wakes_once() {
        let wakes = Arc::new(AtomicUsize::new(0));
        let wake = WakeHandle::new();
        let w = wakes.clone();
        wake.set(Arc::new(move || {
            w.fetch_add(1, Ordering::SeqCst);
        }));
        let (logger, sink) = Logger::new(wake);
        for i in 0..100 {
            logger.log(
                &Record::builder()
                    .level(log::Level::Info)
                    .target("kawoosh::app")
                    .args(format_args!("{i}"))
                    .build(),
            );
        }
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        assert_eq!(sink.drain().count(), 100);
        logger.log(
            &Record::builder()
                .level(log::Level::Info)
                .target("kawoosh::app")
                .args(format_args!("again"))
                .build(),
        );
        assert_eq!(wakes.load(Ordering::SeqCst), 2);
    }
}
