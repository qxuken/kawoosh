//! A snippet live in the text (docs/design/completion.md Decision 7):
//! accepted, it becomes kawoosh's own selections — the field being
//! filled selected, so typing replaces it; a field that repeats several
//! carets edited together; `$0` the last caret; a choice field's
//! options the completion's popup. `snippet next` / `snippet prev`
//! (`<C-l>` / `<C-k>`) go from field to field, keys local to the
//! `snippet` fact, which holds only while one is live; `<Tab>` is left
//! alone.
//!
//! **One at a time, and gone when it ends.** A snippet ends at its last
//! field (`$0`), on leaving insert mode, when a caret leaves its text by
//! anything but a jump, when an edit lands outside it, when its view
//! shows another buffer or another pane takes the keys, on an undo or a
//! redo — the text the history puts back is the history's, not the
//! snippet's — and when another is accepted. Nothing remembers one past
//! its end: with none live, `<C-l>` is nobody's.
//!
//! **Its fields follow the edits** through the buffer's journal, each
//! edit in turn: the field being filled (and any that holds it) grows
//! with what is typed at its edges, every other field keeps out of it,
//! so typing at a field's end never spills into its neighbour. A field
//! nested in the one being filled whose text the typing took is gone.
//! The ranges are bytes; a multi-byte character is one edit like any
//! other. A keystroke costs the fields' ranges carried through its few
//! edits: no text is read.
//!
//! **Variables** are VS Code's, worked out where the snippet is
//! accepted. The texts a variable could carry out of a mask — the
//! selection, the line, the word, the clipboard — are empty when they
//! hold one: a private buffer's text, a mask's, a secret's moment, a
//! word a rule masks (secrets.md Decisions 1, 3 and 9).

use std::ops::Range;
use std::rc::Rc;

use kawoosh_doc::{Bias, BufferId, Edit, Version};
use kawoosh_editor::{Mode, Selection, Selections, ViewId};
use kawoosh_systems::snippet::{self as grammar, Indent, Transform};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use kawoosh_editor::Spec;

/// The fact that holds while a snippet is live: its keys' place.
pub const FACT: &str = "snippet";

/// The live snippet.
pub struct Live {
    pub view: ViewId,
    pub buffer: BufferId,
    /// The buffer's version its ranges are at.
    seen: Version,
    /// The editor's moves through history then: another is an undo or a
    /// redo.
    moves: u64,
    /// Its text, a range each place it was put (several carets accepted
    /// it): a caret or an edit outside every one ends it.
    pub spans: Vec<Range<usize>>,
    /// Its fields in the order they are visited, `$0` last.
    pub fields: Vec<LiveField>,
    /// The field being filled.
    pub at: usize,
}

/// One field of the live snippet.
pub struct LiveField {
    pub index: u32,
    /// Its places, a caret each when it is the one being filled.
    pub ranges: Vec<Range<usize>>,
    /// A choice field's options, offered in the popup as it is reached.
    pub choices: Option<Rc<[String]>>,
    /// The places a transform of it fills, rewritten as it is left.
    mirrors: Vec<(Range<usize>, Transform)>,
    /// The field it is nested in, by place in `fields`.
    parent: Option<usize>,
    /// Reached once: no longer drawn underlined.
    pub visited: bool,
    /// Nested in a field whose text the typing took: never reached.
    gone: bool,
}

/// Where an edit of the snippet's own lands: a field's place or a
/// mirror's.
#[derive(Clone, Copy)]
enum Place {
    Field(usize, usize),
    Mirror(usize, usize),
}

/// `r` carried through `e`, growing with text put at its edges.
fn grow(r: &Range<usize>, e: &Edit) -> Range<usize> {
    let s = e.transform_offset(r.start, Bias::Left);
    let t = e.transform_offset(r.end, Bias::Right);
    s..t.max(s)
}

/// `r` carried through `e`, keeping out text put at its edges.
fn keep(r: &Range<usize>, e: &Edit) -> Range<usize> {
    let s = e.transform_offset(r.start, Bias::Right);
    let t = e.transform_offset(r.end, Bias::Left);
    s..t.max(s)
}

fn holds(outer: &Range<usize>, inner: &Range<usize>) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

