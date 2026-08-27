//! Application state and the per-frame element tree.
//!
//! [`App`] holds document state (`kawoosh_core`), the view list, and builds
//! the immediate-mode UI as data ([`App::frame`]). Views follow the neovim
//! model generalized (mvp.md Decision 5): a global list of views — editors
//! and terminals — that outlive their panes; the active one fills the pane.
//! `App` knows nothing about SDL; the binary feeds it events and paints what
//! `frame` describes.

use std::collections::HashMap;
use std::sync::Arc;

use kawoosh_core::{
    BufferId, Core, Highlight, HighlightId, HighlightStyle, LayerId, LayerSpec, Rgba, Version,
};
use kawoosh_systems::{lsp, ts};
use kawoosh_term::Terminal;
use kawoosh_ui::{Edges, Element, Size};

use crate::editor::{Mode, Selection, UndoEntry};
use crate::keys::{Key, KeyPress};

// Theme, deliberately close to the reference screenshots.
pub const BG: [u8; 4] = [0x21, 0x21, 0x21, 0xFF];
pub const FG: [u8; 4] = [0xE6, 0xE6, 0xE6, 0xFF];
pub const BAR_BG: [u8; 4] = [0x2B, 0x2B, 0x2B, 0xFF];
pub const BAR_ACTIVE: [u8; 4] = [0x1A, 0x53, 0xC7, 0xFF];
pub const DIM: [u8; 4] = [0x9A, 0x9A, 0x9A, 0xFF];

/// Custom-element ids for the view leaves.
pub const EDITOR_VIEW: u64 = 1;
pub const TERMINAL_VIEW: u64 = 2;

pub struct EditorState {
    pub buffer: BufferId,
    pub title: String,
    /// Backing file, when there is one; save targets it.
    pub path: Option<std::path::PathBuf>,
    pub language: Option<ts::Language>,
    /// A highlight job for this buffer is in flight.
    pub syntax_pending: bool,
    /// Buffer version last synced to the lsp system.
    pub lsp_synced: Option<Version>,
    pub mode: Mode,
    /// First visible line.
    pub scroll: usize,
    pub selections: Vec<Selection>,
    /// One register per selection; paste cycles when counts differ.
    pub registers: Vec<Vec<u8>>,
    pub undo: Vec<UndoEntry>,
    pub redo: Vec<UndoEntry>,
    /// First key of a pending multi-key sequence (`g`, `d`).
    pub pending: Option<char>,
}

impl EditorState {
    fn new(buffer: BufferId, title: impl Into<String>, path: Option<std::path::PathBuf>) -> Self {
        let language = path.as_deref().and_then(ts::Language::detect);
        Self {
            buffer,
            title: title.into(),
            path,
            language,
            syntax_pending: false,
            lsp_synced: None,
            mode: Mode::default(),
            scroll: 0,
            selections: vec![Selection::caret(0)],
            registers: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            pending: None,
        }
    }
}

pub struct TerminalView {
    pub id: usize,
    pub terminal: Terminal,
    pub title: String,
}

pub enum View {
    Editor(EditorState),
    Terminal(TerminalView),
}

impl View {
    pub fn title(&self) -> &str {
        match self {
            View::Editor(ed) => &ed.title,
            View::Terminal(term) => &term.title,
        }
    }
}

/// The syntax theme: one derived layer plus capture-name → highlight ids.
pub struct Syntax {
    pub layer: LayerId,
    pub map: Arc<HashMap<String, HighlightId>>,
}

fn fg(color: u32) -> HighlightStyle {
    HighlightStyle {
        fg: Some(Rgba(0xFF00_0000 | color)),
        ..HighlightStyle::default()
    }
}

/// Effects the app asks the shell to perform (async, system-bound).
pub enum Effect {
    GotoDefinition,
}

fn underline(color: u32) -> HighlightStyle {
    HighlightStyle {
        underline: Some(Rgba(0xFF00_0000 | color)),
        ..HighlightStyle::default()
    }
}

