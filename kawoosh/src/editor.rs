//! Modal, selection-first editing (docs/design/mvp.md, Decision 4).
//!
//! Every editing command operates on the whole selection set; a single caret
//! is the size-1 case. Multi-selection edits apply in ascending order and
//! ride the journal: each pending or already-placed cursor is carried across
//! intervening version bumps with `transform_offset`, so cursor #37 lands
//! correctly after edits #1-36 shifted the text. One `Checkpoint` per command
//! batch (and per insert session) makes multi-edits atomic under undo —
//! checkpoints are O(1) retained roots, so this is cheap by construction.

use std::ops::Range;

use kawoosh_core::{Bias, Buffer, Checkpoint, Version};

use crate::app::App;
use crate::keys::{Key, KeyPress};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Normal,
    Insert,
    Visual,
}

impl Mode {
    pub fn label(&self) -> &'static str {
        match self {
            Mode::Normal => "NOR",
            Mode::Insert => "INS",
            Mode::Visual => "VIS",
        }
    }
}

/// One selection: `anchor..head`, exclusive, either direction. A caret has
/// `anchor == head`. `goal` is the byte column vertical movement aims for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
    pub goal: Option<usize>,
}

impl Selection {
    pub fn caret(pos: usize) -> Self {
        Self { anchor: pos, head: pos, goal: None }
    }

    pub fn is_caret(&self) -> bool {
        self.anchor == self.head
    }

    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }
}

pub struct UndoEntry {
    pub checkpoint: Checkpoint,
    pub selections: Vec<Selection>,
    /// Version when the entry was pushed; a no-op batch is popped again.
    pub version: Version,
}

// -- byte/char helpers -------------------------------------------------------

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn byte_at(buf: &Buffer, off: usize) -> Option<u8> {
    buf.text().byte_at(off)
}

/// Next UTF-8 boundary after `off` (clamped to len).
fn next_char(buf: &Buffer, off: usize) -> usize {
    let len = buf.len();
    if off >= len {
        return len;
    }
    let mut next = off + 1;
    while next < len {
        match byte_at(buf, next) {
            Some(b) if b & 0xC0 == 0x80 => next += 1,
            _ => break,
        }
    }
    next
}

/// Previous UTF-8 boundary before `off`.
fn prev_char(buf: &Buffer, off: usize) -> usize {
    if off == 0 {
        return 0;
    }
    let mut prev = off - 1;
    while prev > 0 {
        match byte_at(buf, prev) {
            Some(b) if b & 0xC0 == 0x80 => prev -= 1,
            _ => break,
        }
    }
    prev
}

fn line_bounds(buf: &Buffer, off: usize) -> (usize, Range<usize>) {
    let line = buf.line_of_offset(off.min(buf.len()));
    let range = buf.line_range(line).unwrap_or(0..0);
    (line, range)
}

impl App {
    fn buf(&self) -> &Buffer {
        self.core.buffer(self.buffer).expect("app buffer exists")
    }

    // -- selection maintenance ----------------------------------------------

    /// Sort by position, merge overlapping ranges, dedupe coincident carets.
    pub fn normalize_selections(&mut self) {
        if self.selections.is_empty() {
            self.selections.push(Selection::caret(0));
        }
        self.selections.sort_by_key(|s| (s.range().start, s.range().end));
        let mut merged: Vec<Selection> = Vec::with_capacity(self.selections.len());
        for sel in self.selections.drain(..) {
            match merged.last_mut() {
                Some(last)
                    if sel.range().start < last.range().end
                        || (sel.is_caret() && last.is_caret() && sel.head == last.head) =>
                {
                    // Extend the previous selection to cover both.
                    let start = last.range().start.min(sel.range().start);
                    let end = last.range().end.max(sel.range().end);
                    let backwards = last.head < last.anchor;
                    *last = if backwards {
                        Selection { anchor: end, head: start, goal: None }
                    } else {
                        Selection { anchor: start, head: end, goal: None }
                    };
                }
                _ => merged.push(sel),
            }
        }
        self.selections = merged;
    }

    // -- undo ---------------------------------------------------------------