impl Live {
    /// Whether offset `o` is in its text, either edge included.
    ///
    /// An edge is its text's only where the field being filled touches
    /// it: a caret past the last character with no field there has left
    /// it, and typing on there is typing after it (the LuaSnip
    /// complaint: `<C-l>` must not reach back into it).
    fn has(&self, o: usize) -> bool {
        self.spans.iter().any(|s| {
            (s.start < o && o < s.end) || ((o == s.start || o == s.end) && self.touches(o))
        })
    }

    /// Whether a place of the field being filled starts or ends at `o`.
    fn touches(&self, o: usize) -> bool {
        self.fields
            .get(self.at)
            .is_some_and(|f| f.ranges.iter().any(|r| r.start == o || r.end == o))
    }

    /// Whether edit `e` lands inside its text: within a span, and at an
    /// edge only where the field being filled touches it (`own`, the
    /// snippet's own edits, may land at any).
    fn takes(&self, e: &Edit, own: bool) -> bool {
        self.spans.iter().any(|s| {
            holds(s, &e.range)
                && (own
                    || ((e.range.start > s.start || self.touches(s.start))
                        && (e.range.end < s.end || self.touches(s.end))))
        })
    }

    /// Whether field `inner` is nested, at any depth, in field `outer`.
    fn nested_in(&self, inner: usize, outer: usize) -> bool {
        let mut at = self.fields[inner].parent;
        // A chain no longer than the fields: no loop can hold it.
        for _ in 0..self.fields.len() {
            match at {
                Some(p) if p == outer => return true,
                Some(p) => at = self.fields[p].parent,
                None => return false,
            }
        }
        false
    }

    /// The ranges carried through `edits`, with field `current` (none
    /// for the snippet's own edits) — and the fields it is nested in —
    /// growing with what is typed at its edges. False when one landed
    /// outside its text.
    fn follow(&mut self, edits: &[Edit], current: Option<usize>) -> bool {
        let before: Vec<bool> = self
            .fields
            .iter()
            .map(|f| f.ranges.iter().any(|r| !r.is_empty()))
            .collect();
        let grows: Vec<bool> = (0..self.fields.len())
            .map(|i| current.is_some_and(|c| i == c || self.nested_in(c, i)))
            .collect();
        for e in edits {
            if !self.takes(e, current.is_none()) {
                return false;
            }
            for s in &mut self.spans {
                *s = grow(s, e);
            }
            for (f, grows) in self.fields.iter_mut().zip(&grows) {
                for r in &mut f.ranges {
                    *r = if *grows { grow(r, e) } else { keep(r, e) };
                }
                for (r, _) in &mut f.mirrors {
                    *r = keep(r, e);
                }
            }
        }
        // A field nested in the one being filled whose text the typing
        // took is no field any more (VS Code's rule): a place of it
        // there emptied. `$0` stays.
        if let Some(c) = current {
            let last = self.fields.len() - 1;
            let cur = self.fields[c].ranges.clone();
            for (i, was) in before.iter().enumerate().take(last) {
                if !was || !self.nested_in(i, c) {
                    continue;
                }
                let emptied = self.fields[i]
                    .ranges
                    .iter()
                    .any(|r| r.is_empty() && cur.iter().any(|o| holds(o, r)));
                if emptied {
                    self.fields[i].gone = true;
                }
            }
        }
        true
    }

    /// The next field to reach, forward or back, if any.
    fn next(&self, forward: bool) -> Option<usize> {
        let ok = |i: &usize| {
            let f = &self.fields[*i];
            !f.gone && !f.ranges.is_empty()
        };
        if forward {
            (self.at + 1..self.fields.len()).find(ok)
        } else {
            (0..self.at).rev().find(ok)
        }
    }
}

/// The variables of one place a snippet goes (VS Code's set).
struct Vars {
    filename: String,
    directory: String,
    filepath: String,
    relative: String,
    workspace: String,
    clipboard: String,
    line_comment: String,
    block: (String, String),
    /// Per place: the selected text, the line, the word, the line's
    /// number from 0, the caret's number from 0.
    selected: String,
    line: String,
    word: String,
    line_index: usize,
    cursor: usize,
}

