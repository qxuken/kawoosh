//! Kawoosh's headless harness is kui's `Core` (kui.md D8): a frame is a
//! function of the tree and the input so far, so a test presses keys
//! through the real `App::view` / `on_event` and asserts on the model
//! *and* on the drawn rows. This mirrors `kui-devtools`'s `Drive` (not a
//! published crate) in the hundred lines kawoosh needs. The Rust tests
//! take it as `Drive` (`kawoosh/tests/drive.rs`), and `kawoosh test
//! PATH…` (roadmap step 8) drives a Lua script with it: the script runs
//! as a coroutine, `kawoosh.press(keys)` yields the keys to press,
//! `kawoosh.frame(n)` frames to draw and `kawoosh.wait(fn)` frames until
//! a reading holds, and between resumes the editor is published to Lua,
//! so `kawoosh.buf.text()`, `kawoosh.mode()`, `kawoosh.message()` and
//! `kawoosh.picker.state()` read the state as it is. A plain `assert`
//! or `error` fails the run with its traceback. The bundled plugins'
//! Lua tests (`kawoosh/lua/tests/*.lua`) run on it, from `cargo test`
//! and from the CLI alike.

use std::path::Path;

use kawoosh_editor::keymap::{LEADER, parse_notation};
use kawoosh_lua::TestStep;
use kui_native::{
    App, Core, Extension, Extensions, InputEvent, KeyCode, KeyMods, KeyPress, Size, UiEvent, Vec2,
};

use crate::app::Kawoosh;

/// The window a Lua test is drawn in.
const TEST_VIEWPORT: (f32, f32) = (1000.0, 700.0);

pub struct Harness {
    pub core: Core,
    viewport: Size,
    /// The display's scale, 1 unless a test sets it: at 1.75 or 2.175 a
    /// 20 px line is not whole physical pixels, where joins go wrong.
    pub scale: f32,
    now: f64,
    pub frames: u64,
    /// The extensions filling the frame's slots — the Lua runtime, when
    /// a test attaches it — routed the way the runner routes them.
    pub exts: Extensions,
}

impl Harness {
    pub fn new(w: f32, h: f32) -> Self {
        let mut core = Core::new();
        core.set_diagnostics(true);
        core.set_inspect(true);
        Self {
            core,
            viewport: Size::new(w, h),
            scale: 1.0,
            now: 0.0,
            frames: 0,
            exts: Extensions::new(),
        }
    }

    /// Loads an extension under `ns`, as `Launcher::extension_as` would.
    pub fn extension(&mut self, ns: &str, ext: impl Extension + 'static) {
        self.exts
            .push_as(ns, Box::new(ext))
            .expect("a free namespace");
    }

    pub fn frame(&mut self, app: &mut impl App) {
        self.frames += 1;
        self.core.set_time(self.now);
        let mut ui = self
            .core
            .frame_with(self.viewport, self.scale, &mut self.exts);
        app.view(&mut ui);
        ui.finish();
        let pending = self.core.take_pending_events();
        self.exts.route(pending, |ev| app.on_event(ev));
    }

    pub fn advance(&mut self, secs: f64) {
        self.now += secs;
    }

    pub fn input(&mut self, app: &mut impl App, ev: InputEvent) -> Vec<UiEvent> {
        let out = self.core.handle_input(ev);
        self.exts.route(out.clone(), |ev| app.on_event(ev));
        out
    }

    /// A key by the name a binding spells it, both channels, then the
    /// release, then a frame.
    pub fn key(&mut self, app: &mut impl App, name: &str, mods: KeyMods) {
        let code = KeyCode::from_name(name).unwrap_or(KeyCode::Unknown);
        let mut press = KeyPress::new(code, mods);
        if let KeyCode::Char(c) = code
            && !mods.ctrl
            && !mods.alt
            && !mods.super_key
        {
            press = press.with_text(c.to_string());
        }
        self.input(app, InputEvent::KeyDown(press.clone()));
        if let Some(ev) = press.edit_event() {
            self.input(app, ev);
        }
        self.input(app, InputEvent::KeyUp(press.released()));
        self.frame(app);
    }