    fn begin_undo(&mut self) {
        let buf = self.buf();
        self.undo.push(UndoEntry {
            checkpoint: buf.checkpoint(),
            selections: self.selections.clone(),
            version: buf.version(),
        });
        self.redo.clear();
    }

    /// Drop the top undo entry if nothing was actually edited since.
    fn prune_noop_undo(&mut self) {
        if let Some(top) = self.undo.last()
            && top.version == self.buf().version()
        {
            self.undo.pop();
        }
    }

    pub fn undo(&mut self) -> bool {
        let Some(entry) = self.undo.pop() else {
            return false;
        };
        let buf = self.buf();
        self.redo.push(UndoEntry {
            checkpoint: buf.checkpoint(),
            selections: self.selections.clone(),
            version: buf.version(),
        });
        let buffer = self.buffer;
        if let Some(buf) = self.core.buffer_mut(buffer) {
            buf.restore(&entry.checkpoint);
        }
        self.selections = entry.selections;
        self.clamp_selections();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(entry) = self.redo.pop() else {
            return false;
        };
        let buf = self.buf();
        self.undo.push(UndoEntry {
            checkpoint: buf.checkpoint(),
            selections: self.selections.clone(),
            version: buf.version(),
        });
        let buffer = self.buffer;
        if let Some(buf) = self.core.buffer_mut(buffer) {
            buf.restore(&entry.checkpoint);
        }
        self.selections = entry.selections;
        self.clamp_selections();
        true
    }

    fn clamp_selections(&mut self) {
        let len = self.buf().len();
        for sel in &mut self.selections {
            sel.anchor = sel.anchor.min(len);
            sel.head = sel.head.min(len);
        }
        self.normalize_selections();
    }

    // -- the multicursor edit engine ----------------------------------------

    /// Apply one edit per selection, ascending, carrying every not-yet-applied
    /// selection forward through the journal. `make` receives the selection in
    /// *current* coordinates and returns `(range, replacement, caret offset
    /// relative to range.start)`, or `None` to leave that selection alone.
    pub fn apply_per_selection(
        &mut self,
        mut make: impl FnMut(&Buffer, &Selection) -> Option<(Range<usize>, Vec<u8>, usize)>,
    ) -> bool {
        self.normalize_selections();
        let v0 = self.buf().version();
        let sels = self.selections.clone();
        let mut placed: Vec<(usize, Version)> = Vec::with_capacity(sels.len());
        let mut mutated = false;

        for sel in &sels {
            let buf = self.buf();
            let vnow = buf.version();
            let len = buf.len();
            let anchor = buf
                .transform_offset(sel.anchor, v0, Bias::Left)
                .unwrap_or_else(|_| sel.anchor.min(len));
            let head = buf
                .transform_offset(sel.head, v0, Bias::Left)
                .unwrap_or_else(|_| sel.head.min(len));
            let current = Selection { anchor, head, goal: None };

            match make(buf, &current) {
                Some((range, insert, rel)) => {
                    match self.core.try_replace(self.buffer, range.clone(), &insert) {
                        Ok(version) => {
                            placed.push((range.start + rel, version));
                            mutated = true;
                        }
                        Err(_refused) => placed.push((head, vnow)),
                    }
                }
                None => placed.push((head, vnow)),
            }
        }

        let buf = self.buf();
        let vf = buf.version();
        let len = buf.len();
        self.selections = placed
            .into_iter()
            .map(|(pos, version)| {
                let pos = if version == vf {
                    pos
                } else {
                    buf.transform_offset(pos, version, Bias::Left)
                        .unwrap_or_else(|_| pos.min(len))
                };
                Selection::caret(pos.min(len))
            })
            .collect();
        self.normalize_selections();
        mutated
    }

    // -- primitive edits ----------------------------------------------------

    pub fn insert_text(&mut self, text: &[u8]) -> bool {
        let text = text.to_vec();
        self.apply_per_selection(|_, sel| {
            Some((sel.head..sel.head, text.clone(), text.len()))
        })
    }

    fn backspace(&mut self) -> bool {
        self.apply_per_selection(|buf, sel| {
            if sel.head == 0 {
                return None;
            }
            let start = prev_char(buf, sel.head);
            Some((start..sel.head, Vec::new(), 0))
        })
    }