impl Vars {
    fn get(&self, name: &str) -> Option<String> {
        let (now, offset) = local_now();
        let civil = civil(now + offset);
        Some(match name {
            "TM_SELECTED_TEXT" => self.selected.clone(),
            "TM_CURRENT_LINE" => self.line.clone(),
            "TM_CURRENT_WORD" => self.word.clone(),
            "TM_LINE_INDEX" => self.line_index.to_string(),
            "TM_LINE_NUMBER" => (self.line_index + 1).to_string(),
            "TM_FILENAME" => self.filename.clone(),
            "TM_FILENAME_BASE" => match self.filename.rsplit_once('.') {
                Some((base, _)) if !base.is_empty() => base.to_string(),
                _ => self.filename.clone(),
            },
            "TM_DIRECTORY" => self.directory.clone(),
            "TM_FILEPATH" => self.filepath.clone(),
            "RELATIVE_FILEPATH" => self.relative.clone(),
            "CLIPBOARD" => self.clipboard.clone(),
            "WORKSPACE_FOLDER" => self.workspace.clone(),
            "WORKSPACE_NAME" => std::path::Path::new(&self.workspace)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            "CURSOR_INDEX" => self.cursor.to_string(),
            "CURSOR_NUMBER" => (self.cursor + 1).to_string(),
            "LINE_COMMENT" => self.line_comment.clone(),
            "BLOCK_COMMENT_START" => self.block.0.clone(),
            "BLOCK_COMMENT_END" => self.block.1.clone(),
            "CURRENT_YEAR" => format!("{:04}", civil.0),
            "CURRENT_YEAR_SHORT" => format!("{:02}", civil.0.rem_euclid(100)),
            "CURRENT_MONTH" => format!("{:02}", civil.1),
            "CURRENT_MONTH_NAME" => MONTHS[civil.1 as usize - 1].to_string(),
            "CURRENT_MONTH_NAME_SHORT" => MONTHS[civil.1 as usize - 1][..3].to_string(),
            "CURRENT_DATE" => format!("{:02}", civil.2),
            "CURRENT_DAY_NAME" => DAYS[civil.3].to_string(),
            "CURRENT_DAY_NAME_SHORT" => DAYS[civil.3][..3].to_string(),
            "CURRENT_HOUR" => format!("{:02}", (now + offset).rem_euclid(86_400) / 3600),
            "CURRENT_MINUTE" => format!("{:02}", (now + offset).rem_euclid(3600) / 60),
            "CURRENT_SECOND" => format!("{:02}", (now + offset).rem_euclid(60)),
            "CURRENT_SECONDS_UNIX" => now.to_string(),
            "CURRENT_TIMEZONE_OFFSET" => {
                let m = offset / 60;
                let sign = if m < 0 { '-' } else { '+' };
                format!("{sign}{:02}:{:02}", m.abs() / 60, m.abs() % 60)
            }
            "RANDOM" => format!("{:06}", random() % 1_000_000),
            "RANDOM_HEX" => format!("{:06x}", random() & 0xFF_FFFF),
            "UUID" => {
                let (a, b) = (random(), random());
                let b = (b & 0x3FFF_FFFF_FFFF_FFFF) | 0x8000_0000_0000_0000;
                let a = (a & !0xF000) | 0x4000;
                format!(
                    "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
                    a >> 32,
                    (a >> 16) & 0xFFFF,
                    a & 0xFFFF,
                    b >> 48,
                    b & 0xFFFF_FFFF_FFFF
                )
            }
            _ => return None,
        })
    }
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// Seconds since the epoch, and the local offset from UTC.
fn local_now() -> (i64, i64) {
    let now = kawoosh_systems::store::now();
    (now, crate::notify::utc_offset(now))
}

/// A local time's year, month, day and weekday (0 Sunday).
fn civil(local: i64) -> (i64, i64, i64, usize) {
    let days = local.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    // 1970-01-01 was a Thursday.
    (year, month, day, (days + 4).rem_euclid(7) as usize)
}

/// Sixty-four bits nobody can guess from the last: the standard
/// library's random hash keys over a counter.
fn random() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    thread_local! {
        static STATE: (std::collections::hash_map::RandomState, std::cell::Cell<u64>) =
            (Default::default(), Default::default());
    }
    STATE.with(|(s, n)| {
        n.set(n.get() + 1);
        let mut h = s.build_hasher();
        h.write_u64(n.get());
        h.finish()
    })
}

impl Kawoosh {
    /// Whether a snippet is live in the focused pane: the `snippet`
    /// fact, its keys' place.
    pub(crate) fn snippet_on(&self) -> bool {
        self.snippet
            .as_ref()
            .is_some_and(|l| self.focused_view() == Some(l.view))
    }

