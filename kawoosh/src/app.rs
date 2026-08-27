//! Application state and the per-frame element tree.
//!
//! [`App`] holds document state (`kawoosh_core`), the view list, and builds
//! the immediate-mode UI as data ([`App::frame`]). Views follow the neovim
//! model generalized (mvp.md Decision 5): a global list of views — editors
//! and terminals — that outlive their panes; the active one fills the pane.
//! `App` knows nothing about SDL; the binary feeds it events and paints what
//! `frame` describes.

use kawoosh_core::{BufferId, Core};
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
    fn new(buffer: BufferId, title: impl Into<String>) -> Self {
        Self {
            buffer,
            title: title.into(),
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

pub struct App {
    pub core: Core,
    pub views: Vec<View>,
    pub active: usize,
    next_terminal_id: usize,
}

impl App {
    pub fn open(title: impl Into<String>, bytes: &[u8]) -> Self {
        let mut app = Self {
            core: Core::default(),
            views: Vec::new(),
            active: 0,
            next_terminal_id: 0,
        };
        app.open_editor(title, bytes);
        app
    }

    /// Create an editor view over fresh buffer contents and activate it.
    pub fn open_editor(&mut self, title: impl Into<String>, bytes: &[u8]) -> BufferId {
        let buffer = self.core.create_buffer();
        self.core.set_text(buffer, bytes);
        self.views.push(View::Editor(EditorState::new(buffer, title)));
        self.active = self.views.len() - 1;
        buffer
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
        self.open_editor(title, &text);
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
