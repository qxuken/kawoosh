//! A shell line's arguments completed in the command line by a program
//! outside (compile.md Decision 23): carapace, or what
//! `cmdline.complete_with` names, asked `COMPLETER PROG export PROG
//! WORDS… CURRENT` in the directory the line will run in, its JSON's
//! `values` the candidates and their `description`s the details. What
//! `:compile` completes a line that is no named command with, and
//! `:!`, `:shell` and `:terminal` the program they run.
//!
//! Nothing waits on it: each question is asked on a job of its own,
//! bounded by [`PATIENCE`] and [`CAP`], and the line is completed again
//! when it answers. An answer is kept by its question — completer,
//! directory, the words before the caret's, the current word's stem —
//! for the prompt's life, so typing through a word asks once; one for
//! a question the line no longer asks is kept and not shown, and one
//! arriving after the prompt closed is dropped. A program the completer
//! does not know, a completer that fails, hangs or prints what is no
//! answer: nothing, quietly.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::rc::Rc;
use std::time::Duration;

use kawoosh_editor::{Layer, Setting};
use kawoosh_systems::io::IoMsg;

use crate::app::Kawoosh;

/// How long a completer is waited for, its answer then dropped.
pub const PATIENCE: Duration = Duration::from_secs(2);
/// The most a completer may print: past it the answer is dropped and
/// the completer stopped. carapace's longest (git's every flag) is
/// some 10 KiB.
pub const CAP: usize = 1 << 20;
/// The most values one answer gives.
pub const MOST: usize = 2000;
/// Questions out at once: past it a new one waits for a later key, or
/// for an answer, which completes the line again.
const IN_FLIGHT: usize = 4;

/// A question put to the completer.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct Ask {
    completer: String,
    dir: PathBuf,
    /// The program and the words after it, before the current one.
    words: Vec<String>,
    /// What of the current word the completer is given (see [`stem`]).
    stem: String,
}

enum Answer {
    /// Out on a job, by serial.
    Asked(u64),
    /// Each value and its description.
    Got(Rc<Vec<(String, String)>>),
}

/// The answers kept while the `:` prompt is open.
#[derive(Default)]
pub struct ShellComplete {
    answers: HashMap<Ask, Answer>,
    serial: u64,
    /// Whether the completer was found on the PATH, by name, looked up
    /// once a prompt.
    found: Option<(String, bool)>,
}

impl ShellComplete {
    /// The prompt closed: nothing kept, and an answer still out is
    /// dropped when it lands.
    pub fn clear(&mut self) {
        self.answers.clear();
        self.found = None;
    }

    fn in_flight(&self) -> usize {
        self.answers
            .values()
            .filter(|a| matches!(a, Answer::Asked(_)))
            .count()
    }
}

/// What of the word being typed the completer is asked with: up to its
/// last `/` or `=` (a directory's entries, a flag's values come from
/// there), else `-` for a flag (carapace offers the flags only to a
/// word that starts with one), else nothing — the rest is narrowed
/// here, so typing through a word asks once.
fn stem(current: &str) -> &str {
    match current.rfind(['/', '=']) {
        Some(i) => &current[..=i],
        None if current.starts_with('-') => "-",
        None => "",
    }
}

/// The words of the command the caret is in, from a shell line's text
/// before the current word: past the last `|`, `;`, `&&` or `||`
/// standing as a word. None when the text does not split — a quote
/// open — or the command is an environment assignment's.
fn command_words(before: &str) -> Option<Vec<String>> {
    let words = shell_words::split(before).ok()?;
    let from = words
        .iter()
        .rposition(|w| matches!(w.as_str(), "|" | ";" | "&&" | "||"))
        .map(|i| i + 1)
        .unwrap_or(0);
    let words = words[from..].to_vec();
    let program = words.first()?;
    if program.contains('=') {
        return None;
    }
    Some(words)
}

