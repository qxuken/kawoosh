//! A terminal's session as an asciicast file: what asciinema records
//! and `wezterm record` writes, and what a bug in a terminal is shown
//! with — the bytes a program sent, when, and the sizes the screen was,
//! for another terminal to play back and draw as this one did.
//!
//! [`Recorder`] writes version 2, which every player reads: a header
//! line (`{"version": 2, "width": 80, "height": 24, …}`), then one
//! line an event, `[seconds since the start, code, data]` — `o` the
//! program's output, `i` what was typed, `r` a resize (`"120x40"`),
//! `m` a marker. [`Cast::parse`] reads version 2 and version 3
//! (asciinema 3, whose times are each since the event before, its size
//! under `term`). [`Player`] feeds a [`Cast`] to a [`Terminal`] at the
//! pace it was recorded.
//!
//! Output is text in the format, so the bytes are UTF-8 there: a
//! character a read split is carried whole into the next event, and a
//! byte that is no UTF-8 at all is written as U+FFFD, as asciinema
//! writes it.

use std::fs::File;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::{TermSize, Terminal};

/// A terminal being recorded: each event written as it comes, so a
/// window that dies mid-recording leaves what came before it.
pub struct Recorder {
    out: File,
    path: PathBuf,
    start: Instant,
    /// The bytes the last output ended inside a character with.
    carry: Vec<u8>,
    /// The first write that failed; nothing is written after it.
    error: Option<io::Error>,
}

/// A recording finished ([`Recorder::finish`]).
#[derive(Debug)]
pub struct Recorded {
    pub path: PathBuf,
    pub length: Duration,
}

