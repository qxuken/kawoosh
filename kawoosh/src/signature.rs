//! Signature help (docs/design/completion.md Decision 10's second
//! half): the signature of the call the caret is in, asked of the
//! server (`textDocument/signatureHelp`) and shown in the one
//! information panel — the parameter the caret is in lit, its
//! documentation's first paragraph and the signature's under it.
//!
//! **When** (`completion.signature`): `manual` on `signature` (`<C-g>`
//! in insert mode, again to put it away); `auto` also as the server's
//! trigger characters are typed (`(` and `,` for one that names none);
//! `off` never.
//!
//! **While it is on show** a [`Live`] follows the caret: each key that
//! moves it or edits the text asks again (one ask in flight; an answer
//! for a place the caret left, or a version that moved, is dropped as
//! it lands and the place the caret is at asked instead), so the lit
//! parameter follows the arguments as `,` is typed. It is gone when the
//! caret leaves the parentheses it was opened in — the calls entered on
//! the way in are kept, so leaving an inner call goes back to the outer
//! one — on leaving insert mode or the pane, on `signature close`
//! (`<C-e>`), and when the server knows of no call there.
//!
//! **One panel** (Decision 10): the selected candidate's documentation
//! while the popup is walked (`<C-n>` / `<C-p>`, or `completion docs`),
//! the signature otherwise; with no popup it sits above the caret's
//! line. Its overloads are `signature next` / `prev` (`<C-n>` / `<C-p>`)
//! while no completion is on show.
//!
//! **`completion.callable = "args"`** puts a function's brackets with
//! its parameters as snippet fields: the server's own snippet where it
//! sends one with brackets, else `name()` with the carets inside and
//! the parameters filled in from the signature once the server says it
//! — only where nothing moved since ([`Fill`]).

use std::ops::Range;
use std::rc::Rc;

use kawoosh_doc::{BufferId, Version};
use kawoosh_editor::{Mode, Selection, Spec, ViewId};
use kawoosh_systems::lsp::{Cmd, SignatureContext, SignatureHelp};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::completion_view::Panel;

/// The fact that holds while a signature is drawn and no completion is
/// on show: its keys' place (local-maps.md).
pub const FACT: &str = "signature";

/// How far before the caret a call's `(` is looked for, and how long a
/// call may be: past it, the caret has left it.
const SCAN: usize = 64 << 10;

/// `completion.signature`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    Manual,
    Auto,
    Off,
}

/// A signature on show, or asked for, in one view.
pub struct Live {
    pub view: ViewId,
    pub buffer: BufferId,
    /// The `(` of each call the caret is in since it opened, outermost
    /// first: the caret leaving the last goes back to the one before,
    /// leaving the first ends it.
    pub opens: Vec<usize>,
    /// The version and the caret it was last brought to.
    seen: (Version, usize),
    /// Opened by `signature`, not by typing: "no signature here" said.
    manual: bool,
    /// The server's last answer, for the place the caret was at.
    pub help: Option<Rc<SignatureHelp>>,
    /// The overload on show, and whether the user chose it (kept over
    /// the server's through its answers).
    pub active: usize,
    chosen: bool,
    /// The panel, built once an answer or a choice: masked once.
    pub panel: Option<Rc<Panel>>,
}

/// A function accepted with `completion.callable = "args"`: `name()`
/// put with the carets inside, its parameters filled in once the
/// server says its signature — where the text and the carets are still
/// as they were left.
pub struct Fill {
    view: ViewId,
    buffer: BufferId,
    version: Version,
    /// Every caret, between the brackets, as the selections hold them.
    carets: Vec<usize>,
}

/// A signature's panel as drawn, for a test: its head, the parameter
/// lit in it, which of how many, its body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    pub head: String,
    pub lit: Option<String>,
    pub of: Option<(usize, usize)>,
    pub body: String,
}

/// What signature help keeps between keys.
#[derive(Default)]
pub struct State {
    pub live: Option<Live>,
    fill: Option<Fill>,
    /// The one ask in flight: the buffer, the offset, the version.
    asking: Option<(BufferId, usize, Version)>,
    /// How many were asked: a test's count.
    pub asks: usize,
}