    /// Snippet `body` put in buffer `buffer` in place of each of
    /// `ranges` (a caret's word each, ascending), and live: its first
    /// field selected in every place, or with only `$0`, the carets
    /// there and nothing live. `as_is` keeps its lines as they are
    /// rather than indented as the line each goes into. Another live
    /// snippet ends first. False when nothing was put (a read-only
    /// buffer).
    pub(crate) fn snippet_accept(
        &mut self,
        view: ViewId,
        buffer: BufferId,
        ranges: &[Range<usize>],
        body: &str,
        as_is: bool,
    ) -> bool {
        self.snippet_drop();
        if ranges.is_empty() || !self.ed.buffers.contains_key(buffer) {
            return false;
        }
        let nodes = grammar::parse(body);
        let shared = self.snippet_vars(buffer, body);
        let unit = match as_is {
            true => String::new(),
            false => self.ed.indent_unit_in(buffer),
        };
        let mut expanded = Vec::with_capacity(ranges.len());
        for (k, r) in ranges.iter().enumerate() {
            let vars = self.snippet_vars_at(&shared, buffer, view, r.clone(), k, body);
            let buf = &self.ed.buffers[buffer];
            let ls = buf.line_start(buf.line_of(r.start));
            let lead = buf.slice(ls..r.start);
            let base = match as_is {
                true => String::new(),
                false => lead[..lead.len() - lead.trim_start().len()].to_string(),
            };
            let indent = Indent {
                base,
                unit: unit.clone(),
                eol: String::new(),
            };
            expanded.push(grammar::expand(&nodes, &mut |n| vars.get(n), &indent));
        }
        let edits: Vec<(Range<usize>, String)> = ranges
            .iter()
            .zip(&expanded)
            .map(|(r, e)| (r.clone(), e.text.clone()))
            .collect();
        let Some(starts) = self.ed.apply_edits_at(buffer, &edits) else {
            return false;
        };
        // Every place's fields joined, by number: field N is its places
        // in each copy. A variable's default can hold a field one copy
        // has and another not (`${TM_CURRENT_WORD:${1:x}}`), so the
        // fields are every copy's, in the order a copy visits them.
        let shift = |k: usize, r: &Range<usize>| starts[k] + r.start..starts[k] + r.end;
        let mut order: Vec<u32> = Vec::new();
        for e in &expanded {
            for f in &e.fields {
                if !order.contains(&f.index) {
                    order.push(f.index);
                }
            }
        }
        // Numbered ascending, `$0` last, as `expand` orders each copy.
        order.sort_by_key(|i| (*i == 0, *i));
        let place = |index: u32| order.iter().position(|i| *i == index);
        let fields: Vec<LiveField> = order
            .iter()
            .map(|&index| {
                let of =
                    |e: &'_ grammar::Expanded| e.fields.iter().find(|f| f.index == index).cloned();
                let first = expanded
                    .iter()
                    .find_map(of)
                    .expect("a field is some copy's");
                LiveField {
                    index,
                    ranges: expanded
                        .iter()
                        .enumerate()
                        .flat_map(|(k, e)| {
                            of(e)
                                .into_iter()
                                .flat_map(move |f| f.ranges.into_iter().map(move |r| shift(k, &r)))
                        })
                        .collect(),
                    choices: first.choices.clone().map(Rc::from),
                    mirrors: expanded
                        .iter()
                        .enumerate()
                        .flat_map(|(k, e)| {
                            of(e).into_iter().flat_map(move |f| {
                                f.mirrors.into_iter().map(move |(r, t)| (shift(k, &r), t))
                            })
                        })
                        .collect(),
                    parent: first.parent.and_then(place),
                    visited: false,
                    gone: false,
                }
            })
            .collect();
        let spans = expanded
            .iter()
            .enumerate()
            .map(|(k, e)| starts[k]..starts[k] + e.text.len())
            .collect();
        let buf = &self.ed.buffers[buffer];
        self.snippet = Some(Live {
            view,
            buffer,
            seen: buf.version(),
            moves: self.ed.history_moves,
            spans,
            fields,
            at: 0,
        });
        self.follow_caret = true;
        self.snippet_reach(0);
        true
    }

    /// The live snippet ended: gone, and its field's selections no
    /// longer typed over (`View::fill`).
    fn snippet_drop(&mut self) {
        if let Some(live) = self.snippet.take()
            && let Some(v) = self.ed.views.get_mut(live.view)
        {
            v.fill = None;
        }
    }