/// carapace's answer: `{ "values": [{ "value", "description" }…] }`;
/// anything else is none.
pub fn parse(out: &[u8]) -> Option<Vec<(String, String)>> {
    let json: serde_json::Value = serde_json::from_slice(out).ok()?;
    let values = json.get("values")?.as_array()?;
    Some(
        values
            .iter()
            .filter_map(|v| {
                let value = v.get("value")?.as_str()?;
                let about = v.get("description").and_then(|d| d.as_str());
                Some((value.to_string(), about.unwrap_or_default().to_string()))
            })
            .filter(|(v, _)| !v.is_empty())
            .take(MOST)
            .collect(),
    )
}

/// Asks `completer` for `words`' next word starting `current`, in
/// `dir`: `COMPLETER PROG export PROG ARGS… CURRENT`. None when it
/// could not start, outlasted [`PATIENCE`] (stopped then), printed past
/// [`CAP`] (stopped too) or printed no answer. Run on a job: it waits.
pub fn ask(
    completer: &str,
    dir: &Path,
    words: &[String],
    current: &str,
) -> Option<Vec<(String, String)>> {
    let program = Path::new(words.first()?)
        .file_name()?
        .to_string_lossy()
        .into_owned();
    let mut command = kawoosh_systems::io::command(completer);
    command
        .arg(&program)
        .arg("export")
        .arg(&program)
        .args(&words[1..])
        .arg(current)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = kawoosh_systems::spawn::spawn(&mut command).ok()?;
    let Some(stdout) = child.stdout.take() else {
        child.kill().ok();
        child.wait().ok();
        return None;
    };
    // Read on a thread of its own, so the wait is bounded even when a
    // child the completer started holds the pipe open.
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = std::thread::Builder::new()
        .name("line complete read".into())
        .spawn(move || {
            let mut out = Vec::new();
            stdout.take(CAP as u64 + 1).read_to_end(&mut out).ok();
            tx.send(out).ok();
        });
    let out = match reader {
        Ok(_) => rx.recv_timeout(PATIENCE).ok(),
        Err(_) => None,
    };
    if !matches!(child.try_wait(), Ok(Some(_))) {
        child.kill().ok();
    }
    child.wait().ok();
    let out = out?;
    if out.len() > CAP {
        return None;
    }
    parse(&out)
}

impl Kawoosh {
    /// The completer `cmdline.complete_with` names, or None: off, an app
    /// that loaded no config (`Config::completer`), or named by a
    /// project's settings — a project may turn it off but not
    /// name a program to run at every key, which its `settings.lua`, as
    /// data, is not trusted to (compile.md Decision 23).
    pub(crate) fn line_completer(&self) -> Option<String> {
        const PATH: &str = "cmdline.complete_with";
        if !self.config.completer {
            return None;
        }
        let s = &self.ed.settings;
        for layer in [Layer::Session, Layer::Project, Layer::User, Layer::Default] {
            match s.layer_value(layer, PATH) {
                Some(Setting::Bool(false)) => return None,
                Some(Setting::Str(_)) if layer == Layer::Project => {}
                Some(Setting::Str(p)) if p.is_empty() => return None,
                Some(Setting::Str(p)) => return Some(p.clone()),
                _ => {}
            }
        }
        None
    }