/// The `(` of the innermost call `caret` is in: the last unmatched one
/// within [`SCAN`] before it.
pub(crate) fn innermost_open(text: &[u8], base: usize, caret: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, &b) in text[..caret - base].iter().enumerate().rev() {
        match b {
            b')' => depth += 1,
            b'(' if depth == 0 => return Some(base + i),
            b'(' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// Whether the call opened at `open` holds the caret: nothing between
/// them closes it.
fn encloses(between: &[u8]) -> bool {
    let mut depth = 0isize;
    for &b in between {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    true
}

/// A documentation's first paragraph: its code blocks left out (the
/// signature is the head already), up to the first blank line.
pub(crate) fn first_paragraph(doc: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut fenced = false;
    for line in doc.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            if !out.is_empty() {
                break;
            }
            continue;
        }
        if fenced {
            continue;
        }
        if line.trim().is_empty() {
            if out.is_empty() {
                continue;
            }
            break;
        }
        out.push(line.trim_end());
    }
    out.join("\n")
}

/// A parameter's name as a snippet field's default: before its type or
/// default where the label says one (`a: i32`, `b?: string`, `c=3`,
/// `mut d: u8`), else the label as the server wrote it (`int a`, `a
/// int`) — which language puts the name where is not kawoosh's to know.
pub(crate) fn param_name(label: &str) -> String {
    let l = label.trim();
    let Some(cut) = l.find([':', '=']) else {
        return l.to_string();
    };
    let name = l[..cut]
        .split_whitespace()
        .last()
        .unwrap_or("")
        .trim_end_matches('?');
    match name.is_empty() {
        true => l.to_string(),
        false => name.to_string(),
    }
}

/// `text` as a snippet's plain text: its `$`, `}` and `\` escaped.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '$' | '}' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Whether a candidate of `kind` is called: a function's, a method's,
/// a constructor's (`completion.callable`).
pub(crate) fn callable_kind(kind: &str) -> bool {
    matches!(kind, "function" | "method" | "constructor")
}

/// A called candidate's name: what its text has before its first `(`.
pub(crate) fn call_name(text: &str) -> Option<&str> {
    let name = text[..text.find('(')?].trim_end();
    (!name.is_empty()).then_some(name)
}

impl Kawoosh {
    /// `completion.signature` for `view`.
    pub(crate) fn signature_when(&self, view: ViewId) -> When {
        match self
            .completion_word(view, "completion.signature")
            .as_deref()
        {
            Some("auto") => When::Auto,
            Some("off") => When::Off,
            _ => When::Manual,
        }
    }

    /// `completion.callable` for `view`: whether a function's
    /// parameters are put (`args`), not its name alone.
    pub(crate) fn callable_args(&self, view: ViewId) -> bool {
        self.completion_word(view, "completion.callable").as_deref() == Some("args")
    }

    /// The server's characters that ask for a signature as they are
    /// typed, and those that ask again while one is on show — `(` and
    /// `,` for a server that names none.
    fn signature_triggers(&self, buffer: BufferId) -> (Vec<String>, Vec<String>) {
        let caps = self.lsp.caps.get(self.ed.buffers[buffer].language.as_ref());
        let triggers = caps
            .map(|c| c.signature_triggers.clone())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| vec!["(".into(), ",".into()]);
        let retriggers = caps
            .map(|c| c.signature_retriggers.clone())
            .unwrap_or_default();
        (triggers, retriggers)
    }

    /// Whether `buffer`'s server is there to ask for a signature: not a
    /// private buffer's (it is never sent), and a server that says it
    /// does — or has not said yet.
    fn signature_askable(&self, buffer: BufferId) -> bool {
        !self.ed.buffers[buffer].private
            && self.lsp_answers(buffer)
            && self.caps_of(buffer).signature
    }

    /// The bytes of `buffer` from `from` to `to`, at most [`SCAN`] of
    /// them back from `to`, and where they start.
    fn signature_text(&self, buffer: BufferId, from: usize, to: usize) -> (Vec<u8>, usize) {
        let buf = &self.ed.buffers[buffer];
        let to = to.min(buf.len());
        let start = buf.floor_char(from.max(to.saturating_sub(SCAN)));
        (buf.slice(start..to).into_bytes(), start)
    }

    // --------------------------------------------------------- asking

    /// `signature`: the signature of the call at the caret, asked for —
    /// or, one on show, put away.
    fn signature_toggle(&mut self) {
        if self.signature.live.take().is_some() {
            return;
        }
        if self.pane_mode() != Mode::Insert {
            return;
        }
        let Some(view) = self.focused_view() else {
            return;
        };
        if self.signature_when(view) == When::Off {
            self.ed.message = "signature help is off here (completion.signature)".into();
            return;
        }
        let buffer = self.ed.views[view].buffer;
        if !self.signature_askable(buffer) {
            self.ed.message = "no signature help here".into();
            return;
        }
        if !self.signature_open(view, true) {
            self.ed.message = "not inside a call".into();
            return;
        }
        self.signature_ask(
            SignatureContext {
                trigger_kind: 1,
                trigger_character: None,
                is_retrigger: false,
                active: None,
            },
            true,
        );
    }

    /// A signature opened at `view`'s caret, for the call it is in;
    /// false when it is in none.
    fn signature_open(&mut self, view: ViewId, manual: bool) -> bool {
        let buffer = self.ed.views[view].buffer;
        let caret = self.ed.views[view].sels.primary().head;
        let (text, base) = self.signature_text(buffer, 0, caret);
        let Some(open) = innermost_open(&text, base, caret) else {
            return false;
        };
        self.signature.live = Some(Live {
            view,
            buffer,
            opens: vec![open],
            seen: (self.ed.buffers[buffer].version(), caret),
            manual,
            help: None,
            active: 0,
            chosen: false,
            panel: None,
        });
        true
    }

    /// The server asked for the signature at the focused caret. One ask
    /// in flight: while one is, a key asks nothing — its answer lands
    /// for a place the caret left, and the place it is at is asked then.
    /// `fresh` (an open, a fill) asks whatever is in flight.
    fn signature_ask(&mut self, context: SignatureContext, fresh: bool) {
        let Some(view) = self.focused_view() else {
            return;
        };
        let buffer = self.ed.views[view].buffer;
        if !self.signature_askable(buffer) {
            return;
        }
        if self.signature.asking.is_some() && !fresh {
            return;
        }
        let offset = self.ed.views[view].sels.primary().head;
        let version = self.ed.buffers[buffer].version();
        self.signature.asking = Some((buffer, offset, version));
        self.signature.asks += 1;
        self.positional_cmd(Cmd::SignatureHelp {
            buffer,
            offset,
            version,
            context,
        });
    }

    /// What the next ask carries: asked again while one is on show, the
    /// key's character where it is a trigger, the server's last answer
    /// with the overload on show.
    fn signature_context(&self, typed: Option<&str>) -> SignatureContext {
        let live = self.signature.live.as_ref();
        let character = typed.filter(|t| {
            live.is_some_and(|l| {
                let (t1, t2) = self.signature_triggers(l.buffer);
                t1.iter().chain(t2.iter()).any(|x| x == t)
            })
        });
        let active = live.and_then(|l| {
            let h = l.help.as_ref()?;
            let mut raw = (*h.raw).clone();
            raw["activeSignature"] = serde_json::json!(l.active);
            Some(std::sync::Arc::new(raw))
        });
        SignatureContext {
            trigger_kind: if character.is_some() { 2 } else { 3 },
            trigger_character: character.map(str::to_string),
            is_retrigger: live.is_some_and(|l| l.help.is_some()),
            active,
        }
    }

    /// A signature answered — handed in by a test as by the server. Only
    /// the ask in flight is read, and only for the place the caret is
    /// still at: an answer for a place it left, or a version that
    /// moved, is dropped as it lands, and the place it is at is asked.
    pub fn signature_answered(
        &mut self,
        buffer: BufferId,
        version: Version,
        offset: usize,
        help: Result<Option<SignatureHelp>, String>,
    ) {
        if self.signature.asking != Some((buffer, offset, version)) {
            return;
        }
        self.signature.asking = None;
        let here = self.focused_view().is_some_and(|v| {
            self.ed.views[v].buffer == buffer
                && self.ed.views[v].sels.primary().head == offset
                && self.ed.buffers[buffer].version() == version
        });
        let fill = self
            .signature
            .fill
            .take()
            .filter(|f| here && f.buffer == buffer && f.version == version);
        if self
            .signature
            .live
            .as_ref()
            .is_some_and(|l| l.buffer == buffer)
        {
            if !here {
                let context = self.signature_context(None);
                self.signature_ask(context, false);
            } else {
                match &help {
                    Err(e) => log::debug!("signature help: {e}"),
                    Ok(None) => {
                        let l = self.signature.live.take().unwrap();
                        if l.manual && l.help.is_none() {
                            self.ed.message = "no signature here".into();
                        }
                    }
                    Ok(Some(h)) => {
                        let l = self.signature.live.as_mut().unwrap();
                        l.active = match l.chosen && l.active < h.signatures.len() {
                            true => l.active,
                            false => h.active_signature,
                        };
                        l.help = Some(Rc::new(h.clone()));
                        self.signature_build_panel();
                    }
                }
            }
        }
        if let (Some(f), Ok(Some(h))) = (fill, &help) {
            self.signature_fill(f, h);
        }
    }

    /// The ask in flight, for a test that hands its answer in: the
    /// buffer, the offset, the version.
    pub fn signature_asking(&self) -> Option<(BufferId, usize, Version)> {
        self.signature.asking
    }

    /// The servers no longer hold `buffer`: the ask in flight for it is
    /// not waited on.
    pub(crate) fn signature_lsp_gone(&mut self, buffer: BufferId) {
        if self.signature.asking.is_some_and(|(b, _, _)| b == buffer) {
            self.signature.asking = None;
        }
    }

    // ------------------------------------------------------- following

    /// After a key: the signature in step with the caret — asked again
    /// where it moved or the text changed, gone where it left the call —
    /// or, with `auto`, opened by a trigger character typed into the
    /// text.
    pub(crate) fn signature_after_key(&mut self, typed: Option<&str>, was_insert: bool) {
        self.signature_follow(typed);
        let Some(view) = self.focused_view() else {
            return;
        };
        if self.signature.live.is_some()
            || !was_insert
            || self.pane_mode() != Mode::Insert
            || self.signature_when(view) != When::Auto
        {
            return;
        }
        let Some(t) = typed else {
            return;
        };
        let buffer = self.ed.views[view].buffer;
        if !self.signature_triggers(buffer).0.iter().any(|x| x == t)
            || !self.signature_askable(buffer)
        {
            return;
        }
        if self.signature_open(view, false) {
            self.signature_ask(
                SignatureContext {
                    trigger_kind: 2,
                    trigger_character: Some(t.to_string()),
                    is_retrigger: false,
                    active: None,
                },
                true,
            );
        }
    }

    /// Per frame: what moved by no key (a server's edit, a reload)
    /// followed, and a signature whose pane lost the keys gone.
    pub(crate) fn sync_signature(&mut self) {
        self.signature_follow(None);
    }

    /// The signature brought to the caret: its calls carried through
    /// the edits since, those the caret left let go — all of them ends
    /// it — one it went into kept; asked again where anything moved.
    fn signature_follow(&mut self, typed: Option<&str>) {
        let focused = self.focused_view();
        let insert = self.pane_mode() == Mode::Insert;
        if let Some(f) = &self.signature.fill {
            let still = focused == Some(f.view)
                && self.ed.views.get(f.view).is_some_and(|v| {
                    v.buffer == f.buffer
                        && v.sels.iter().map(|s| s.head).eq(f.carets.iter().copied())
                })
                && self
                    .ed
                    .buffers
                    .get(f.buffer)
                    .is_some_and(|b| b.version() == f.version);
            if !still {
                self.signature.fill = None;
            }
        }
        let Some(l) = &self.signature.live else {
            return;
        };
        let alive = insert
            && focused == Some(l.view)
            && self
                .ed
                .views
                .get(l.view)
                .is_some_and(|v| v.buffer == l.buffer)
            && self.ed.buffers.get(l.buffer).is_some_and(|b| !b.private);
        if !alive {
            self.signature.live = None;
            return;
        }
        let buf = &self.ed.buffers[l.buffer];
        let caret = self.ed.views[l.view].sels.primary().head;
        let version = buf.version();
        if (version, caret) == l.seen {
            return;
        }
        // Each `(` carried through the edits, kept while it is one.
        let mut opens: Vec<usize> = if version == l.seen.0 {
            l.opens.clone()
        } else {
            l.opens
                .iter()
                .filter_map(|&o| {
                    buf.journal()
                        .transform_offset(o, l.seen.0, kawoosh_doc::Bias::Right)
                        .ok()
                })
                .filter(|&o| o < buf.len() && buf.slice(o..o + 1) == "(")
                .collect()
        };
        let buffer = l.buffer;
        while let Some(&o) = opens.last() {
            let inside = caret > o && caret - o <= SCAN && {
                let (between, _) = self.signature_text(buffer, o + 1, caret);
                encloses(&between)
            };
            if inside {
                break;
            }
            opens.pop();
        }
        let Some(&top) = opens.last() else {
            self.signature.live = None;
            return;
        };
        // A call the caret went into since: kept, so leaving it comes
        // back to this one.
        let (text, base) = self.signature_text(buffer, top + 1, caret);
        if let Some(inner) = innermost_open(&text, base, caret)
            && inner > top
        {
            opens.push(inner);
        }
        let context = self.signature_context(typed);
        let l = self.signature.live.as_mut().unwrap();
        l.opens = opens;
        l.seen = (version, caret);
        self.signature_ask(context, false);
    }

    // ---------------------------------------------------------- shown

    /// The panel for the overload on show: its label with the parameter
    /// the caret is in lit, which of several it is, the parameter's
    /// documentation's first paragraph and the signature's — every text
    /// masked as the buffer's rules and every open buffer's masks say
    /// (secrets.md Decision 9), built once, not each frame.
    fn signature_build_panel(&mut self) {
        let Some(l) = &self.signature.live else {
            return;
        };
        let Some(h) = l.help.clone() else {
            return;
        };
        let Some(sig) = h.signatures.get(l.active) else {
            return;
        };
        let buffer = l.buffer;
        let n = h.signatures.len();
        // The server's parameter is its active signature's: another
        // overload chosen reads its own, else the same place.
        let param = sig.active_parameter.or(h.active_parameter);
        let p = param.and_then(|i| sig.parameters.get(i));
        let lit = p.and_then(|p| p.range.clone());
        let mut body: Vec<String> = Vec::new();
        for doc in [
            p.and_then(|p| p.documentation.as_deref()),
            sig.documentation.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            let para = first_paragraph(doc);
            if !para.is_empty() {
                body.push(para);
            }
        }
        let body = body.join("\n\n");
        let label = sig.label.clone();
        let words = self.masked_words_now();
        let head = self.signature_mask(buffer, &label, &words);
        // Lit where the masks left the label as it is, or left each
        // piece as they leave the whole.
        let lit = lit.filter(|r| r.end <= label.len()).and_then(|r| {
            if head == label {
                return Some(r);
            }
            let a = self.signature_mask(buffer, &label[..r.start], &words);
            let b = self.signature_mask(buffer, &label[r.clone()], &words);
            let c = self.signature_mask(buffer, &label[r.end..], &words);
            (format!("{a}{b}{c}") == head).then(|| a.len()..a.len() + b.len())
        });
        let body = self.signature_mask(buffer, &body, &words);
        let l = self.signature.live.as_mut().unwrap();
        l.panel = Some(Rc::new(Panel {
            head: Some(head),
            lit,
            of: (n > 1).then_some((l.active + 1, n)),
            body,
            scroll: std::cell::Cell::new(0),
            room: std::cell::Cell::new(0),
        }));
    }

    /// The words inside any open buffer's masks now (secrets.md
    /// Decisions 3 and 9): the completion's indexes keep only the
    /// buffers they indexed, and a signature needs no completion.
    fn masked_words_now(&mut self) -> std::collections::HashSet<String> {
        let mut out = std::collections::HashSet::new();
        let ids: Vec<BufferId> = self.ed.buffers.keys().collect();
        for id in ids {
            for m in self.masks_of(id) {
                let hidden = self.ed.buffers[id].slice(m);
                out.extend(
                    hidden
                        .split(|c: char| !kawoosh_systems::words::is_word_char(c))
                        .filter(|w| !w.is_empty())
                        .map(str::to_string),
                );
            }
        }
        out
    }

    /// `text` as a panel about `buffer` may draw it: what its rules
    /// mask, and every word inside a mask of an open buffer, as `•`.
    fn signature_mask(
        &mut self,
        buffer: BufferId,
        text: &str,
        words: &std::collections::HashSet<String>,
    ) -> String {
        let (path, language) = self
            .ed
            .buffers
            .get(buffer)
            .map(|b| (b.path.clone(), b.language.to_string()))
            .unwrap_or_default();
        let t = self.mask_text(path.as_deref(), &language, text);
        let t = crate::completion::mask_words(&self.completion.masked, &t);
        crate::completion::mask_words_by(&t, |w| words.contains(w))
    }

    /// The signature's panel `view` draws this frame, where one is on
    /// show for it.
    pub(crate) fn signature_panel(&self, view: ViewId) -> Option<Rc<Panel>> {
        let l = self.signature.live.as_ref()?;
        if l.view != view || self.focused_view() != Some(view) || self.pane_mode() != Mode::Insert {
            return None;
        }
        l.panel.clone()
    }

    /// The one information panel `view` draws this frame (Decision 10),
    /// and whether it is the signature: the selected candidate's
    /// documentation while the popup is walked or the documentation was
    /// asked for, the signature otherwise, else the documentation.
    pub(crate) fn info_panel(&self, view: ViewId) -> Option<(Rc<Panel>, bool)> {
        let docs = self.completion_panel(view);
        let walking = docs.is_some()
            && self
                .completion
                .session
                .as_ref()
                .is_some_and(|s| s.walking || s.docs.pinned);
        if walking {
            return docs.map(|p| (p, false));
        }
        match self.signature_panel(view) {
            Some(p) => Some((p, true)),
            None => docs.map(|p| (p, false)),
        }
    }

    /// Where the signature alone hangs in `view`: the start of the
    /// call's name where its `(` is on the caret's line, else the
    /// caret's line's text.
    pub(crate) fn signature_hang(&self, view: ViewId) -> Option<usize> {
        let l = self.signature.live.as_ref()?;
        let buf = &self.ed.buffers[l.buffer];
        let caret = self.ed.views[view].sels.primary().head;
        let ln = buf.line_of(caret);
        let ls = buf.line_start(ln);
        let open = *l.opens.last()?;
        if open >= ls && open <= caret {
            let before = buf.slice(ls..open);
            return Some(ls + crate::completion::ident_start(before.trim_end()));
        }
        let line = buf.slice(ls..caret);
        Some(ls + (line.len() - line.trim_start().len()))
    }

    /// Whether the signature is on show where the keys are, with no
    /// completion over it: the `signature` fact.
    pub(crate) fn signature_on_show(&self) -> bool {
        self.signature_shown
            && !self.completion_on_show()
            && self
                .focused_view()
                .is_some_and(|v| self.signature_panel(v).is_some())
    }

    /// The signature's panel as drawn, for a test.
    pub fn signature_text_shown(&self) -> Option<Shown> {
        let v = self.focused_view()?;
        let (p, sig) = self.info_panel(v)?;
        if !sig {
            return None;
        }
        let head = p.head.clone().unwrap_or_default();
        let lit = p.lit.clone().and_then(|r| head.get(r).map(str::to_string));
        Some(Shown {
            head,
            lit,
            of: p.of,
            body: p.body.clone(),
        })
    }

    // ----------------------------------------------------------- keys

    /// `signature next` / `prev`: the next overload, round.
    fn signature_step(&mut self, forward: bool) {
        let Some(l) = self.signature.live.as_mut() else {
            return;
        };
        let Some(n) = l.help.as_ref().map(|h| h.signatures.len()) else {
            return;
        };
        if n < 2 {
            return;
        }
        l.active = if forward {
            (l.active + 1) % n
        } else {
            (l.active + n - 1) % n
        };
        l.chosen = true;
        self.signature_build_panel();
    }

    /// `signature close`.
    fn signature_close(&mut self) {
        self.signature.live = None;
    }

    // ----------------------------------------------- callable = "args"

    /// A function accepted with `completion.callable = "args"`, its name
    /// put already in place of the word — at each caret (`every`), else
    /// at the primary one: its brackets after it — unless the text goes
    /// on with one — the carets inside, and its parameters asked of the
    /// server to be filled in. With `auto`, the signature opens as for a
    /// `(` typed.
    pub(crate) fn signature_call(&mut self, view: ViewId, buffer: BufferId, every: bool) {
        let buf = &self.ed.buffers[buffer];
        let caret = self.ed.views[view].sels.primary().head;
        if buf
            .slice(caret..buf.floor_char((caret + 4).min(buf.len())))
            .starts_with('(')
        {
            return;
        }
        if every {
            self.ed.insert_text(view, "()");
            self.ed.views[view]
                .sels
                .map(|s| Selection::point(s.head.saturating_sub(1)));
        } else {
            self.ed.apply_edits(buffer, &[(caret..caret, "()".into())]);
            let sels = &mut self.ed.views[view].sels;
            let p = sels.primary;
            sels.items[p] = Selection::point(caret + 1);
        }
        self.follow_caret = true;
        if !self.signature_askable(buffer) {
            return;
        }
        self.signature.fill = Some(Fill {
            view,
            buffer,
            version: self.ed.buffers[buffer].version(),
            carets: self.ed.views[view].sels.iter().map(|s| s.head).collect(),
        });
        if self.signature_when(view) == When::Auto && self.signature.live.is_none() {
            self.signature_open(view, false);
        }
        self.signature_ask(
            SignatureContext {
                trigger_kind: 2,
                trigger_character: Some("(".into()),
                is_retrigger: false,
                active: None,
            },
            true,
        );
    }

    /// The parameters of the signature the server said for an accepted
    /// function, as snippet fields between its brackets at every caret,
    /// `$0` after them; none, the carets after the brackets.
    fn signature_fill(&mut self, f: Fill, h: &SignatureHelp) {
        let Some(sig) = h.signatures.get(h.active_signature) else {
            return;
        };
        let buf = &self.ed.buffers[f.buffer];
        let closes = f
            .carets
            .iter()
            .all(|&c| c < buf.len() && buf.slice(c..c + 1) == ")");
        if !closes {
            return;
        }
        let mut ranges: Vec<Range<usize>> = f.carets.iter().map(|&c| c..c + 1).collect();
        ranges.sort_by_key(|r| r.start);
        if sig.parameters.is_empty() {
            self.ed.views[f.view]
                .sels
                .map(|s| Selection::point(s.head + 1));
            return;
        }
        let fields: Vec<String> = sig
            .parameters
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let label = match p.range.clone() {
                    Some(r) => sig.label.get(r).unwrap_or(&p.label).to_string(),
                    None => p.label.clone(),
                };
                format!("${{{}:{}}}", i + 1, escape(&param_name(&label)))
            })
            .collect();
        let body = format!("{})$0", fields.join(", "));
        self.snippet_accept(f.view, f.buffer, &ranges, &body, true);
    }
}