    /// `keys(app, "jjj ww")`: one plain key per character, a space being
    /// the space key.
    pub fn keys(&mut self, app: &mut impl App, seq: &str) {
        for c in seq.chars() {
            if c == ' ' {
                let press = KeyPress::new(KeyCode::Space, KeyMods::default()).with_text(" ");
                self.input(app, InputEvent::KeyDown(press.clone()));
                self.input(app, InputEvent::KeyUp(press.released()));
                self.frame(app);
            } else {
                self.key(app, &c.to_string(), KeyMods::default());
            }
        }
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
                Some("space") => {
                    let press = KeyPress::new(KeyCode::Space, mods).with_text(" ");
                    self.input(app, InputEvent::KeyDown(press.clone()));
                    self.input(app, InputEvent::KeyUp(press.released()));
                    self.frame(app);
                    continue;
                }
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

    pub fn ctrl(&mut self, app: &mut impl App, name: &str) {
        self.key(app, name, KeyMods::NONE.with_ctrl());
    }

    /// An IME commit or the clipboard's answer: text that did not come
    /// from a key press, as the OS delivers it to a key sink.
    pub fn text(&mut self, app: &mut impl App, s: &str) {
        self.input(app, InputEvent::Commit(s.to_string()));
        self.frame(app);
    }

    pub fn wheel(&mut self, app: &mut impl App, x: f32, y: f32, dx: f32, dy: f32) {
        self.input(app, InputEvent::CursorMoved(Vec2::new(x, y)));
        self.input(app, InputEvent::Scroll(Vec2::new(dx, dy)));
        self.frame(app);
    }

    /// The pointer moved to `x`, `y`, and a frame.
    pub fn hover(&mut self, app: &mut impl App, x: f32, y: f32) {
        self.input(app, InputEvent::CursorMoved(Vec2::new(x, y)));
        self.frame(app);
    }

    pub fn click(&mut self, app: &mut impl App, x: f32, y: f32) {
        self.input(app, InputEvent::CursorMoved(Vec2::new(x, y)));
        self.input(app, InputEvent::mouse_down(1));
        self.input(app, InputEvent::mouse_up());
        self.frame(app);
    }

    /// Two clicks at one point, the second counted as the second, as the
    /// OS counts a double click into the press.
    pub fn double_click(&mut self, app: &mut impl App, x: f32, y: f32) {
        self.click(app, x, y);
        self.input(app, InputEvent::mouse_down(2));
        self.input(app, InputEvent::mouse_up());
        self.frame(app);
    }

    /// A press at one point, the pointer taken to another in two steps
    /// (past kui's click slop, so the node's `on_drag` reports moves and
    /// the release is no click), a frame drawn while held, the release.
    pub fn drag(&mut self, app: &mut impl App, from: (f32, f32), to: (f32, f32)) {
        self.input(app, InputEvent::CursorMoved(Vec2::new(from.0, from.1)));
        self.input(app, InputEvent::mouse_down(1));
        self.input(
            app,
            InputEvent::CursorMoved(Vec2::new(from.0 + 8.0, from.1 + 8.0)),
        );
        self.input(app, InputEvent::CursorMoved(Vec2::new(to.0, to.1)));
        self.frame(app);
        self.input(app, InputEvent::mouse_up());
        self.frame(app);
    }

    /// Where the node labelled `label` was drawn last frame, as (x, y,
    /// w, h); the first one when several carry the label.
    pub fn rect_of(&self, label: &str) -> Option<(f32, f32, f32, f32)> {
        self.core
            .nodes()
            .iter()
            .find(|n| n.label.as_deref() == Some(label))
            .map(|n| (n.rect.x, n.rect.y, n.rect.w, n.rect.h))
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
        let mut out = Vec::new();
        for lines in nodes.iter().filter(|n| n.label.as_deref() == Some("lines")) {
            let mut i = 0;
            while i < nodes.len() {
                if nodes[i].parent == Some(lines.key) {
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
                } else {
                    i += 1;
                }
            }
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

    /// Every warning the core raised so far; a test asserts it empty.
    pub fn warnings(&mut self) -> Vec<String> {
        self.core
            .take_warnings()
            .into_iter()
            .map(|w| format!("{}: {}", w.code, w.message))
            .collect()
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
    h.extension("lua", ext);
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