    /// `body` put at the focused pane's carets (each selection's text
    /// replaced) and live, as an accepted snippet is: what a test
    /// drives, and the door the user's own snippets will come through
    /// (completion.md's round 4).
    pub fn snippet_expand(&mut self, body: &str) -> bool {
        let Some(view) = self.focused_view() else {
            return false;
        };
        let buffer = self.ed.views[view].buffer;
        let mut ranges: Vec<Range<usize>> =
            self.ed.views[view].sels.iter().map(|s| s.range()).collect();
        ranges.sort_by_key(|r| (r.start, r.end));
        self.snippet_accept(view, buffer, &ranges, body, false)
    }

    /// Field `i` reached: its places selected, its choices offered —
    /// or, `$0`, the carets there and the snippet done.
    fn snippet_reach(&mut self, i: usize) {
        let Some(live) = self.snippet.as_mut() else {
            return;
        };
        // A field with no place left (none had one, or the typing took
        // them all) is never reached: the next one that has, else the end.
        let Some(i) = (i..live.fields.len()).find(|&j| {
            let f = &live.fields[j];
            !f.ranges.is_empty() && (!f.gone || j + 1 == live.fields.len())
        }) else {
            self.snippet_drop();
            return;
        };
        live.at = i;
        let f = &mut live.fields[i];
        f.visited = true;
        let items: Vec<Selection> = f
            .ranges
            .iter()
            .map(|r| Selection::new(r.start, r.end))
            .collect();
        let choices = f.choices.clone();
        let first = f.ranges.first().cloned();
        let (view, last) = (live.view, i + 1 == live.fields.len());
        if let Some(v) = self.ed.views.get_mut(view) {
            v.sels = Selections { items, primary: 0 };
            v.sels.normalize();
            v.goal_col = None;
            // Typing goes over these, and only these (`View::fill`).
            v.fill = Some(v.sels.clone());
        }
        self.follow_caret = true;
        if last {
            self.snippet = None;
            return;
        }
        if let (Some(options), Some(r)) = (choices, first) {
            self.completion_choices(view, r, &options);
        }
    }

    /// `snippet next` / `snippet prev`: the field left (its transforms'
    /// places rewritten from it), the next reached. Nothing with none
    /// live, or none that way.
    fn snippet_jump(&mut self, forward: bool) {
        self.sync_snippet();
        let Some(live) = self.snippet.as_ref() else {
            return;
        };
        let Some(next) = live.next(forward) else {
            return;
        };
        self.snippet_fill_mirrors();
        if self.snippet.is_none() {
            return;
        }
        self.completion.session = None;
        self.snippet_reach(next);
    }

    /// The places a transform of the field being filled fills,
    /// rewritten from its text.
    fn snippet_fill_mirrors(&mut self) {
        let Some(live) = self.snippet.as_ref() else {
            return;
        };
        let f = &live.fields[live.at];
        let Some(r) = f.ranges.first() else {
            return;
        };
        if f.mirrors.is_empty() {
            return;
        }
        let text = self.ed.buffers[live.buffer].slice(r.clone());
        let at = live.at;
        let puts = f
            .mirrors
            .iter()
            .enumerate()
            .map(|(m, (_, t))| (Place::Mirror(at, m), t.apply(&text)))
            .collect();
        // The field left does not grow with what lands beside it.
        self.snippet_put(puts, None);
    }

    /// The choice taken from the popup: every place of the field being
    /// filled made `text`, the carets after it.
    pub(crate) fn snippet_choose(&mut self, text: &str) {
        self.snippet_follow_edits();
        let Some(live) = self.snippet.as_ref() else {
            return;
        };
        let at = live.at;
        let puts = (0..live.fields[at].ranges.len())
            .map(|r| (Place::Field(at, r), text.to_string()))
            .collect();
        // The field and those it is nested in hold what is chosen.
        self.snippet_put(puts, Some(at));
        let Some(live) = self.snippet.as_ref() else {
            return;
        };
        let items: Vec<Selection> = live.fields[at]
            .ranges
            .iter()
            .map(|r| Selection::point(r.end))
            .collect();
        if let Some(v) = self.ed.views.get_mut(live.view) {
            v.sels = Selections { items, primary: 0 };
            v.sels.normalize();
        }
        self.follow_caret = true;
    }