fn build_diagnostics(core: &mut Core) -> lsp::DiagTheme {
    // Above syntax; squiggle colors only, so syntax fg shows through.
    let layer = core.create_layer(LayerSpec::derived("diagnostics").with_z(20));
    lsp::DiagTheme {
        layer,
        error: core.create_highlight(Highlight::styled(underline(0xD54E53))),
        warning: core.create_highlight(Highlight::styled(underline(0xE7C547))),
        info: core.create_highlight(Highlight::styled(underline(0x7AA6DA))),
    }
}

fn build_syntax(core: &mut Core) -> Syntax {
    let layer = core.create_layer(LayerSpec::derived("syntax").with_z(10));

    let palette: &[(&str, u32)] = &[
        ("keyword", 0xC397D8),
        ("function", 0x7AA6DA),
        ("string", 0xB9CA4A),
        ("comment", 0x969896),
        ("type", 0xE7C547),
        ("constant", 0xE78C45),
        ("number", 0xE78C45),
        ("property", 0xDE935F),
        ("attribute", 0xDE935F),
        ("constructor", 0xE7C547),
        ("escape", 0x70C0B1),
        ("label", 0x70C0B1),
        ("operator", 0xC5C8C6),
        ("punctuation", 0x8F8F8F),
    ];

    let map = palette
        .iter()
        .map(|(name, color)| {
            (name.to_string(), core.create_highlight(Highlight::styled(fg(*color))))
        })
        .collect();

    Syntax { layer, map: Arc::new(map) }
}

pub struct App {
    pub core: Core,
    pub views: Vec<View>,
    pub active: usize,
    pub syntax: Syntax,
    pub diagnostics: lsp::DiagTheme,
    /// The workspace root (mvp.md Decision 7b; explicit workspaces later).
    pub root: std::path::PathBuf,
    /// Async requests for the shell to route to systems.
    pub effects: Vec<Effect>,
    next_terminal_id: usize,
}

impl App {
    pub fn open(title: impl Into<String>, bytes: &[u8]) -> Self {
        let mut core = Core::default();
        let syntax = build_syntax(&mut core);
        let diagnostics = build_diagnostics(&mut core);
        let mut app = Self {
            core,
            views: Vec::new(),
            active: 0,
            syntax,
            diagnostics,
            root: std::env::current_dir().unwrap_or_else(|_| "/".into()),
            effects: Vec::new(),
            next_terminal_id: 0,
        };
        app.open_editor(title, bytes, None);
        app
    }

    /// Full-text sync commands for every rust buffer the lsp pool has not
    /// seen at its current version.
    pub fn lsp_jobs(&mut self) -> Vec<lsp::Cmd> {
        let root = self.root.clone();
        let mut jobs = Vec::new();
        for view in &mut self.views {
            let View::Editor(ed) = view else { continue };
            if ed.language != Some(ts::Language::Rust) {
                continue;
            }
            let Some(path) = ed.path.clone() else { continue };
            let Some(buf) = self.core.buffer(ed.buffer) else { continue };
            let version = buf.version();
            if ed.lsp_synced == Some(version) {
                continue;
            }
            ed.lsp_synced = Some(version);
            let mut text = Vec::with_capacity(buf.len());
            buf.read_into(0..buf.len(), &mut text);
            let path = if path.is_absolute() { path } else { root.join(&path) };
            jobs.push(lsp::Cmd::Sync {
                buffer: ed.buffer,
                root: root.clone(),
                path,
                version,
                text,
            });
        }
        jobs
    }

