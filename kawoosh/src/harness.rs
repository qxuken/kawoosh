//! Kawoosh's headless harness is kui's `Core` (kui.md D8): a frame is a
//! function of the tree and the input so far, so a test presses keys
//! through the real `App::view` / `on_event` and asserts on the model
//! *and* on the drawn rows. The driving is kui's (`kui_native::testing::
//! Drive`); what is here is kawoosh's own: keys in map notation and the
//! readings of its rows, gutters and floats. The Rust tests take it as
//! `Drive` (`kawoosh/tests/drive.rs`), and `kawoosh test
//! PATH…` (roadmap step 8) drives a Lua script with it: the script runs
//! as a coroutine, `kawoosh.press(keys)` yields the keys to press,
//! `kawoosh.frame(n)` frames to draw and `kawoosh.wait(fn)` frames until
//! a reading holds, and between resumes the editor is published to Lua,
//! so `kawoosh.buf.text()`, `kawoosh.mode()`, `kawoosh.message()` and
//! `kawoosh.picker.state()` read the state as it is. A plain `assert`
//! or `error` fails the run with its traceback. The bundled plugins'
//! Lua tests (`kawoosh/lua/tests/*.lua`) run on it, from `cargo test`
//! and from the CLI alike.

use std::ops::{Deref, DerefMut};
use std::path::Path;

use kawoosh_editor::keymap::{LEADER, parse_notation};
use kawoosh_lua::TestStep;
use kui_native::testing::Drive;
use kui_native::{Core, KeyMods, Rect};

use crate::app::Kawoosh;

/// The window a Lua test is drawn in.
const TEST_VIEWPORT: (f32, f32) = (1000.0, 700.0);

/// kui's headless [`Drive`], framing after every gesture, with
/// kawoosh's own readings of what it drew: the panes' rows and gutters,
/// the confirm, the corner — and keys pressed in map notation.
pub struct Harness(pub Drive);

impl Deref for Harness {
    type Target = Drive;
    fn deref(&self) -> &Drive {
        &self.0
    }
}

impl DerefMut for Harness {
    fn deref_mut(&mut self) -> &mut Drive {
        &mut self.0
    }
}

impl Harness {
    /// A `w` by `h` window, kui's diagnostics on so a test can assert
    /// it raised no warning.
    pub fn new(w: f32, h: f32) -> Self {
        let mut core = Core::new();
        core.set_diagnostics(true);
        Self(Drive::new(core, w, h).framing())
    }

    /// Where the node labelled `label` was hit last frame (kui's
    /// `rect_of`, by the name a test knows it by).
    pub fn rect(&mut self, label: &str) -> Option<Rect> {
        let key = self.key_of(label)?;
        self.rect_of(key)
    }

    /// Keys in map notation — `jj`, `<C-w>v`, `<leader>f`, `<A-J>`,
    /// `:w<CR>` — each token a press: a named key by its name, a chord's
    /// upper-case letter as the letter with Shift, `<leader>` as what
    /// the keymap says it stands for.
    pub fn press(&mut self, app: &mut Kawoosh, notation: &str) {
        let leader = app.ed.keymap.leader().to_string();
        for tok in parse_notation(notation) {
            if tok == LEADER {
                self.press(app, &leader);
                continue;
            }
            let Some(inner) = tok.strip_prefix('<').and_then(|s| s.strip_suffix('>')) else {
                self.keys(app, &tok);
                continue;
            };
            let mut mods = KeyMods::default();
            let mut parts: Vec<&str> = inner.split('-').collect();
            // `<C-->`: a dash under a modifier.
            if parts.len() > 1 && parts.last() == Some(&"") {
                parts.pop();
                parts.pop();
                parts.push("-");
            }
            let base = parts.pop().unwrap_or("");
            for m in parts {
                match m {
                    "C" | "c" => mods.ctrl = true,
                    "A" | "a" | "M" | "m" => mods.alt = true,
                    "D" | "d" => mods.super_key = true,
                    "S" | "s" => mods.shift = true,
                    _ => {}
                }
            }
            let named = match base {
                "Esc" => Some("escape"),
                "CR" | "Enter" | "Return" => Some("enter"),
                "Tab" => Some("tab"),
                "BS" => Some("backspace"),
                "Del" => Some("delete"),
                "Space" => Some("space"),
                "Up" => Some("up"),
                "Down" => Some("down"),
                "Left" => Some("left"),
                "Right" => Some("right"),
                "Home" => Some("home"),
                "End" => Some("end"),
                "PageUp" => Some("pageup"),
                "PageDown" => Some("pagedown"),
                "Insert" => Some("insert"),
                f if f.len() <= 3
                    && f.starts_with('F')
                    && f[1..].chars().all(|c| c.is_ascii_digit()) =>
                {
                    None
                }
                _ => None,
            };
            let chord = mods.ctrl || mods.alt || mods.super_key;
            let name: String = match named {
                Some(n) => n.to_string(),
                None if base.starts_with('F') && base.len() > 1 => base.to_lowercase(),
                None if chord && base.len() == 1 && base.as_bytes()[0].is_ascii_uppercase() => {
                    mods.shift = true;
                    base.to_ascii_lowercase()
                }
                None => base.to_string(),
            };
            self.key(app, &name, mods);
        }
    }