/// The signature's commands: `signature` bound in insert mode, the rest
/// local to the `signature` fact (editor/src/commands.rs).
pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("signature").doc(
                "the signature of the call the caret is in, its parameter lit; again, put away",
            ),
            |k, _| k.signature_toggle(),
        ),
        cmd(
            Spec::new("signature next")
                .when(&[FACT])
                .doc("the signature's next overload"),
            |k, _| k.signature_step(true),
        ),
        cmd(
            Spec::new("signature prev")
                .when(&[FACT])
                .doc("the signature's previous overload"),
            |k, _| k.signature_step(false),
        ),
        cmd(
            Spec::new("signature close")
                .when(&[FACT])
                .doc("the signature put away"),
            |k, _| k.signature_close(),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_innermost_call_is_the_last_open_bracket() {
        let t = b"f(a, g(b), h(";
        assert_eq!(innermost_open(t, 0, t.len()), Some(12));
        let t = b"f(a, g(b), c";
        assert_eq!(innermost_open(t, 0, t.len()), Some(1));
        let t = b"f(a) + b";
        assert_eq!(innermost_open(t, 0, t.len()), None);
        assert_eq!(innermost_open(b"xx f(", 100, 105), Some(104));
        assert!(encloses(b"a, g(b), c"));
        assert!(!encloses(b"a) + b"));
    }

    #[test]
    fn a_parameter_is_named_where_its_label_says_so() {
        assert_eq!(param_name("a: i32"), "a");
        assert_eq!(param_name("mut b: &str"), "b");
        assert_eq!(param_name("c?: string"), "c");
        assert_eq!(param_name("d=3"), "d");
        assert_eq!(param_name("int e"), "int e");
        assert_eq!(param_name("f int"), "f int");
        assert_eq!(param_name("*args"), "*args");
        assert_eq!(escape("a$b}c\\"), "a\\$b\\}c\\\\");
        assert_eq!(call_name("foo(a, b)"), Some("foo"));
        assert_eq!(call_name("foo"), None);
        assert_eq!(call_name("(x)"), None);
    }

    #[test]
    fn the_first_paragraph_leaves_code_out() {
        assert_eq!(
            first_paragraph("Does f.\nTwice.\n\nMore."),
            "Does f.\nTwice."
        );
        assert_eq!(
            first_paragraph("```rust\nfn f()\n```\n\nDoes f.\n\nMore."),
            "Does f."
        );
        assert_eq!(first_paragraph(""), "");
    }
}
