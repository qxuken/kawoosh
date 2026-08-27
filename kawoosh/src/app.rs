//! Application state and the per-frame element tree.
//!
//! [`App`] holds document state (`kawoosh_core`) and view state, and builds
//! the immediate-mode UI as data ([`App::frame`]). It knows nothing about
//! SDL; the binary feeds it events and paints what `frame` describes.

use kawoosh_core::{BufferId, Core};
use kawoosh_ui::{Edges, Element, Size};

use crate::editor::{Mode, Selection, UndoEntry};

// Theme, deliberately close to the reference screenshots.
pub const BG: [u8; 4] = [0x21, 0x21, 0x21, 0xFF];
pub const FG: [u8; 4] = [0xE6, 0xE6, 0xE6, 0xFF];
pub const BAR_BG: [u8; 4] = [0x2B, 0x2B, 0x2B, 0xFF];
pub const BAR_ACTIVE: [u8; 4] = [0x1A, 0x53, 0xC7, 0xFF];
pub const DIM: [u8; 4] = [0x9A, 0x9A, 0x9A, 0xFF];

/// Custom-element id for the editor view leaf.
pub const EDITOR_VIEW: u64 = 1;

pub struct App {
    pub core: Core,
    pub buffer: BufferId,
    /// First visible line of the editor view.
    pub scroll: usize,
    pub title: String,

    pub mode: Mode,
    pub selections: Vec<Selection>,
    /// One register per selection; paste cycles when counts differ.
    pub registers: Vec<Vec<u8>>,
    pub undo: Vec<UndoEntry>,
    pub redo: Vec<UndoEntry>,
    /// First key of a pending multi-key sequence (`g`, `d`).
    pub pending: Option<char>,
}

impl App {
    pub fn open(title: impl Into<String>, bytes: &[u8]) -> Self {
        let mut core = Core::default();
        let buffer = core.create_buffer();
        core.set_text(buffer, bytes);
        Self {
            core,
            buffer,
            scroll: 0,
            title: title.into(),
            mode: Mode::default(),
            selections: vec![Selection::caret(0)],
            registers: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            pending: None,
        }
    }

    /// Scroll so the primary head's line is visible in a `rows`-tall view.
    pub fn ensure_visible(&mut self, rows: usize) -> bool {
        let Some(primary) = self.selections.first() else {
            return false;
        };
        let buf = self.core.buffer(self.buffer).expect("app buffer exists");
        let line = buf.line_of_offset(primary.head.min(buf.len()));
        let rows = rows.max(1);

        let target = if line < self.scroll {
            line
        } else if line >= self.scroll + rows {
            line + 1 - rows
        } else {
            return false;
        };
        self.scroll = target;
        true
    }

    pub fn line_count(&self) -> usize {
        self.core.buffer(self.buffer).map_or(1, |b| b.line_count())
    }

    pub fn scroll_by(&mut self, delta: isize) -> bool {
        let max = self.line_count().saturating_sub(1);
        let target = self.scroll.saturating_add_signed(delta).min(max);
        let moved = target != self.scroll;
        self.scroll = target;
        moved
    }

    pub fn scroll_to(&mut self, line: usize) -> bool {
        let target = line.min(self.line_count().saturating_sub(1));
        let moved = target != self.scroll;
        self.scroll = target;
        moved
    }

    /// Build this frame's element tree. `scale` converts pt-ish constants to
    /// device pixels; `line_height` sizes the bars to match the text engine.
    pub fn frame(&self, scale: f32, line_height: f32) -> Element {
        let bar = line_height + 4.0 * scale;
        let pad = Edges::xy(8.0 * scale, 2.0 * scale);

        // A fit-width active tab sitting in a full-width bar, as in the
        // reference screenshots.
        let tab = Element::row(vec![Element::text(self.title.clone(), FG)])
            .padding(pad)
            .bg(BAR_ACTIVE);
        let title_bar = Element::row(vec![tab])
            .width(Size::Grow(1.0))
            .height(Size::Fixed(bar))
            .bg(BAR_BG);

        let editor = Element::custom(EDITOR_VIEW, Size::Grow(1.0), Size::Grow(1.0));

        let mode = if self.selections.len() > 1 {
            format!("{} ×{}", self.mode.label(), self.selections.len())
        } else {
            self.mode.label().to_string()
        };
        let line = self
            .selections
            .first()
            .map(|sel| {
                let buf = self.core.buffer(self.buffer).expect("app buffer exists");
                buf.line_of_offset(sel.head.min(buf.len())) + 1
            })
            .unwrap_or(1);
        let position = format!("{}/{}", line, self.line_count());
        let status = Element::row(vec![
            Element::text(mode, FG),
            Element::text(self.title.clone(), DIM),
            Element::spacer(),
            Element::text(position, DIM),
        ])
        .gap(12.0 * scale)
        .width(Size::Grow(1.0))
        .height(Size::Fixed(bar))
        .padding(pad)
        .bg(BAR_BG);

        Element::col(vec![title_bar, editor, status])
    }
}