impl Recorder {
    /// A recording into `path` (made, its directory with it), of a
    /// screen `size` big now; `env` is what the program was given that
    /// a player wants to know (`TERM`, `SHELL`, `LANG`).
    pub fn create(
        path: &Path,
        size: TermSize,
        env: &[(String, String)],
        title: Option<&str>,
    ) -> io::Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut out = File::create(path)?;
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let env: serde_json::Map<String, serde_json::Value> = env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone().into()))
            .collect();
        let mut header = serde_json::json!({
            "version": 2,
            "width": size.cols,
            "height": size.rows,
            "timestamp": timestamp,
            "env": env,
        });
        if let Some(t) = title.filter(|t| !t.is_empty()) {
            header["title"] = t.into();
        }
        out.write_all(format!("{header}\n").as_bytes())?;
        Ok(Self {
            out,
            path: path.to_path_buf(),
            start: Instant::now(),
            carry: Vec::new(),
            error: None,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The program's output: the whole characters of it and what the
    /// last read carried; a character it ends inside waits for the next.
    pub(crate) fn output(&mut self, bytes: &[u8]) {
        self.carry.extend_from_slice(bytes);
        let (text, used) = utf8_whole(&self.carry);
        self.carry.drain(..used);
        if !text.is_empty() {
            self.event("o", &text);
        }
    }

    /// What was typed, for a bug in the keys a program was sent.
    pub(crate) fn input(&mut self, bytes: &[u8]) {
        self.event("i", &String::from_utf8_lossy(bytes));
    }

    pub(crate) fn resize(&mut self, size: TermSize) {
        self.event("r", &format!("{}x{}", size.cols, size.rows));
    }

    /// A point in the recording named `label`: where the bug is.
    pub fn marker(&mut self, label: &str) {
        self.event("m", label);
    }

    fn event(&mut self, code: &str, data: &str) {
        if self.error.is_some() {
            return;
        }
        let at = self.start.elapsed().as_secs_f64();
        let data = serde_json::Value::from(data);
        let line = format!("[{at:.6}, \"{code}\", {data}]\n");
        if let Err(e) = self.out.write_all(line.as_bytes()) {
            self.error = Some(e);
        }
    }

    /// The recording ended: what was carried written as it is, and the
    /// file's place and length — or the write that failed.
    pub fn finish(mut self) -> io::Result<Recorded> {
        if !self.carry.is_empty() {
            let text = String::from_utf8_lossy(&std::mem::take(&mut self.carry)).into_owned();
            self.event("o", &text);
        }
        if let Some(e) = self.error.take() {
            return Err(e);
        }
        self.out.flush()?;
        Ok(Recorded {
            path: self.path,
            length: self.start.elapsed(),
        })
    }
}

/// `bytes` as text up to a character it ends inside, and how many
/// bytes that took; a byte that starts no character is U+FFFD.
fn utf8_whole(bytes: &[u8]) -> (String, usize) {
    let mut text = String::new();
    let mut rest = bytes;
    loop {
        match std::str::from_utf8(rest) {
            Ok(s) => {
                text.push_str(s);
                return (text, bytes.len());
            }
            Err(e) => {
                let (good, bad) = rest.split_at(e.valid_up_to());
                // SAFETY: `valid_up_to` is where the valid UTF-8 ends.
                text.push_str(unsafe { std::str::from_utf8_unchecked(good) });
                match e.error_len() {
                    // Ends inside a character: the rest waits.
                    None => return (text, bytes.len() - bad.len()),
                    Some(n) => {
                        text.push('\u{FFFD}');
                        rest = &bad[n..];
                    }
                }
            }
        }
    }
}

/// A recording read back.
#[derive(Clone, Debug, PartialEq)]
pub struct Cast {
    /// The screen's size as the recording starts.
    pub size: TermSize,
    pub title: Option<String>,
    /// What the program was given, as the header says (`TERM`, `LANG`).
    pub env: Vec<(String, String)>,
    /// In order, each at its time since the start.
    pub events: Vec<Entry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub at: Duration,
    pub kind: Kind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Output(String),
    Input(String),
    Resize(TermSize),
    Marker(String),
    /// Version 3's end: the program's exit status.
    Exit(i64),
}

impl Cast {
    /// An asciicast of version 2 or 3. An event of a code it does not
    /// know is passed over, as players pass it.
    pub fn parse(text: &str) -> Result<Self, String> {
        use serde_json::Value;
        let mut lines = text
            .lines()
            .enumerate()
            .filter(|(_, l)| !l.trim().is_empty() && !l.starts_with('#'));
        let (_, first) = lines.next().ok_or("an empty file")?;
        let header: Value = serde_json::from_str(first).map_err(|e| format!("its header: {e}"))?;
        let version = header["version"].as_u64().ok_or("no asciicast header")?;
        let dim = |v: &Value| v.as_u64().map(|n| n.clamp(1, u16::MAX as u64) as u16);
        let size = match version {
            2 => (dim(&header["width"]), dim(&header["height"])),
            3 => (dim(&header["term"]["cols"]), dim(&header["term"]["rows"])),
            v => return Err(format!("asciicast version {v}: 2 and 3 are read")),
        };
        let (Some(cols), Some(rows)) = size else {
            return Err("its header has no size".into());
        };
        let env = header["env"]
            .as_object()
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        let mut events = Vec::new();
        let mut at = 0.0f64;
        for (n, line) in lines {
            let e: Value =
                serde_json::from_str(line).map_err(|e| format!("line {}: {e}", n + 1))?;
            let (Some(t), Some(code)) = (e[0].as_f64(), e[1].as_str()) else {
                return Err(format!("line {}: not an event", n + 1));
            };
            // Version 3's time is since the event before.
            at = if version == 3 { at + t } else { t };
            let data = || e[2].as_str().unwrap_or_default().to_string();
            let kind = match code {
                "o" => Kind::Output(data()),
                "i" => Kind::Input(data()),
                "m" => Kind::Marker(data()),
                "r" => match data().split_once('x').map(|(c, r)| (c.parse(), r.parse())) {
                    Some((Ok(cols), Ok(rows))) if cols > 0 && rows > 0 => {
                        Kind::Resize(TermSize { rows, cols })
                    }
                    _ => continue,
                },
                "x" => Kind::Exit(
                    e[2].as_i64()
                        .or_else(|| e[2].as_str()?.parse().ok())
                        .unwrap_or(0),
                ),
                _ => continue,
            };
            events.push(Entry {
                at: Duration::from_secs_f64(at.max(0.0)),
                kind,
            });
        }
        Ok(Self {
            size: TermSize { rows, cols },
            title: header["title"].as_str().map(str::to_string),
            env,
            events,
        })
    }

    pub fn read(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        Self::parse(&text)
    }

    /// From the start to the last event.
    pub fn length(&self) -> Duration {
        self.events.last().map_or(Duration::ZERO, |e| e.at)
    }
}

/// The longest a replay waits between two events: a recording left
/// running while its author read something is not watched in full.
pub const IDLE: Duration = Duration::from_secs(2);

/// A [`Cast`] played into a [`Terminal`]: each event fed once its time
/// has come, with no wait longer than [`IDLE`].
pub struct Player {
    events: Vec<Entry>,
    next: usize,
    started: Instant,
}

impl Player {
    /// `cast` from its start, now; its waits cut to [`IDLE`].
    pub fn new(cast: Cast, now: Instant) -> Self {
        let mut events = cast.events;
        let (mut was, mut at) = (Duration::ZERO, Duration::ZERO);
        for e in &mut events {
            at += e.at.saturating_sub(was).min(IDLE);
            was = e.at;
            e.at = at;
        }
        Self {
            events,
            next: 0,
            started: now,
        }
    }

    /// A terminal for `cast` to play into: no process behind it, the
    /// recording's size.
    pub fn screen(cast: &Cast) -> Terminal {
        Terminal::headless(cast.size)
    }

    /// The events whose time has come by `now` fed to `term` — output
    /// parsed, a resize made; what it would answer a program is
    /// dropped, there being none. The time the next one is due, or
    /// None once all were.
    pub fn step(&mut self, term: &mut Terminal, now: Instant) -> Option<Instant> {
        while let Some(e) = self.events.get(self.next) {
            let due = self.started + e.at;
            if due > now {
                return Some(due);
            }
            match &e.kind {
                Kind::Output(text) => term.feed(text.as_bytes()),
                Kind::Resize(size) => term.resize(*size),
                Kind::Input(_) | Kind::Marker(_) | Kind::Exit(_) => {}
            }
            self.next += 1;
        }
        term.take_sent();
        None
    }

    pub fn done(&self) -> bool {
        self.next >= self.events.len()
    }

    /// How far it has played, and how long it is.
    pub fn progress(&self, now: Instant) -> (Duration, Duration) {
        let total = self.events.last().map_or(Duration::ZERO, |e| e.at);
        (
            now.saturating_duration_since(self.started).min(total),
            total,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kawoosh-cast-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    /// A character a read split arrives whole in the next event; a byte
    /// that is no UTF-8 is U+FFFD; what is left at the end is written.
    #[test]
    fn a_split_character_is_carried_whole() {
        let path = temp("split.cast");
        let size = TermSize { rows: 24, cols: 80 };
        let env = [("TERM".to_string(), "xterm-256color".to_string())];
        let mut r = Recorder::create(&path, size, &env, Some("t")).unwrap();
        let word = "Зобков".as_bytes();
        r.output(&word[..3]);
        r.output(&word[3..]);
        r.output(b"a\xffb");
        r.resize(TermSize {
            rows: 40,
            cols: 120,
        });
        r.input(b"q");
        r.output(&word[..1]);
        r.finish().unwrap();
        let cast = Cast::read(&path).unwrap();
        assert_eq!(cast.size, size);
        assert_eq!(cast.title.as_deref(), Some("t"));
        assert_eq!(cast.env, env);
        let kinds: Vec<Kind> = cast.events.into_iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            [
                Kind::Output("З".into()),
                Kind::Output("обков".into()),
                Kind::Output("a\u{FFFD}b".into()),
                Kind::Resize(TermSize {
                    rows: 40,
                    cols: 120
                }),
                Kind::Input("q".into()),
                Kind::Output("\u{FFFD}".into()),
            ]
        );
    }

    /// Version 3 counts each time from the event before, its size
    /// under `term`.
    #[test]
    fn version_3_times_add_up() {
        let text = r#"{"version": 3, "term": {"cols": 100, "rows": 30}}
# a comment
[0.5, "o", "a"]
[0.25, "r", "90x20"]
[1.0, "x", "0"]
"#;
        let cast = Cast::parse(text).unwrap();
        assert_eq!(
            cast.size,
            TermSize {
                rows: 30,
                cols: 100
            }
        );
        let at: Vec<f64> = cast.events.iter().map(|e| e.at.as_secs_f64()).collect();
        assert_eq!(at, [0.5, 0.75, 1.75]);
        assert_eq!(cast.events[2].kind, Kind::Exit(0));
    }

    /// The player feeds what is due, keeps the rest, and never waits
    /// longer than [`IDLE`].
    #[test]
    fn the_player_keeps_the_pace_with_idle_cut() {
        let text = "{\"version\": 2, \"width\": 20, \"height\": 3}\n\
                    [0.0, \"o\", \"one\"]\n\
                    [60.0, \"o\", \" two\"]\n\
                    [60.5, \"r\", \"10x3\"]\n";
        let cast = Cast::parse(text).unwrap();
        let mut term = Player::screen(&cast);
        let now = Instant::now();
        let mut p = Player::new(cast, now);
        let row = |t: &Terminal| {
            let s = t.screen();
            s.cells[..s.cols].iter().map(|c| c.ch).collect::<String>()
        };
        assert_eq!(p.step(&mut term, now), Some(now + IDLE));
        assert!(row(&term).starts_with("one "), "{:?}", row(&term));
        assert_eq!(
            p.step(&mut term, now + IDLE),
            Some(now + IDLE + Duration::from_millis(500))
        );
        assert!(row(&term).starts_with("one two"));
        assert_eq!(p.step(&mut term, now + Duration::from_secs(9)), None);
        assert_eq!(term.size(), TermSize { rows: 3, cols: 10 });
        assert!(p.done());
    }
}