    /// Delete each selection's range (or the char under a caret), capturing
    /// the removed text into the per-selection registers.
    fn delete_selections(&mut self) -> bool {
        self.capture_registers(true);
        self.apply_per_selection(|buf, sel| {
            let range = if sel.is_caret() {
                sel.head..next_char(buf, sel.head)
            } else {
                sel.range()
            };
            if range.is_empty() {
                return None;
            }
            Some((range, Vec::new(), 0))
        })
    }

    fn delete_lines(&mut self) -> bool {
        self.capture_registers(false);
        self.apply_per_selection(|buf, sel| {
            let (_, start_range) = line_bounds(buf, sel.range().start);
            let (_, end_range) = line_bounds(buf, sel.range().end);
            // Include the trailing newline; if last line, the leading one.
            let end = if end_range.end < buf.len() {
                end_range.end + 1
            } else {
                end_range.end
            };
            let start = if end_range.end >= buf.len() && start_range.start > 0 {
                start_range.start - 1
            } else {
                start_range.start
            };
            Some((start..end, Vec::new(), 0))
        })
    }

    /// Fill registers from the current selections. With `collapsed_char`, a
    /// caret captures the character under it.
    fn capture_registers(&mut self, collapsed_char: bool) {
        self.normalize_selections();
        let buf = self.buf();
        self.registers = self
            .selections
            .iter()
            .map(|sel| {
                let range = if sel.is_caret() && collapsed_char {
                    sel.head..next_char(buf, sel.head)
                } else {
                    sel.range()
                };
                let mut out = Vec::with_capacity(range.len());
                buf.read_into(range, &mut out);
                out
            })
            .collect();
    }

    fn yank(&mut self) -> bool {
        self.capture_registers(true);
        if self.mode == Mode::Visual {
            self.mode = Mode::Normal;
            for sel in &mut self.selections {
                let start = sel.range().start;
                *sel = Selection::caret(start);
            }
        }
        true
    }

    fn paste(&mut self) -> bool {
        if self.registers.is_empty() {
            return false;
        }
        self.begin_undo();
        let registers = self.registers.clone();
        let mut index = 0;
        let mutated = self.apply_per_selection(move |buf, sel| {
            let text = registers[index % registers.len()].clone();
            index += 1;
            if text.is_empty() {
                return None;
            }
            if sel.is_caret() {
                // Paste after the cursor, vim-style.
                let at = next_char(buf, sel.head);
                let len = text.len();
                Some((at..at, text, len.saturating_sub(1)))
            } else {
                let len = text.len();
                Some((sel.range(), text, len.saturating_sub(1)))
            }
        });
        self.prune_noop_undo();
        mutated
    }

    // -- movement -----------------------------------------------------------

    /// Move every head through `step`; anchors follow unless extending.
    fn move_heads(
        &mut self,
        extend: bool,
        step: impl Fn(&Buffer, usize, Option<usize>) -> (usize, Option<usize>),
    ) -> bool {
        let buffer_moved;
        {
            let buf = self.core.buffer(self.buffer).expect("app buffer exists");
            let mut moved = false;
            for sel in &mut self.selections {
                let (head, goal) = step(buf, sel.head, sel.goal);
                moved |= head != sel.head;
                sel.head = head;
                sel.goal = goal;
                if !extend {
                    sel.anchor = head;
                }
            }
            buffer_moved = moved;
        }
        self.normalize_selections();
        buffer_moved
    }

    fn move_horiz(&mut self, extend: bool, delta: isize) -> bool {
        self.move_heads(extend, |buf, head, _| {
            let pos = if delta < 0 {
                prev_char(buf, head)
            } else {
                next_char(buf, head)
            };
            (pos, None)
        })
    }

    fn move_vert(&mut self, extend: bool, delta: isize) -> bool {
        self.move_heads(extend, |buf, head, goal| {
            let (line, range) = line_bounds(buf, head);
            let col = goal.unwrap_or(head - range.start);
            let target = line.saturating_add_signed(delta);
            match buf.line_range(target) {
                Some(range) => {
                    let head = (range.start + col).min(range.end);
                    (head, Some(col))
                }
                None => (head, Some(col)),
            }
        })
    }