    /// The snippet's own edits: each place made its text, every other
    /// range carried past them, field `current` growing as typing grows
    /// it.
    fn snippet_put(&mut self, puts: Vec<(Place, String)>, current: Option<usize>) {
        let Some(live) = self.snippet.as_ref() else {
            return;
        };
        let range = |p: &Place| match *p {
            Place::Field(f, r) => live.fields[f].ranges[r].clone(),
            Place::Mirror(f, m) => live.fields[f].mirrors[m].0.clone(),
        };
        let edits: Vec<(Range<usize>, String)> =
            puts.iter().map(|(p, t)| (range(p), t.clone())).collect();
        let buffer = live.buffer;
        let since = self.ed.buffers[buffer].version();
        let Some(starts) = self.ed.apply_edits_at(buffer, &edits) else {
            return;
        };
        let buf = &self.ed.buffers[buffer];
        let version = buf.version();
        let Ok(edits) = buf
            .journal()
            .edits_since(since)
            .map(|e| e.cloned().collect::<Vec<Edit>>())
        else {
            self.snippet_drop();
            return;
        };
        let live = self.snippet.as_mut().unwrap();
        if !live.follow(&edits, current) {
            self.snippet_drop();
            return;
        }
        for ((p, t), start) in puts.iter().zip(starts) {
            let r = start..start + t.len();
            match *p {
                Place::Field(f, i) => live.fields[f].ranges[i] = r,
                Place::Mirror(f, m) => live.fields[f].mirrors[m].0 = r,
            }
        }
        live.seen = version;
    }

    /// After a key and every frame: the live snippet's fields brought
    /// to the text, or the snippet ended by whatever ends one.
    pub(crate) fn sync_snippet(&mut self) {
        self.snippet_check();
        let on = self.snippet_on();
        self.ed.fact(FACT, on);
    }

    /// The live snippet brought to the text, or ended.
    fn snippet_check(&mut self) {
        let Some(live) = self.snippet.as_ref() else {
            return;
        };
        let (view, buffer) = (live.view, live.buffer);
        let in_insert = self
            .ed
            .views
            .get(view)
            .is_some_and(|v| v.buffer == buffer && v.mode == Mode::Insert);
        // Another pane with the keys; the candidates picker over this
        // one is still this one's (a snippet taken from it is live with
        // the keys back in its pane).
        let picking = self.lsp.candidates_from.is_some() && self.focused_view().is_none();
        if !in_insert || !(self.focused_view() == Some(view) || picking) {
            self.snippet_drop();
            return;
        }
        if !self.snippet_follow_edits() {
            return;
        }
        let live = self.snippet.as_ref().unwrap();
        let inside = self.ed.views[view]
            .sels
            .iter()
            .all(|s| live.has(s.anchor) && live.has(s.head));
        if !inside {
            self.snippet_drop();
        }
    }

    /// The live snippet's ranges carried through the edits since they
    /// were last, the field being filled growing with them — or the
    /// snippet ended, when one landed outside it or the history put its
    /// text back (an undo, a redo). False when it ended.
    fn snippet_follow_edits(&mut self) -> bool {
        let Some(live) = self.snippet.as_mut() else {
            return false;
        };
        let followed = match self.ed.buffers.get(live.buffer) {
            None => false,
            Some(buf) if buf.version() == live.seen => return true,
            Some(buf) => {
                self.ed.history_moves == live.moves
                    && match buf.journal().edits_since(live.seen) {
                        Ok(edits) => {
                            let edits: Vec<Edit> = edits.cloned().collect();
                            let at = live.at;
                            let ok = live.follow(&edits, Some(at));
                            live.seen = buf.version();
                            ok
                        }
                        Err(_) => false,
                    }
            }
        };
        if !followed {
            self.snippet_drop();
        }
        followed
    }

    /// The places of `view`'s live snippet not reached yet, with text
    /// in them: drawn faintly underlined.
    pub fn snippet_marks(&self, view: ViewId) -> Vec<Range<usize>> {
        let Some(live) = self.snippet.as_ref().filter(|l| l.view == view) else {
            return Vec::new();
        };
        let mut out: Vec<Range<usize>> = live
            .fields
            .iter()
            .filter(|f| !f.visited && !f.gone)
            .flat_map(|f| f.ranges.iter().filter(|r| !r.is_empty()).cloned())
            .collect();
        out.sort_by_key(|r| (r.start, r.end));
        out
    }