    /// The text of every row under every node labelled `lines` in the
    /// last frame, pane by pane, top to bottom, runs joined — one
    /// document line each. What is drawn on the row but is not the
    /// line's — a `Role::None` box: an annotation, a diagnostic's
    /// message, the completion ghost — is left out; `row_extras` has it.
    pub fn line_rows(&self) -> Vec<String> {
        self.rows().into_iter().map(|(line, _)| line).collect()
    }

    /// Each drawn row's text after its line — annotations, a
    /// diagnostic's message — under `Role::None`, joined.
    pub fn row_extras(&self) -> Vec<String> {
        self.rows().into_iter().map(|(_, extra)| extra).collect()
    }

    /// Where each drawn row's text after its line starts and where the
    /// line's own text ends, in window pixels — `(note_x, text_end)` for
    /// the rows that have a note, top to bottom: a column of notes
    /// starts at one x, past every line. A wrapped row's line number is
    /// in the row too, before its text, and is no note.
    pub fn note_places(&self) -> Vec<(f32, f32)> {
        let nodes = self.core.nodes();
        let holders: Vec<_> = nodes
            .iter()
            .filter(|n| matches!(n.label.as_deref(), Some("lines" | "lines above")))
            .map(|n| n.key)
            .collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < nodes.len() {
            if !nodes[i].parent.is_some_and(|p| holders.contains(&p)) {
                i += 1;
                continue;
            }
            let depth = nodes[i].depth;
            let (mut note, mut end, mut texted) = (None, 0.0f32, false);
            let mut j = i + 1;
            while j < nodes.len() && nodes[j].depth > depth {
                if nodes[j].role == Some(kui_native::Role::None) {
                    let d = nodes[j].depth;
                    j += 1;
                    while j < nodes.len() && nodes[j].depth > d {
                        if texted && nodes[j].text.as_ref().is_some_and(|t| !t.is_empty()) {
                            note = note.or(Some(nodes[j].rect.x));
                        }
                        j += 1;
                    }
                    continue;
                }
                if nodes[j].text.is_some() {
                    end = end.max(nodes[j].rect.x + nodes[j].rect.w);
                    texted = true;
                }
                j += 1;
            }
            if let Some(x) = note {
                out.push((x, end));
            }
            i = j;
        }
        out
    }

    /// The numbers in every plain pane's gutter in the last frame,
    /// pane by pane, top to bottom.
    pub fn gutter_texts(&self) -> Vec<String> {
        let nodes = self.core.nodes();
        let mut out = Vec::new();
        for (at, g) in nodes.iter().enumerate() {
            if g.label.as_deref() == Some("gutter") {
                out.extend(
                    nodes[at + 1..]
                        .iter()
                        .take_while(|n| n.depth > g.depth)
                        .filter_map(|n| n.text.clone()),
                );
            }
        }
        out
    }

    fn rows(&self) -> Vec<(String, String)> {
        let nodes = self.core.nodes();
        // A pane's rows are the children of its `lines`, and of the
        // float a tall pane stacks the rows above its caret's in
        // (`lines above`, in the `above` box), in tree order: top down.
        let holders: Vec<_> = nodes
            .iter()
            .filter(|n| matches!(n.label.as_deref(), Some("lines" | "lines above")))
            .map(|n| n.key)
            .collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < nodes.len() {
            let row = nodes[i].parent.is_some_and(|p| holders.contains(&p))
                && nodes[i].label.as_deref() != Some("above");
            if !row {
                i += 1;
                continue;
            }
            let depth = nodes[i].depth;
            let (mut line, mut extra) = (String::new(), String::new());
            let mut j = i + 1;
            while j < nodes.len() && nodes[j].depth > depth {
                if nodes[j].role == Some(kui_native::Role::None) {
                    let d = nodes[j].depth;
                    j += 1;
                    while j < nodes.len() && nodes[j].depth > d {
                        if let Some(t) = &nodes[j].text {
                            extra.push_str(t);
                        }
                        j += 1;
                    }
                    continue;
                }
                if let Some(t) = &nodes[j].text {
                    line.push_str(t);
                }
                j += 1;
            }
            out.push((line, extra));
            i = j;
        }
        out
    }