    fn move_line_start(&mut self, extend: bool) -> bool {
        self.move_heads(extend, |buf, head, _| {
            let (_, range) = line_bounds(buf, head);
            (range.start, None)
        })
    }

    fn move_line_end(&mut self, extend: bool) -> bool {
        self.move_heads(extend, |buf, head, _| {
            let (_, range) = line_bounds(buf, head);
            (range.end, None)
        })
    }

    fn move_doc_start(&mut self, extend: bool) -> bool {
        self.move_heads(extend, |_, _, _| (0, None))
    }

    fn move_doc_end(&mut self, extend: bool) -> bool {
        self.move_heads(extend, |buf, _, _| (buf.len(), None))
    }

    fn move_word_fwd(&mut self, extend: bool) -> bool {
        self.move_heads(extend, |buf, head, _| {
            let len = buf.len();
            let mut pos = head;
            // Skip the rest of the current word, then whitespace/punct runs.
            while pos < len && byte_at(buf, pos).is_some_and(is_word_byte) {
                pos = next_char(buf, pos);
            }
            while pos < len && !byte_at(buf, pos).is_some_and(is_word_byte) {
                pos = next_char(buf, pos);
            }
            (pos, None)
        })
    }

    fn move_word_back(&mut self, extend: bool) -> bool {
        self.move_heads(extend, |buf, head, _| {
            let mut pos = head;
            while pos > 0 && !byte_at(buf, prev_char(buf, pos)).is_some_and(is_word_byte) {
                pos = prev_char(buf, pos);
            }
            while pos > 0 && byte_at(buf, prev_char(buf, pos)).is_some_and(is_word_byte) {
                pos = prev_char(buf, pos);
            }
            (pos, None)
        })
    }

    // -- multicursor --------------------------------------------------------

    fn add_cursor_below(&mut self) -> bool {
        let Some(last) = self.selections.last().copied() else {
            return false;
        };
        let buf = self.buf();
        let (line, range) = line_bounds(buf, last.head);
        let col = last.goal.unwrap_or(last.head - range.start);
        let Some(next) = buf.line_range(line + 1) else {
            return false;
        };
        let head = (next.start + col).min(next.end);
        self.selections.push(Selection {
            anchor: head,
            head,
            goal: Some(col),
        });
        self.normalize_selections();
        true
    }

    fn collapse_cursors(&mut self) -> bool {
        if self.selections.len() <= 1 {
            return false;
        }
        self.selections.truncate(1);
        true
    }

    // -- mode changes -------------------------------------------------------

    fn enter_insert(&mut self) {
        self.begin_undo();
        self.mode = Mode::Insert;
        for sel in &mut self.selections {
            sel.anchor = sel.head;
        }
    }

    fn leave_insert(&mut self) {
        self.mode = Mode::Normal;
        self.prune_noop_undo();
    }

    fn open_line(&mut self, below: bool) {
        self.begin_undo();
        self.mode = Mode::Insert;
        self.apply_per_selection(|buf, sel| {
            let (_, range) = line_bounds(buf, sel.head);
            if below {
                Some((range.end..range.end, b"\n".to_vec(), 1))
            } else {
                Some((range.start..range.start, b"\n".to_vec(), 0))
            }
        });
    }

    // -- dispatch -----------------------------------------------------------

    /// Insert-mode text from the platform's text-input path (never keycodes).
    pub fn handle_text(&mut self, text: &str) -> bool {
        if self.mode != Mode::Insert || text.is_empty() {
            return false;
        }
        self.insert_text(text.as_bytes())
    }

    pub fn handle_key(&mut self, kp: KeyPress) -> bool {
        let dirty = match self.mode {
            Mode::Insert => self.key_insert(kp),
            Mode::Normal | Mode::Visual => self.key_modal(kp),
        };
        dirty
    }