    /// Apply an lsp event; returns whether the visible view changed.
    pub fn apply_lsp(&mut self, event: lsp::Event) -> bool {
        match event {
            lsp::Event::Diagnostics { buffer, update } => {
                let _ = self.core.apply(buffer, update);
                matches!(self.active_view(), View::Editor(ed) if ed.buffer == buffer)
            }
            lsp::Event::Definition { path, line, character } => {
                let bytes = std::fs::read(&path).unwrap_or_default();
                let offset = lsp::offset_of_position(&bytes, line, character);

                // Reuse an existing view of the same file if there is one.
                let existing = self.views.iter().position(|view| {
                    matches!(view, View::Editor(ed) if ed.path.as_deref() == Some(path.as_path()))
                });
                match existing {
                    Some(index) => self.active = index,
                    None => {
                        let title = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.to_string_lossy().into_owned());
                        self.open_editor(title, &bytes, Some(path));
                    }
                }
                if let Some(ed) = self.active_editor_mut() {
                    ed.selections = vec![Selection::caret(offset)];
                }
                true
            }
        }
    }

    /// Highlight jobs for every damaged, language-bearing editor buffer with
    /// no job already in flight.
    pub fn syntax_jobs(&mut self) -> Vec<ts::Job> {
        let layer = self.syntax.layer;
        let map = Arc::clone(&self.syntax.map);
        let mut jobs = Vec::new();
        for view in &mut self.views {
            let View::Editor(ed) = view else { continue };
            let Some(language) = ed.language else { continue };
            if ed.syntax_pending {
                continue;
            }
            let Some(buf) = self.core.buffer(ed.buffer) else {
                continue;
            };
            if buf.damage(layer).is_empty() {
                continue;
            }
            ed.syntax_pending = true;
            jobs.push(ts::Job {
                buffer: ed.buffer,
                layer,
                language,
                snapshot: buf.snapshot(),
                map: Arc::clone(&map),
            });
        }
        jobs
    }

    /// Apply a worker result; returns whether the active view shows it.
    pub fn apply_syntax(&mut self, result: ts::Result_) -> bool {
        for view in &mut self.views {
            if let View::Editor(ed) = view
                && ed.buffer == result.buffer
            {
                ed.syntax_pending = false;
            }
        }
        let _ = self.core.apply(result.buffer, result.update);
        matches!(self.active_view(), View::Editor(ed) if ed.buffer == result.buffer)
    }

    /// Create an editor view over fresh buffer contents and activate it.
    pub fn open_editor(
        &mut self,
        title: impl Into<String>,
        bytes: &[u8],
        path: Option<std::path::PathBuf>,
    ) -> BufferId {
        let buffer = self.core.create_buffer();
        self.core.set_text(buffer, bytes);
        self.views
            .push(View::Editor(EditorState::new(buffer, title, path)));
        self.active = self.views.len() - 1;
        buffer
    }

    /// Write the active editor's buffer back to its file, if it has one.
    pub fn save_active(&mut self) -> bool {
        let Some(ed) = self.active_editor() else {
            return false;
        };
        let Some(path) = ed.path.clone() else {
            return false;
        };
        let Some(buf) = self.core.buffer(ed.buffer) else {
            return false;
        };
        let mut out = Vec::with_capacity(buf.len());
        buf.read_into(0..buf.len(), &mut out);
        std::fs::write(path, out).is_ok()
    }

    /// Adopt a spawned terminal as a view and activate it.
    pub fn open_terminal(&mut self, terminal: Terminal) -> usize {
        let id = self.next_terminal_id;
        self.next_terminal_id += 1;
        self.views.push(View::Terminal(TerminalView {
            id,
            terminal,
            title: format!("term {}", id + 1),
        }));
        self.active = self.views.len() - 1;
        id
    }

    /// Close the active view (never the last one). The removed view is
    /// returned so the shell can resolve `--wait` clients watching it.
    pub fn close_active_view(&mut self) -> Option<View> {
        if self.views.len() <= 1 {
            return None;
        }
        let view = self.views.remove(self.active);
        if self.active >= self.views.len() {
            self.active = self.views.len() - 1;
        }
        if let View::Editor(ed) = &view {
            self.core.remove_buffer(ed.buffer);
        }
        Some(view)
    }

    pub fn cycle_view(&mut self, delta: isize) -> bool {
        if self.views.len() < 2 {
            return false;
        }
        let n = self.views.len() as isize;
        self.active = ((self.active as isize + delta).rem_euclid(n)) as usize;
        true
    }

    pub fn active_view(&self) -> &View {
        &self.views[self.active]
    }

    pub fn active_editor(&self) -> Option<&EditorState> {
        match self.active_view() {
            View::Editor(ed) => Some(ed),
            _ => None,
        }
    }

    pub fn active_editor_mut(&mut self) -> Option<&mut EditorState> {
        match &mut self.views[self.active] {
            View::Editor(ed) => Some(ed),
            _ => None,
        }
    }

    pub fn active_terminal_mut(&mut self) -> Option<&mut TerminalView> {
        match &mut self.views[self.active] {
            View::Terminal(term) => Some(term),
            _ => None,
        }
    }

    pub fn terminal_by_id_mut(&mut self, id: usize) -> Option<&mut TerminalView> {
        self.views.iter_mut().find_map(|view| match view {
            View::Terminal(term) if term.id == id => Some(term),
            _ => None,
        })
    }

    /// Materialize the active terminal's scrollback into an editor view
    /// (mvp.md Decision 3: copy-mode is a buffer, not a mode).
    pub fn scrollback_to_buffer(&mut self) -> bool {
        let Some(term) = self.active_terminal_mut() else {
            return false;
        };
        let title = format!("[{}]", term.title);
        let text = term.terminal.scrollback_text();
        self.open_editor(title, &text, None);
        true
    }

    pub fn line_count(&self) -> usize {
        self.active_editor()
            .and_then(|ed| self.core.buffer(ed.buffer))
            .map_or(1, |b| b.line_count())
    }

    pub fn scroll_by(&mut self, delta: isize) -> bool {
        let max = self.line_count().saturating_sub(1);
        let Some(ed) = self.active_editor_mut() else {
            return false;
        };
        let target = ed.scroll.saturating_add_signed(delta).min(max);
        let moved = target != ed.scroll;
        ed.scroll = target;
        moved
    }

    pub fn scroll_to(&mut self, line: usize) -> bool {
        let max = self.line_count().saturating_sub(1);
        let Some(ed) = self.active_editor_mut() else {
            return false;
        };
        let target = line.min(max);
        let moved = target != ed.scroll;
        ed.scroll = target;
        moved
    }

    /// Scroll so the primary head's line is visible in a `rows`-tall view.
    pub fn ensure_visible(&mut self, rows: usize) -> bool {
        let Some(ed) = self.active_editor() else {
            return false;
        };
        let Some(primary) = ed.selections.first() else {
            return false;
        };
        let Some(buf) = self.core.buffer(ed.buffer) else {
            return false;
        };
        let line = buf.line_of_offset(primary.head.min(buf.len()));
        let rows = rows.max(1);
        let scroll = ed.scroll;

        let target = if line < scroll {
            line
        } else if line >= scroll + rows {
            line + 1 - rows
        } else {
            return false;
        };
        if let Some(ed) = self.active_editor_mut() {
            ed.scroll = target;
        }
        true
    }

    /// Whether the platform text-input path should be running.
    pub fn wants_text_input(&self) -> bool {
        match self.active_view() {
            View::Editor(ed) => ed.mode == Mode::Insert,
            View::Terminal(_) => true,
        }
    }

    /// Route text (never keycodes) to the active view.
    pub fn handle_text(&mut self, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        match &mut self.views[self.active] {
            View::Editor(_) => self.handle_editor_text(text),
            View::Terminal(term) => {
                term.terminal.input(text.as_bytes());
                false
            }
        }
    }

    /// Route a key press to the active view.
    pub fn handle_key(&mut self, kp: KeyPress) -> bool {
        match &mut self.views[self.active] {
            View::Editor(_) => self.handle_editor_key(kp),
            View::Terminal(term) => {
                if let Some(bytes) = encode_terminal_key(kp) {
                    term.terminal.input(&bytes);
                }
                false
            }
        }
    }

    /// Build this frame's element tree. `scale` converts pt-ish constants to
    /// device pixels; `line_height` sizes the bars to match the text engine.
    pub fn frame(&self, scale: f32, line_height: f32) -> Element {
        let bar = line_height + 4.0 * scale;
        let pad = Edges::xy(8.0 * scale, 2.0 * scale);

        // One tab per view; the active one gets the accent.
        let tabs: Vec<Element> = self
            .views
            .iter()
            .enumerate()
            .map(|(i, view)| {
                let active = i == self.active;
                let color = if active { FG } else { DIM };
                let tab = Element::row(vec![Element::text(view.title().to_string(), color)])
                    .padding(pad);
                if active { tab.bg(BAR_ACTIVE) } else { tab }
            })
            .collect();
        let title_bar = Element::row(tabs)
            .width(Size::Grow(1.0))
            .height(Size::Fixed(bar))
            .bg(BAR_BG);

        let (view_leaf, status_left, status_right) = match self.active_view() {
            View::Editor(ed) => {
                let mode = if ed.selections.len() > 1 {
                    format!("{} ×{}", ed.mode.label(), ed.selections.len())
                } else {
                    ed.mode.label().to_string()
                };
                let line = ed
                    .selections
                    .first()
                    .and_then(|sel| {
                        let buf = self.core.buffer(ed.buffer)?;
                        Some(buf.line_of_offset(sel.head.min(buf.len())) + 1)
                    })
                    .unwrap_or(1);
                (
                    Element::custom(EDITOR_VIEW, Size::Grow(1.0), Size::Grow(1.0)),
                    mode,
                    format!("{}/{}", line, self.line_count()),
                )
            }
            View::Terminal(term) => (
                Element::custom(TERMINAL_VIEW, Size::Grow(1.0), Size::Grow(1.0)),
                "TERM".to_string(),
                format!("{}×{}", term.terminal.size().cols, term.terminal.size().rows),
            ),
        };

        let status = Element::row(vec![
            Element::text(status_left, FG),
            Element::text(self.active_view().title().to_string(), DIM),
            Element::spacer(),
            Element::text(status_right, DIM),
        ])
        .gap(12.0 * scale)
        .width(Size::Grow(1.0))
        .height(Size::Fixed(bar))
        .padding(pad)
        .bg(BAR_BG);

        Element::col(vec![title_bar, view_leaf, status])
    }
}

/// Encode a non-text key for the pty. Printable characters arrive through
/// the text-input path instead, so they are deliberately not handled here.
pub fn encode_terminal_key(kp: KeyPress) -> Option<Vec<u8>> {
    if kp.mods.ctrl {
        if let Key::Char(c) = kp.key
            && c.is_ascii_alphabetic()
        {
            return Some(vec![(c.to_ascii_uppercase() as u8) & 0x1F]);
        }
        return None;
    }

    Some(match kp.key {
        Key::Enter => b"\r".to_vec(),
        Key::Esc => b"\x1b".to_vec(),
        Key::Backspace => b"\x7f".to_vec(),
        Key::Tab => b"\t".to_vec(),
        Key::Up => b"\x1b[A".to_vec(),
        Key::Down => b"\x1b[B".to_vec(),
        Key::Right => b"\x1b[C".to_vec(),
        Key::Left => b"\x1b[D".to_vec(),
        Key::Home => b"\x1b[H".to_vec(),
        Key::End => b"\x1b[F".to_vec(),
        Key::PageUp => b"\x1b[5~".to_vec(),
        Key::PageDown => b"\x1b[6~".to_vec(),
        Key::Delete => b"\x1b[3~".to_vec(),
        Key::Char(_) => return None,
    })
}