    /// What every place of snippet `body` in `buffer` shares. A text a
    /// mask could hide is read only when the body names its variable.
    fn snippet_vars(&mut self, buffer: BufferId, body: &str) -> Vars {
        let buf = &self.ed.buffers[buffer];
        let path = buf.path.clone();
        let filename = path
            .as_deref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| buf.name.clone());
        let directory = path
            .as_deref()
            .and_then(|p| p.parent())
            .map(|d| d.display().to_string())
            .unwrap_or_default();
        let workspace = self.moments.workspace().to_string();
        let filepath = path
            .as_deref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let relative = path
            .as_deref()
            .map(|p| {
                p.strip_prefix(&workspace)
                    .unwrap_or(p)
                    .display()
                    .to_string()
            })
            .unwrap_or_default();
        let tokens = self.ed.comment_tokens_in(buffer);
        let clipboard = match body.contains("CLIPBOARD") {
            true => self.snippet_clipboard(buffer),
            false => String::new(),
        };
        Vars {
            filename,
            directory,
            filepath,
            relative,
            workspace,
            clipboard,
            line_comment: tokens.line.unwrap_or_default(),
            block: tokens.block.unwrap_or_default(),
            selected: String::new(),
            line: String::new(),
            word: String::new(),
            line_index: 0,
            cursor: 0,
        }
    }

    /// The register's newest text as `$CLIPBOARD` may put it: nothing
    /// when it is a secret's, or holds a word a mask or a rule hides.
    fn snippet_clipboard(&mut self, buffer: BufferId) -> String {
        let Some(m) = self.ed.memory.head() else {
            return String::new();
        };
        if m.secret {
            return String::new();
        }
        let text = m.text.clone();
        if self.snippet_hides(buffer, &text) {
            return String::new();
        }
        text
    }

    /// Whether `text` holds what a mask hides: a word inside an open
    /// buffer's mask, or what the rules mask in `buffer`'s file.
    fn snippet_hides(&mut self, buffer: BufferId, text: &str) -> bool {
        let (path, language) = match self.ed.buffers.get(buffer) {
            Some(b) => (b.path.clone(), b.language.to_string()),
            None => return true,
        };
        if text.is_empty() {
            return false;
        }
        if crate::completion::holds_masked(&self.completion.masked, text)
            || self.mask_text(path.as_deref(), &language, text) != text
        {
            return true;
        }
        // What every open buffer's masks cover now — the words the
        // completion's indexes keep are only the buffers it indexed.
        let words: std::collections::HashSet<&str> = text
            .split(|c: char| !kawoosh_systems::words::is_word_char(c))
            .filter(|w| !w.is_empty())
            .collect();
        let ids: Vec<BufferId> = self.ed.buffers.keys().collect();
        for id in ids {
            for m in self.masks_of(id) {
                let hidden = self.ed.buffers[id].slice(m);
                let hit = text.contains(hidden.trim()) && !hidden.trim().is_empty()
                    || hidden
                        .split(|c: char| !kawoosh_systems::words::is_word_char(c))
                        .any(|w| !w.is_empty() && words.contains(w));
                if hit {
                    return true;
                }
            }
        }
        false
    }

    /// The variables of the place `r` (the word replaced) at caret `k`,
    /// the texts a mask could hide read only where `body` names them.
    fn snippet_vars_at(
        &mut self,
        shared: &Vars,
        buffer: BufferId,
        view: ViewId,
        r: Range<usize>,
        k: usize,
        body: &str,
    ) -> Vars {
        let buf = &self.ed.buffers[buffer];
        let private = buf.private;
        let ln = buf.line_of(r.start);
        let line_range = buf.line_range(ln);
        let line = buf.slice(line_range.clone());
        let word = {
            let ls = line_range.start;
            let at = r.end.clamp(ls, line_range.end) - ls;
            let start = line[..at]
                .char_indices()
                .rev()
                .take_while(|(_, c)| kawoosh_systems::words::is_word_char(*c))
                .last()
                .map_or(at, |(i, _)| i);
            let end = at
                + line[at..]
                    .char_indices()
                    .find(|(_, c)| !kawoosh_systems::words::is_word_char(*c))
                    .map_or(line.len() - at, |(i, _)| i);
            (ls + start..ls + end, line[start..end].to_string())
        };
        let selected = self
            .ed
            .views
            .get(view)
            .and_then(|v| v.sels.items.get(k).copied())
            .filter(|s| !s.is_empty())
            .map(|s| (s.range(), buf.slice(s.range())));
        let named = |v: &str| body.contains(v);
        let (line, word, selected) = (
            named("TM_CURRENT_LINE").then_some(line),
            named("TM_CURRENT_WORD").then_some(word),
            selected.filter(|_| named("TM_SELECTED_TEXT")),
        );
        // A text of the buffer is put only where no mask covers it.
        let masks = match line.is_some() || word.is_some() || selected.is_some() {
            true => self.masks_of(buffer),
            false => Vec::new(),
        };
        let mut text = |range: Range<usize>, t: String| -> String {
            let masked = masks
                .iter()
                .any(|m| m.start < range.end.max(range.start + 1) && range.start < m.end);
            if private || masked || self.snippet_hides(buffer, &t) {
                String::new()
            } else {
                t
            }
        };
        let line = line.map(|l| text(line_range, l)).unwrap_or_default();
        let word = word.map(|(r, w)| text(r, w)).unwrap_or_default();
        let selected = selected.map(|(r, t)| text(r, t)).unwrap_or_default();
        Vars {
            filename: shared.filename.clone(),
            directory: shared.directory.clone(),
            filepath: shared.filepath.clone(),
            relative: shared.relative.clone(),
            workspace: shared.workspace.clone(),
            clipboard: shared.clipboard.clone(),
            line_comment: shared.line_comment.clone(),
            block: shared.block.clone(),
            selected,
            line,
            word,
            line_index: ln,
            cursor: k,
        }
    }
}