    fn key_insert(&mut self, kp: KeyPress) -> bool {
        match kp.key {
            Key::Esc => {
                self.leave_insert();
                true
            }
            Key::Backspace => self.backspace(),
            Key::Enter => self.insert_text(b"\n"),
            Key::Tab => self.insert_text(b"    "),
            Key::Left => self.move_horiz(false, -1),
            Key::Right => self.move_horiz(false, 1),
            Key::Up => self.move_vert(false, -1),
            Key::Down => self.move_vert(false, 1),
            _ => false,
        }
    }

    fn key_modal(&mut self, kp: KeyPress) -> bool {
        let extend = self.mode == Mode::Visual;

        // Pending multi-key sequences (gg, dd).
        if let Some(prefix) = self.pending.take() {
            return match (prefix, kp.key) {
                ('g', Key::Char('g')) if !kp.mods.shift => self.move_doc_start(extend),
                ('d', Key::Char('d')) if !kp.mods.shift => {
                    self.begin_undo();
                    let mutated = self.delete_lines();
                    self.prune_noop_undo();
                    mutated
                }
                _ => false,
            };
        }

        match (kp.key, kp.mods.ctrl, kp.mods.shift) {
            (Key::Char('r'), true, _) => self.redo(),

            (Key::Char('h'), false, false) | (Key::Left, ..) => self.move_horiz(extend, -1),
            (Key::Char('l'), false, false) | (Key::Right, ..) => self.move_horiz(extend, 1),
            (Key::Char('j'), false, false) | (Key::Down, ..) => self.move_vert(extend, 1),
            (Key::Char('k'), false, false) | (Key::Up, ..) => self.move_vert(extend, -1),
            (Key::Char('0'), false, false) | (Key::Home, ..) => self.move_line_start(extend),
            (Key::Char('4'), false, true) | (Key::End, ..) => self.move_line_end(extend),
            (Key::Char('w'), false, false) => self.move_word_fwd(extend),
            (Key::Char('b'), false, false) => self.move_word_back(extend),
            (Key::Char('g'), false, false) => {
                self.pending = Some('g');
                false
            }
            (Key::Char('g'), false, true) => self.move_doc_end(extend),

            (Key::Char('i'), false, false) => {
                self.enter_insert();
                true
            }
            (Key::Char('a'), false, false) => {
                self.move_horiz(false, 1);
                self.enter_insert();
                true
            }
            (Key::Char('i'), false, true) => {
                self.move_line_start(false);
                self.enter_insert();
                true
            }
            (Key::Char('a'), false, true) => {
                self.move_line_end(false);
                self.enter_insert();
                true
            }
            (Key::Char('o'), false, false) => {
                self.open_line(true);
                true
            }
            (Key::Char('o'), false, true) => {
                self.open_line(false);
                true
            }

            (Key::Char('v'), false, false) => {
                self.mode = if self.mode == Mode::Visual {
                    for sel in &mut self.selections {
                        sel.anchor = sel.head;
                    }
                    Mode::Normal
                } else {
                    Mode::Visual
                };
                true
            }

            (Key::Char('x'), false, false) | (Key::Delete, ..) => {
                self.begin_undo();
                let mutated = self.delete_selections();
                self.prune_noop_undo();
                if self.mode == Mode::Visual {
                    self.mode = Mode::Normal;
                }
                mutated
            }
            (Key::Char('d'), false, false) => {
                if self.mode == Mode::Visual {
                    self.begin_undo();
                    let mutated = self.delete_selections();
                    self.prune_noop_undo();
                    self.mode = Mode::Normal;
                    mutated
                } else {
                    self.pending = Some('d');
                    false
                }
            }
            (Key::Char('y'), false, false) => self.yank(),
            (Key::Char('p'), false, false) => self.paste(),
            (Key::Char('u'), false, false) => self.undo(),

            (Key::Char('c'), false, true) => self.add_cursor_below(),
            (Key::Char(','), false, false) => self.collapse_cursors(),

            (Key::Esc, ..) => {
                self.pending = None;
                let had_many = self.selections.len() > 1;
                if self.mode == Mode::Visual {
                    self.mode = Mode::Normal;
                }
                for sel in &mut self.selections {
                    sel.anchor = sel.head;
                }
                self.normalize_selections();
                had_many || true
            }

            _ => false,
        }
    }
}
