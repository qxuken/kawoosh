//! `.` and macros: the command stream, kept (docs/design/keys.md,
//! mvp.md D4). Every key that ran a command and every run of text
//! typed in insert mode is a [`Step`]; `.` replays the steps of the
//! last change and `q` / `@` a named span of them. A step is not a key:
//! it is the command as it ran — its name, arguments, count and the
//! character it asked for — so a macro survives a remap and replays
//! through the registry the keys went through.

use std::collections::HashMap;

/// How deep replays nest before one is refused: a macro that plays
/// itself ends here, since no step fails the way vim's motions do.
pub const DEPTH: usize = 100;

/// The steps an outermost replay may run in all, nested ones included.
pub const BUDGET: usize = 100_000;

/// One thing the keys did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// A command as it was run — what [`crate::Editor::run`] was given
    /// — and the character a `takes_char` command got after it.
    Command {
        name: String,
        args: Vec<String>,
        count: Option<usize>,
        arg_char: Option<char>,
    },
    /// Text typed in insert mode, the prompt's field included; runs of
    /// it are one step.
    Text(String),
}

/// The stream, kept two ways: the change under way for `.`, and the
/// span `q` is recording.
#[derive(Default)]
pub struct Recorder {
    /// The steps of the change under way: from the first that left
    /// something open on the view — an operator, a character to come,
    /// insert or visual mode, the prompt — to the one that closes it.
    pub(crate) current: Vec<Step>,
    /// The last change, `.`'s.
    pub last_change: Vec<Step>,
    /// `q`'s register and the steps so far.
    recording: Option<(char, Vec<Step>)>,
    /// Every macro by its register.
    pub macros: HashMap<char, Vec<Step>>,
    /// The register `@` played last, `@@`'s.
    pub last_played: Option<char>,
    /// How many replays are under way; nothing is recorded inside one,
    /// the step that started it having been.
    pub(crate) depth: usize,
    /// The steps left to the outermost replay.
    pub(crate) budget: usize,
}

impl Recorder {
    /// The register `q` is recording into.
    pub fn recording(&self) -> Option<char> {
        self.recording.as_ref().map(|(c, _)| *c)
    }

    /// Starts recording into `c`; an upper-case letter appends to its
    /// lower-case register, as vim's does.
    pub(crate) fn start(&mut self, c: char) {
        let (c, append) = match c.is_ascii_uppercase() {
            true => (c.to_ascii_lowercase(), true),
            false => (c, false),
        };
        let steps = match append {
            true => self.macros.get(&c).cloned().unwrap_or_default(),
            false => Vec::new(),
        };
        self.recording = Some((c, steps));
    }

    /// Ends the recording, the register now holding it.
    pub(crate) fn stop(&mut self) -> Option<char> {
        let (c, steps) = self.recording.take()?;
        self.macros.insert(c, steps);
        Some(c)
    }

    /// A step into the recording, if one is on.
    pub(crate) fn record(&mut self, step: &Step) {
        if let Some((_, steps)) = &mut self.recording {
            push(steps, step.clone());
        }
    }
}

/// `step` onto `steps`, text joining the text before it.
pub(crate) fn push(steps: &mut Vec<Step>, step: Step) {
    if let (Some(Step::Text(last)), Step::Text(t)) = (steps.last_mut(), &step) {
        last.push_str(t);
        return;
    }
    steps.push(step);
}