    /// The completer's candidates for the word `current` after
    /// `before` — a shell line's text up to it — run in `dir`: each value
    /// that starts with the word, quoted as a word of the line, and its
    /// description. Nothing until the completer answers (asked now when
    /// it has not been), nothing for a host's directory, nothing with no
    /// completer on the PATH.
    pub(crate) fn shell_arg_candidates(
        &mut self,
        before: &str,
        current: &str,
        dir: &Path,
    ) -> Vec<(String, String)> {
        if current.contains(['\'', '"', '\\']) || kawoosh_systems::fs::domain_of(dir).is_some() {
            return Vec::new();
        }
        let Some(words) = command_words(before) else {
            return Vec::new();
        };
        let Some(completer) = self.line_completer() else {
            return Vec::new();
        };
        let found = match &self.line_complete.found {
            Some((name, found)) if *name == completer => *found,
            _ => {
                // Unknown while the shell's PATH is still asked for: not
                // kept, so asked again at the next key.
                let Some(found) = kawoosh_systems::io::on_path(&completer) else {
                    return Vec::new();
                };
                self.line_complete.found = Some((completer.clone(), found));
                found
            }
        };
        if !found {
            return Vec::new();
        }
        let stem = stem(current).to_string();
        let key = Ask {
            completer,
            dir: dir.to_path_buf(),
            words,
            stem,
        };
        match self.line_complete.answers.get(&key) {
            Some(Answer::Got(values)) => {
                return values
                    .iter()
                    .filter(|(v, _)| v.starts_with(current))
                    .map(|(v, about)| (crate::compile::quote_arg(v), about.clone()))
                    .collect();
            }
            Some(Answer::Asked(_)) => return Vec::new(),
            None if self.line_complete.in_flight() >= IN_FLIGHT => return Vec::new(),
            None => {}
        }
        self.line_complete.serial += 1;
        let serial = self.line_complete.serial;
        self.line_complete
            .answers
            .insert(key.clone(), Answer::Asked(serial));
        self.pending_jobs += 1;
        self.io.run("line complete", move || IoMsg::LineCompleted {
            serial,
            values: ask(&key.completer, &key.dir, &key.words, &key.stem),
        });
        Vec::new()
    }

    /// The completer answered question `serial`: kept under it, and the
    /// line completed again — showing it only if the line still asks it.
    /// Dropped when the prompt it was asked for has closed.
    pub(crate) fn line_completed(&mut self, serial: u64, values: Option<Vec<(String, String)>>) {
        self.pending_jobs = self.pending_jobs.saturating_sub(1);
        let Some(slot) = self
            .line_complete
            .answers
            .values_mut()
            .find(|a| matches!(a, Answer::Asked(s) if *s == serial))
        else {
            return;
        };
        if values.is_none() {
            log::debug!("cmdline.complete_with: no answer");
        }
        *slot = Answer::Got(Rc::new(values.unwrap_or_default()));
        self.cmdline_refresh();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stem_is_what_the_completer_narrows_from() {
        assert_eq!(stem(""), "");
        assert_eq!(stem("comm"), "");
        assert_eq!(stem("--am"), "-");
        assert_eq!(stem("-"), "-");
        assert_eq!(stem("--cleanup=st"), "--cleanup=");
        assert_eq!(stem("src/ma"), "src/");
        assert_eq!(stem("~/p/q"), "~/p/");
    }

    #[test]
    fn the_command_is_the_one_the_caret_is_in() {
        let w = |s: &str| command_words(s);
        assert_eq!(w("git commit "), Some(vec!["git".into(), "commit".into()]));
        assert_eq!(w("make && cargo "), Some(vec!["cargo".into()]));
        assert_eq!(w("echo 'a b' "), Some(vec!["echo".into(), "a b".into()]));
        assert_eq!(w("echo 'a b "), None, "a quote open");
        assert_eq!(w("FOO=1 cargo "), None);
        assert_eq!(w(""), None);
    }

    #[test]
    fn an_answer_is_carapace_s_values_or_none() {
        let out = br#"{"version":"v1","messages":[],"values":[{"value":"commit","display":"commit","description":"Record changes"},{"value":"--amend"},{"value":""}]}"#;
        assert_eq!(
            parse(out),
            Some(vec![
                ("commit".into(), "Record changes".into()),
                ("--amend".into(), String::new()),
            ])
        );
        assert_eq!(parse(b"not json at all"), None);
        assert_eq!(parse(b"[1, 2]"), None);
        assert_eq!(parse(b""), None);
    }
}
