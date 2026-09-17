//! Kawoosh's headless harness is kui's `Core` (kui.md D8): a frame is a
//! function of the tree and the input so far, so a test presses keys
//! through the real `App::view` / `on_event` and asserts on the model
//! *and* on the drawn rows. This mirrors `kui-devtools`'s `Drive` (not a
//! published crate) in the hundred lines kawoosh needs.

#![allow(dead_code)]

use kui::{
    App, Core, Extension, Extensions, InputEvent, KeyCode, KeyMods, KeyPress, Size, UiEvent, Vec2,
};

pub struct Drive {
    pub core: Core,
    viewport: Size,
    now: f64,
    pub frames: u64,
    /// The extensions filling the frame's slots — the Lua runtime, when
    /// a test attaches it — routed the way the runner routes them.
    pub exts: Extensions,
}

impl Drive {
    pub fn new(w: f32, h: f32) -> Self {
        let mut core = Core::new();
        core.set_diagnostics(true);
        core.set_inspect(true);
        Self {
            core,
            viewport: Size::new(w, h),
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
        let mut ui = self.core.frame_with(self.viewport, 1.0, &mut self.exts);
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

    pub fn ctrl(&mut self, app: &mut impl App, name: &str) {
        self.key(
            app,
            name,
            KeyMods {
                ctrl: true,
                ..Default::default()
            },
        );
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

    pub fn click(&mut self, app: &mut impl App, x: f32, y: f32) {
        self.input(app, InputEvent::CursorMoved(Vec2::new(x, y)));
        self.input(app, InputEvent::mouse_down(1));
        self.input(app, InputEvent::mouse_up());
        self.frame(app);
    }

    /// The text of every row under every node labelled `lines` in the
    /// last frame, pane by pane, top to bottom, runs joined — one
    /// document line each.
    pub fn line_rows(&self) -> Vec<String> {
        let nodes = self.core.nodes();
        let mut out = Vec::new();
        for lines in nodes.iter().filter(|n| n.label.as_deref() == Some("lines")) {
            let mut i = 0;
            while i < nodes.len() {
                if nodes[i].parent == Some(lines.key) {
                    let depth = nodes[i].depth;
                    let mut s = String::new();
                    let mut j = i + 1;
                    while j < nodes.len() && nodes[j].depth > depth {
                        if let Some(t) = &nodes[j].text {
                            s.push_str(t);
                        }
                        j += 1;
                    }
                    out.push(s);
                    i = j;
                } else {
                    i += 1;
                }
            }
        }
        out
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