    /// The text nodes of the confirm float, top to bottom: the title,
    /// the lines, the buttons; none when no confirm is up.
    pub fn confirm_texts(&self) -> Vec<String> {
        let nodes = self.core.nodes();
        let Some(at) = nodes
            .iter()
            .position(|n| n.label.as_deref() == Some("confirm"))
        else {
            return Vec::new();
        };
        nodes[at + 1..]
            .iter()
            .take_while(|n| n.depth > nodes[at].depth)
            .filter_map(|n| n.text.clone())
            .collect()
    }

    /// The text nodes drawn in the notification floats (`notify.rs`) —
    /// the toasts at the top, then the corner — top to bottom; none
    /// when nothing is on show.
    pub fn corner_texts(&self) -> Vec<String> {
        let nodes = self.core.nodes();
        let mut out = Vec::new();
        for label in ["toasts", "corner"] {
            let Some(at) = nodes.iter().position(|n| n.label.as_deref() == Some(label)) else {
                continue;
            };
            // Preorder: the float's subtree is the run after it that is
            // deeper.
            out.extend(
                nodes[at + 1..]
                    .iter()
                    .take_while(|n| n.depth > nodes[at].depth)
                    .filter_map(|n| n.text.clone()),
            );
        }
        out
    }
}

/// Runs the Lua test at `path` headless: a fresh editor with the
/// bundled plugins, the script as a coroutine, its yields the keys and
/// frames. `Err` is the failure — an `assert`'s message with its
/// traceback, a wait that ran out, a kui warning the run raised.
pub fn run_file(path: &Path) -> Result<(), String> {
    // A script's `:cd` moves its editor's tab, not the process
    // (docs/design/workspaces.md), so scripts do not disturb each
    // other.
    run_script(path)
}

/// The view with the keys, for the state published to the script: the
/// focused editor pane's, else the field a Lua pane's keys are on (a
/// picker's query), so `kawoosh.mode()` and `kawoosh.buf` read it.
fn keyed_view(app: &Kawoosh) -> Option<kawoosh_editor::ViewId> {
    app.focused_view().or_else(|| {
        let pane = app.layout.focused();
        let Some(crate::layout::Content::Lua(name)) = app.layout.content(pane) else {
            return None;
        };
        let field = app.scripting.rt.as_ref()?.field_focus(&name)?;
        app.ed.find_field(&field)
    })
}

fn run_script(path: &Path) -> Result<(), String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut app = Kawoosh::new("*scratch*", "");
    app.jobs_inline = true;
    let ext = app.attach_lua()?;
    let mut h = Harness::new(TEST_VIEWPORT.0, TEST_VIEWPORT.1);
    h.extension("lua", ext)?;
    h.frame(&mut app);
    let rt = app.scripting.rt.clone().ok_or("no lua runtime")?;
    rt.start_test(&path.display().to_string(), &src)?;
    loop {
        rt.publish(&app.ed, keyed_view(&app));
        let step = rt.resume_test()?;
        app.drain_lua();
        match step {
            TestStep::Done => break,
            TestStep::Press(keys) => h.press(&mut app, &keys),
            TestStep::Frame(n) => {
                for _ in 0..n.max(1) {
                    h.frame(&mut app);
                }
            }
            TestStep::Sleep(ms) => {
                std::thread::sleep(std::time::Duration::from_millis(ms));
                h.advance(ms as f64 / 1000.0);
                h.frame(&mut app);
            }
        }
    }
    let warnings = h.warnings();
    if !warnings.is_empty() {
        return Err(format!("kui warned: {}", warnings.join("; ")));
    }
    Ok(())
}

/// `kawoosh test PATH…`: each file run, a line per result on stdout,
/// and the exit code — 0 when every one passed.
pub fn run_files(paths: &[String]) -> i32 {
    if paths.is_empty() {
        eprintln!("kawoosh test: no files given");
        return 2;
    }
    let mut failed = 0;
    for p in paths {
        match run_file(Path::new(p)) {
            Ok(()) => println!("ok   {p}"),
            Err(e) => {
                failed += 1;
                println!(
                    "FAIL {p}\n{}",
                    e.lines()
                        .map(|l| format!("     {l}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                );
            }
        }
    }
    println!("{} passed, {failed} failed", paths.len() - failed);
    i32::from(failed > 0)
}