/// The snippet's commands; their keys are local to the `snippet` fact
/// (editor/src/commands.rs).
pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("snippet next")
                .when(&[FACT])
                .doc("the live snippet's next field; at its last, the snippet done"),
            |k, _| k.snippet_jump(true),
        ),
        cmd(
            Spec::new("snippet prev")
                .when(&[FACT])
                .doc("the live snippet's field before"),
            |k, _| k.snippet_jump(false),
        ),
    ]
}

#[cfg(test)]
// A field of one place is what these mean.
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;

    #[test]
    fn the_civil_date_and_weekday() {
        // 2026-10-17 12:00 UTC, a Saturday.
        assert_eq!(civil(1_792_238_400), (2026, 10, 17, 6));
        assert_eq!(civil(0), (1970, 1, 1, 4));
    }

    #[test]
    fn a_uuid_is_version_four() {
        let v = Vars {
            filename: String::new(),
            directory: String::new(),
            filepath: String::new(),
            relative: String::new(),
            workspace: String::new(),
            clipboard: String::new(),
            line_comment: String::new(),
            block: Default::default(),
            selected: String::new(),
            line: String::new(),
            word: String::new(),
            line_index: 0,
            cursor: 0,
        };
        let u = v.get("UUID").unwrap();
        assert_eq!(u.len(), 36, "{u}");
        assert_eq!(&u[14..15], "4", "{u}");
        assert!(matches!(&u[19..20], "8" | "9" | "a" | "b"), "{u}");
        assert_eq!(v.get("RANDOM").unwrap().len(), 6);
        assert_eq!(v.get("RANDOM_HEX").unwrap().len(), 6);
        assert_eq!(v.get("NOPE"), None);
    }

    #[test]
    fn fields_follow_typing_without_spilling() {
        let f = |ranges: Vec<Range<usize>>| LiveField {
            index: 1,
            ranges,
            choices: None,
            mirrors: Vec::new(),
            parent: None,
            visited: false,
            gone: false,
        };
        // `${1:a}${2:b}` with field 1 being filled: typing at its end.
        let mut live = Live {
            view: ViewId::default(),
            buffer: BufferId::default(),
            seen: Version::INITIAL,
            moves: 0,
            spans: vec![0..2],
            fields: vec![f(vec![0..1]), f(vec![1..2]), f(vec![2..2])],
            at: 0,
        };
        let ins = |at: usize, n: usize| Edit {
            range: at..at,
            new_len: n,
        };
        assert!(live.follow(&[ins(1, 3)], Some(0)));
        assert_eq!(live.fields[0].ranges, [0..4]);
        assert_eq!(live.fields[1].ranges, [4..5]);
        assert_eq!(live.fields[2].ranges, [5..5]);
        assert_eq!(live.spans, [0..5]);
        // Outside it: refused.
        assert!(!live.follow(&[ins(9, 1)], Some(0)));
    }
}
