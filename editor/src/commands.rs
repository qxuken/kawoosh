//! The built-in commands and the default keymap. Every command is written
//! over the whole selection set; vim's operator + motion notation drives
//! it (mvp.md Decision 4: "neovim-style keymaps drive a selection-set
//! engine fine"): an operator waits, the next motion or text object
//! extends every selection, and the operator applies to all of them.

use std::ops::Range;

use kawoosh_doc::Buffer;

use crate::keymap::{Keymap, Mode};
use crate::motions as m;
use crate::{
    ArgKind, Args, CharArg, Ctx, Editor, Effect, Kind, Layer, MotionKind, Prompt, Selection,
    Setting, Spec, ViewId,
};

/// `:set`'s line split into its path and its value: the value is past
/// a `=` or the path's first space, whichever comes first
/// (`font.family=Iosevka`, `font.family Iosevka`); none for a flag, a
/// `PATH?` or a `PATH!`. The command line completes the value from the
/// same split.
pub fn set_value(line: &str) -> Option<(&str, &str)> {
    let i = line.find(|c: char| c == '=' || c.is_whitespace())?;
    let (path, rest) = line.split_at(i);
    Some(match rest.strip_prefix('=') {
        Some(v) => (path, v),
        None => (path, rest.trim_start()),
    })
}

fn view<'a>(ed: &'a Editor, ctx: &Ctx) -> &'a crate::View {
    &ed.views[ctx.view]
}

fn extend(ed: &Editor, view: ViewId) -> bool {
    ed.mode(view) == Mode::Visual || ed.pending_op.is_some()
}

/// Moves every head by `f(buf, head, count)`; the anchor follows unless
/// extending.
fn motion(ed: &mut Editor, ctx: &Ctx, f: impl Fn(&Buffer, usize, usize) -> usize) {
    let ext = extend(ed, ctx.view);
    let id = view(ed, ctx).buffer;
    let buf = &ed.buffers[id];
    let v = &mut ed.views[ctx.view];
    v.sels
        .map(|s| s.with_head(f(buf, s.head, ctx.count).min(buf.len()), ext));
    v.goal_col = None;
}

fn vertical(ed: &mut Editor, ctx: &Ctx, dy: i64) {
    let ext = extend(ed, ctx.view);
    let id = view(ed, ctx).buffer;
    let buf = &ed.buffers[id];
    let v = &mut ed.views[ctx.view];
    let goal = v.goal_col;
    let mut new_goal = goal;
    let primary = v.sels.primary;
    let mut items = v.sels.items.clone();
    for (i, s) in items.iter_mut().enumerate() {
        let (ln, col) = m::line_col(buf, s.head);
        let col = if i == primary {
            goal.unwrap_or(col)
        } else {
            col
        };
        if i == primary {
            new_goal = Some(col);
        }
        let last = buf.line_count() as i64 - 1;
        let target = (ln as i64 + dy * ctx.count as i64).clamp(0, last) as usize;
        let head = m::offset_at(buf, target, col);
        *s = s.with_head(head, ext);
    }
    v.sels.items = items;
    v.sels.normalize();
    v.goal_col = new_goal;
}

fn clamp_sels(ed: &mut Editor, view: ViewId) {
    let id = ed.views[view].buffer;
    let len = ed.buffers[id].len();
    ed.views[view]
        .sels
        .map(|s| Selection::new(s.anchor.min(len), s.head.min(len)));
}

fn line_range_of_sel(buf: &Buffer, s: &Selection, extra_lines: usize) -> Range<usize> {
    let a = buf.line_of(s.start());
    let b = buf.line_of(s.end()) + extra_lines;
    let b = b.min(buf.line_count() - 1);
    let start = buf.line_start(a);
    let end = buf.line_range(b).end;
    // Take the line break too — `\r\n` whole — unless it is the last
    // line, then the one before it.
    if end < buf.len() {
        start..buf.line_start(b + 1)
    } else if start > 0 && a > 0 {
        buf.line_range(a - 1).end..end
    } else {
        start..end
    }
}

/// The lines a selection is on, and `extra_lines` after them, from the
/// first's start to the last's end — no newline on either side.
fn whole_lines_of_sel(buf: &Buffer, s: &Selection, extra_lines: usize) -> Range<usize> {
    let a = buf.line_of(s.start());
    let b = (buf.line_of(s.end()) + extra_lines).min(buf.line_count() - 1);
    buf.line_start(a)..buf.line_range(b).end
}

/// The lines an operator's range covers, first and last. A linewise
/// range on the last line of a text with no break after it starts at
/// the break before it (`line_range_of_sel`), which is the line
/// above's: its first line is the one after that break. The range is
/// then a break and a line with none after it — an empty last line
/// the break alone.
/// An edit and the line it is on.
pub(crate) type LineEdit = (usize, (Range<usize>, String));

/// The first non-blank character the ranges' lines have, where a
/// comment operator asks which language it is in; the first range's
/// start when every line is blank.
fn first_text_at(buf: &Buffer, ranges: &[(Range<usize>, bool)]) -> usize {
    ranges
        .iter()
        .flat_map(|(r, lw)| {
            let (a, b) = op_lines(buf, r, *lw);
            a..=b
        })
        .find(|&ln| !buf.line_text(ln).trim().is_empty())
        .map(|ln| m::first_nonblank(buf, ln))
        .unwrap_or_else(|| ranges.first().map_or(0, |(r, _)| r.start))
}

/// The edits that toggle `lines` as comments (docs/design/comments.md
/// Decision 2), each with its line: blank lines set aside, the rest
/// uncommented when every one starts with the token after its indent
/// (and ends with the closer, for a block pair), else commented, the
/// opener put at their least indent — the longest common prefix of
/// their leading whitespace, bytes not columns — with a space after
/// it, the closer with a space before it at the line's end. `None`
/// when there is no line but blank ones.
pub(crate) fn comment_lines(
    buf: &Buffer,
    lines: &[usize],
    tokens: &crate::CommentTokens,
) -> Option<Vec<LineEdit>> {
    let (open, close): (&str, &str) = match (&tokens.line, &tokens.block) {
        (Some(t), _) => (t, ""),
        (None, Some((o, c))) => (o, c),
        (None, None) => return None,
    };
    let rows: Vec<(usize, Range<usize>, String)> = lines
        .iter()
        .map(|&ln| {
            let r = buf.line_range(ln);
            (ln, r.clone(), buf.slice(r))
        })
        .filter(|(_, _, t)| !t.trim().is_empty())
        .collect();
    if rows.is_empty() {
        return None;
    }
    let commented = rows.iter().all(|(_, _, t)| {
        let s = t.trim_start();
        s.starts_with(open) && (close.is_empty() || s.trim_end().ends_with(close))
    });
    let mut out = Vec::new();
    if commented {
        for (ln, r, t) in &rows {
            let ws = t.len() - t.trim_start().len();
            let mut end = ws + open.len();
            if t[end..].starts_with(' ') {
                end += 1;
            }
            out.push((*ln, (r.start + ws..r.start + end, String::new())));
            if !close.is_empty() {
                let trimmed = t.trim_end().len();
                let mut start = trimmed - close.len();
                if start > end && t[..start].ends_with(' ') {
                    start -= 1;
                }
                out.push((*ln, (r.start + start..r.start + trimmed, String::new())));
            }
        }
    } else {
        let mut prefix: Option<&str> = None;
        for (_, _, t) in &rows {
            let ws = &t[..t.len() - t.trim_start().len()];
            prefix = Some(match prefix {
                None => ws,
                Some(p) => {
                    let n = p
                        .bytes()
                        .zip(ws.bytes())
                        .take_while(|(a, b)| a == b)
                        .count();
                    &p[..n]
                }
            });
        }
        let at = prefix.map_or(0, str::len);
        for (ln, r, t) in &rows {
            out.push((*ln, (r.start + at..r.start + at, format!("{open} "))));
            if !close.is_empty() {
                let trimmed = t.trim_end().len();
                out.push((
                    *ln,
                    (r.start + trimmed..r.start + trimmed, format!(" {close}")),
                ));
            }
        }
    }
    Some(out)
}

fn op_lines(buf: &Buffer, r: &Range<usize>, linewise: bool) -> (usize, usize) {
    let brk = match (buf.byte_at(r.start), buf.byte_at(r.start + 1)) {
        (Some(b'\n'), _) => 1,
        (Some(b'\r'), Some(b'\n')) => 2,
        _ => 0,
    };
    let last_line = linewise
        && brk > 0
        && r.end == buf.len()
        && (r.end == r.start + brk || buf.byte_at(r.end - 1) != Some(b'\n'));
    let first = if last_line { r.start + brk } else { r.start };
    (
        buf.line_of(first),
        buf.line_of(r.end.saturating_sub(1).max(first)),
    )
}

/// Each selection on the first line its range covered, at its first
/// non-blank: after `=` `>` `<`, which change no line's count.
fn to_first_lines(ed: &mut Editor, view: ViewId, firsts: &[usize]) {
    if firsts.is_empty() {
        return;
    }
    let buf = &ed.buffers[ed.views[view].buffer];
    let v = &mut ed.views[view];
    let mut i = 0;
    v.sels.map(|_| {
        let ln = firsts[i.min(firsts.len() - 1)];
        i += 1;
        Selection::point(m::first_nonblank(buf, ln.min(buf.line_count() - 1)))
    });
}

/// The range an operator applies to, per the motion's kind.
pub(crate) fn op_range(
    buf: &Buffer,
    s: &Selection,
    kind: MotionKind,
    count: usize,
) -> (Range<usize>, bool) {
    match kind {
        MotionKind::Exclusive => (s.range(), false),
        MotionKind::Inclusive => {
            let r = s.range();
            // Inclusive of the char under the head, never of a newline
            // (`d$` on an empty line): a motion's, where a selection's
            // takes it (`sel_range`).
            let end = if r.end == buf.line_range(buf.line_of(r.end)).end {
                r.end
            } else {
                buf.next_char(r.end)
            };
            (r.start..end, false)
        }
        MotionKind::Linewise => (line_range_of_sel(buf, s, count.saturating_sub(1)), true),
    }
}

/// What a selection covers in visual mode: its characters, the one
/// under its far end in — a line's newline too, when that end stands on
/// it, since the selection is drawn over it there (vim's `v$`, and `v`
/// on an empty line), as `x` on it takes it.
pub(crate) fn sel_range(buf: &Buffer, s: &Selection) -> Range<usize> {
    let r = s.range();
    r.start..buf.next_char(r.end)
}

/// What `x` and `s` take from each selection: under `V` its lines, a
/// visual selection its `sel_range`, else COUNT characters from the
/// caret, a count stopping at the line's end — or, the caret on the
/// line's newline, that newline, the next line joined on as it is
/// (vim's `gJ`), once whatever the count; the last line has none.
fn chars_under(ed: &Editor, ctx: &Ctx) -> Vec<(Range<usize>, bool)> {
    let buf = &ed.buffers[view(ed, ctx).buffer];
    let visual = ed.mode(ctx.view) == Mode::Visual;
    ed.views[ctx.view]
        .sels
        .iter()
        .map(|s| {
            // `Vx` `Vs` take the lines, as `d` and `c` do there.
            if visual && ed.views[ctx.view].visual_linewise {
                return (line_range_of_sel(buf, s, 0), true);
            }
            if visual && !s.is_empty() {
                return (sel_range(buf, s), false);
            }
            let le = buf.line_range(buf.line_of(s.head)).end;
            if s.head == le {
                return (s.head..buf.next_char(le), false);
            }
            let mut e = s.head;
            for _ in 0..ctx.count.max(1) {
                if e < le {
                    e = buf.next_char(e);
                }
            }
            (s.head..e, false)
        })
        .collect()
}

/// The text an operator took, remembered: the `"` register and the
/// memory's newest moment — unless `"_` named the black hole for it,
/// where it goes nowhere.
fn set_register(
    ed: &mut Editor,
    id: kawoosh_doc::BufferId,
    ranges: &[(Range<usize>, bool)],
    texts: &[String],
    linewise: bool,
    took: crate::Took,
) {
    if ed.pending_register == Some('_') {
        return;
    }
    let mut joined = if texts.len() == 1 {
        texts[0].clone()
    } else {
        texts.join("\n")
    };
    let mut origin = (ranges.len() == 1).then(|| ranges[0].0.clone());
    // A linewise range on the last line is the newline before it and
    // the line (`line_range_of_sel`); in the register the line is a
    // line like any other, its newline after it — so `p` puts it below
    // the caret's line, not an empty line and then it.
    let before = ["\r\n", "\n"].into_iter().find(|b| joined.starts_with(b));
    if linewise
        && !joined.ends_with('\n')
        && let Some(brk) = before
    {
        joined.drain(..brk.len());
        joined.push_str(brk);
        if let Some(r) = &mut origin {
            r.start += brk.len();
        }
    }
    let version = ed.buffers[id].version();
    // A private buffer's text is a secret in the register and never on
    // the system clipboard (docs/design/secrets.md Decision 2).
    let secret = ed.buffers[id].private;
    ed.memory.remember(crate::Moment {
        text: joined.clone(),
        linewise,
        took,
        origin: origin.map(|range| crate::RegisterOrigin {
            buffer: id,
            version,
            range,
        }),
        from: ed.buffers[id].name.clone(),
        at: std::time::Instant::now(),
        secret,
    });
    if secret {
        text_buffer::wipe_string(&mut joined);
    } else {
        ed.effects.push(Effect::SetClipboard(joined));
    }
}

/// Runs operator `op` over `ranges` (one per selection).
pub(crate) fn apply_operator(
    ed: &mut Editor,
    view: ViewId,
    op: &str,
    ranges: Vec<(Range<usize>, bool)>,
) {
    let id = ed.views[view].buffer;
    let texts: Vec<String> = ranges
        .iter()
        .map(|(r, _)| ed.buffers[id].slice(r.clone()))
        .collect();
    let linewise = ranges.iter().any(|(_, l)| *l);
    match op {
        "yank" => {
            set_register(ed, id, &ranges, &texts, linewise, crate::Took::Yank);
            let len = ed.buffers[id].len();
            ed.flash = Some(crate::Flash {
                buffer: id,
                version: ed.buffers[id].version(),
                ranges: ranges
                    .iter()
                    .map(|(r, _)| r.start.min(len)..r.end.min(len))
                    .collect(),
                at: std::time::Instant::now(),
            });
            // The caret goes to the start of what was yanked; on a
            // linewise yank it stays (`yy` on the last line: the range
            // starts with the newline before it, which is not its line).
            let v = &mut ed.views[view];
            let mut i = 0;
            v.sels.map(|s| {
                let (r, lw) = &ranges[i.min(ranges.len() - 1)];
                i += 1;
                Selection::point(if *lw { s.head } else { r.start })
            });
            ed.message = format!(
                "yanked {} line(s)",
                texts
                    .iter()
                    .map(|t| t.matches('\n').count().max(1))
                    .sum::<usize>()
            );
        }
        // `erase` is a delete the register never hears of: insert's
        // Backspace, Delete and `<C-w>`, as vim's — only an operator's
        // text is remembered, and on the clipboard.
        "delete" | "change" | "erase" => {
            match op {
                "change" => set_register(ed, id, &ranges, &texts, linewise, crate::Took::Change),
                "delete" => set_register(ed, id, &ranges, &texts, linewise, crate::Took::Delete),
                _ => {}
            }
            let edits = ranges
                .iter()
                .enumerate()
                .map(|(i, (r, lw))| {
                    // `cc` keeps the line's indent and its newline, the
                    // lines' (`op_lines`) from the first's start to the
                    // last's end.
                    if op == "change" && *lw {
                        let buf = &ed.buffers[id];
                        let (a, b) = op_lines(buf, r, true);
                        let indent = m::indent_of(buf, a);
                        let inner = buf.line_start(a)..buf.line_range(b).end;
                        (i, inner, indent)
                    } else {
                        (i, r.clone(), String::new())
                    }
                })
                .collect();
            ed.edit_each(view, edits, |start, len| Selection::point(start + len));
            if op == "change" {
                ed.set_mode(view, Mode::Insert);
            } else {
                // After a delete the caret sits where the text was; on a
                // linewise delete, at the line's first non-blank.
                if linewise {
                    let buf = &ed.buffers[id];
                    let v = &mut ed.views[view];
                    v.sels
                        .map(|s| Selection::point(m::first_nonblank(buf, buf.line_of(s.head))));
                }
            }
        }
        // `gu` `gU` `g~`, and `u` `U` `~` on a selection: the text's
        // case, the caret at the start of what was turned.
        "case lower" | "case upper" | "case toggle" => {
            let edits = ranges
                .iter()
                .zip(&texts)
                .enumerate()
                .map(|(i, ((r, _), t))| (i, r.clone(), case_turned(t, op)))
                .collect();
            ed.edit_each(view, edits, |start, _| Selection::point(start));
        }
        // `gsa` + a motion: the ranges wait for the pair's character
        // (`surround wrap`); a linewise one keeps its newline outside.
        "surround add" => {
            let buf = &ed.buffers[id];
            let ranges = ranges
                .iter()
                .map(|(r, lw)| {
                    if *lw && r.end > r.start && buf.char_at(buf.prev_char(r.end)) == Some('\n') {
                        r.start..buf.prev_char(r.end)
                    } else {
                        r.clone()
                    }
                })
                .collect();
            ed.surround.ranges = Some(ranges);
            ed.await_char("surround wrap");
        }
        // `ga` + a motion: what it covers on each line waits for the
        // character to line up on (`align on`).
        "align" => {
            let regions = align_regions(&ed.buffers[id], &ranges);
            ed.surround.align = Some(Align {
                regions,
                which: String::new(),
            });
            ed.await_char("align on");
        }
        // `=`: each line of the ranges at the indent the syntax says,
        // top down; the caret on the first one's first non-blank.
        "reindent" => {
            let unit = ed.indent_unit(id);
            let mut indenter = ed.indenter.take();
            let buf = &ed.buffers[id];
            let lines: Vec<(usize, usize)> =
                ranges.iter().map(|(r, lw)| op_lines(buf, r, *lw)).collect();
            let firsts: Vec<usize> = lines.iter().map(|(a, _)| *a).collect();
            let mut spans = lines;
            spans.sort_unstable();
            let mut edits = Vec::new();
            let mut answered = true;
            let mut done = 0;
            for (a, b) in spans {
                let a = a.max(done);
                if a > b {
                    continue;
                }
                done = b + 1;
                let Some(said) = indenter
                    .as_mut()
                    .and_then(|x| x.lines(id, buf, a..b + 1, &unit))
                else {
                    answered = false;
                    break;
                };
                for (ln, want) in (a..=b).zip(said) {
                    let Some(want) = want else { continue };
                    let own = m::indent_of(buf, ln);
                    if own != want {
                        let start = buf.line_start(ln);
                        edits.push((usize::MAX - ln, start..start + own.len(), want));
                    }
                }
            }
            ed.indenter = indenter;
            if !answered {
                let lang = ed.buffers[id].language.to_string();
                ed.message = format!("no indent rules for {lang}");
                return;
            }
            ed.edit_each(view, edits, |start, _| Selection::point(start));
            to_first_lines(ed, view, &firsts);
        }
        "indent" | "dedent" => {
            let ts = ed.shiftwidth_in(id);
            let unit = ed.indent_unit_in(id);
            let buf = &ed.buffers[id];
            let mut edits = Vec::new();
            let mut seen = std::collections::HashSet::new();
            let mut firsts = Vec::new();
            for (i, (r, lw)) in ranges.iter().enumerate() {
                let (a, b) = op_lines(buf, r, *lw);
                firsts.push(a);
                for ln in a..=b {
                    if !seen.insert(ln) {
                        continue;
                    }
                    let start = buf.line_start(ln);
                    if op == "indent" {
                        if !buf.line_range(ln).is_empty() {
                            edits.push((usize::MAX - ln, start..start, unit.clone()));
                        }
                    } else {
                        let text = buf.line_text(ln);
                        let n = if text.starts_with('\t') {
                            1
                        } else {
                            text.chars().take(ts).take_while(|c| *c == ' ').count()
                        };
                        if n > 0 {
                            edits.push((usize::MAX - ln, start..start + n, String::new()));
                        }
                    }
                }
                let _ = i;
            }
            ed.edit_each(view, edits, |start, _| Selection::point(start));
            to_first_lines(ed, view, &firsts);
        }
        // `gc`: the ranges' lines commented, or uncommented when every
        // one already is (docs/design/comments.md Decision 2); each
        // selection judged alone, a line two share edited once.
        "comment" => {
            // The tokens are the layer's at the first non-blank line's
            // first character (Decision 4): one language per `gc`.
            let at = first_text_at(&ed.buffers[id], &ranges);
            let (lang, tokens) = ed.comment_tokens_at(id, at);
            if tokens.line.is_none() && tokens.block.is_none() {
                ed.message = format!("no comment token for {lang}");
                return;
            }
            let buf = &ed.buffers[id];
            let mut edits = Vec::new();
            let mut seen = std::collections::HashSet::new();
            let mut any = false;
            for (r, lw) in &ranges {
                let (a, b) = op_lines(buf, r, *lw);
                let lines: Vec<usize> = (a..=b).collect();
                let Some(plan) = comment_lines(buf, &lines, &tokens) else {
                    continue;
                };
                any = true;
                // A line an earlier selection had is its; a line may
                // take two edits (a block pair's), both kept.
                for (ln, edit) in plan {
                    if !seen.contains(&ln) {
                        edits.push(edit);
                    }
                }
                seen.extend(lines);
            }
            if !any {
                ed.message = "nothing to comment".into();
                return;
            }
            // The caret stays where it stood on its line (`gcc`); after
            // a motion it goes to the first line's first non-blank.
            let extended: Vec<bool> = ed.views[view]
                .sels
                .iter()
                .map(|s| s.anchor != s.head)
                .collect();
            edit_keeping(ed, view, edits);
            let buf = &ed.buffers[id];
            let v = &mut ed.views[view];
            let mut i = 0;
            v.sels.map(|s| {
                let ext = extended[i.min(extended.len() - 1)];
                i += 1;
                if ext {
                    Selection::point(m::first_nonblank(buf, buf.line_of(s.start())))
                } else {
                    s
                }
            });
        }
        // `gb`: each range wrapped in the block pair as one — the
        // opener before its first non-blank character, the closer
        // after its last — or unwrapped when it is one already
        // (docs/design/comments.md Decision 8).
        "comment block" => {
            let at = first_text_at(&ed.buffers[id], &ranges);
            let (lang, tokens) = ed.comment_tokens_at(id, at);
            let Some((open, close)) = tokens.block else {
                ed.message = format!("no block comment pair for {lang}");
                return;
            };
            let buf = &ed.buffers[id];
            let mut edits = Vec::new();
            for (r, lw) in &ranges {
                let span = if *lw {
                    let (a, b) = op_lines(buf, r, *lw);
                    let lines: Vec<usize> = (a..=b)
                        .filter(|&ln| !buf.line_text(ln).trim().is_empty())
                        .collect();
                    let (Some(&first), Some(&last)) = (lines.first(), lines.last()) else {
                        continue;
                    };
                    let end = buf.line_range(last);
                    m::first_nonblank(buf, first)..end.start + buf.slice(end).trim_end().len()
                } else {
                    r.start.min(buf.len())..r.end.min(buf.len())
                };
                if span.start >= span.end {
                    continue;
                }
                let t = buf.slice(span.clone());
                if t.len() >= open.len() + close.len()
                    && t.starts_with(&open)
                    && t.ends_with(&close)
                {
                    let mut a = open.len();
                    if t[a..].starts_with(' ') {
                        a += 1;
                    }
                    let mut b = t.len() - close.len();
                    if b > a && t[..b].ends_with(' ') {
                        b -= 1;
                    }
                    edits.push((span.start..span.start + a, String::new()));
                    edits.push((span.start + b..span.end, String::new()));
                } else {
                    edits.push((span.start..span.start, format!("{open} ")));
                    edits.push((span.end..span.end, format!(" {close}")));
                }
            }
            if edits.is_empty() {
                ed.message = "nothing to comment".into();
                return;
            }
            let extended: Vec<bool> = ed.views[view]
                .sels
                .iter()
                .map(|s| s.anchor != s.head)
                .collect();
            edit_keeping(ed, view, edits);
            let buf = &ed.buffers[id];
            let v = &mut ed.views[view];
            let mut i = 0;
            v.sels.map(|s| {
                let ext = extended[i.min(extended.len() - 1)];
                i += 1;
                if ext {
                    Selection::point(m::first_nonblank(buf, buf.line_of(s.start())))
                } else {
                    s
                }
            });
        }
        "join" => {
            let buf = &ed.buffers[id];
            let mut edits = Vec::new();
            for (i, (r, _)) in ranges.iter().enumerate() {
                let a = buf.line_of(r.start);
                let mut b = buf.line_of(r.end.saturating_sub(1).max(r.start));
                if b == a {
                    b = a + 1;
                }
                for ln in a..b.min(buf.line_count() - 1) {
                    let end = buf.line_range(ln).end;
                    let next = buf.line_range(ln + 1);
                    let text = buf.slice(next.clone());
                    let lead = text.len() - text.trim_start().len();
                    let sep = if text.trim_start().is_empty() {
                        ""
                    } else {
                        " "
                    };
                    edits.push((i, end..next.start + lead, sep.to_string()));
                }
            }
            ed.edit_each(view, edits, |start, _| Selection::point(start));
        }
        _ => ed.message = format!("unknown operator {op}"),
    }
    clamp_sels(ed, view);
}

/// `comment lines` and `comment block lines`: a pending `comment` or
/// `comment block` doubled (`gcc`, `gbc`, whichever is pending), `c`
/// as it was after any other operator, and with none pending `op` over
/// COUNT lines at the caret, or the selection's.
fn comment_lines_cmd(ed: &mut Editor, ctx: &Ctx, op: &'static str) {
    match ed.pending_op {
        Some((pending @ ("comment" | "comment block"), _)) => operator(ed, ctx, pending),
        Some(_) => operator(ed, ctx, "change"),
        None => {
            if ed.mode(ctx.view) == Mode::Visual {
                operator(ed, ctx, op);
                return;
            }
            let id = view(ed, ctx).buffer;
            let buf = &ed.buffers[id];
            let n = ctx.count.max(1);
            let ranges: Vec<(Range<usize>, bool)> = ed.views[ctx.view]
                .sels
                .iter()
                .map(|s| (line_range_of_sel(buf, s, n - 1), true))
                .collect();
            apply_operator(ed, ctx.view, op, ranges);
        }
    }
}

fn operator(ed: &mut Editor, ctx: &Ctx, op: &'static str) {
    if ed.mode(ctx.view) == Mode::Visual {
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let linewise = ed.views[ctx.view].visual_linewise;
        let ranges: Vec<(Range<usize>, bool)> = ed.views[ctx.view]
            .sels
            .iter()
            .map(|s| {
                if linewise {
                    (line_range_of_sel(buf, s, 0), true)
                } else {
                    (sel_range(buf, s), false)
                }
            })
            .collect();
        // One that waits for a character — `ga`, `gsa` — keeps the
        // selection on show, as it was, until the character comes; the
        // character's command ends visual mode.
        if !matches!(op, "align" | "surround add") {
            ed.set_mode(ctx.view, Mode::Normal);
        }
        apply_operator(ed, ctx.view, op, ranges);
        return;
    }
    match ed.pending_op.take() {
        // `dd`, `yy`, `cc`, `>>`: the operator on whole lines.
        Some((prev, count)) if prev == op => {
            let n = (count.max(1) * ctx.count.max(1)).max(1);
            let id = view(ed, ctx).buffer;
            let buf = &ed.buffers[id];
            let ranges: Vec<(Range<usize>, bool)> = ed.views[ctx.view]
                .sels
                .iter()
                .map(|s| (line_range_of_sel(buf, s, n - 1), true))
                .collect();
            apply_operator(ed, ctx.view, op, ranges);
        }
        Some(_) => {}
        None => ed.pending_op = Some((op, if ctx.has_count { ctx.count } else { 0 })),
    }
}

fn textobject(ed: &mut Editor, ctx: &Ctx, around: bool) {
    let Some(c) = ctx.arg_char else {
        return;
    };
    if let Some(object) = syntax_object_of(c) {
        return syntax_textobject(ed, ctx, c, object, around);
    }
    let visual = ed.mode(ctx.view) == Mode::Visual;
    let id = view(ed, ctx).buffer;
    let buf = &ed.buffers[id];
    let v = &mut ed.views[ctx.view];
    let mut ok = true;
    let mut found = false;
    v.sels.map(|s| {
        let range = match c {
            'p' => paragraph_object(buf, s.head, around),
            'w' => {
                let (a, b) = m::word_at(buf, s.head);
                if around {
                    let mut e = b;
                    while buf.char_at(e).is_some_and(|c| c == ' ' || c == '\t') {
                        e = buf.next_char(e);
                    }
                    Some(a..e)
                } else {
                    Some(a..b)
                }
            }
            'W' => {
                let (a, b) = m::bigword_at(buf, s.head);
                if a == b {
                    None
                } else if around {
                    let mut e = b;
                    while buf.char_at(e).is_some_and(|c| c == ' ' || c == '\t') {
                        e = buf.next_char(e);
                    }
                    Some(a..e)
                } else {
                    Some(a..b)
                }
            }
            '(' | ')' | 'b' => bracket_object(buf, s.head, '(', ')', around),
            '[' | ']' => bracket_object(buf, s.head, '[', ']', around),
            '{' | '}' | 'B' => bracket_object(buf, s.head, '{', '}', around),
            '<' | '>' => bracket_object(buf, s.head, '<', '>', around),
            '"' | '\'' | '`' => quote_object(buf, s.head, c, around),
            _ => None,
        };
        match range {
            // A paragraph is lines: in visual mode the head sits on the
            // last one's newline, and the selection goes linewise.
            Some(r) if c == 'p' && visual => {
                found = true;
                Selection::new(r.start, buf.prev_char(r.end))
            }
            // A visual selection's head is on its last character (the
            // range's end is exclusive), so `vi(` ends on the byte
            // before `)` and `d` takes exactly the inside; under an
            // operator the range is taken as it is.
            Some(r) if visual && r.end > r.start => {
                found = true;
                Selection::new(r.start, buf.prev_char(r.end))
            }
            Some(r) => {
                found = true;
                Selection::new(r.start, r.end)
            }
            None => {
                ok = false;
                s
            }
        }
    });
    if c == 'p' && visual {
        v.visual_linewise = true;
    }
    if !ok {
        ed.message = format!("no text object for {c}");
    }
    // Found nowhere, the operator waiting on it is off: `yiW` with no
    // WORD yanked nothing over the register.
    if !found {
        ed.pending_op = None;
    }
}

/// The syntax text objects by their key (docs/design/nodes.md Decision
/// 9): the object a grammar's `textobjects.scm` names. `f` a function,
/// `c` a class — a struct, an enum, an impl, an interface: the type —
/// `a` an argument or a parameter, `/` a comment, `T` a test, `e` an
/// entry (an array's element, a table's pair).
pub const SYNTAX_OBJECTS: &[(char, &str)] = &[
    ('f', "function"),
    ('c', "class"),
    ('a', "parameter"),
    ('/', "comment"),
    ('T', "test"),
    ('e', "entry"),
];

fn syntax_object_of(c: char) -> Option<&'static str> {
    SYNTAX_OBJECTS
        .iter()
        .find(|(k, _)| *k == c)
        .map(|(_, o)| *o)
}

/// `af` `if` `ac` …: each selection's object from the buffer's syntax,
/// [`pick_object`] choosing among what the query found over it. In
/// visual mode the selection becomes the object, its head on its last
/// character, linewise when every object has its lines to itself; under
/// an operator each that does is taken linewise ([`object_ranges`]).
fn syntax_textobject(ed: &mut Editor, ctx: &Ctx, c: char, object: &str, around: bool) {
    let visual = ed.mode(ctx.view) == Mode::Visual;
    let id = view(ed, ctx).buffer;
    let Some(mut finder) = ed.syntax_objects.take() else {
        ed.message = format!("no text object for {c} here: no syntax");
        ed.pending_op = None;
        return;
    };
    let buf = &ed.buffers[id];
    let mut picked = Vec::with_capacity(ed.views[ctx.view].sels.len());
    let mut why = None;
    for s in ed.views[ctx.view].sels.iter() {
        // The caret's character, or the selected bytes — which a visual
        // selection of more than one has the object grow past.
        let at = sel_range(buf, s);
        let grow = visual && s.start() != s.end();
        // To the line's end, for an object after the caret on its line.
        let line_end = buf.line_range(buf.line_of(at.start)).end;
        match finder.find(id, buf, object, at.start..line_end.max(at.end)) {
            Ok(found) => {
                let r = pick_object(&found, at, around, grow, ctx.count, line_end);
                // A list's item goes with its separator.
                let item = around && matches!(object, "parameter" | "entry");
                picked.push(r.map(|r| if item { with_separator(buf, r) } else { r }));
            }
            Err(e) => {
                why = Some(e);
                picked.push(None);
            }
        }
    }
    ed.syntax_objects = Some(finder);
    let found = picked.iter().any(Option::is_some);
    let lines = found
        && picked
            .iter()
            .flatten()
            .all(|r| has_its_lines(&ed.buffers[id], r));
    let buf = &ed.buffers[id];
    let v = &mut ed.views[ctx.view];
    let mut i = 0;
    v.sels.map(|s| {
        let r = picked[i.min(picked.len() - 1)].clone();
        i += 1;
        match r {
            // As the other objects: a visual head on the last character.
            Some(r) if visual && r.end > r.start => Selection::new(r.start, buf.prev_char(r.end)),
            Some(r) => Selection::new(r.start, r.end),
            None => s,
        }
    });
    if visual && found {
        v.visual_linewise = lines;
    }
    if picked.iter().any(Option::is_none) {
        ed.message = why.unwrap_or_else(|| format!("no {object} here"));
    }
    if !found {
        ed.pending_op = None;
    } else if !visual && ed.pending_op.is_some() {
        ed.object_lines = true;
    }
}

/// Which object a syntax text object takes over `at` — the caret's
/// character, or a visual selection's bytes — from what the query found:
/// the `count`th smallest whose around is over `at` (its inside, for a
/// match with no around), and of it the part asked for. An object found
/// with several parts of a kind — a pair's key and its value, a field's
/// name and its type, one match each — gives the one over `at`, or, none
/// being, the nearest: `cie` on a value is the value, and `if` on a
/// function's signature its body. A count steps out to the objects
/// around, never to a sibling part. `grow`, a visual selection of more
/// than a character, asks for one past the selection: `vafaf` is the
/// function around. With none over `at`, the first object that starts
/// after it and before `line_end`, its line's end: `cia` with the caret
/// on the `(`.
pub(crate) fn pick_object(
    found: &[crate::SyntaxObject],
    at: Range<usize>,
    around: bool,
    grow: bool,
    count: usize,
    line_end: usize,
) -> Option<Range<usize>> {
    let part = |f: &crate::SyntaxObject| {
        if around {
            f.around.clone()
        } else {
            f.inside.clone()
        }
    };
    // What an object is found over: its around — for an inside its
    // pattern took alone, as nvim's queries write them, the smallest
    // around round it.
    let key = |f: &crate::SyntaxObject| {
        f.around.clone().or_else(|| {
            let inside = f.inside.clone()?;
            found
                .iter()
                .filter_map(|g| g.around.clone())
                .filter(|a| a.start <= inside.start && inside.end <= a.end)
                .min_by_key(|a| a.len())
                .or(Some(inside))
        })
    };
    let over = |r: &Range<usize>| r.start <= at.start && at.end <= r.end;
    // How far a part is from `at`: none for one over it or touching it.
    let distance =
        |p: &Range<usize>| at.start.saturating_sub(p.end) + p.start.saturating_sub(at.end);
    // Each object over `at`, by its key, with the parts it was found
    // with — a match each.
    let mut objects: Vec<(Range<usize>, Vec<Range<usize>>)> = Vec::new();
    for f in found {
        let (Some(k), Some(p)) = (key(f), part(f)) else {
            continue;
        };
        if !over(&k) || (grow && !(over(&p) && p != at)) {
            continue;
        }
        match objects.iter_mut().find(|(o, _)| *o == k) {
            Some((_, parts)) => parts.push(p),
            None => objects.push((k, vec![p])),
        }
    }
    let mut over_at: Vec<(usize, Range<usize>)> = objects
        .into_iter()
        .filter_map(|(k, parts)| {
            let p = parts
                .into_iter()
                .min_by_key(|p| (!over(p), distance(p), p.len(), p.start))?;
            Some((k.len(), p))
        })
        .collect();
    over_at.sort_by_key(|(k, p)| (*k, p.len(), p.start));
    let mut seen: Vec<Range<usize>> = Vec::new();
    over_at.retain(|(_, p)| {
        let new = !seen.contains(p);
        if new {
            seen.push(p.clone());
        }
        new
    });
    if !over_at.is_empty() {
        return over_at.into_iter().nth(count.max(1) - 1).map(|(_, p)| p);
    }
    if grow {
        return None;
    }
    // Held to the line: tree-sitter finishes a match it started inside
    // the range asked about, so what starts on a later line is found too.
    found
        .iter()
        .filter_map(|f| {
            let (k, p) = (key(f)?, part(f)?);
            (k.start >= at.end && k.start < line_end)
                .then(|| ((k.start, std::cmp::Reverse(k.len()), p.start), p))
        })
        .min_by_key(|(order, _)| *order)
        .map(|(_, p)| p)
}

/// A list item's around — an argument, an entry — with what separates
/// it from the next: the `,` after it on its last line (the query's
/// around may hold it already) and the blanks after that; or, the last
/// item, the `,` before it and the blanks between, line breaks too. So
/// `daa` leaves `f(a, b)` as `f(b)` from `a` and `f(a)` from `b`, as
/// targets.vim's does, and the last item of a list across lines takes
/// the comma that ended the line before it: the list stays well-formed.
fn with_separator(buf: &Buffer, r: Range<usize>) -> Range<usize> {
    let line = buf.line_range(buf.line_of(r.end));
    let blank = |c: char| c == ' ' || c == '\t';
    let skip_after = |mut e: usize| {
        while e < line.end && buf.char_at(e).is_some_and(blank) {
            e = buf.next_char(e);
        }
        e
    };
    if r.end > r.start && buf.char_at(buf.prev_char(r.end)) == Some(',') {
        return r.start..skip_after(r.end);
    }
    let after = skip_after(r.end);
    if after < line.end && buf.char_at(after) == Some(',') {
        return r.start..skip_after(buf.next_char(after));
    }
    let mut b = r.start;
    while b > 0
        && buf
            .char_at(buf.prev_char(b))
            .is_some_and(char::is_whitespace)
    {
        b = buf.prev_char(b);
    }
    if b > 0 && buf.char_at(buf.prev_char(b)) == Some(',') {
        return buf.prev_char(b)..r.end;
    }
    r
}

/// Whether `r` has its lines to itself: nothing but blanks before it on
/// its first line and after it on its last — a function, a comment, a
/// parameter on lines of their own.
fn has_its_lines(buf: &Buffer, r: &Range<usize>) -> bool {
    if r.end <= r.start {
        return false;
    }
    let first = buf.line_of(r.start);
    let before = buf.slice(buf.line_start(first)..r.start);
    let end = buf.line_range(buf.line_of(buf.prev_char(r.end))).end;
    let after = if r.end < end {
        buf.slice(r.end..end)
    } else {
        String::new()
    };
    before.trim().is_empty() && after.trim().is_empty()
}

/// What an operator takes from a text object's selections: each as it
/// is, or — after a syntax object (`lines`) — the lines of one that has
/// them to itself, linewise, as `dd` takes them: `daf` leaves no blank
/// line where the function was, and `yaf` puts back as lines.
pub(crate) fn object_ranges(ed: &Editor, view: ViewId, lines: bool) -> Vec<(Range<usize>, bool)> {
    let buf = &ed.buffers[ed.views[view].buffer];
    ed.views[view]
        .sels
        .iter()
        .map(|s| {
            let r = s.range();
            if lines && has_its_lines(buf, &r) {
                let last = Selection::new(r.start, buf.prev_char(r.end));
                (line_range_of_sel(buf, &last, 0), true)
            } else {
                (r, false)
            }
        })
        .collect()
}

/// `]f` `[f`: every head to the start of the next (previous) object of
/// the syntax, COUNT of them on — as far as there are. A motion: `d]f`
/// deletes up to the next function, `v]f` extends to it.
fn syntax_jump(ed: &mut Editor, ctx: &Ctx, object: &str, forward: bool) {
    let extend = ed.mode(ctx.view) == Mode::Visual || ed.pending_op.is_some();
    let id = view(ed, ctx).buffer;
    let Some(mut finder) = ed.syntax_objects.take() else {
        ed.message = format!("no {object} to go to: no syntax");
        return;
    };
    let buf = &ed.buffers[id];
    let mut to = Vec::with_capacity(ed.views[ctx.view].sels.len());
    let mut why = None;
    for s in ed.views[ctx.view].sels.iter() {
        let h = s.head.min(buf.len());
        let within = if forward { h..buf.len() } else { 0..h };
        let mut starts: Vec<usize> = match finder.find(id, buf, object, within) {
            Ok(found) => found
                .iter()
                .filter_map(|f| f.around.as_ref().or(f.inside.as_ref()).map(|r| r.start))
                .filter(|&st| if forward { st > h } else { st < h })
                .collect(),
            Err(e) => {
                why = Some(e);
                Vec::new()
            }
        };
        starts.sort_unstable();
        starts.dedup();
        if !forward {
            starts.reverse();
        }
        to.push(starts.into_iter().take(ctx.count.max(1)).next_back());
    }
    ed.syntax_objects = Some(finder);
    if to.iter().all(Option::is_none) {
        let way = if forward { "next" } else { "previous" };
        ed.message = why.unwrap_or_else(|| format!("no {way} {object}"));
    }
    let v = &mut ed.views[ctx.view];
    let mut i = 0;
    v.sels.map(|s| {
        let t = to[i.min(to.len() - 1)];
        i += 1;
        t.map_or(s, |t| s.with_head(t, extend))
    });
    v.goal_col = None;
}

/// `%`: each head on a bracket to the one matching it; on a keyword of
/// a block its language closes with a word, to the block's next —
/// `if` to `else` to `end` and back round — as the syntax tree has it
/// ([`SyntaxObjects::partner`]). Under an operator the whole of both
/// keywords is taken, as a bracket pair's both brackets are: `d%` on an
/// `if` deletes through its `end`, on the `end` back through the `if`.
fn match_pair(ed: &mut Editor, ctx: &Ctx) {
    let ext = extend(ed, ctx.view);
    let op = ed.pending_op.is_some();
    let id = view(ed, ctx).buffer;
    let mut finder = ed.syntax_objects.take();
    let buf = &ed.buffers[id];
    let v = &mut ed.views[ctx.view];
    v.sels.map(|s| {
        let h = s.head.min(buf.len());
        if let Some(to) = m::matching_bracket(buf, h) {
            return s.with_head(to, ext);
        }
        let Some((from, to)) = finder.as_mut().and_then(|f| f.partner(id, buf, h)) else {
            return s;
        };
        if !op {
            s.with_head(to.start, ext)
        } else if to.start > from.start {
            s.with_head(buf.prev_char(to.end), ext)
        } else {
            Selection::new(buf.prev_char(from.end), to.start)
        }
    });
    v.goal_col = None;
    ed.syntax_objects = finder;
}

/// The paragraph at `o`: its run of non-blank lines — or of blank ones,
/// on a blank — whole, the newline of the last included; around it, the
/// blank lines after it, or before it when none follow, or the
/// paragraph after a run of blanks.
fn paragraph_object(buf: &Buffer, o: usize, around: bool) -> Option<Range<usize>> {
    let blank = |ln: usize| buf.slice(buf.line_range(ln)).trim().is_empty();
    let last = buf.line_count().saturating_sub(1);
    let ln = buf.line_of(o);
    let kind = blank(ln);
    let mut a = ln;
    while a > 0 && blank(a - 1) == kind {
        a -= 1;
    }
    let mut b = ln;
    while b < last && blank(b + 1) == kind {
        b += 1;
    }
    if around {
        if b < last {
            let want = !kind;
            while b < last && blank(b + 1) == want {
                b += 1;
            }
        } else if !kind {
            while a > 0 && blank(a - 1) {
                a -= 1;
            }
        }
    }
    Some(buf.line_start(a)..buf.next_char(buf.line_range(b).end))
}

fn bracket_object(
    buf: &Buffer,
    o: usize,
    open: char,
    close: char,
    around: bool,
) -> Option<Range<usize>> {
    // Walk back to the enclosing open bracket.
    let mut depth = 0i32;
    let mut p = o;
    let start = loop {
        let c = buf.char_at(p)?;
        if c == close && p != o {
            depth += 1;
        } else if c == open {
            if depth == 0 {
                break p;
            }
            depth -= 1;
        }
        if p == 0 {
            return None;
        }
        p = buf.prev_char(p);
    };
    // The pair is this object's, not `%`'s: `i<` pairs `<` with `>`.
    let end = m::matching_pair(buf, start, open, close, true)?;
    if around {
        Some(start..buf.next_char(end))
    } else {
        Some(buf.next_char(start)..end)
    }
}

/// How many lines up or down a quote object looks for the other end of
/// a string its line leaves open.
const QUOTE_REACH_LINES: usize = 100;

/// `i"` `a"` and the other quotes, vim's rule, on the cursor's line:
/// off a quote, the nearest one before the cursor opens — the next
/// after it when there is none — and the next after that closes; on a
/// quote, the quotes are paired from the line's start, the cursor's
/// pair taken. Pairing from the start off a quote too let one stray
/// quote earlier on the line — in a regex, a comment — turn every
/// string after it inside out. A quote after an odd run of backslashes
/// is escaped and neither opens nor closes.
///
/// Where the line leaves the string open — a terminal's output wrapped
/// mid-string, a string written over several lines — the other end is
/// looked for on the lines around, `QUOTE_REACH_LINES` either way: a
/// quote before the cursor with none after it closes on the first quote
/// below; an odd count of quotes all after the cursor means the first
/// of them closes what the last quote above opened; a line with no
/// quote is between the last above and the first below.
fn quote_object(buf: &Buffer, o: usize, q: char, around: bool) -> Option<Range<usize>> {
    let ln = buf.line_of(o);
    let quotes_on = |ln: usize| -> Vec<usize> {
        let range = buf.line_range(ln);
        let text = buf.slice(range.clone());
        let bytes = text.as_bytes();
        let escaped =
            |i: usize| bytes[..i].iter().rev().take_while(|&&b| b == b'\\').count() % 2 == 1;
        text.match_indices(q)
            .map(|(i, _)| i)
            .filter(|&i| !escaped(i))
            .map(|i| range.start + i)
            .collect()
    };
    let above = || {
        (ln.saturating_sub(QUOTE_REACH_LINES)..ln)
            .rev()
            .find_map(|l| quotes_on(l).last().copied())
    };
    let below = || {
        let end = buf.line_count().min(ln + 1 + QUOTE_REACH_LINES);
        (ln + 1..end).find_map(|l| quotes_on(l).first().copied())
    };
    let quotes = quotes_on(ln);
    let (a, b) = if quotes.contains(&o) {
        match quotes
            .chunks(2)
            .find(|c| c[0] <= o && o <= *c.last().unwrap())?
        {
            [a, b] => (*a, *b),
            _ => (o, below()?),
        }
    } else {
        let before = quotes.iter().rev().find(|&&i| i < o).copied();
        let mut after = quotes.iter().filter(|&&i| i > o).copied();
        match (before, after.next()) {
            (Some(a), Some(b)) => (a, b),
            (Some(a), None) => (a, below()?),
            (None, Some(b)) => match (quotes.len() % 2 == 1).then(above).flatten() {
                Some(a) => (a, b),
                None => (b, after.next()?),
            },
            (None, None) => (above()?, below()?),
        }
    };
    let n = q.len_utf8();
    if around {
        Some(a..b + n)
    } else {
        Some(a + n..b)
    }
}

/// Past this many bytes the match count is not taken on the frame: the
/// shell counts on a thread (`Effect::CountMatches`) and the message
/// says so until it does. Below it a parallel count is a millisecond.
pub const COUNT_ON_FRAME_BYTES: usize = 8 << 20;

/// The message a search that landed shows: the pattern, the count if it
/// is known for this text — taken now for a small buffer, remembered
/// from the thread's answer for a big one — or the word that it is
/// being counted, which asks the shell for it.
pub fn search_message(ed: &mut Editor, id: kawoosh_doc::BufferId, wrapped: bool) -> String {
    let Some(search) = ed.search.as_mut() else {
        return String::new();
    };
    let version = ed.buffers[id].version();
    let text = ed.buffers[id].tree();
    let count = match search.count {
        Some((b, v, n)) if b == id && v == version => Some(n),
        _ if text.len() <= COUNT_ON_FRAME_BYTES => {
            let n = crate::search::count(text, &search.re);
            search.count = Some((id, version, n));
            Some(n)
        }
        _ => None,
    };
    let pat = &search.pattern;
    let wrapped = if wrapped { " · wrapped" } else { "" };
    match count {
        Some(n) => format!("/{pat}  {n} match(es){wrapped}"),
        None => {
            ed.effects.push(Effect::CountMatches(id));
            format!("/{pat}  counting…{wrapped}")
        }
    }
}

/// `n` / `N`: every head to the next (or previous) match from where it
/// is, wrapping round the end — each a walk from the cursor that stops
/// at the first hit (`crate::search`), so the cost is the distance, not
/// the file. A walk that reads its frame's budget without one hands the
/// primary's rest to the shell (`Effect::SearchContinue`); the other
/// selections stay. The message carries the count (`search_message`).
fn search(ed: &mut Editor, ctx: &Ctx, forward: bool) {
    let Some(search) = ed.search.clone() else {
        ed.message = "no previous search".into();
        return;
    };
    ed.search_hl = true;
    use crate::search::{FRAME_BUDGET, Walk, walk_backward, walk_forward};
    let re = &search.re;
    let pat = &search.pattern;
    let id = view(ed, ctx).buffer;
    let text = ed.buffers[id].tree();
    let len = text.len();
    let mut wrapped = false;
    let mut found_any = false;
    let mut handed_over = None;
    let ext = extend(ed, ctx.view);
    let primary = ed.views[ctx.view].sels.primary();
    // The targets, one per selection, before anything moves: the walks
    // borrow the text and the selections are the view's.
    let mut targets: Vec<Option<usize>> = Vec::new();
    for s in ed.views[ctx.view].sels.iter() {
        let mut head = s.head;
        let mut target = None;
        for _ in 0..ctx.count.max(1) {
            let walk = if forward {
                match walk_forward(text, re, (head + 1).min(len), FRAME_BUDGET) {
                    Walk::NotFound => {
                        wrapped = true;
                        walk_forward(text, re, 0, FRAME_BUDGET)
                    }
                    w => w,
                }
            } else {
                match walk_backward(text, re, head, FRAME_BUDGET) {
                    Walk::NotFound => {
                        wrapped = true;
                        walk_backward(text, re, len, FRAME_BUDGET)
                    }
                    w => w,
                }
            };
            match walk {
                Walk::Found(h) => {
                    head = h.start;
                    target = Some(h.start);
                }
                Walk::Exhausted(at) => {
                    // The primary's walk goes on off the frame, from
                    // where it is now (a count already walked lands the
                    // rest at once); another selection's stops here.
                    if *s == primary && handed_over.is_none() {
                        handed_over = Some((head, at));
                    }
                    break;
                }
                Walk::NotFound => break,
            }
        }
        found_any |= target.is_some();
        targets.push(target);
    }
    if let Some((head, at)) = handed_over {
        ed.effects.push(Effect::SearchContinue {
            buffer: id,
            view: ctx.view,
            head,
            at,
            forward,
        });
    }
    if !found_any {
        ed.message = if handed_over.is_some() {
            format!("/{pat}  searching…")
        } else {
            ed.bell = true;
            format!("not found: {pat}")
        };
        return;
    }
    let mut targets = targets.into_iter();
    ed.views[ctx.view]
        .sels
        .map(|s| match targets.next().flatten() {
            Some(a) => s.with_head(a, ext),
            None => s,
        });
    ed.message = if handed_over.is_some() {
        format!("/{pat}  searching…")
    } else {
        search_message(ed, id, wrapped)
    };
}

/// `[range]s{d}pat{d}rep{d}flags` taken apart: `[range, pat, rep,
/// flags]`, or none when `line` is not that. The delimiter is whatever
/// follows the `s` that is not a letter, a digit or a space; `\{d}` in
/// the pattern or the replacement is the delimiter itself, and the
/// replacement may stop early (`:s/a/b`).
pub fn parse_substitute(line: &str) -> Option<Vec<String>> {
    let range_len = line
        .find(|c: char| {
            !matches!(
                c,
                '0'..='9' | '%' | ',' | '.' | '$' | '\'' | '<' | '>' | '+' | '-'
            )
        })
        .unwrap_or(line.len());
    let (range, rest) = line.split_at(range_len);
    let rest = rest.strip_prefix('s')?;
    let delim = rest.chars().next()?;
    if delim.is_alphanumeric() || delim.is_whitespace() || delim == '\\' {
        return None;
    }
    let rest = &rest[delim.len_utf8()..];
    // Split on the unescaped delimiter, keeping other escapes as they
    // are for the regex and the template to read.
    let mut parts: Vec<String> = vec![String::new()];
    let mut chars = rest.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some(&n) if n == delim => {
                    parts.last_mut().unwrap().push(n);
                    chars.next();
                }
                Some(&n) => {
                    let p = parts.last_mut().unwrap();
                    p.push('\\');
                    p.push(n);
                    chars.next();
                }
                None => parts.last_mut().unwrap().push('\\'),
            }
        } else if c == delim && parts.len() < 3 {
            parts.push(String::new());
        } else {
            parts.last_mut().unwrap().push(c);
        }
    }
    let pat = parts.first().cloned().unwrap_or_default();
    if pat.is_empty() {
        return None;
    }
    let rep = parts.get(1).cloned().unwrap_or_default();
    let flags = parts.get(2).cloned().unwrap_or_default();
    Some(vec![range.to_string(), pat, rep, flags])
}

/// The replacement as the regex crate expands it: vim's `&` is the
/// whole match (`$0`), `\&` a literal `&`, `\n` and `\t` the characters,
/// `\1` the group (`$1`); a `$` the user wrote is kept for `$1` /
/// `${name}` / `$$`.
fn substitute_template(rep: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(rep.len());
    let mut chars = rep.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '&' => out.extend_from_slice(b"${0}"),
            '\\' => match chars.next() {
                Some('&') => out.push(b'&'),
                Some('n') => out.push(b'\n'),
                Some('t') => out.push(b'\t'),
                Some('\\') => out.push(b'\\'),
                Some(d @ '0'..='9') => {
                    out.extend_from_slice(format!("${{{d}}}").as_bytes());
                }
                Some(other) => {
                    let mut b = [0; 4];
                    out.extend_from_slice(other.encode_utf8(&mut b).as_bytes());
                }
                None => out.push(b'\\'),
            },
            other => {
                let mut b = [0; 4];
                out.extend_from_slice(other.encode_utf8(&mut b).as_bytes());
            }
        }
    }
    out
}

/// The lines an ex range names, as byte ranges of whole lines with
/// their newlines: `%` every line, `N` / `N,M` (1-based, `.` the
/// primary's line, `$` the last, `+N` / `-N` from the current), `'<,'>`
/// the lines the selections cover, and none — the lines every selection
/// touches, one range each.
fn substitute_ranges(ed: &Editor, ctx: &Ctx, spec: &str) -> Result<Vec<Range<usize>>, String> {
    let v = view(ed, ctx);
    let buf = &ed.buffers[v.buffer];
    let last = buf.line_count() - 1;
    let cur = buf.line_of(v.sels.primary().head);
    let lines_of = |a: usize, b: usize| -> Range<usize> {
        let (a, b) = (a.min(last), b.min(last));
        let start = buf.line_range(a.min(b)).start;
        let end = buf.line_range(a.max(b)).end;
        // The newline (or `\r\n`) belongs to the line.
        let mut end = end;
        while end < buf.len() && matches!(buf.byte_at(end), Some(b'\r' | b'\n')) {
            end += 1;
            if buf.byte_at(end - 1) == Some(b'\n') {
                break;
            }
        }
        start..end
    };
    let addr = |s: &str| -> Result<usize, String> {
        let s = s.trim();
        Ok(match s {
            "." | "" => cur,
            "$" => last,
            _ if s.starts_with('+') => cur + s[1..].parse::<usize>().unwrap_or(1),
            _ if s.starts_with('-') => cur.saturating_sub(s[1..].parse::<usize>().unwrap_or(1)),
            "'<" | "'>" => cur,
            _ => s
                .parse::<usize>()
                .map(|n| n.saturating_sub(1))
                .map_err(|_| format!("bad range: {spec}"))?,
        })
    };
    let mut ranges: Vec<Range<usize>> = match spec {
        "" => v
            .sels
            .iter()
            .map(|s| {
                let (a, b) = (
                    buf.line_of(s.start()),
                    buf.line_of(s.end().saturating_sub(usize::from(!s.is_empty()))),
                );
                lines_of(a, b)
            })
            .collect(),
        "%" => vec![lines_of(0, last)],
        "'<,'>" => v
            .sels
            .iter()
            .map(|s| {
                lines_of(
                    buf.line_of(s.start()),
                    buf.line_of(s.end().saturating_sub(usize::from(!s.is_empty()))),
                )
            })
            .collect(),
        _ => match spec.split_once(',') {
            Some((a, b)) => vec![lines_of(addr(a)?, addr(b)?)],
            None => {
                let a = addr(spec)?;
                vec![lines_of(a, a)]
            }
        },
    };
    // In order, joined where they touch, so each byte is searched once.
    ranges.sort_by_key(|r| r.start);
    let mut joined: Vec<Range<usize>> = Vec::new();
    for r in ranges {
        match joined.last_mut() {
            Some(l) if r.start <= l.end => l.end = l.end.max(r.end),
            _ => joined.push(r),
        }
    }
    Ok(joined)
}

/// `:[range]s/pat/rep/[flags]` — flags `g` (every match on a line, not
/// the first), `i` (case-insensitive). The matches are found on every
/// core (`search::substitutions`) and applied as one edit, one tree for
/// the lot however many (`Buffer::replace_many`); the selections follow
/// the text and the primary lands at the start of the last line changed,
/// as vim's does. The pattern becomes the search `n` walks.
fn substitute(ed: &mut Editor, ctx: &Ctx) {
    let (range, pat, rep, flags) = match ctx.args.as_slice() {
        [r, p, s, f] => (r.as_str(), p.as_str(), s.as_str(), f.as_str()),
        _ => {
            ed.message = "usage: :[range]s/pattern/replacement/[gi]".into();
            return;
        }
    };
    let global = flags.contains('g');
    let pattern = if flags.contains('i') {
        format!("(?i){pat}")
    } else {
        pat.to_string()
    };
    if let Err(e) = ed.set_search(&pattern, false) {
        ed.message = e;
        return;
    }
    let re = ed.search.as_ref().unwrap().re.clone();
    let id = view(ed, ctx).buffer;
    if ed.buffers[id].read_only {
        ed.message = "buffer is read-only".into();
        return;
    }
    if ed.buffers[id].loading.is_some() {
        ed.message = "still opening".into();
        return;
    }
    let ranges = match substitute_ranges(ed, ctx, range) {
        Ok(r) => r,
        Err(e) => {
            ed.message = e;
            return;
        }
    };
    let template = substitute_template(rep);
    let started = std::time::Instant::now();
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    let mut lines = 0;
    for r in ranges {
        let text = ed.buffers[id].tree();
        let found = crate::search::substitutions(text, &re, r, global, &template);
        lines += found.lines;
        for (range, bytes) in found.edits {
            let s = String::from_utf8(bytes)
                .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
            edits.push((range, s));
        }
    }
    // In a multibuffer, a match outside the excerpts — a file's name in
    // its header — is not a file's text, and is left.
    let mut kept_out = 0;
    if ed.is_multi(id) {
        let before = edits.len();
        edits.retain(|(r, t)| ed.multi_refuses(id, &[(r.clone(), t.as_str())]).is_none());
        kept_out = before - edits.len();
    }
    if edits.is_empty() {
        ed.message = if kept_out > 0 {
            format!("no match in the excerpts: {pat}")
        } else {
            format!("no match: {pat}")
        };
        return;
    }
    let count = edits.len();
    let v0 = ed.buffers[id].version();
    let last_start = edits.last().map(|(r, _)| r.start).unwrap_or(0);
    {
        let refs: Vec<(Range<usize>, &str)> =
            edits.iter().map(|(r, t)| (r.clone(), t.as_str())).collect();
        ed.buffers[id].replace_many(&refs);
    }
    drop(edits);
    // The selections follow the text; the primary to the last changed
    // line's start.
    let (buffers, views) = (&ed.buffers, &mut ed.views);
    let buf = &buffers[id];
    let journal = buf.journal();
    let primary = views[ctx.view].sels.primary();
    let last_line_start = {
        let at = journal
            .transform_offset(last_start, v0, kawoosh_doc::Bias::Left)
            .unwrap_or(last_start);
        buf.line_range(buf.line_of(at)).start
    };
    views[ctx.view].sels.map(|s| {
        if s == primary {
            Selection::new(last_line_start, last_line_start)
        } else {
            let t = |o: usize| {
                journal
                    .transform_offset(o, v0, kawoosh_doc::Bias::Left)
                    .unwrap_or(o)
            };
            Selection::new(t(s.anchor), t(s.head))
        }
    });
    if ed.mode(ctx.view) == Mode::Visual {
        ed.set_mode(ctx.view, Mode::Normal);
    }
    let took = started.elapsed();
    ed.message = format!(
        "{count} substitution(s) on {lines} line(s){}{}",
        if took.as_millis() >= 100 {
            format!(" in {:.1}s", took.as_secs_f64())
        } else {
            String::new()
        },
        if kept_out > 0 {
            format!(", {kept_out} outside the excerpts left")
        } else {
            String::new()
        }
    );
    ed.sync_multis();
}

/// Writes `buf` to `path` through a file beside it, renamed over the
/// target once whole: the text streams out piece by piece (a big file is
/// never one string), a crash mid-write leaves the old file, and a
/// buffer whose text is the file's own mapping keeps reading the old
/// inode rather than the bytes being written over it.
pub(crate) fn save_beside(
    buf: &kawoosh_doc::Buffer,
    path: &std::path::Path,
) -> std::io::Result<()> {
    // A host's file: the text whole, through its domain, which writes a
    // sibling and renames it over (docs/design/domains.md Decision 4).
    if let Some(host) = kawoosh_doc::fs::remote(path) {
        let (fs, p) = host?;
        let mut bytes = Vec::with_capacity(buf.len());
        buf.write_to(&mut bytes)?;
        // The host's path, cut on `/` alone: a `\` there is a name's.
        if let Some(dir) = kawoosh_doc::paths::host_parent(&p)
            && !dir.as_os_str().is_empty()
            && fs.stat(dir).is_err()
        {
            fs.create(dir, true)?;
        }
        return fs.write(&p, &bytes);
    }
    // A link is written through, not replaced; the file's mode stays.
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mode = std::fs::metadata(&path).ok().map(|m| m.permissions());
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.kawoosh~"));
    let result = (|| {
        // A new file in a directory that is not there yet: the
        // directory first (a `.kawoosh/settings.lua` from its template).
        if let Some(dir) = path.parent()
            && !dir.exists()
        {
            std::fs::create_dir_all(dir)?;
        }
        let mut out = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(&tmp)?);
        buf.write_to(&mut out)?;
        std::io::Write::flush(&mut out)?;
        out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        if let Some(mode) = mode {
            std::fs::set_permissions(&tmp, mode)?;
        }
        std::fs::rename(&tmp, &path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// `:w`: the buffer written — now, or once formatted when its
/// `format_on_save` is on (then `after` is the shell's to do, and this
/// answers false). `:w!` writes at once, unformatted.
fn write(ed: &mut Editor, ctx: &Ctx, after: crate::AfterWrite) -> bool {
    let id = view(ed, ctx).buffer;
    if let Some(p) = ctx.args.first() {
        ed.buffers[id].path = Some(std::path::PathBuf::from(p));
        ed.buffers[id].name = kawoosh_doc::paths::file_name(std::path::Path::new(p))
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.clone());
    }
    if ed.is_multi(id) {
        return ed.write_multi(id, ctx.bang());
    }
    let buf = &ed.buffers[id];
    // A hooked buffer is written by its hook, on the shell's side, which
    // marks it saved when the hook says it is (`Effect::Write`) — a hook
    // that asks first leaves it modified until the answer.
    if buf.path.is_none() && buf.hook.is_some() {
        ed.effects.push(Effect::Write(id));
        return true;
    }
    let Some(path) = buf.path.clone() else {
        ed.message = "no file name (use :w <path>)".into();
        return false;
    };
    if buf.loading.is_some() {
        ed.message = "still opening".into();
        return false;
    }
    // Someone else wrote the file since it was read: asked, not written
    // over — `:w!` does. Not for `:w PATH`, which names a file of its
    // own choosing.
    if ctx.args.is_empty() && !ctx.bang() && ed.disk_state(id) == crate::disk::Disk::Changed {
        ed.message = format!(
            "\"{}\" changed on disk since it was read; :w! writes over it, :e! loads it",
            path.display()
        );
        ed.effects.push(Effect::DiskConflict(id));
        return false;
    }
    if !ctx.bang() && ed.formats_on_save(id) {
        ed.message = "formatting before writing…".into();
        ed.effects.push(Effect::FormatThenWrite {
            buffers: vec![id],
            after,
        });
        return false;
    }
    ed.write_now(id)
}

/// `:e!`: the file's text as it is on disk, put in as one journaled
/// edit — undoable — and the buffer clean on it. A file that is gone
/// is said so, the buffer left as it is.
fn reload(ed: &mut Editor, ctx: &Ctx) {
    let id = view(ed, ctx).buffer;
    ed.message = match ed.reload_from_disk(id) {
        Ok(m) | Err(m) => m,
    };
}

fn paste(ed: &mut Editor, ctx: &Ctx, after: bool) {
    put(ed, ctx.view, ctx.count.max(1), after, None);
}

/// The case `op` turns `t` to: `case lower`, `case upper`, `case toggle`.
fn case_turned(t: &str, op: &str) -> String {
    match op {
        "case lower" => t.to_lowercase(),
        "case upper" => t.to_uppercase(),
        _ => {
            let mut out = String::with_capacity(t.len());
            for c in t.chars() {
                if c.is_uppercase() {
                    out.extend(c.to_lowercase());
                } else {
                    out.extend(c.to_uppercase());
                }
            }
            out
        }
    }
}

/// Whether there is a register to put, saying why not when there is
/// none: nothing taken yet, or a secret that went, whose covering text
/// is not what `p` was pressed for.
fn has_register(ed: &mut Editor) -> bool {
    if ed.pending_register == Some('_') {
        ed.message = "the _ register is always empty".into();
        false
    } else if ed.memory.spent() {
        ed.message =
            "the register's secret is gone — put once, or its time was up: yank it again".into();
        false
    } else if ed.memory.head().is_none() {
        ed.message = "nothing to paste".into();
        false
    } else {
        true
    }
}

/// Whether a put into buffer `id` spends the register's head
/// (docs/design/secrets.md Decision 2, amended 2026-09-27): a secret
/// leaving for a buffer that is not private is put once; inside a
/// private buffer nothing leaves, so a put spends nothing there, and
/// what is put becomes a secret if it was not one.
fn spend_on_put(ed: &mut Editor, id: kawoosh_doc::BufferId) -> bool {
    let n = ed.memory.len();
    if n == 0 {
        return false;
    }
    if ed.buffers[id].private {
        ed.memory.make_secret(n - 1);
        return false;
    }
    ed.memory.head().is_some_and(|m| m.secret)
}

/// `p` / `P` on a visual selection: every selection replaced with the
/// register, COUNT times, and normal mode. `p` puts what it replaced in
/// the register, as vim's does, so the next `p` swaps it back; `P`
/// (`keep`) leaves the register as it was, for one text put over many.
/// Lines put over characters go on lines of their own; characters over
/// lines are a line.
fn paste_over(ed: &mut Editor, ctx: &Ctx, keep: bool) {
    if !has_register(ed) {
        return;
    }
    let id = view(ed, ctx).buffer;
    let once = spend_on_put(ed, id);
    let Some(head) = ed.memory.head() else {
        return;
    };
    let mut text = head.text.repeat(ctx.count.max(1));
    let from_lines = head.linewise;
    let buf = &ed.buffers[id];
    let lines = ed.views[ctx.view].visual_linewise;
    let ranges: Vec<(Range<usize>, bool)> = ed.views[ctx.view]
        .sels
        .iter()
        .map(|s| {
            if lines {
                (line_range_of_sel(buf, s, 0), true)
            } else {
                (sel_range(buf, s), false)
            }
        })
        .collect();
    let body = text.trim_end_matches('\n').to_string();
    let edits: Vec<(usize, Range<usize>, String)> = ranges
        .iter()
        .enumerate()
        .map(|(i, (r, _))| {
            // A linewise range on the last line is the newline before
            // it and the line (`line_range_of_sel`).
            let last = lines && buf.char_at(r.start) == Some('\n') && r.end == buf.len();
            let put = match (lines, from_lines) {
                (true, _) if last => format!("\n{body}"),
                (true, _) => format!("{body}\n"),
                (false, true) => format!("\n{body}\n"),
                (false, false) => text.clone(),
            };
            (i, r.clone(), put)
        })
        .collect();
    let texts: Vec<String> = ranges.iter().map(|(r, _)| buf.slice(r.clone())).collect();
    if once {
        let n = ed.memory.len() - 1;
        ed.memory.forget(n);
    }
    if !keep {
        set_register(ed, id, &ranges, &texts, lines, crate::Took::Delete);
    }
    ed.set_mode(ctx.view, Mode::Normal);
    let linewise_put = lines || from_lines;
    ed.edit_each(ctx.view, edits, move |start, len| {
        if linewise_put {
            Selection::point(start)
        } else {
            Selection::point(start + len.saturating_sub(1))
        }
    });
    if linewise_put {
        let buf = &ed.buffers[id];
        ed.views[ctx.view].sels.map(|s| {
            // Past the newline a put over characters starts with, or the
            // one before the last line: onto the first line put.
            let at = if buf.byte_at(s.head) == Some(b'\n') {
                buf.next_char(s.head)
            } else {
                s.head
            };
            Selection::point(m::first_nonblank(buf, buf.line_of(at.min(buf.len()))))
        });
    }
    ed.last_put = None;
    text_buffer::wipe_string(&mut text);
    clamp_sels(ed, ctx.view);
}

/// `multi more`: the excerpt at the primary caret grown `way` by the
/// argument's lines, else COUNT's, else `multi.expand`'s; the message
/// says how many came, or why none did.
fn multi_more(ed: &mut Editor, ctx: &Ctx, way: crate::Grow) {
    let n = match ctx.arg(0).map(str::parse::<usize>) {
        Some(Ok(n)) if n > 0 => n,
        Some(_) => {
            ed.message = "multi more: a number of lines, 1 or more".into();
            return;
        }
        None if ctx.has_count => ctx.count,
        None => ed
            .settings
            .int("multi.expand")
            .map_or(5, |n| n.max(1) as usize),
    };
    let id = view(ed, ctx).buffer;
    let head = view(ed, ctx).sels.primary().head;
    ed.message = match ed.multi_grow(id, head, way, n) {
        Ok(l) => format!("{l} more line{}", if l == 1 { "" } else { "s" }),
        Err(why) => why,
    };
}

/// `[<Space>` / `]<Space>`: COUNT empty lines above or below each
/// caret's line — once a line, however many carets are on it — and every
/// selection kept where it was in the text.
fn blank_lines(ed: &mut Editor, ctx: &Ctx, below: bool) {
    let id = view(ed, ctx).buffer;
    let buf = &ed.buffers[id];
    let n = ctx.count.max(1);
    let mut inserts: Vec<usize> = ed.views[ctx.view]
        .sels
        .iter()
        .map(|s| {
            let ln = buf.line_of(s.head);
            if below {
                buf.line_range(ln).end
            } else {
                buf.line_start(ln)
            }
        })
        .collect();
    inserts.sort_unstable();
    inserts.dedup();
    let blank = "\n".repeat(n);
    // A caret moves by the lines put before it: above, one at its own
    // line's start moves down with it; below, the insert at its line's
    // end is after it even where the line is empty.
    let carry = |pos: usize| -> usize {
        let before = inserts
            .iter()
            .filter(|&&at| if below { at < pos } else { at <= pos })
            .count();
        pos + before * n
    };
    let items: Vec<Selection> = ed.views[ctx.view]
        .sels
        .iter()
        .map(|s| Selection::new(carry(s.anchor), carry(s.head)))
        .collect();
    let primary = ed.views[ctx.view].sels.primary;
    let edits = inserts
        .iter()
        .enumerate()
        .map(|(i, &at)| (i, at..at, blank.clone()))
        .collect();
    ed.edit_each(ctx.view, edits, |start, _| Selection::point(start));
    let v = &mut ed.views[ctx.view];
    v.sels.items = items;
    v.sels.primary = primary;
    v.sels.normalize();
}

/// Puts the register `count` times at every selection, after it or
/// before; `walk` is a `[p` `]p` walk under way (its order and place),
/// kept on the put it makes — none starts one at the register.
fn put(ed: &mut Editor, view: ViewId, count: usize, after: bool, walk: Option<(Vec<u64>, usize)>) {
    if !has_register(ed) {
        return;
    }
    let id = ed.views[view].buffer;
    let once = spend_on_put(ed, id);
    let Some(head) = ed.memory.head() else {
        return;
    };
    let mut text = head.text.repeat(count);
    let linewise = head.linewise;
    let before = ed.views[view].sels.clone();
    let version = ed.buffers[id].version();
    let sels = before.items.clone();
    let buf = &ed.buffers[id];
    let edits: Vec<(usize, Range<usize>, String)> = sels
        .iter()
        .enumerate()
        .map(|(i, s)| {
            if linewise {
                let ln = buf.line_of(s.head);
                let mut t = text.clone();
                if !t.ends_with('\n') {
                    t.push('\n');
                }
                // The lines' own break, `\r\n` from a CRLF file.
                let brk = if t.ends_with("\r\n") { "\r\n" } else { "\n" };
                if after {
                    let end = buf.line_range(ln).end;
                    if end >= buf.len() {
                        let lines = t.strip_suffix(brk).unwrap_or(&t);
                        (i, end..end, format!("{brk}{lines}"))
                    } else {
                        let next = buf.line_start(ln + 1);
                        (i, next..next, t)
                    }
                } else {
                    let start = buf.line_start(ln);
                    (i, start..start, t)
                }
            } else {
                let at = if after && !s.is_empty() {
                    s.end()
                } else if after {
                    buf.next_char(s.head)
                        .min(buf.line_range(buf.line_of(s.head)).end)
                } else {
                    s.head
                };
                (i, at..at, text.clone())
            }
        })
        .collect();
    ed.edit_each(view, edits, move |start, len| {
        if linewise {
            Selection::point(start)
        } else {
            Selection::point(start + len.saturating_sub(1))
        }
    });
    if linewise {
        let buf = &ed.buffers[id];
        let v = &mut ed.views[view];
        v.sels.map(|s| {
            let ln = buf.line_of(s.head);
            let at_break = matches!(buf.byte_at(s.head), Some(b'\n' | b'\r'));
            let ln = if after && s.head > 0 && at_break {
                ln + 1
            } else {
                ln
            };
            Selection::point(m::first_nonblank(buf, ln.min(buf.line_count() - 1)))
        });
    }
    if once {
        let head = ed.memory.len() - 1;
        ed.memory.forget(head);
        text_buffer::wipe_string(&mut text);
        ed.last_put = None;
    } else if ed.buffers[id].version() != version {
        let (order, at) = walk.unwrap_or_else(|| {
            let n = ed.memory.len();
            ((0..n).rev().filter_map(|i| ed.memory.id(i)).collect(), 0)
        });
        ed.last_put = Some(crate::LastPut {
            view,
            buffer: id,
            version: ed.buffers[id].version(),
            before,
            after,
            count,
            order,
            at,
        });
    }
}

/// `[p` `]p`: the last put replaced with the text one older (or newer)
/// in the memory as it stood when the put was made — COUNT steps, a
/// secret or a forgotten one stepped over — and that text made the
/// register's head, so `p` puts it next. The put is undone and made
/// again, so one `u` takes the whole of it back; the replaced puts stay
/// in the undo tree as branches.
fn put_step(ed: &mut Editor, ctx: &Ctx, older: bool) {
    let Some(lp) = ed.last_put.clone() else {
        ed.message = "the last change was not a put".into();
        return;
    };
    if lp.view != ctx.view || ed.buffers.get(lp.buffer).map(|b| b.version()) != Some(lp.version) {
        ed.last_put = None;
        ed.message = "the last change was not a put".into();
        return;
    }
    let mut found = None;
    let mut steps = ctx.count.max(1);
    let mut j = lp.at;
    loop {
        j = match older {
            true => j + 1,
            false => match j.checked_sub(1) {
                Some(j) => j,
                None => break,
            },
        };
        if j >= lp.order.len() {
            break;
        }
        if let Some(i) = ed.memory.position(lp.order[j])
            && !ed.memory.moments()[i].secret
        {
            found = Some((j, i));
            steps -= 1;
            if steps == 0 {
                break;
            }
        }
    }
    let Some((j, i)) = found else {
        ed.message = if older {
            "no older text"
        } else {
            "no newer text"
        }
        .into();
        return;
    };
    if !ed.undo(ctx.view) {
        return;
    }
    // The undo settled the command's checkpoint; the put made again is
    // a node of its own, a child of the text before the first put.
    ed.views[ctx.view].sels = lp.before.clone();
    ed.open_checkpoint(ctx.view);
    ed.memory.recall(i);
    ed.effects.push(Effect::Recalled);
    let n = lp.order.len();
    put(ed, ctx.view, lp.count, lp.after, Some((lp.order, j)));
    let shown = ed
        .memory
        .head()
        .map(|m| {
            m.shown()
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(60)
                .collect::<String>()
        })
        .unwrap_or_default();
    ed.message = format!("put {} of {n}: {shown}", j + 1);
}

pub fn install(ed: &mut Editor) {
    use MotionKind::*;

    // ---- motions
    ed.motion("move left", Exclusive, |b, o, n| {
        let mut o = o;
        let ls = b.line_start(b.line_of(o));
        for _ in 0..n {
            if o > ls {
                o = b.prev_char(o);
            }
        }
        o
    });
    ed.motion("move right", Exclusive, |b, o, n| {
        let mut o = o;
        let le = b.line_range(b.line_of(o)).end;
        for _ in 0..n {
            if o < le {
                o = b.next_char(o);
            }
        }
        o
    });
    ed.register_kind("move down", Kind::Motion(Linewise), |ed, ctx| {
        vertical(ed, ctx, 1)
    });
    ed.register_kind("move up", Kind::Motion(Linewise), |ed, ctx| {
        vertical(ed, ctx, -1)
    });
    ed.motion("line start", Exclusive, |b, o, _| {
        b.line_start(b.line_of(o))
    });
    ed.motion("line nonblank", Exclusive, |b, o, _| {
        m::first_nonblank(b, b.line_of(o))
    });
    ed.motion("line end", Inclusive, |b, o, n| {
        // Onto the last char, as vim's `$`; an empty line stays put.
        let ln = (b.line_of(o) + n - 1).min(b.line_count() - 1);
        let r = b.line_range(ln);
        if r.end > r.start {
            b.prev_char(r.end)
        } else {
            r.end
        }
    });
    ed.motion("line end insert", Exclusive, |b, o, _| {
        b.line_range(b.line_of(o)).end
    });
    ed.register_kind("word next", Kind::Motion(Exclusive), |ed, ctx| {
        // Under an operator, `w` stops at the end of its line (`dw` on the
        // last word never joins lines); under `c`, from inside a word, at
        // the word's end, the space after it kept — vim's two special
        // cases.
        let op = ed.pending_op.is_some();
        let change = matches!(ed.pending_op, Some(("change", _)));
        motion(ed, ctx, |b, o, n| {
            if change && let Some(end) = m::change_word_end(b, o, n, false) {
                return end;
            }
            let target = (0..n).fold(o, |o, _| m::next_word_start(b, o));
            let le = b.line_range(b.line_of(o)).end;
            if op && target > le { le } else { target }
        });
    });
    ed.motion("word prev", Exclusive, |b, o, n| {
        (0..n).fold(o, |o, _| m::prev_word_start(b, o))
    });
    ed.motion("word end", Inclusive, |b, o, n| {
        (0..n).fold(o, |o, _| m::next_word_end(b, o))
    });
    ed.motion("word end back", Inclusive, |b, o, n| {
        (0..n).fold(o, |o, _| m::prev_word_end(b, o))
    });
    // Insert's ⌥→: one past the word's end, the caret after it as a
    // Mac puts it — `word end` stands on its last char, as vim's `e`.
    ed.motion("word end insert", Exclusive, |b, o, n| {
        (0..n).fold(o, |o, _| m::word_end_after(b, o))
    });
    // vim's WORDs: what whitespace alone ends — a path, `a.b(c)`.
    ed.register_kind("bigword next", Kind::Motion(Exclusive), |ed, ctx| {
        let op = ed.pending_op.is_some();
        let change = matches!(ed.pending_op, Some(("change", _)));
        motion(ed, ctx, |b, o, n| {
            if change && let Some(end) = m::change_word_end(b, o, n, true) {
                return end;
            }
            let target = (0..n).fold(o, |o, _| m::next_bigword_start(b, o));
            let le = b.line_range(b.line_of(o)).end;
            if op && target > le { le } else { target }
        });
    });
    ed.motion("bigword prev", Exclusive, |b, o, n| {
        (0..n).fold(o, |o, _| m::prev_bigword_start(b, o))
    });
    ed.motion("bigword end", Inclusive, |b, o, n| {
        (0..n).fold(o, |o, _| m::next_bigword_end(b, o))
    });
    ed.motion("bigword end back", Inclusive, |b, o, n| {
        (0..n).fold(o, |o, _| m::prev_bigword_end(b, o))
    });
    ed.motion("paragraph next", Exclusive, |b, o, n| {
        (0..n).fold(o, |o, _| m::paragraph_next(b, o))
    });
    ed.motion("paragraph prev", Exclusive, |b, o, n| {
        (0..n).fold(o, |o, _| m::paragraph_prev(b, o))
    });
    // `H` `M` `L`: the pane's top, middle and bottom line, COUNT lines
    // in from the top or the bottom — inside `scrolloff`'s margin, as
    // vim's, so the pane holds still, but for the buffer's own ends.
    for (name, at) in [
        ("screen top", 0u8),
        ("screen middle", 1),
        ("screen bottom", 2),
    ] {
        ed.register_kind(name, Kind::Motion(Linewise), move |ed, ctx| {
            let v = &ed.views[ctx.view];
            let rows = v.rows.max(1);
            let last = ed.buffers[v.buffer].line_count().saturating_sub(1);
            let top = v.top.min(last);
            let bottom = (top + rows - 1).min(last);
            let margin = ed
                .settings
                .int("scrolloff")
                .map(|n| n.max(0) as usize)
                .unwrap_or(3)
                .min((rows - 1) / 2);
            let first = if top == 0 { 0 } else { top + margin };
            let end = if bottom == last {
                bottom
            } else {
                bottom - margin
            };
            let n = if ctx.has_count { ctx.count - 1 } else { 0 };
            let ln = match at {
                0 => (first + n).min(end),
                1 => top + (bottom - top) / 2,
                _ => end.saturating_sub(n).max(first),
            };
            goto_line(ed, ctx.view, ln + 1);
        });
    }
    ed.register_kind("goto file start", Kind::Motion(Linewise), |ed, ctx| {
        let n = if ctx.has_count { ctx.count } else { 1 };
        goto_line(ed, ctx.view, n);
    });
    ed.register_kind("goto file end", Kind::Motion(Linewise), |ed, ctx| {
        let n = if ctx.has_count {
            ctx.count
        } else {
            ed.buffer_of(ctx.view).line_count()
        };
        goto_line(ed, ctx.view, n);
    });
    ed.register_kind("goto line", Kind::Motion(Linewise), |ed, ctx| {
        goto_line(ed, ctx.view, ctx.count)
    });
    ed.register_kind("page half down", Kind::Motion(Linewise), |ed, ctx| {
        let n = (ed.views[ctx.view].rows / 2).max(1) as i64;
        vertical(
            ed,
            &Ctx {
                count: 1,
                ..ctx.clone()
            },
            n,
        );
    });
    ed.register_kind("page half up", Kind::Motion(Linewise), |ed, ctx| {
        let n = (ed.views[ctx.view].rows / 2).max(1) as i64;
        vertical(
            ed,
            &Ctx {
                count: 1,
                ..ctx.clone()
            },
            -n,
        );
    });
    ed.register_kind("page down", Kind::Motion(Linewise), |ed, ctx| {
        let n = (ed.views[ctx.view].rows.saturating_sub(2)).max(1) as i64;
        vertical(
            ed,
            &Ctx {
                count: 1,
                ..ctx.clone()
            },
            n,
        );
    });
    ed.register_kind("page up", Kind::Motion(Linewise), |ed, ctx| {
        let n = (ed.views[ctx.view].rows.saturating_sub(2)).max(1) as i64;
        vertical(
            ed,
            &Ctx {
                count: 1,
                ..ctx.clone()
            },
            -n,
        );
    });
    ed.register_kind("match_bracket", Kind::Motion(Inclusive), match_pair);
    for (name, forward, till) in [
        ("find char", true, false),
        ("find char back", false, false),
        ("till char", true, true),
        ("till char back", false, true),
    ] {
        ed.register_kind_char(
            name,
            Kind::Motion(if forward { Inclusive } else { Exclusive }),
            move |ed, ctx| {
                let Some(c) = ctx.arg_char else { return };
                ed.last_find = Some((c, forward, till));
                motion(ed, ctx, |b, o, n| {
                    let ln = b.line_of(o);
                    let range = b.line_range(ln);
                    let text = b.slice(range.clone());
                    let rel = o - range.start;
                    let mut found = None;
                    let mut left = n;
                    if forward {
                        for (i, ch) in text.char_indices() {
                            if i > rel && ch == c {
                                left -= 1;
                                if left == 0 {
                                    found = Some(i);
                                    break;
                                }
                            }
                        }
                    } else {
                        for (i, ch) in text.char_indices().rev() {
                            if i < rel && ch == c {
                                left -= 1;
                                if left == 0 {
                                    found = Some(i);
                                    break;
                                }
                            }
                        }
                    }
                    match found {
                        Some(i) if till && forward => {
                            let abs = range.start + i;
                            b.prev_char(abs)
                        }
                        Some(i) if till => b.next_char(range.start + i),
                        Some(i) => range.start + i,
                        None => o,
                    }
                });
            },
        );
    }
    // `;`: the last `f` / `t` again, across lines, as many as COUNT.
    for (name, back) in [("find repeat", false), ("find repeat back", true)] {
        let kind = Kind::Motion(if back { Exclusive } else { Inclusive });
        ed.register_kind(name, kind, move |ed, ctx| {
            let Some((c, fwd, till)) = ed.last_find else {
                ed.message = "no previous find".into();
                return;
            };
            let forward = fwd != back;
            motion(ed, ctx, move |b, o, n| {
                find_across(b, o, c, forward, till, n).unwrap_or(o)
            });
        });
    }
    ed.register_kind("search next", Kind::Motion(Exclusive), |ed, ctx| {
        search(ed, ctx, true)
    });
    ed.register_kind("search prev", Kind::Motion(Exclusive), |ed, ctx| {
        search(ed, ctx, false)
    });
    ed.register_with_args("substitute", Args::rest(&[ArgKind::Text]), substitute);
    // `*`: the word under the caret, whole — or, in visual mode, the
    // primary selection's text as it is, the selections put down at
    // their starts first (vim's `*` over a selection), so the caret
    // lands on the next match and nothing is stretched to it.
    ed.register("search word", |ed, ctx| {
        let id = view(ed, ctx).buffer;
        let pattern = if ed.mode(ctx.view) == Mode::Visual {
            let sels = &ed.views[ctx.view].sels;
            let primary = sels.primary();
            let at = sels.iter().position(|s| *s == primary).unwrap_or(0);
            let r = sel_ranges(ed, ctx.view).swap_remove(at);
            let text = regex::escape(&ed.buffers[id].slice(r));
            ed.set_mode(ctx.view, Mode::Normal);
            ed.views[ctx.view].sels.map(|s| Selection::point(s.start()));
            text
        } else {
            let buf = &ed.buffers[id];
            let (a, b) = m::word_at(buf, ed.views[ctx.view].sels.primary().head);
            if a == b {
                return;
            }
            format!(r"\b{}\b", regex::escape(&buf.slice(a..b)))
        };
        if let Err(e) = ed.set_search(&pattern, false) {
            ed.message = e;
            return;
        }
        search(ed, ctx, true);
    });

    // ---- text objects (operator-pending and visual)
    // The object is named by the key: `di"` on a Russian layout is the
    // quotes, where ⇧ on that key typed `Э` (`CharArg`).
    ed.register_kind_key("textobject inner", Kind::TextObject, |ed, ctx| {
        textobject(ed, ctx, false)
    });
    ed.register_kind_key("textobject around", Kind::TextObject, |ed, ctx| {
        textobject(ed, ctx, true)
    });
    // The syntax's own, walked: `]f` `[f` (docs/design/nodes.md
    // Decision 9).
    for (name, forward) in [("function next", true), ("function prev", false)] {
        ed.register_kind(name, Kind::Motion(Exclusive), move |ed, ctx| {
            syntax_jump(ed, ctx, "function", forward)
        });
    }

    // ---- operators
    for op in [
        "delete",
        "change",
        "yank",
        "indent",
        "dedent",
        "reindent",
        "case lower",
        "case upper",
        "case toggle",
        "comment",
        "comment block",
    ] {
        ed.register_kind(op, Kind::Operator, move |ed, ctx| operator(ed, ctx, op));
    }
    // `gcc`: `c` after a pending `comment` is the operator doubled —
    // its line, COUNT lines — and after any other operator what `c`
    // was there (`cc` changes the line, `dc` nothing). With no operator
    // pending (`:comment lines`, Lua, the socket) the selections' lines
    // at once, COUNT lines each (docs/design/comments.md Decision 5).
    // `gbc` the same for the block pair (Comment.nvim's letters: `b`
    // after an operator is the word-back motion, so no `gbb`).
    ed.register("comment lines", |ed, ctx| {
        comment_lines_cmd(ed, ctx, "comment")
    });
    ed.register("comment block lines", |ed, ctx| {
        comment_lines_cmd(ed, ctx, "comment block")
    });
    // `~`: COUNT characters from the caret their case turned, the caret
    // past them, as vim's (`notildeop`).
    ed.register("case toggle char", |ed, ctx| {
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let ranges: Vec<(Range<usize>, bool)> = ed.views[ctx.view]
            .sels
            .iter()
            .map(|s| {
                let le = buf.line_range(buf.line_of(s.head)).end;
                let mut e = s.head;
                for _ in 0..ctx.count.max(1) {
                    if e < le {
                        e = buf.next_char(e);
                    }
                }
                (s.head..e, false)
            })
            .collect();
        apply_operator(ed, ctx.view, "case toggle", ranges.clone());
        // Onto the character after the last turned, or the line's last.
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let mut i = 0;
        ed.views[ctx.view].sels.map(|s| {
            let (r, _) = &ranges[i.min(ranges.len() - 1)];
            i += 1;
            let e = s.head + (r.end - r.start);
            let range = buf.line_range(buf.line_of(s.head));
            if e < range.end {
                Selection::point(e)
            } else {
                Selection::point(buf.prev_char(range.end).max(range.start))
            }
        });
    });
    // `[<Space>` `]<Space>`: COUNT blank lines above or below each
    // caret's line, the carets staying where they are (unimpaired's).
    for (name, below) in [("line blank above", false), ("line blank below", true)] {
        ed.register(name, move |ed, ctx| blank_lines(ed, ctx, below));
    }
    // A multibuffer's excerpt grown (docs/design/search.md Decision 13):
    // `zo` `<S-CR>` both ways, `zk` `zj` above and below — N lines, a
    // COUNT's, else `multi.expand`.
    for (name, way, doc) in [
        (
            "multi more",
            crate::Grow::Both,
            "COUNT more of the file's lines (`multi.expand`) above and below the excerpt at the caret — on a `⋯`, both sides toward it (`zo`, `<S-CR>`)",
        ),
        (
            "multi more above",
            crate::Grow::Above,
            "COUNT more of the file's lines (`multi.expand`) above the excerpt at the caret (`zk`)",
        ),
        (
            "multi more below",
            crate::Grow::Below,
            "COUNT more of the file's lines (`multi.expand`) below the excerpt at the caret (`zj`)",
        ),
    ] {
        ed.register_spec(
            Spec::new(name)
                .when(&["language:multibuffer"])
                .args(Args::new(&[ArgKind::Text]))
                .doc(doc),
            move |ed, ctx| multi_more(ed, ctx, way),
        );
    }
    ed.register("join", |ed, ctx| {
        let n = ctx.count.max(2) - 1;
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        // Whole lines, not `line_range_of_sel`: a range that reaches the
        // last line takes the newline *before* its first line (`dd`'s
        // rule), and a join that began on that newline began a line up.
        let extra = if ed.mode(ctx.view) == Mode::Visual {
            0
        } else {
            n
        };
        let ranges: Vec<(Range<usize>, bool)> = ed.views[ctx.view]
            .sels
            .iter()
            .map(|s| (whole_lines_of_sel(buf, s, extra), true))
            .collect();
        if ed.mode(ctx.view) == Mode::Visual {
            ed.set_mode(ctx.view, Mode::Normal);
        }
        apply_operator(ed, ctx.view, "join", ranges);
    });
    ed.register("delete char", |ed, ctx| {
        let ranges = chars_under(ed, ctx);
        if ed.mode(ctx.view) == Mode::Visual {
            ed.set_mode(ctx.view, Mode::Normal);
        }
        // Nothing under any caret — the text's end — keeps the register.
        if ranges.iter().all(|(r, _)| r.is_empty()) {
            return;
        }
        apply_operator(ed, ctx.view, "delete", ranges);
    });
    // `X` stops at the line start; insert's Backspace goes on past it,
    // the line joined to the one above, as vim's `backspace=eol`.
    ed.register("delete char back", |ed, ctx| {
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let joins = ed.mode(ctx.view) == Mode::Insert;
        let ranges: Vec<(Range<usize>, bool)> = ed.views[ctx.view]
            .sels
            .iter()
            .map(|s| {
                let ls = if joins {
                    0
                } else {
                    buf.line_start(buf.line_of(s.head))
                };
                let mut a = s.head;
                for _ in 0..ctx.count.max(1) {
                    if a > ls {
                        a = buf.prev_char(a);
                    }
                }
                (a..s.head, false)
            })
            .collect();
        // `X` is `dh`, into the register; Backspace is not.
        let op = if joins { "erase" } else { "delete" };
        apply_operator(ed, ctx.view, op, ranges);
    });
    ed.register("change char", |ed, ctx| {
        let ranges = chars_under(ed, ctx);
        ed.set_mode(ctx.view, Mode::Normal);
        apply_operator(ed, ctx.view, "change", ranges);
    });
    // `D`, into the register; insert's ⌘⌦ is an erase, as its ⌘⌫ is.
    ed.register("delete to end", |ed, ctx| {
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let ranges = ed.views[ctx.view]
            .sels
            .iter()
            .map(|s| (s.head..buf.line_range(buf.line_of(s.head)).end, false))
            .collect();
        let op = if ed.mode(ctx.view) == Mode::Insert {
            "erase"
        } else {
            "delete"
        };
        apply_operator(ed, ctx.view, op, ranges);
    });
    ed.register("change to end", |ed, ctx| {
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let ranges = ed.views[ctx.view]
            .sels
            .iter()
            .map(|s| (s.head..buf.line_range(buf.line_of(s.head)).end, false))
            .collect();
        apply_operator(ed, ctx.view, "change", ranges);
    });
    // COUNT characters under each caret, vim's `r`: each becomes CHAR,
    // the caret on the last; a caret short of COUNT on its line is left
    // alone. `<Tab>` puts what insert's `<Tab>` would, a tab or the
    // spaces to the next stop, for each. `<CR>` makes the lot one line
    // break, as insert's `<CR>` does (its indent, the blanks after it
    // gone), the caret stepped back as `<Esc>` steps it — but not in
    // a field, which is one line. A `\r\n` line's `\r` is its break's,
    // not a character to replace.
    ed.register_with_char("replace char", |ed, ctx| {
        let Some(c) = ctx.arg_char else { return };
        if c == '\n' && ed.is_field(ctx.view) {
            return;
        }
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let mut edits = Vec::new();
        let mut last = 0;
        for (i, s) in ed.views[ctx.view].sels.iter().enumerate() {
            let le = buf.line_range(buf.line_of(s.head)).end;
            let mut e = s.head;
            let mut n = 0;
            while n < ctx.count && e < le {
                e = buf.next_char(e);
                n += 1;
            }
            if n == ctx.count && s.head >= last {
                last = e;
                edits.push((i, s.head..e));
            }
        }
        if c != '\n' {
            // A tab is `<Tab>`'s, each from the column the last left —
            // the carets' before it on its line too.
            let ts = ed.tabstop_in(id);
            let mut cols = m::EditCols::default();
            let edits = edits
                .into_iter()
                .map(|(i, r)| {
                    let text = if c == '\t' {
                        let start = cols.at(buf, r.start, ts);
                        let mut col = start;
                        let text: String = (0..ctx.count)
                            .map(|_| {
                                let t = ed.tab_from(id, col);
                                col = t.chars().fold(col, |col, c| m::advance(col, c, ts));
                                t
                            })
                            .collect();
                        cols.put(buf, r.clone(), start, &text, ts);
                        text
                    } else {
                        c.to_string().repeat(ctx.count)
                    };
                    (i, r, text)
                })
                .collect();
            let back = c.len_utf8();
            return ed.edit_each(ctx.view, edits, move |start, len| {
                Selection::point(start + len - back)
            });
        }
        // The breaks go where the carets are once the characters are
        // gone, and nowhere else.
        let mut gone = 0;
        let at: Vec<usize> = edits
            .iter()
            .map(|(_, r)| {
                gone += r.len();
                r.end - gone
            })
            .collect();
        if at.is_empty() {
            return;
        }
        let edits = edits
            .into_iter()
            .map(|(i, r)| (i, r, String::new()))
            .collect();
        ed.edit_each(ctx.view, edits, |start, _| Selection::point(start));
        break_lines(ed, ctx.view, Some(&at));
    });

    // ---- insert
    ed.register("insert", |ed, ctx| ed.set_mode(ctx.view, Mode::Insert));
    ed.register("append", |ed, ctx| {
        motion(ed, ctx, |b, o, _| {
            let le = b.line_range(b.line_of(o)).end;
            if o < le { b.next_char(o) } else { o }
        });
        ed.set_mode(ctx.view, Mode::Insert);
    });
    ed.register("insert line start", |ed, ctx| {
        motion(ed, ctx, |b, o, _| m::first_nonblank(b, b.line_of(o)));
        ed.set_mode(ctx.view, Mode::Insert);
    });
    ed.register("append line end", |ed, ctx| {
        motion(ed, ctx, |b, o, _| b.line_range(b.line_of(o)).end);
        ed.set_mode(ctx.view, Mode::Insert);
    });
    ed.register("open below", |ed, ctx| open_line(ed, ctx, true));
    ed.register("open above", |ed, ctx| open_line(ed, ctx, false));
    ed.register("normal", |ed, ctx| {
        let was_insert = ed.mode(ctx.view) == Mode::Insert;
        let was_normal = ed.mode(ctx.view) == Mode::Normal;
        ed.set_mode(ctx.view, Mode::Normal);
        // `<Esc>` in normal mode is a ladder, the top rung with
        // something to do (docs/design/roadmap.md, decision 3): a
        // pending operator, the extra cursors, the search highlight,
        // nothing. A prompt has its own `<Esc>` (`prompt cancel`).
        if was_normal {
            if ed.pending_op.take().is_some() {
                return;
            }
            if ed.views[ctx.view].sels.len() > 1 {
                ed.views[ctx.view].sels.keep_primary();
                return;
            }
            if ed.search.is_some() && ed.search_hl {
                ed.search_hl = false;
            }
            return;
        }
        ed.pending_op = None;
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let v = &mut ed.views[ctx.view];
        if was_insert {
            // Leaving insert steps back one, like vim, but not past the
            // line start.
            v.sels.map(|s| {
                let ls = buf.line_start(buf.line_of(s.head));
                let h = if s.head > ls {
                    buf.prev_char(s.head)
                } else {
                    s.head
                };
                Selection::point(h)
            });
        } else {
            v.sels.map(Selection::collapse);
        }
    });
    // A newline at every caret (`break_lines`).
    ed.register("insert newline", |ed, ctx| {
        if ed.is_field(ctx.view) {
            return ed.insert_text(ctx.view, "\n");
        }
        break_lines(ed, ctx.view, None);
    });
    ed.register("insert tab", |ed, ctx| ed.insert_text(ctx.view, "\t"));
    ed.register("delete to start", |ed, ctx| {
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let edits = ed.views[ctx.view]
            .sels
            .iter()
            .enumerate()
            .map(|(i, s)| {
                (
                    i,
                    buf.line_start(buf.line_of(s.head))..s.head,
                    String::new(),
                )
            })
            .collect();
        ed.edit_each(ctx.view, edits, |start, _| Selection::point(start));
    });
    // The whole line under each caret, from insert mode: `dd` without
    // leaving it, the text into the register as `dd` puts it.
    ed.register("delete line", |ed, ctx| {
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let ranges: Vec<(Range<usize>, bool)> = ed.views[ctx.view]
            .sels
            .iter()
            .map(|s| (line_range_of_sel(buf, s, 0), true))
            .collect();
        apply_operator(ed, ctx.view, "delete", ranges);
    });
    ed.register("delete word back", |ed, ctx| {
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let ranges = ed.views[ctx.view]
            .sels
            .iter()
            .map(|s| {
                (
                    m::prev_word_start(buf, s.head).max(buf.line_start(buf.line_of(s.head)))
                        ..s.head,
                    false,
                )
            })
            .collect();
        apply_operator(ed, ctx.view, "erase", ranges);
    });
    // `<C-w>`'s mirror, insert's ⌥⌦: to the end of the word after the
    // caret, never past its line.
    ed.register("delete word forward", |ed, ctx| {
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let ranges = ed.views[ctx.view]
            .sels
            .iter()
            .map(|s| {
                let le = buf.line_range(buf.line_of(s.head)).end;
                (s.head..m::word_end_after(buf, s.head).min(le), false)
            })
            .collect();
        apply_operator(ed, ctx.view, "erase", ranges);
    });
    ed.register("delete forward", |ed, ctx| {
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let ranges = ed.views[ctx.view]
            .sels
            .iter()
            .map(|s| (s.head..buf.next_char(s.head), false))
            .collect();
        apply_operator(ed, ctx.view, "erase", ranges);
    });
    ed.register("paste after", |ed, ctx| paste(ed, ctx, true));
    ed.register("paste over", |ed, ctx| paste_over(ed, ctx, false));
    ed.register("paste over keep", |ed, ctx| paste_over(ed, ctx, true));
    ed.register("paste before", |ed, ctx| paste(ed, ctx, false));
    ed.register("put older", |ed, ctx| put_step(ed, ctx, true));
    ed.register("put newer", |ed, ctx| put_step(ed, ctx, false));
    ed.register("paste clipboard", |ed, _| {
        ed.effects.push(Effect::RequestPaste)
    });
    ed.register("undo", |ed, ctx| {
        for _ in 0..ctx.count.max(1) {
            if !ed.undo(ctx.view) {
                ed.message = "already at oldest change".into();
                break;
            }
        }
    });
    ed.register("redo", |ed, ctx| {
        for _ in 0..ctx.count.max(1) {
            if !ed.redo(ctx.view) {
                ed.message = "already at newest change".into();
                break;
            }
        }
    });
    // In time over the whole tree, where `u` follows one branch.
    ed.register("undo older", |ed, ctx| {
        for _ in 0..ctx.count.max(1) {
            if !ed.undo_by_time(ctx.view, true) {
                ed.message = "already at oldest change".into();
                break;
            }
        }
    });
    ed.register("undo newer", |ed, ctx| {
        for _ in 0..ctx.count.max(1) {
            if !ed.undo_by_time(ctx.view, false) {
                ed.message = "already at newest change".into();
                break;
            }
        }
    });

    // ---- visual and selections
    ed.register("visual", |ed, ctx| {
        if ed.mode(ctx.view) == Mode::Visual && !ed.views[ctx.view].visual_linewise {
            ed.set_mode(ctx.view, Mode::Normal);
        } else {
            ed.set_mode(ctx.view, Mode::Visual);
        }
    });
    ed.register("visual line", |ed, ctx| {
        if ed.mode(ctx.view) == Mode::Visual && ed.views[ctx.view].visual_linewise {
            ed.set_mode(ctx.view, Mode::Normal);
        } else {
            ed.set_mode(ctx.view, Mode::Visual);
            ed.views[ctx.view].visual_linewise = true;
        }
    });
    ed.register("cursor swap", |ed, ctx| {
        ed.views[ctx.view]
            .sels
            .map(|s| Selection::new(s.head, s.anchor));
    });
    ed.register("cursor primary", |ed, ctx| {
        ed.views[ctx.view].sels.keep_primary()
    });
    ed.register("cursor rotate", |ed, ctx| {
        ed.views[ctx.view].sels.rotate(ctx.count.max(1) as i64)
    });
    ed.register("cursor rotate back", |ed, ctx| {
        ed.views[ctx.view].sels.rotate(-(ctx.count.max(1) as i64))
    });
    ed.register("increment", |ed, ctx| number_step(ed, ctx, 1));
    ed.register("decrement", |ed, ctx| number_step(ed, ctx, -1));
    ed.register("move line down", |ed, ctx| move_lines(ed, ctx, true));
    ed.register("move line up", |ed, ctx| move_lines(ed, ctx, false));
    ed.register("nudge left", |ed, ctx| nudge(ed, ctx, false));
    ed.register("nudge right", |ed, ctx| nudge(ed, ctx, true));
    ed.register("select all", |ed, ctx| {
        let len = ed.buffer_of(ctx.view).len();
        ed.views[ctx.view].sels = crate::Selections::single(Selection::new(0, len));
        ed.set_mode(ctx.view, Mode::Visual);
    });
    ed.register("cursor below", |ed, ctx| add_cursor(ed, ctx, 1));
    ed.register("cursor above", |ed, ctx| add_cursor(ed, ctx, -1));
    ed.register("cursor lines", |ed, ctx| carets_per_line(ed, ctx, true));
    ed.register("cursor lines back", |ed, ctx| {
        carets_per_line(ed, ctx, false)
    });
    ed.register("select next", select_next);
    ed.register("select all matches", select_all_matches);
    for (name, how) in [
        ("select within", crate::Select::Within),
        ("select split", crate::Select::Split),
        ("select keep", crate::Select::Keep),
    ] {
        ed.register_with_args(name, Args::rest(&[ArgKind::Text]), move |ed, ctx| {
            select_command(ed, ctx, how)
        });
    }
    ed.register("select lines", select_lines);
    ed.register("select drop primary", select_drop_primary);
    // `S`: `cc` in one key.
    ed.register("change line", |ed, ctx| {
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let n = ctx.count.max(1);
        let ranges: Vec<(Range<usize>, bool)> = ed.views[ctx.view]
            .sels
            .iter()
            .map(|s| (line_range_of_sel(buf, s, n - 1), true))
            .collect();
        apply_operator(ed, ctx.view, "change", ranges);
    });
    // Surrounds (`gs`, as mini.surround's): an operator that then takes
    // the pair's character, a delete and a replace by character.
    ed.register_kind("surround add", Kind::Operator, |ed, ctx| {
        operator(ed, ctx, "surround add")
    });
    ed.register_with_key("surround wrap", surround_wrap);
    // Align (`ga`, easy-align's): an operator over what a motion or the
    // selections cover that then takes the character to line it up on
    // — `*` or a count before it for every occurrence or the Nth — or
    // `<CR>` and a pattern. With a pattern (`:align PAT`, `:align *
    // PAT`) the selections' regions, at once.
    ed.register_spec(
        Spec::new("align")
            .kind(Kind::Operator)
            .args(Args::rest(&[ArgKind::Text])),
        |ed, ctx| {
            if ctx.args.is_empty() {
                operator(ed, ctx, "align")
            } else {
                align_on(ed, ctx)
            }
        },
    );
    ed.register_spec(
        Spec::new("align on")
            .takes_char()
            .args(Args::rest(&[ArgKind::Text])),
        align_on,
    );
    // The prompt is the character's answer: a replay, which runs this
    // step rather than pressing `<CR>`, is left waiting on no key. The
    // `*` or count typed before `<CR>` comes as the argument, kept for
    // the answer.
    ed.register_spec(
        Spec::new("align ask").args(Args::rest(&[ArgKind::Text])),
        |ed, ctx| {
            ed.awaiting_char = None;
            if let Some(a) = &mut ed.surround.align {
                a.which = ctx.args.join(" ");
            }
            ed.open_prompt(ctx.view, Prompt::Align);
        },
    );
    ed.register_with_key("surround delete", surround_delete);
    ed.register_with_key("surround replace", surround_replace);
    ed.register_with_key("surround replace with", surround_replace_with);

    // `"` names the register the next command takes into or puts from.
    // Only `_` so far, the black hole; `""` is the one there always is,
    // named all the same, so a count before it is the command's.
    ed.register_with_key("register", |ed, ctx| match ctx.arg_char {
        Some(c @ ('_' | '"')) => ed.pending_register = Some(c),
        Some(c) => {
            ed.count = None;
            ed.register_count = None;
            ed.message = format!("no register {c}: only _, the black hole");
        }
        None => {}
    });

    // ---- the stream: `.` and macros (docs/design/keys.md; `repeat`)
    ed.register("repeat", |ed, ctx| {
        ed.repeat_change(ctx.view, ctx.has_count.then_some(ctx.count));
    });
    // `q` ends a recording at once, so it asks for its register only
    // when none is on — `takes_char` would wait for one either way.
    ed.register_spec(
        Spec::new("macro record").char_arg(CharArg::Key),
        |ed, ctx| {
            if let Some(c) = ed.repeat.stop() {
                ed.message = format!("recorded @{c}");
                return;
            }
            match ctx.arg_char {
                None => ed.await_char("macro record"),
                Some(c) if c.is_ascii_alphanumeric() => {
                    ed.repeat.start(c);
                    ed.message.clear();
                }
                Some(c) => ed.message = format!("no register {c} to record into"),
            }
        },
    );
    ed.register_with_key("macro play", |ed, ctx| {
        let Some(c) = ctx.arg_char else {
            return;
        };
        if c == ':' {
            let Some(line) = ed.cmd_history.last().cloned() else {
                ed.message = "no command line yet".into();
                return;
            };
            for _ in 0..ctx.count.max(1) {
                ed.execute(ctx.view, &line);
            }
            return;
        }
        let c = match c {
            '@' => match ed.repeat.last_played {
                Some(c) => c,
                None => {
                    ed.message = "no macro played yet".into();
                    return;
                }
            },
            c => c.to_ascii_lowercase(),
        };
        let Some(steps) = ed.repeat.macros.get(&c).cloned() else {
            ed.message = format!("no macro @{c}");
            return;
        };
        ed.repeat.last_played = Some(c);
        for _ in 0..ctx.count.max(1) {
            ed.replay(ctx.view, &steps);
        }
    });

    // ---- prompts and ex commands
    ed.register("command", |ed, ctx| {
        ed.open_prompt(ctx.view, Prompt::Command);
    });
    // The prompt's own keys, each a command gated on `prompt`, so the
    // field's other keys are the editor's — motions, operators, undo.
    ed.register_spec(
        Spec::new("prompt submit")
            .when(&["prompt"])
            .doc("run the prompt's line: an ex line, or a search"),
        |ed, _| ed.submit_prompt(),
    );
    ed.register_spec(
        Spec::new("prompt cancel")
            .when(&["prompt"])
            .doc("leave the prompt with nothing done"),
        |ed, _| ed.cancel_prompt(),
    );
    ed.register_spec(
        Spec::new("prompt backspace")
            .when(&["prompt"])
            .doc("delete the character before the caret, or leave an empty prompt"),
        |ed, ctx| ed.prompt_backspace(ctx.view),
    );
    ed.register_spec(
        Spec::new("prompt history prev")
            .when(&["prompt"])
            .doc("the older line starting with what was typed"),
        |ed, _| ed.walk_history(true),
    );
    ed.register_spec(
        Spec::new("prompt history next")
            .when(&["prompt"])
            .doc("the newer line starting with what was typed, then what was typed"),
        |ed, _| ed.walk_history(false),
    );
    ed.register("search", |ed, ctx| ed.open_search(ctx.view, false));
    ed.register("search back", |ed, ctx| ed.open_search(ctx.view, true));
    ed.register_spec(
        Spec::new("write")
            .alias(&["w"])
            .args(Args::new(&[ArgKind::Path]))
            .bang("write over a file changed on disk since it was read")
            .doc("write the buffer to its file, or to PATH"),
        |ed, ctx| {
            // A save ends the typing: `<C-s>` from insert or visual
            // mode leaves the view in normal mode, as `<Esc>` would,
            // whether or not the write went through.
            if ed.mode(ctx.view) != Mode::Normal {
                ed.run(ctx.view, "normal", &[], None);
            }
            write(ed, ctx, crate::AfterWrite::Nothing);
        },
    );
    // Whether unsaved changes let `:q` through is the shell's call
    // (`Effect::Quit`): with a store they are kept for the next launch,
    // without one it refuses as vim does.
    ed.register_spec(
        Spec::new("quit")
            .alias(&["q"])
            .bang("discard unsaved changes")
            .doc("close the pane, or the app from the last one"),
        |ed, ctx| {
            ed.effects.push(Effect::Quit { force: ctx.bang() });
        },
    );
    ed.register_spec(
        Spec::new("quit all")
            .alias(&["qa", "qall", "quitall"])
            .bang("discard unsaved changes")
            .doc("close the app, whatever is open"),
        |ed, ctx| {
            ed.effects.push(Effect::QuitAll { force: ctx.bang() });
        },
    );
    ed.register_spec(
        Spec::new("write quit")
            .alias(&["wq", "x"])
            .args(Args::new(&[ArgKind::Path]))
            .bang("write over a file changed on disk since it was read")
            .doc("write, then quit"),
        |ed, ctx| {
            if write(ed, ctx, crate::AfterWrite::Quit) {
                ed.effects.push(Effect::Quit { force: false });
            }
        },
    );
    ed.register_spec(
        Spec::new("write all")
            .alias(&["wa", "wall"])
            .doc("write every modified file; one changed on disk since it was read is named, not written"),
        |ed, _| {
            let w = ed.write_all();
            ed.message = w.message();
            if !w.deferred.is_empty() {
                ed.effects.push(Effect::FormatThenWrite {
                    buffers: w.deferred,
                    after: crate::AfterWrite::Nothing,
                });
            }
        },
    );
    // Quits only when everything was written: a file changed on disk,
    // or a write that failed, stays open with the message saying which.
    ed.register_spec(
        Spec::new("write quit all")
            .alias(&["wqa", "xa"])
            .doc("write every file, then quit"),
        |ed, _| {
            let w = ed.write_all();
            if !w.complete() {
                ed.message = w.message();
            } else if w.deferred.is_empty() {
                ed.effects.push(Effect::QuitAll { force: false });
            } else {
                // The quit once the formats have been written.
                ed.message = w.message();
                ed.effects.push(Effect::FormatThenWrite {
                    buffers: w.deferred,
                    after: crate::AfterWrite::QuitAll,
                });
            }
        },
    );
    // `<leader>y*`: the file's path onto the clipboard and into the
    // register (`Editor::copy_text`), in the form the word after `path
    // copy` names — a private buffer's too, since a path is not the
    // secret. Bare, `path copy` is `path copy relative`. A `dir`
    // listing binds the same keys to its entry's.
    let copy = |ed: &mut Editor, view: ViewId, form: &str| match ed.path_form(view, form) {
        Ok(p) => {
            let from = ed.buffers[ed.views[view].buffer].name.clone();
            ed.message = format!("copied {p}");
            ed.copy_text(p, &from);
        }
        Err(why) => ed.message = why,
    };
    ed.register_spec(
        Spec::new("path copy").doc("copy the file's path from the working directory"),
        move |ed, ctx| copy(ed, ctx.view, "relative"),
    );
    for (form, doc) in crate::PATH_FORMS {
        ed.register_spec(
            Spec::new(&format!("path copy {form}")).doc(doc),
            move |ed, ctx| copy(ed, ctx.view, form),
        );
    }
    // `:e path` opens; `:e!` alone loads the disk's text into the
    // buffer as one undoable change, so what was unsaved is a `u` away
    // — a draft restored over a file that moved on disk, looked at
    // both ways. `:e! path` is `:e path` (the changes are kept either
    // way).
    ed.register_spec(
        Spec::new("edit")
            .alias(&["e"])
            .args(Args::new(&[ArgKind::Path]))
            .bang("reload the disk's text, as one undoable change")
            .doc("open PATH"),
        |ed, ctx| match ctx.args.first() {
            Some(p) => ed.effects.push(Effect::Open(p.into())),
            None if ctx.bang() => reload(ed, ctx),
            None => ed.message = "edit what?".into(),
        },
    );
    // `:set PATH=VALUE` or `:set PATH VALUE`, `:set FLAG` or `:set
    // +FLAG`, `:set -FLAG` set into the session layer, shaped like the
    // value already there; `:set PATH?` says what it is and where it
    // came from; `:set PATH!` takes the session's value back out. Not
    // vim's `noFLAG`: a path may start with `no` (`notes.enabled`), none
    // starts with a sign.
    ed.register_spec(
        Spec::new("set")
            .alias(&["se"])
            .args(Args::rest(&[ArgKind::Option]))
            .doc("set an option for the session (PATH=VALUE, PATH VALUE, +FLAG, -FLAG, PATH?, PATH!)"),
        |ed, ctx| {
            // One setting per line: what follows the path is the value,
            // spaces and all (`:set compile.default=cargo test`).
            let a = ctx.args.join(" ");
            if a.is_empty() {
                ed.message = "set what? (:set PATH=VALUE, :set PATH?)".into();
                return;
            }
            let a = a.as_str();
            let (path, value) = match set_value(a) {
                Some((k, v)) => (k.to_string(), Setting::parse_like(v, ed.settings.get(k))),
                None => {
                    if let Some(path) = a.strip_suffix('?') {
                        // A value as the buffer the keys are in reads it —
                        // its language's, its `.editorconfig`'s; a table
                        // as the tree has it, every layer merged.
                        let id = ed.views[ctx.view].buffer;
                        let scoped = ed
                            .settings
                            .get(path)
                            .is_none_or(|v| !v.is_table())
                            .then(|| ed.settings.scoped_origin(path, ed.scope_of(id)))
                            .flatten();
                        ed.message = match (scoped, ed.settings.get(path)) {
                            (Some((v, from)), _) => format!("{path} = {v}  ({from})"),
                            (None, Some(v)) => match ed.settings.origin(path) {
                                Some(from) => format!("{path} = {v}  ({from})"),
                                None => format!("{path} = {v}"),
                            },
                            (None, None) => format!("{path} is not set"),
                        };
                        return;
                    }
                    if let Some(path) = a.strip_suffix('!') {
                        ed.settings.unset(Layer::Session, path);
                        return;
                    }
                    match a.strip_prefix('-') {
                        Some(flag) => (flag.to_string(), Setting::Bool(false)),
                        None => (
                            a.strip_prefix('+').unwrap_or(a).to_string(),
                            Setting::Bool(true),
                        ),
                    }
                }
            };
            if path.is_empty() {
                ed.message = "set: no path (:set PATH=VALUE)".into();
                return;
            }
            ed.settings.set(Layer::Session, &path, value);
        },
    );
    ed.register_spec(
        Spec::new("echo")
            .args(Args::rest(&[ArgKind::Text]))
            .doc("put TEXT in the message line"),
        |ed, ctx| ed.message = ctx.args.join(" "),
    );
    // `:map MODE KEYS COMMAND ARGS...`: a binding for the session, as a
    // plugin's `kawoosh.map` makes one; `:map <buffer> MODE KEYS
    // COMMAND` one local to the buffer the keys are in, vim's spelling
    // (docs/design/local-maps.md).
    ed.register_spec(
        Spec::new("map")
            .args(Args::rest(&[
                ArgKind::Text,
                ArgKind::Text,
                ArgKind::Command,
            ]))
            .doc("bind KEYS in MODE to COMMAND; `<buffer>` first, in this buffer only"),
        |ed, ctx| {
            let (scope, args) = match ctx.args.split_first() {
                Some((first, rest)) if first.eq_ignore_ascii_case("<buffer>") => {
                    let id = ed.views[ctx.view].buffer;
                    (Some(crate::buffer_scope(id)), rest)
                }
                _ => (None, ctx.args.as_slice()),
            };
            match args {
                [mode, keys, cmd @ ..] if !cmd.is_empty() => match (Mode::from_short(mode), &scope)
                {
                    (Some(m), Some(s)) => ed.keymap.bind_local(s, m, keys, &cmd.join(" "), &[]),
                    (Some(m), None) => ed.keymap.bind(m, keys, &cmd.join(" ")),
                    (None, _) => ed.message = format!("map: unknown mode {mode}"),
                },
                _ => {
                    ed.message =
                        "map what? (:map [<buffer>] MODE KEYS COMMAND; :map list to see them)"
                            .into()
                }
            }
        },
    );

    // `:map group KEYS NAME...`: what the keys open, for the which-key.
    ed.register_spec(
        Spec::new("map group")
            .args(Args::rest(&[ArgKind::Text]))
            .doc("name what KEYS open, for the which-key (`:map group <leader>x extras`)"),
        |ed, ctx| match ctx.args.as_slice() {
            [keys, name @ ..] if !name.is_empty() => ed.keymap.describe(keys, &name.join(" ")),
            _ => ed.message = "map group what? (:map group KEYS NAME)".into(),
        },
    );

    for (name, doc) in DOCS {
        match ed.commands.spec_mut(name) {
            Some(spec) => spec.doc = crate::host::for_host(doc, crate::host::Host::HERE),
            None => debug_assert!(false, "DOCS names no command: {name}"),
        }
    }
    for name in JUMPS {
        match ed.commands.spec_mut(name) {
            Some(spec) => spec.jump = true,
            None => debug_assert!(false, "JUMPS names no command: {name}"),
        }
    }
}

/// The engine's moves that are jumps however short (docs/design/jumps.md
/// Decision 2): vim's, where both ends can share one screen. `H` `M`
/// `L` stay on it and `{` `}` step through the text, so they are not;
/// a big one is noticed anyway.
const JUMPS: &[&str] = &[
    "goto file start",
    "goto file end",
    "goto line",
    "search next",
    "search prev",
    "search word",
    "match_bracket",
];

/// What each command of the keymap does, one line, for the `:commands`
/// pane — the ex commands carry theirs on the spec above; these are
/// the ones written as a key. Every command has one (the modal test
/// checks), and a name here that is no command is a debug assertion.
/// "Every selection" is the rule (mvp.md Decision 4), so a line says
/// "the caret" only where the primary alone is meant.
const DOCS: &[(&str, &str)] = &[
    // motions
    ("move left", "a character left, within the line"),
    ("move right", "a character right, within the line"),
    ("move down", "a line down, keeping the column aimed for"),
    ("move up", "a line up, keeping the column aimed for"),
    ("line start", "the line's first column"),
    ("line nonblank", "the line's first non-blank"),
    ("line end", "the line's last character"),
    (
        "line end insert",
        "past the line's last character (insert mode's End)",
    ),
    ("word next", "the start of the next word"),
    ("word prev", "the start of the previous word"),
    ("word end", "the end of the word"),
    ("word end back", "the end of the previous word (`ge`)"),
    (
        "word end insert",
        "past the end of the word, or of the next on whitespace (insert mode's {{mac:⌥→}}{{pc:<C-Right>}})",
    ),
    (
        "bigword next",
        "the start of the next WORD — only whitespace ends one (`W`)",
    ),
    ("bigword prev", "the start of the previous WORD (`B`)"),
    ("bigword end", "the end of the WORD (`E`)"),
    ("bigword end back", "the end of the previous WORD (`gE`)"),
    ("paragraph next", "the blank line after the paragraph (`}`)"),
    (
        "paragraph prev",
        "the blank line before the paragraph (`{`)",
    ),
    (
        "screen top",
        "the pane's top line, or COUNT lines below it (`H`)",
    ),
    ("screen middle", "the pane's middle line (`M`)"),
    (
        "screen bottom",
        "the pane's bottom line, or COUNT lines above it (`L`)",
    ),
    ("goto file start", "the first line, or line COUNT"),
    ("goto file end", "the last line, or line COUNT"),
    ("goto line", "line COUNT (`:42` too)"),
    ("page half down", "half a screen down"),
    ("page half up", "half a screen up"),
    ("page down", "a screen down"),
    ("page up", "a screen up"),
    (
        "match_bracket",
        "the bracket matching the one under the caret; on a block's keyword (`if`, `else`, `end`) the block's next",
    ),
    ("find char", "onto the next CHAR in the line"),
    ("find char back", "onto the previous CHAR in the line"),
    ("till char", "before the next CHAR in the line"),
    ("till char back", "after the previous CHAR in the line"),
    ("search next", "the next match of the search, wrapping"),
    ("search prev", "the previous match of the search, wrapping"),
    (
        "search word",
        "search for the word under the caret, forward; in visual mode, for the selection's text",
    ),
    ("search", "open the search prompt, forward"),
    ("search back", "open the search prompt, backward"),
    (
        "substitute",
        "`[range]s/PAT/REP/[g]`: replace in the range, or the line",
    ),
    // text objects
    (
        "textobject inner",
        "select inside a pair, word, paragraph or syntax object: iw, i(, i\", ip, if, ic, ia, ...",
    ),
    (
        "textobject around",
        "select a pair, word, paragraph or syntax object with what surrounds it: aw, a(, ap, af, ...",
    ),
    (
        "function next",
        "the start of the next function, by the syntax, COUNT on (`]f`)",
    ),
    (
        "function prev",
        "the start of the previous function, by the syntax, COUNT back (`[f`)",
    ),
    // operators
    (
        "delete",
        "delete the selection, or wait for a motion; the text goes to the register",
    ),
    ("change", "delete as `delete` does, then insert"),
    (
        "yank",
        "copy the selection, or what a motion covers, to the register",
    ),
    (
        "indent",
        "indent the lines a tabstop (spaces under `expandtab`)",
    ),
    ("dedent", "dedent the lines a tabstop"),
    (
        "reindent",
        "indent the lines as the syntax says (`=`; docs/design/indent.md)",
    ),
    (
        "case lower",
        "lower-case the selection, or what a motion covers (`gu`; `u` on a selection)",
    ),
    (
        "case upper",
        "upper-case the selection, or what a motion covers (`gU`; `U` on a selection)",
    ),
    (
        "case toggle",
        "turn the case of the selection, or what a motion covers (`g~`; `~` on a selection)",
    ),
    (
        "case toggle char",
        "turn the case of COUNT characters from the caret and step past them (`~`)",
    ),
    (
        "line blank above",
        "COUNT empty lines above the caret's line, the caret staying (`[<Space>`)",
    ),
    (
        "line blank below",
        "COUNT empty lines below the caret's line, the caret staying (`]<Space>`)",
    ),
    (
        "join",
        "join COUNT lines (the selection's, in visual) with a space between",
    ),
    (
        "comment",
        "comment the lines a motion or object covers, or the selection's, or uncomment them when every one is (`gc`; docs/design/comments.md)",
    ),
    (
        "comment lines",
        "comment or uncomment COUNT lines at the caret, or the selection's (`gcc`; `:comment lines`)",
    ),
    (
        "comment block",
        "wrap what a motion or object covers, or the selection, in the block comment pair as one, or unwrap it when it is one (`gb`)",
    ),
    (
        "comment block lines",
        "wrap COUNT lines at the caret, or the selection's, in the block pair as one, or unwrap them (`gbc`; `:comment block lines`)",
    ),
    (
        "move line down",
        "move every selection's lines one line down, the selection kept (`<A-j>`)",
    ),
    (
        "move line up",
        "move every selection's lines one line up, the selection kept (`<A-k>`)",
    ),
    (
        "nudge left",
        "dedent the selection's lines, keeping it; drag a `v` selection one column left (`<A-h>`)",
    ),
    (
        "nudge right",
        "indent the selection's lines, keeping it; drag a `v` selection one column right (`<A-l>`)",
    ),
    (
        "increment",
        "add COUNT to the number under or after the caret, per selection (`<C-a>`)",
    ),
    (
        "decrement",
        "subtract COUNT from the number under or after the caret, per selection (`<C-x>`)",
    ),
    ("cursor rotate", "make the next selection the primary (`)`)"),
    (
        "cursor rotate back",
        "make the previous selection the primary (`(`)",
    ),
    (
        "delete char",
        "delete the character under the caret (`x`); on a line's newline, the newline, the next line joined on",
    ),
    (
        "delete char back",
        "delete the character before the caret (`X`; insert's Backspace, which joins the line above at a line's start and, as vim's, leaves the register alone)",
    ),
    (
        "delete to end",
        "delete to the end of the line (`D`{{mac:; insert's ⌘⌦, which leaves the register alone}})",
    ),
    (
        "delete forward",
        "delete the character after the caret, the register left alone (insert's Delete)",
    ),
    (
        "delete word back",
        "delete the word before the caret, the register left alone (insert's <C-w>, {{mac:⌥⌫}}{{pc:<C-BS>}})",
    ),
    (
        "delete word forward",
        "delete to the end of the word after the caret, the register left alone (insert's {{mac:⌥⌦}}{{pc:<C-Del>}})",
    ),
    (
        "delete line",
        "the whole line under each caret, into the register, staying in insert mode (`<C-S-u>`)",
    ),
    (
        "delete to start",
        "delete to the start of the line (insert's <C-u>{{mac:, ⌘⌫}})",
    ),
    (
        "change char",
        "replace the character under the caret with typing (`s`)",
    ),
    (
        "change to end",
        "delete to the end of the line, then insert (`C`)",
    ),
    (
        "replace char",
        "replace COUNT characters under the caret with CHAR (`r`); `<Tab>` as insert's, `<CR>` breaks the line there",
    ),
    // insert
    ("insert", "insert before the caret"),
    (
        "insert line start",
        "insert at the line's first non-blank (`I`)",
    ),
    (
        "insert newline",
        "break the line at the caret, keeping the indent",
    ),
    (
        "insert tab",
        "insert a tab (under `expandtab` the spaces to the next `shiftwidth` stop)",
    ),
    ("append", "insert after the caret (`a`)"),
    ("append line end", "insert at the end of the line (`A`)"),
    ("open below", "a new line below, and insert (`o`)"),
    ("open above", "a new line above, and insert (`O`)"),
    ("normal", "back to normal mode"),
    (
        "paste after",
        "put the register after the caret, or below a linewise one",
    ),
    (
        "paste before",
        "put the register before the caret, or above a linewise one",
    ),
    ("paste clipboard", "put the system clipboard at the caret"),
    (
        "paste over",
        "replace the selection with the register, COUNT times; what it replaced is the register's next (`p` on a selection)",
    ),
    (
        "paste over keep",
        "replace the selection with the register, which stays as it was (`P` on a selection)",
    ),
    (
        "put older",
        "the last put replaced with the text before it in the memory (`[p`), COUNT back; `p` puts it next",
    ),
    (
        "put newer",
        "the last put replaced with the text after it in the memory (`]p`), COUNT on",
    ),
    // undo
    ("undo", "back to the state before"),
    ("redo", "forward again, along the branch last taken"),
    (
        "undo older",
        "the state made before this one, on any branch (`g-`)",
    ),
    // the stream
    (
        "repeat",
        "the last change again, on the selections as they are (`.`); a count replaces its count",
    ),
    (
        "register",
        "the next command's register, named by the next key (`\"`): `_`, the black hole, takes nothing into the register or the clipboard (`\"_d`, `\"_c`, `\"_x`)",
    ),
    (
        "macro record",
        "record the commands into the register named by the next key (`q`), until `q` again; an upper-case letter appends",
    ),
    (
        "macro play",
        "replay the register named by the next key (`@a`), COUNT times; `@@` the one played last, `@:` the last command line",
    ),
    (
        "undo newer",
        "the state made after this one, on any branch (`g+`)",
    ),
    // visual and selections
    ("visual", "extend selections with motions"),
    ("visual line", "extend selections by whole lines"),
    ("cursor swap", "swap each selection's ends (`o`)"),
    (
        "cursor primary",
        "keep the primary selection, drop the rest",
    ),
    ("cursor below", "add a caret on the line below (<C-j>)"),
    ("cursor above", "add a caret on the line above (<C-k>)"),
    (
        "cursor lines",
        "a caret on each line of the selection, at its head's column; the last primary (<C-j> in visual mode)",
    ),
    (
        "cursor lines back",
        "a caret on each line of the selection, at its head's column; the first primary (<C-k> in visual mode)",
    ),
    (
        "select next",
        "select the next match of the selection, or the word under the caret (<C-n>{{mac:, <D-d>}})",
    ),
    (
        "select all matches",
        "select every match of the selection, or of the word under the caret (<C-S-n>{{mac:, <D-L>}})",
    ),
    (
        "select within",
        "the matches of PATTERN inside every selection become the selections; bare, a prompt previewed as it is typed (<leader>vs, helix's s)",
    ),
    (
        "select split",
        "every selection split on PATTERN, the pieces between its matches; bare, a prompt (<leader>vS, helix's S)",
    ),
    (
        "select keep",
        "keep the selections that match PATTERN, or with !PATTERN those that do not; bare, a prompt (<leader>vk, helix's K and <A-K>)",
    ),
    (
        "select lines",
        "every line of every selection its own selection (<leader>vl, helix's <A-s>)",
    ),
    (
        "select drop primary",
        "the primary selection gone, the one before it primary (<A-,>, helix's)",
    ),
    (
        "change line",
        "change the line, keeping its indent (`S`, `cc`)",
    ),
    (
        "find repeat",
        "the next CHAR of the last `f` or `t`, across lines (`;`)",
    ),
    (
        "find repeat back",
        "the last `f` or `t` the other way, across lines",
    ),
    (
        "surround add",
        "wrap what a motion or object covers, or the selection, in the pair CHAR names (`gsa`)",
    ),
    (
        "surround wrap",
        "the pair's character `surround add` waits for",
    ),
    (
        "align",
        "line up what a motion or object covers, or the selections, on the character CHAR names (`ga`) — on each line only inside what is covered — or on a pattern: `<CR>` for CHAR, or `:align PATTERN` over the selections; `*` before CHAR or PATTERN lines up every occurrence, a count the Nth",
    ),
    (
        "align on",
        "the character, or the pattern, `align` lines the lines up on; `*` or a count before it for every occurrence or the Nth",
    ),
    (
        "align ask",
        "the prompt for the pattern `align` lines up on",
    ),
    (
        "surround delete",
        "take the pair CHAR names off from around the caret (`gsd`)",
    ),
    (
        "surround replace",
        "swap the pair CHAR names around the caret for the pair the next character names (`gsr`)",
    ),
    (
        "surround replace with",
        "the second character `surround replace` waits for",
    ),
    (
        "select all",
        "select the whole buffer ({{mac:<D-a>, }}<C-S-a>)",
    ),
    ("command", "open the command line"),
];

fn goto_line(ed: &mut Editor, view: ViewId, n: usize) {
    let ext = extend(ed, view);
    let id = ed.views[view].buffer;
    let buf = &ed.buffers[id];
    let ln = n.max(1).min(buf.line_count()) - 1;
    let target = m::first_nonblank(buf, ln);
    ed.views[view].sels.map(|s| s.with_head(target, ext));
    ed.views[view].goal_col = None;
}

/// A newline at every caret, the new line at the indent the syntax says
/// (docs/design/indent.md), else its line's, a level deeper after an
/// opening bracket; the blanks after the caret go. Between a bracket
/// and its closer the block opens, the closer on a line of its own below
/// the caret's. `only` is `r<CR>`'s: the carets at those offsets break,
/// the rest stay, and each stepped back as `<Esc>` steps it.
fn break_lines(ed: &mut Editor, view: ViewId, only: Option<&[usize]>) {
    let id = ed.views[view].buffer;
    let unit = ed.indent_unit(id);
    let mut indenter = ed.indenter.take();
    let buf = &ed.buffers[id];
    // Where each opened block's caret sits: the edit's start, and how
    // far before its end.
    let mut back: Vec<(usize, usize)> = Vec::new();
    let mut edits: Vec<_> = ed.views[view]
        .sels
        .iter()
        .enumerate()
        .filter(|(_, s)| only.is_none_or(|at| at.contains(&s.head)))
        .map(|(i, s)| {
            let ln = buf.line_of(s.head);
            let line = buf.line_range(ln);
            let at = s.head.clamp(line.start, line.end);
            let after = buf.slice(at..line.end);
            let rest = after.trim_start();
            let blank = after.len() - rest.len();
            let closer = m::opens_block(&buf.slice(line.start..at));
            let between = closer.is_some_and(|c| rest.starts_with(c));
            let said = indenter
                .as_mut()
                .and_then(|x| x.new_line(id, buf, at, &unit));
            if between {
                // The syntax's answer is the closer's line.
                let outer = said.unwrap_or_else(|| m::indent_of(buf, ln));
                let inner = format!("{outer}{}", unit.text);
                back.push((at, 1 + outer.len()));
                return (i, at..at + blank, format!("\n{inner}\n{outer}"));
            }
            let indent = said.unwrap_or_else(|| {
                let own = m::indent_of(buf, ln);
                if closer.is_some() {
                    own + &unit.text
                } else {
                    own
                }
            });
            (i, at..at + blank, format!("\n{indent}"))
        })
        .collect();
    // Two carets in one run of blanks: each takes the blanks up to the
    // next, not the next's too (the carets are in order).
    for k in 1..edits.len() {
        let next = edits[k].1.start;
        let r = &mut edits[k - 1].1;
        r.end = r.end.min(next).max(r.start);
    }
    ed.indenter = indenter;
    let step = only.is_some();
    ed.edit_each(view, edits, move |start, len| {
        let b = back
            .iter()
            .find(|(at, _)| *at == start)
            .map_or(0, |(_, b)| *b);
        // The caret's line starts past the newline; an indent is
        // blanks, a byte each.
        let head = start + len - b;
        Selection::point(if step && head > start + 1 {
            head - 1
        } else {
            head
        })
    });
}

/// `o` / `O`: a line below or above each caret's, at the indent the
/// syntax says for a line break at the end of the line above it (on the
/// first line, `O` has none); else the line's own, a level deeper below
/// a line opening a block or above one closing it.
fn open_line(ed: &mut Editor, ctx: &Ctx, below: bool) {
    let id = view(ed, ctx).buffer;
    let unit = ed.indent_unit(id);
    let mut indenter = ed.indenter.take();
    let buf = &ed.buffers[id];
    let edits = ed.views[ctx.view]
        .sels
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let ln = buf.line_of(s.head);
            let above = if below { Some(ln) } else { ln.checked_sub(1) };
            let said = above.and_then(|a| {
                let at = buf.line_range(a).end;
                indenter.as_mut()?.new_line(id, buf, at, &unit)
            });
            let indent = said.unwrap_or_else(|| {
                let text = buf.line_text(ln);
                let inside = if below {
                    m::opens_block(&text).is_some()
                } else {
                    m::closes_block(&text)
                };
                let own = m::indent_of(buf, ln);
                if inside { own + &unit.text } else { own }
            });
            if below {
                let end = buf.line_range(ln).end;
                (i, end..end, format!("\n{indent}"))
            } else {
                let start = buf.line_start(ln);
                (i, start..start, format!("{indent}\n"))
            }
        })
        .collect();
    ed.indenter = indenter;
    ed.edit_each(ctx.view, edits, move |start, len| {
        Selection::point(if below { start + len } else { start + len - 1 })
    });
    ed.set_mode(ctx.view, Mode::Insert);
}

/// `<C-a>` / `<C-x>`: the number under or after each caret on its line,
/// stepped by the count — a `-` right before it is its sign — with the
/// caret left on its last digit, as vim leaves it. A selection with no
/// number on its line from the caret on stays.
fn number_step(ed: &mut Editor, ctx: &Ctx, sign: i64) {
    let by = ctx.count.max(1) as i64 * sign;
    let id = view(ed, ctx).buffer;
    let buf = &ed.buffers[id];
    let sels = ed.views[ctx.view].sels.items.clone();
    let mut edits = Vec::new();
    for (i, s) in sels.iter().enumerate() {
        let ln = buf.line_of(s.head);
        let range = buf.line_range(ln);
        let text = buf.line_text(ln);
        let rel = s.head.saturating_sub(range.start).min(text.len());
        let bytes = text.as_bytes();
        // The first digit at or after the caret, then back to the run's
        // start (the caret may be mid-number).
        let Some(mut start) = (rel..bytes.len()).find(|&j| bytes[j].is_ascii_digit()) else {
            continue;
        };
        while start > 0 && bytes[start - 1].is_ascii_digit() {
            start -= 1;
        }
        let mut end = start;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        let negative = start > 0 && bytes[start - 1] == b'-';
        let digits = &text[start..end];
        let value: i64 = digits.parse::<i64>().unwrap_or(i64::MAX);
        let value = if negative { -value } else { value };
        let stepped = value.saturating_add(by);
        let from = if negative { start - 1 } else { start };
        // Leading zeros are kept to the width they had (`007` → `008`).
        let width = digits.len();
        let mag = stepped.unsigned_abs().to_string();
        let mut out = String::new();
        if stepped < 0 {
            out.push('-');
        }
        if digits.starts_with('0') && mag.len() < width {
            out.push_str(&"0".repeat(width - mag.len()));
        }
        out.push_str(&mag);
        edits.push((i, range.start + from..range.start + end, out));
    }
    if edits.is_empty() {
        ed.message = "no number here".into();
        return;
    }
    ed.edit_each(ctx.view, edits, |start, len| {
        Selection::point(start + len.saturating_sub(1))
    });
}

/// Where `pos` lands after `edits` (ascending, non-overlapping) are
/// applied, riding right with an insertion at its own byte — a caret
/// at the line start stays on its character when the line is indented
/// — and collapsing to a deletion's start when it was inside one.
pub(crate) fn carried(pos: usize, edits: &[(Range<usize>, usize)]) -> usize {
    let mut delta: isize = 0;
    for (r, new_len) in edits {
        if r.start > pos {
            break;
        }
        if pos >= r.end {
            delta += *new_len as isize - (r.end - r.start) as isize;
        } else {
            return (r.start as isize + delta) as usize;
        }
    }
    (pos as isize + delta) as usize
}

/// Applies `edits` (one per line touched, ascending, keyed by line) and
/// carries every selection through them as [`carried`] says, keeping
/// the mode: what `<A-h>`/`<A-l>` and the line moves want, where an
/// operator would collapse the selection.
fn edit_keeping(ed: &mut Editor, view: ViewId, edits: Vec<(Range<usize>, String)>) {
    if edits.is_empty() {
        return;
    }
    let mut edits = edits;
    edits.sort_by_key(|(r, _)| r.start);
    let shape: Vec<(Range<usize>, usize)> =
        edits.iter().map(|(r, t)| (r.clone(), t.len())).collect();
    let items: Vec<Selection> = ed.views[view]
        .sels
        .iter()
        .map(|s| Selection::new(carried(s.anchor, &shape), carried(s.head, &shape)))
        .collect();
    let primary = ed.views[view].sels.primary;
    let keyed: Vec<(usize, Range<usize>, String)> = edits
        .into_iter()
        .enumerate()
        .map(|(i, (r, t))| (usize::MAX - i, r, t))
        .collect();
    ed.edit_each(view, keyed, |start, _| Selection::point(start));
    let v = &mut ed.views[view];
    v.sels.items = items;
    v.sels.primary = primary;
    v.sels.normalize();
}

/// `<A-j>` / `<A-k>`: every selection's lines one line down or up, the
/// selections riding along and the mode kept. Selections on touching
/// lines are one block — they travel together and never pass one
/// another — and a block against the buffer's edge stays put.
fn move_lines(ed: &mut Editor, ctx: &Ctx, down: bool) {
    for _ in 0..ctx.count.max(1) {
        if !move_lines_once(ed, ctx.view, down) {
            break;
        }
    }
}

fn move_lines_once(ed: &mut Editor, view: ViewId, down: bool) -> bool {
    let id = ed.views[view].buffer;
    let buf = &ed.buffers[id];
    let last = buf.line_count().saturating_sub(1);
    let mut spans: Vec<(usize, usize)> = ed.views[view]
        .sels
        .iter()
        .map(|s| (buf.line_of(s.start()), buf.line_of(s.end())))
        .collect();
    spans.sort_unstable();
    let mut blocks: Vec<(usize, usize)> = Vec::new();
    for (a, b) in spans {
        match blocks.last_mut() {
            Some((_, lb)) if a <= *lb + 1 => *lb = (*lb).max(b),
            _ => blocks.push((a, b)),
        }
    }
    let mut edits = Vec::new();
    for (a, b) in blocks {
        let block = buf.line_start(a)..buf.line_range(b).end;
        let text = buf.slice(block.clone());
        if down {
            if b >= last {
                continue;
            }
            let next = buf.line_range(b + 1);
            let other = buf.slice(next.clone());
            edits.push((block.start..next.end, format!("{other}\n{text}")));
        } else {
            if a == 0 {
                continue;
            }
            let prev = buf.line_range(a - 1);
            let other = buf.slice(prev.clone());
            edits.push((prev.start..block.end, format!("{text}\n{other}")));
        }
    }
    if edits.is_empty() {
        return false;
    }
    // A swapped block's selections move by the other line's length and
    // its newline; `carried` would collapse them into the replaced range,
    // so they are placed by hand.
    let shifts: Vec<(Range<usize>, isize)> = edits
        .iter()
        .map(|(r, t)| {
            let other_len = if down {
                t.find('\n').unwrap_or(0)
            } else {
                t.len() - t.rfind('\n').map_or(t.len(), |i| i + 1)
            };
            let d = other_len as isize + 1;
            (r.clone(), if down { d } else { -d })
        })
        .collect();
    let items: Vec<Selection> = ed.views[view]
        .sels
        .iter()
        .map(|s| {
            let d = shifts
                .iter()
                .find(|(r, _)| r.start <= s.start() && s.end() <= r.end)
                .map_or(0, |(_, d)| *d);
            let at = |p: usize| (p as isize + d).max(0) as usize;
            Selection::new(at(s.anchor), at(s.head))
        })
        .collect();
    let primary = ed.views[view].sels.primary;
    let keyed: Vec<(usize, Range<usize>, String)> = edits
        .into_iter()
        .enumerate()
        .map(|(i, (r, t))| (usize::MAX - i, r, t))
        .collect();
    ed.edit_each(view, keyed, |start, _| Selection::point(start));
    let v = &mut ed.views[view];
    v.sels.items = items;
    v.sels.primary = primary;
    v.sels.normalize();
    true
}

/// `<A-h>` / `<A-l>`: one selection's shape moved a step by its kind. On
/// lines — a bare caret, `V`, insert mode — the lines are dedented or
/// indented a tabstop with the selection kept, so `V<A-l><A-l><A-j>` is
/// one gesture; on characters (`v`) the text is dragged one column
/// left or right within its line, swapping with its neighbour.
fn nudge(ed: &mut Editor, ctx: &Ctx, right: bool) {
    let charwise = ed.mode(ctx.view) == Mode::Visual && !ed.views[ctx.view].visual_linewise;
    for _ in 0..ctx.count.max(1) {
        if charwise {
            drag_chars(ed, ctx, right);
        } else {
            shift_lines(ed, ctx, right);
        }
    }
}

fn shift_lines(ed: &mut Editor, ctx: &Ctx, right: bool) {
    let id = view(ed, ctx).buffer;
    let ts = ed.shiftwidth_in(id);
    let unit = ed.indent_unit_in(id);
    let buf = &ed.buffers[id];
    let mut lines: Vec<usize> = ed.views[ctx.view]
        .sels
        .iter()
        .flat_map(|s| buf.line_of(s.start())..=buf.line_of(s.end()))
        .collect();
    lines.sort_unstable();
    lines.dedup();
    let mut edits = Vec::new();
    for &ln in &lines {
        let start = buf.line_start(ln);
        if right {
            if !buf.line_range(ln).is_empty() {
                edits.push((start..start, unit.clone()));
            }
        } else {
            let text = buf.line_text(ln);
            let n = if text.starts_with('\t') {
                1
            } else {
                text.chars().take(ts).take_while(|c| *c == ' ').count()
            };
            if n > 0 {
                edits.push((start..start + n, String::new()));
            }
        }
    }
    edit_keeping(ed, ctx.view, edits);
}

fn drag_chars(ed: &mut Editor, ctx: &Ctx, right: bool) {
    let id = view(ed, ctx).buffer;
    let buf = &ed.buffers[id];
    let sels = ed.views[ctx.view].sels.items.clone();
    let taken: Vec<Range<usize>> = sels
        .iter()
        .map(|s| s.start()..buf.next_char(s.end()))
        .collect();
    let mut edits = Vec::new();
    let mut moved: Vec<Option<isize>> = vec![None; sels.len()];
    for (i, r) in taken.iter().enumerate() {
        let (edit, text, d) = if right {
            if r.end >= buf.len() || buf.char_at(r.end) == Some('\n') {
                continue;
            }
            let after = buf.next_char(r.end);
            (
                r.start..after,
                format!("{}{}", buf.slice(r.end..after), buf.slice(r.clone())),
                (after - r.end) as isize,
            )
        } else {
            if r.start == 0 {
                continue;
            }
            let before = buf.prev_char(r.start);
            if buf.char_at(before) == Some('\n') {
                continue;
            }
            (
                before..r.end,
                format!("{}{}", buf.slice(r.clone()), buf.slice(before..r.start)),
                -((r.start - before) as isize),
            )
        };
        // The neighbour is another selection's: the two stay.
        if taken
            .iter()
            .enumerate()
            .any(|(j, o)| j != i && o.start < edit.end && edit.start < o.end)
        {
            continue;
        }
        edits.push((usize::MAX - i, edit, text));
        moved[i] = Some(d);
    }
    if edits.is_empty() {
        return;
    }
    let items: Vec<Selection> = sels
        .iter()
        .zip(&moved)
        .map(|(s, d)| {
            let d = d.unwrap_or(0);
            let at = |p: usize| (p as isize + d).max(0) as usize;
            Selection::new(at(s.anchor), at(s.head))
        })
        .collect();
    let primary = ed.views[ctx.view].sels.primary;
    ed.edit_each(ctx.view, edits, |start, _| Selection::point(start));
    let v = &mut ed.views[ctx.view];
    v.sels.items = items;
    v.sels.primary = primary;
    v.sels.normalize();
}

fn add_cursor(ed: &mut Editor, ctx: &Ctx, dy: i64) {
    let id = view(ed, ctx).buffer;
    let buf = &ed.buffers[id];
    let v = &mut ed.views[ctx.view];
    let p = v.sels.primary();
    let (ln, col) = m::line_col(buf, p.head);
    let target = ln as i64 + dy * ctx.count.max(1) as i64;
    if target < 0 || target as usize >= buf.line_count() {
        return;
    }
    let head = m::offset_at(buf, target as usize, col);
    v.sels.push(Selection::point(head), true);
}

/// `cursor lines` / `cursor lines back` (`<C-j>` `<C-k>` in visual
/// mode): every selection becomes a caret on each line it covers, at
/// the column its head is on — vim's visual block, as carets — and
/// visual mode is left for normal with them. The primary is the last
/// line's caret, or the first's going back, so the next `<C-j>` or
/// `<C-k>` in normal mode grows the column the way it was made.
fn carets_per_line(ed: &mut Editor, ctx: &Ctx, down: bool) {
    let id = view(ed, ctx).buffer;
    let buf = &ed.buffers[id];
    let v = &ed.views[ctx.view];
    let mut heads = Vec::new();
    for s in v.sels.iter() {
        let (_, col) = m::line_col(buf, s.head);
        let first = buf.line_of(s.anchor.min(s.head));
        let last = buf.line_of(s.anchor.max(s.head));
        for ln in first..=last {
            heads.push(m::offset_at(buf, ln, col));
        }
    }
    heads.sort_unstable();
    heads.dedup();
    if heads.is_empty() {
        return;
    }
    let primary = if down { heads.len() - 1 } else { 0 };
    ed.views[ctx.view].sels = crate::Selections {
        items: heads.into_iter().map(Selection::point).collect(),
        primary,
    };
    ed.set_mode(ctx.view, Mode::Normal);
}

/// The `n`th `c` from `o` on — or back — across lines: what `;` does
/// after an `f`. A till stops before it, and starts past the one the
/// caret is already before, so a repeat moves.
fn find_across(
    buf: &Buffer,
    o: usize,
    c: char,
    forward: bool,
    till: bool,
    n: usize,
) -> Option<usize> {
    let len = buf.len();
    let mut left = n.max(1);
    if forward {
        let mut p = buf.next_char(o);
        if till && buf.char_at(p) == Some(c) {
            p = buf.next_char(p);
        }
        while p < len {
            if buf.char_at(p) == Some(c) {
                left -= 1;
                if left == 0 {
                    return Some(if till { buf.prev_char(p) } else { p });
                }
            }
            p = buf.next_char(p);
        }
    } else {
        let mut p = o;
        if till && p > 0 && buf.char_at(buf.prev_char(p)) == Some(c) {
            p = buf.prev_char(p);
        }
        while p > 0 {
            p = buf.prev_char(p);
            if buf.char_at(p) == Some(c) {
                left -= 1;
                if left == 0 {
                    return Some(if till { buf.next_char(p) } else { p });
                }
            }
        }
    }
    None
}

/// A surround under way (`gsa`, `gsr`): what the next character is for.
#[derive(Default, Debug)]
pub struct Surround {
    /// The ranges `surround add` collected, waiting for the character
    /// to wrap them in.
    pub ranges: Option<Vec<Range<usize>>>,
    /// The pair `surround replace` will swap, waiting for the new one.
    pub from: Option<char>,
    /// The regions `align` collected, waiting for the character to line
    /// them up on.
    pub align: Option<Align>,
}

/// An `align` under way: what its character or pattern will line up.
#[derive(Default, Debug)]
pub struct Align {
    /// On each line the motion or selections touched, the part they
    /// covered — the whole line under a linewise one — in order, one a
    /// line ([`align_regions`]).
    pub regions: Vec<Range<usize>>,
    /// The `*` or count typed before `<CR>` asked for a pattern, put
    /// before the pattern the prompt answers with ([`align_which`]).
    pub which: String,
}

/// What `align` works over: on each line a range touches, the part of
/// it the range covers — the whole line under a linewise one — so a
/// selection on each line of several (`<C-j>`, then `vi{`) lines up
/// what is inside it, as easy-align does in a visual block, and the
/// first `=` of the line, outside it, stays. One region a line, in
/// order; ranges on one line make one region, from the first's start to
/// the last's end.
fn align_regions(buf: &Buffer, ranges: &[(Range<usize>, bool)]) -> Vec<Range<usize>> {
    let mut by_line: std::collections::BTreeMap<usize, Range<usize>> = Default::default();
    for (r, linewise) in ranges {
        let (first, last) = if *linewise {
            op_lines(buf, r, true)
        } else {
            (
                buf.line_of(r.start),
                buf.line_of(r.end.saturating_sub(1).max(r.start)),
            )
        };
        for ln in first..=last {
            let line = buf.line_range(ln);
            let part = if *linewise {
                line
            } else {
                let start = r.start.max(line.start).min(line.end);
                start..r.end.min(line.end).max(start)
            };
            by_line
                .entry(ln)
                .and_modify(|have| {
                    have.start = have.start.min(part.start);
                    have.end = have.end.max(part.end);
                })
                .or_insert(part);
        }
    }
    by_line.into_values().collect()
}

/// Which of a region's matches `align` lines up: the first, every one —
/// each a column in turn, easy-align's `ga*=` — or the Nth (`ga2=`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Which {
    First,
    Every,
    Nth(usize),
}

impl Which {
    /// How the message names it: `every =`, `the 2nd =`, or just `=`.
    fn label(self, pattern: &str) -> String {
        match self {
            Which::First => pattern.to_string(),
            Which::Every => format!("every {pattern}"),
            Which::Nth(n) => {
                let suffix = match (n % 10, n % 100) {
                    (1, 11) | (2, 12) | (3, 13) => "th",
                    (1, _) => "st",
                    (2, _) => "nd",
                    (3, _) => "rd",
                    _ => "th",
                };
                format!("the {n}{suffix} {pattern}")
            }
        }
    }
}

/// `align on`'s arguments read: `* PATTERN`, `N PATTERN`, or a pattern
/// whole — with a character for the pattern, the arguments are the
/// modifier alone, or nothing. A lone `*` or number with no character
/// is a pattern, so `:align 2` lines up on a `2`; a literal `*` is
/// `\*`.
fn align_which(args: &str, has_char: bool) -> (Which, String) {
    let args = args.trim();
    let (head, rest) = if has_char {
        (args, "")
    } else {
        match args.split_once(char::is_whitespace) {
            Some((h, r)) => (h, r.trim_start()),
            None => ("", args),
        }
    };
    let which = if head == "*" {
        Some(Which::Every)
    } else if !head.is_empty() && head.bytes().all(|b| b.is_ascii_digit()) {
        Some(Which::Nth(head.parse::<usize>().unwrap_or(1).max(1)))
    } else {
        None
    };
    match which {
        Some(w) => (w, rest.to_string()),
        None => (Which::First, args.to_string()),
    }
}

/// The character after `ga` and its motion, or the pattern its
/// prompt asked for (`ga` + motion + `<CR>`, `:align PAT`): every
/// collected region that holds it has its first one moved to the same
/// column — the text before it, back to the region's start, trimmed of
/// trailing space, then padded — with one space before it when any of
/// them had space there, so `a = 1` and `bbb = 2` line up as `a   =
/// 1`, and aligning again changes nothing. With `*` every occurrence,
/// each a column in turn, the text between two of them the next
/// column's "before"; with a count the Nth alone. One edit a column a
/// line, one undo step; the regions without it stay. Without regions
/// collected, what the selections cover ([`sel_regions`]).
fn align_on(ed: &mut Editor, ctx: &Ctx) {
    let regions = match ed.surround.align.take() {
        Some(a) => a.regions,
        None => sel_regions(ed, ctx),
    };
    let (which, pattern) = align_which(&ctx.args.join(" "), ctx.arg_char.is_some());
    let (name, re) = match ctx.arg_char {
        Some(c) => (c.to_string(), None),
        None if pattern.is_empty() => return,
        None => match regex::Regex::new(&pattern) {
            Ok(re) => (pattern, Some(re)),
            Err(e) => {
                ed.message = format!("bad pattern: {e}");
                return;
            }
        },
    };
    let label = which.label(&name);
    // The matches to line up in a region's text, as byte ranges of it.
    let matches = |text: &str| -> Vec<Range<usize>> {
        let all: Vec<Range<usize>> = match (&re, ctx.arg_char) {
            (Some(re), _) => re.find_iter(text).map(|m| m.range()).collect(),
            (None, Some(c)) => text.match_indices(c).map(|(i, m)| i..i + m.len()).collect(),
            (None, None) => Vec::new(),
        };
        match which {
            Which::First => all.into_iter().take(1).collect(),
            Which::Every => all,
            Which::Nth(n) => all.into_iter().nth(n - 1).into_iter().collect(),
        }
    };
    let id = view(ed, ctx).buffer;
    let buf = &ed.buffers[id];
    // The caret goes to the first line's first non-blank, as after any
    // operator over lines.
    let first = regions.first().map_or(0, |r| buf.line_of(r.start));
    // Each region with a match: its line's text and where the line
    // starts, the matches in the text, and — as the columns go by — the
    // column the last one placed ends at (in characters, so a wide or a
    // multibyte one counts once) and the byte after it, where the next
    // column's text begins; before any, the region's start.
    struct Row {
        line: usize,
        text: String,
        matches: Vec<Range<usize>>,
        landed: usize,
        from: usize,
    }
    let mut rows: Vec<Row> = Vec::new();
    for r in regions {
        let line = buf.line_range(buf.line_of(r.start));
        let text = buf.slice(line.clone());
        let at = r.start - line.start;
        let matches: Vec<Range<usize>> = matches(&text[at..r.end - line.start])
            .into_iter()
            .map(|m| at + m.start..at + m.end)
            .collect();
        if matches.is_empty() {
            continue;
        }
        rows.push(Row {
            line: line.start,
            landed: text[..at].chars().count(),
            from: at,
            text,
            matches,
        });
    }
    if rows.is_empty() {
        ed.message = format!("no {label} in the lines");
        return;
    }
    let columns = rows.iter().map(|r| r.matches.len()).max().unwrap_or(0);
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    for k in 0..columns {
        // Each row with a k-th match: the text from the last match
        // placed to it, trimmed of trailing space, and the column that
        // puts the match at; the gap trimmed, to pad.
        let mut cols: Vec<(usize, usize, Range<usize>)> = Vec::new();
        let mut spaced = false;
        for (i, row) in rows.iter().enumerate() {
            let Some(m) = row.matches.get(k) else {
                continue;
            };
            let segment = &row.text[row.from..m.start];
            let trimmed = segment.trim_end();
            spaced |= trimmed.len() < segment.len();
            let col = row.landed + trimmed.chars().count();
            cols.push((i, col, row.from + trimmed.len()..m.start));
        }
        let target = cols.iter().map(|c| c.1).max().unwrap_or(0) + usize::from(spaced);
        for (i, col, gap) in cols {
            let row = &mut rows[i];
            let pad = " ".repeat(target - col);
            if row.text[gap.clone()] != pad {
                edits.push((row.line + gap.start..row.line + gap.end, pad));
            }
            let m = row.matches[k].clone();
            row.landed = target + row.text[m.clone()].chars().count();
            row.from = m.end;
        }
    }
    if edits.is_empty() || ed.apply_edits(id, &edits) {
        ed.message = format!("aligned on {label}");
    }
    let buf = &ed.buffers[id];
    ed.views[ctx.view].sels =
        crate::Selections::single(Selection::point(m::first_nonblank(buf, first)));
    ed.set_mode(ctx.view, Mode::Normal);
}

/// What the selections of `ctx`'s view cover, as regions for `align`
/// ([`align_regions`]) — under `V` their lines whole, else the selected
/// characters, the head's in, on each line they touch.
fn sel_regions(ed: &Editor, ctx: &Ctx) -> Vec<Range<usize>> {
    let buf = &ed.buffers[view(ed, ctx).buffer];
    let ranges: Vec<(Range<usize>, bool)> = sel_ranges(ed, ctx.view)
        .into_iter()
        .map(|r| (r, false))
        .collect();
    align_regions(buf, &ranges)
}

/// The pair a surround character stands for: a bracket either way
/// round (`b` and `B` for the round and curly ones, as vim's objects),
/// else the character on both sides.
fn pair_of(c: char) -> (char, char) {
    match c {
        '(' | ')' | 'b' => ('(', ')'),
        '[' | ']' => ('[', ']'),
        '{' | '}' | 'B' => ('{', '}'),
        '<' | '>' => ('<', '>'),
        c => (c, c),
    }
}

/// The range of the pair `c` names around `o`, both ends included.
fn pair_around(buf: &Buffer, o: usize, c: char) -> Option<Range<usize>> {
    let (open, close) = pair_of(c);
    if open == close {
        quote_object(buf, o, open, true)
    } else {
        bracket_object(buf, o, open, close, true)
    }
}

/// The character after `gsa` and its motion: each range collected is
/// wrapped in the pair it names, one edit per selection.
fn surround_wrap(ed: &mut Editor, ctx: &Ctx) {
    let (Some(c), Some(ranges)) = (ctx.arg_char, ed.surround.ranges.take()) else {
        return;
    };
    let (open, close) = pair_of(c);
    let id = view(ed, ctx).buffer;
    let edits = ranges
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let inner = ed.buffers[id].slice(r.clone());
            (i, r.clone(), format!("{open}{inner}{close}"))
        })
        .collect();
    ed.edit_each(ctx.view, edits, |start, _| Selection::point(start));
    ed.set_mode(ctx.view, Mode::Normal);
}

/// `gsd` + a character: the pair around each caret goes, its inside
/// stays.
fn surround_delete(ed: &mut Editor, ctx: &Ctx) {
    let Some(c) = ctx.arg_char else {
        return;
    };
    let id = view(ed, ctx).buffer;
    let buf = &ed.buffers[id];
    let mut edits = Vec::new();
    for (i, s) in ed.views[ctx.view].sels.iter().enumerate() {
        if let Some(r) = pair_around(buf, s.head, c) {
            let inner = buf.slice(buf.next_char(r.start)..buf.prev_char(r.end));
            edits.push((i, r, inner));
        }
    }
    if edits.is_empty() {
        ed.message = format!("no {c} around the caret");
        return;
    }
    ed.edit_each(ctx.view, edits, |start, _| Selection::point(start));
}

/// `gsr` + the pair's character: kept until the new pair's comes
/// (`surround replace with`).
fn surround_replace(ed: &mut Editor, ctx: &Ctx) {
    let Some(c) = ctx.arg_char else {
        return;
    };
    ed.surround.from = Some(c);
    ed.await_char("surround replace with");
}

/// The second character after `gsr`: the pair the first named, around
/// each caret, becomes the pair this one names.
fn surround_replace_with(ed: &mut Editor, ctx: &Ctx) {
    let (Some(to), Some(from)) = (ctx.arg_char, ed.surround.from.take()) else {
        return;
    };
    let (open, close) = pair_of(to);
    let id = view(ed, ctx).buffer;
    let buf = &ed.buffers[id];
    let mut edits = Vec::new();
    for (i, s) in ed.views[ctx.view].sels.iter().enumerate() {
        if let Some(r) = pair_around(buf, s.head, from) {
            let inner = buf.slice(buf.next_char(r.start)..buf.prev_char(r.end));
            edits.push((i, r, format!("{open}{inner}{close}")));
        }
    }
    if edits.is_empty() {
        ed.message = format!("no {from} around the caret");
        return;
    }
    ed.edit_each(ctx.view, edits, |start, _| Selection::point(start));
}

/// What `select next` and `select all matches` look for: the primary
/// selection's text as is, or — from a bare caret — the word under it,
/// whole (`\b` on both sides, as `*`). A press on a selection whose
/// text's word form is the search already, the last press's, keeps to
/// the word. The search is set to it, so `n` goes on past the
/// selections. The word's range comes back for a bare caret, which the
/// first press selects and nothing more.
fn select_pattern(ed: &mut Editor, ctx: &Ctx) -> Option<Option<Range<usize>>> {
    let id = view(ed, ctx).buffer;
    let buf = &ed.buffers[id];
    let p = ed.views[ctx.view].sels.primary();
    let (pattern, word) = if p.is_empty() {
        let (a, b) = m::word_at(buf, p.head);
        if a == b {
            ed.message = "no word under the caret".into();
            return None;
        }
        (
            format!(r"\b{}\b", regex::escape(&buf.slice(a..b))),
            Some(a..b),
        )
    } else {
        let (r, _) = op_range(buf, &p, MotionKind::Inclusive, 1);
        let text = regex::escape(&buf.slice(r));
        let word = format!(r"\b{text}\b");
        match &ed.search {
            Some(s) if s.is(&word, false) => (word, None),
            _ => (text, None),
        }
    };
    if let Err(e) = ed.set_search(&pattern, false) {
        ed.message = e;
        return None;
    }
    Some(word)
}

/// A selection over `r` as a visual one lies, the head on the last
/// character: an operator takes exactly the match.
fn over(buf: &Buffer, r: Range<usize>) -> Selection {
    let head = if r.end > r.start {
        buf.prev_char(r.end)
    } else {
        r.start
    };
    Selection::new(r.start, head)
}

/// `<D-d>` (Zed's; helix's `*` with the selection kept): a selection on
/// the next match of the primary's text — the word under a bare caret,
/// which the first press selects — added beside the ones there and made
/// primary, in visual mode, so a press more takes the next and an
/// operator takes them all. The next is looked for from the last
/// selection on, round the end, past what is selected already; when
/// that is every match, the message says so and nothing moves.
fn select_next(ed: &mut Editor, ctx: &Ctx) {
    let Some(word) = select_pattern(ed, ctx) else {
        return;
    };
    let id = view(ed, ctx).buffer;
    let buf = &ed.buffers[id];
    if let Some(w) = word {
        ed.views[ctx.view].sels = crate::Selections::single(over(buf, w));
        ed.set_mode(ctx.view, Mode::Visual);
        return;
    }
    let Some(re) = ed.search.as_ref().map(|s| s.re.clone()) else {
        return;
    };
    let text = buf.tree();
    let len = text.len();
    let sels = &ed.views[ctx.view].sels;
    let starts: Vec<usize> = sels.iter().map(Selection::start).collect();
    let after = sels
        .iter()
        .map(|s| op_range(buf, s, MotionKind::Inclusive, 1).0.end)
        .max()
        .unwrap_or(0)
        .min(len);
    let mut from = after;
    let mut wrapped = false;
    let found = loop {
        match crate::search::find_forward(text, &re, from) {
            Some(r) if wrapped && r.start >= after => break None,
            Some(r) if starts.contains(&r.start) => from = r.end.max(r.start + 1),
            Some(r) => break Some(r),
            None if !wrapped => {
                wrapped = true;
                from = 0;
            }
            None => break None,
        }
    };
    match found {
        Some(r) => {
            let s = over(buf, r);
            ed.views[ctx.view].sels.push(s, true);
            ed.set_mode(ctx.view, Mode::Visual);
        }
        None => ed.message = "every match is selected".into(),
    }
}

/// `<D-L>` (Zed's): a selection on every match of the primary's text,
/// or of the word under a bare caret, the primary the one at the caret,
/// in visual mode — one operator over them all.
fn select_all_matches(ed: &mut Editor, ctx: &Ctx) {
    let Some(word) = select_pattern(ed, ctx) else {
        return;
    };
    let Some(re) = ed.search.as_ref().map(|s| s.re.clone()) else {
        return;
    };
    let id = view(ed, ctx).buffer;
    let buf = &ed.buffers[id];
    let text = buf.tree();
    let hits = crate::search::hits_in(text, &re, 0..text.len());
    if hits.is_empty() {
        ed.message = "no match".into();
        return;
    }
    let at = match word {
        Some(w) => w.start,
        None => ed.views[ctx.view].sels.primary().start(),
    };
    let primary = hits.iter().position(|r| r.start == at).unwrap_or(0);
    let items: Vec<Selection> = hits.into_iter().map(|r| over(buf, r)).collect();
    let mut sels = crate::Selections { items, primary };
    sels.normalize();
    ed.views[ctx.view].sels = sels;
    ed.set_mode(ctx.view, Mode::Visual);
}

/// What each selection of `view` covers: its lines under `V` (the last
/// one's newline off, so a piece does not end on it), else its
/// characters, the head's included, a newline under its end with them
/// (`sel_range`) — as an operator in visual mode takes them.
pub(crate) fn sel_ranges(ed: &Editor, view_id: ViewId) -> Vec<Range<usize>> {
    let v = &ed.views[view_id];
    let buf = &ed.buffers[v.buffer];
    let lines = ed.mode(view_id) == Mode::Visual && v.visual_linewise;
    v.sels
        .iter()
        .map(|s| {
            if lines {
                let r = line_range_of_sel(buf, s, 0);
                let nl = r.end > r.start && buf.slice(r.end - 1..r.end) == "\n";
                r.start..r.end - usize::from(nl)
            } else {
                sel_range(buf, s)
            }
        })
        .collect()
}

/// The pattern of a `select` command, read as `/` reads one, or the
/// message saying what is wrong with it.
fn select_regex(pattern: &str) -> Result<regex::bytes::Regex, String> {
    crate::search::Search::new(pattern, true).map(|s| s.re)
}

/// The new selection set `pieces` make, per old selection, over `view`:
/// visual selections as a match lies (the head on its last character),
/// the primary the first inside the old primary. None when there are
/// none.
fn set_pieces(ed: &mut Editor, view_id: ViewId, pieces: Vec<Vec<Range<usize>>>) -> bool {
    let old = ed.views[view_id].sels.primary;
    let buf = &ed.buffers[ed.views[view_id].buffer];
    let mut items = Vec::new();
    let mut primary = 0;
    for (i, rs) in pieces.into_iter().enumerate() {
        if i == old && !rs.is_empty() {
            primary = items.len();
        }
        items.extend(
            rs.into_iter()
                .filter(|r| r.end > r.start)
                .map(|r| over(buf, r)),
        );
    }
    if items.is_empty() {
        return false;
    }
    let primary = primary.min(items.len() - 1);
    let mut sels = crate::Selections { items, primary };
    sels.normalize();
    let v = &mut ed.views[view_id];
    v.sels = sels;
    v.visual_linewise = false;
    v.goal_col = None;
    ed.set_mode(view_id, Mode::Visual);
    true
}

/// The matches of `re` inside `range` of `buf`, read in that text alone
/// (so `^` is its start), empty ones left out.
fn matches_in(buf: &Buffer, re: &regex::bytes::Regex, range: Range<usize>) -> Vec<Range<usize>> {
    let text = buf.slice(range.clone());
    re.find_iter(text.as_bytes())
        .filter(|m| m.end() > m.start())
        .map(|m| range.start + m.start()..range.start + m.end())
        .collect()
}

/// The three that read a pattern, run with it: the matches inside every
/// selection become the selections (`within`), every selection split on
/// them (`split`), or the selections that match kept — those that do
/// not, for a pattern after `!` (`keep`). A pattern that makes no
/// selection says so and changes nothing.
pub(crate) fn select_by(ed: &mut Editor, view_id: ViewId, how: crate::Select, pattern: &str) {
    use crate::Select;
    let (negate, pat) = match (how, pattern.strip_prefix('!')) {
        (Select::Keep, Some(rest)) => (true, rest),
        _ => (false, pattern),
    };
    let re = match select_regex(pat) {
        Ok(re) => re,
        Err(e) => {
            ed.message = e;
            return;
        }
    };
    let ranges = sel_ranges(ed, view_id);
    let buf = &ed.buffers[ed.views[view_id].buffer];
    let pieces: Vec<Vec<Range<usize>>> = ranges
        .into_iter()
        .map(|r| match how {
            Select::Within => matches_in(buf, &re, r),
            Select::Split => {
                let mut out = Vec::new();
                let mut at = r.start;
                for m in matches_in(buf, &re, r.clone()) {
                    out.push(at..m.start);
                    at = m.end;
                }
                out.push(at..r.end);
                out
            }
            Select::Keep => {
                let hit = re.is_match(buf.slice(r.clone()).as_bytes());
                if hit != negate { vec![r] } else { Vec::new() }
            }
        })
        .collect();
    if !set_pieces(ed, view_id, pieces) {
        ed.message = match how {
            Select::Keep => "no selection is left".into(),
            _ => "no match in the selections".into(),
        };
    }
}

/// `select within` / `split` / `keep`: with a pattern, run; bare, a
/// prompt that previews what `<CR>` will make.
fn select_command(ed: &mut Editor, ctx: &Ctx, how: crate::Select) {
    if ctx.args.is_empty() {
        ed.open_prompt(ctx.view, Prompt::Select { how });
        return;
    }
    select_by(ed, ctx.view, how, &ctx.args.join(" "));
}

/// `select lines`, helix's `<A-s>`: every line of every selection its
/// own selection, its newline off; an empty line is none.
fn select_lines(ed: &mut Editor, ctx: &Ctx) {
    let ranges = sel_ranges(ed, ctx.view);
    let buf = &ed.buffers[ed.views[ctx.view].buffer];
    let pieces: Vec<Vec<Range<usize>>> = ranges
        .into_iter()
        .map(|r| {
            let (a, b) = (
                buf.line_of(r.start),
                buf.line_of(r.end.saturating_sub(1).max(r.start)),
            );
            (a..=b)
                .map(|ln| {
                    let lr = buf.line_range(ln);
                    let end = if buf.slice(lr.clone()).ends_with('\n') {
                        lr.end - 1
                    } else {
                        lr.end
                    };
                    lr.start.max(r.start)..end.min(r.end)
                })
                .collect()
        })
        .collect();
    if !set_pieces(ed, ctx.view, pieces) {
        ed.message = "no line with text in the selections".into();
    }
}

/// `select drop primary`, helix's `<A-,>`: the primary selection gone,
/// the one before it primary.
fn select_drop_primary(ed: &mut Editor, ctx: &Ctx) {
    let sels = &mut ed.views[ctx.view].sels;
    if sels.len() < 2 {
        ed.message = "one selection".into();
        return;
    }
    let p = sels.primary;
    sels.items.remove(p);
    sels.primary = p.saturating_sub(1).min(sels.items.len() - 1);
}

/// The keymap the engine ships: vim's letters where vim has them, and
/// beside them the clusters docs/design/keys.md lays out — a prefix or
/// a modifier per family, so a hand that knows one member of a family
/// finds the rest: `<C-w>` and `<C-hjkl>` for panes, `[x` / `]x` for
/// the next and previous of a thing, `g` for going somewhere, Alt (and
/// ⌘ for the Zed fingers) for selections, `<leader>` groups for the
/// rest.
pub fn default_keymap(km: &mut Keymap) {
    use Mode::*;
    let n = [
        ("h", "move left"),
        ("<Left>", "move left"),
        ("<BS>", "move left"),
        ("l", "move right"),
        ("<Right>", "move right"),
        ("j", "move down"),
        ("<Down>", "move down"),
        ("k", "move up"),
        ("<Up>", "move up"),
        ("0", "line start"),
        ("<Home>", "line start"),
        ("^", "line nonblank"),
        ("$", "line end"),
        ("<End>", "line end"),
        // helix's `gh` / `gl`: the line's ends under `g`, where `gg`
        // and `G` are the file's.
        ("gh", "line nonblank"),
        ("gl", "line end"),
        ("w", "word next"),
        ("b", "word prev"),
        ("e", "word end"),
        ("ge", "word end back"),
        ("W", "bigword next"),
        ("B", "bigword prev"),
        ("E", "bigword end"),
        ("gE", "bigword end back"),
        ("}", "paragraph next"),
        ("{", "paragraph prev"),
        ("H", "screen top"),
        ("M", "screen middle"),
        ("L", "screen bottom"),
        ("gg", "goto file start"),
        ("G", "goto file end"),
        ("<C-d>", "page half down"),
        ("<C-u>", "page half up"),
        ("<C-f>", "page down"),
        ("<PageDown>", "page down"),
        ("<C-b>", "page up"),
        ("<PageUp>", "page up"),
        ("%", "match_bracket"),
        ("f", "find char"),
        ("F", "find char back"),
        ("t", "till char"),
        ("T", "till char back"),
        (";", "find repeat"),
        ("n", "search next"),
        ("N", "search prev"),
        ("*", "search word"),
        ("/", "search"),
        ("?", "search back"),
        (":", "command"),
        ("d", "delete"),
        ("c", "change"),
        ("y", "yank"),
        (">", "indent"),
        ("<", "dedent"),
        ("=", "reindent"),
        ("gu", "case lower"),
        ("gU", "case upper"),
        ("g~", "case toggle"),
        ("gc", "comment"),
        ("gb", "comment block"),
        ("~", "case toggle char"),
        ("J", "join"),
        ("x", "delete char"),
        ("<Del>", "delete char"),
        ("X", "delete char back"),
        ("D", "delete to end"),
        ("C", "change to end"),
        ("s", "change char"),
        ("S", "change line"),
        ("r", "replace char"),
        ("\"", "register"),
        ("i", "insert"),
        ("a", "append"),
        ("I", "insert line start"),
        ("A", "append line end"),
        ("o", "open below"),
        ("O", "open above"),
        ("p", "paste after"),
        ("P", "paste before"),
        ("u", "undo"),
        ("<C-r>", "redo"),
        ("U", "redo"),
        ("g-", "undo older"),
        ("g+", "undo newer"),
        // The stream: `.` and macros, vim's keys.
        (".", "repeat"),
        ("q", "macro record"),
        ("@", "macro play"),
        ("v", "visual"),
        ("V", "visual line"),
        // Selections (docs/design/keys.md): Ctrl counts them, Alt moves
        // one — vim-visual-multi's Ctrl keys, ⌘ the same for Zed's
        // fingers — and `(` `)` walk the primary round.
        (",", "cursor primary"),
        ("(", "cursor rotate back"),
        (")", "cursor rotate"),
        ("<C-j>", "cursor below"),
        ("<C-k>", "cursor above"),
        ("<C-Down>", "cursor below"),
        ("<C-Up>", "cursor above"),
        ("<C-n>", "select next"),
        ("<D-d>", "select next"),
        ("<C-S-n>", "select all matches"),
        ("<D-S-l>", "select all matches"),
        ("<D-a>", "select all"),
        // Ctrl-shift where there is no ⌘, as the clipboard's `<C-S-v>`
        // and the panes' `<C-S-1>` are: `<C-a>` is vim's increment.
        ("<C-S-a>", "select all"),
        ("<A-j>", "move line down"),
        ("<A-k>", "move line up"),
        ("<A-h>", "nudge left"),
        ("<A-l>", "nudge right"),
        // Alt with Shift sizes the pane the way Alt moves the line —
        // the fast pair, no prefix. Carrying a pane is `<C-w>HJKL`
        // below, where vim puts it.
        ("<A-S-h>", "pane narrower"),
        ("<A-S-l>", "pane wider"),
        ("<A-S-j>", "pane shorter"),
        ("<A-S-k>", "pane taller"),
        // Numbers: vim's, per selection.
        ("<C-a>", "increment"),
        ("<C-x>", "decrement"),
        // A terminal pane's copy mode: wezterm's chord, run from the
        // pane by `Kawoosh::pane_chord` (the shell's `scrollback`).
        ("<C-S-x>", "scrollback"),
        // The syntax tree's nodes, helix's way: out, back in, along.
        ("<A-o>", "select node"),
        ("<A-i>", "select node child"),
        ("<A-n>", "select node next"),
        ("<A-p>", "select node prev"),
        ("<A-u>", "node parent"),
        ("<Esc>", "normal"),
        ("<C-c>", "normal"),
        // The file: `<C-s>` from any mode, and vim's `ZZ` / `ZQ`.
        ("<C-s>", "write"),
        ("<D-s>", "write"),
        ("ZZ", "write quit"),
        ("ZQ", "quit!"),
        // The tab's jumps (docs/design/jumps.md): vim's keys; kui tells
        // `<C-i>` from `<Tab>`, which stays free.
        ("<C-o>", "jump back"),
        ("<C-i>", "jump forward"),
        // Panes, tabs, the dock: the shell's commands (Effect::Shell),
        // under `<C-w>` as vim's, and the four moves on `<C-S-hjkl>` —
        // one spelling, the same in every mode and every kind of pane
        // (`Kawoosh::pane_chord`).
        ("<C-w>v", "vsplit"),
        ("<C-w>s", "split"),
        ("<C-w>q", "close"),
        ("<C-w>c", "close"),
        // The command line from a pane whose `:` is the pty's or a
        // view's own (a terminal, a Lua view): the cluster's spelling.
        ("<C-w>:", "command"),
        ("<C-w>o", "only"),
        ("<C-w>w", "pane next"),
        ("<C-w>x", "pane swap"),
        // The pane carried a place: the shifted letter, vim's "to the
        // far side" read as one step — a strip's column along the
        // ribbon, a tree's pane past its neighbour.
        ("<C-w>H", "pane move left"),
        ("<C-w>L", "pane move right"),
        ("<C-w>J", "pane move down"),
        ("<C-w>K", "pane move up"),
        // The pane out of its column's stack into one of its own, and
        // the next column's top pane into it: `e` for expel and `i`
        // for in, as `<A-o>` and `<A-i>` are the syntax node's.
        ("<C-w>e", "pane expel"),
        ("<C-w>i", "pane consume"),
        // Sizing again, vim's own spelling: `<` `>` the width, `-` `+`
        // the height, beside the `<A-S-…>` chords that need no prefix.
        ("<C-w><", "pane narrower"),
        ("<C-w>>", "pane wider"),
        ("<C-w>-", "pane shorter"),
        ("<C-w>+", "pane taller"),
        ("<C-w>h", "pane left"),
        ("<C-w>j", "pane down"),
        ("<C-w>k", "pane up"),
        ("<C-w>l", "pane right"),
        ("<C-w><Left>", "pane left"),
        ("<C-w><Down>", "pane down"),
        ("<C-w><Up>", "pane up"),
        ("<C-w><Right>", "pane right"),
        ("<C-S-h>", "pane left"),
        ("<C-S-j>", "pane down"),
        ("<C-S-k>", "pane up"),
        ("<C-S-l>", "pane right"),
        ("<C-w>t", "tab new"),
        ("<C-w>d", "dock"),
        // The pane into the dock and back out, the shifted letter as
        // `HJKL` carry the pane.
        ("<C-w>D", "pane dock"),
        ("<C-w>!", "terminal"),
        // `.` for here: where the pane in front is.
        ("<C-w>.", "terminal here"),
        // `/` for the directory here, listed beside: `-` lists it in
        // the pane, and `<C-w>-` is vim's shorter.
        ("<C-w>/", "dir here"),
        ("<C-w>n", "toast"),
        ("gt", "tab next"),
        ("gT", "tab prev"),
        // The Nth column of a strip, the Nth pane of a tree: ⌘ with
        // the digit where there is a ⌘, and ctrl-shift with it
        // everywhere else. Both are spellings no pty can use, so they
        // reach a column from a terminal pane too
        // (`Kawoosh::pane_chord`) — where a plain `<C-3>` is the
        // shell's, which is why it is not bound (2026-09-22).
        ("<D-1>", "pane goto 1"),
        ("<D-2>", "pane goto 2"),
        ("<D-3>", "pane goto 3"),
        ("<D-4>", "pane goto 4"),
        ("<D-5>", "pane goto 5"),
        ("<D-6>", "pane goto 6"),
        ("<D-7>", "pane goto 7"),
        ("<D-8>", "pane goto 8"),
        ("<D-9>", "pane goto 9"),
        ("<C-S-1>", "pane goto 1"),
        ("<C-S-2>", "pane goto 2"),
        ("<C-S-3>", "pane goto 3"),
        ("<C-S-4>", "pane goto 4"),
        ("<C-S-5>", "pane goto 5"),
        ("<C-S-6>", "pane goto 6"),
        ("<C-S-7>", "pane goto 7"),
        ("<C-S-8>", "pane goto 8"),
        ("<C-S-9>", "pane goto 9"),
        // `z`: vim's scrolling, read on the ribbon — the focused
        // column to an edge, or the middle.
        ("zv", "mask reveal"),
        ("zs", "strip left"),
        ("ze", "strip right"),
        ("zz", "strip center"),
        // `]x` / `[x`: the next and the previous of a thing.
        ("]b", "buffer next"),
        ("[b", "buffer prev"),
        ("]t", "tab next"),
        ("[t", "tab prev"),
        ("]<Space>", "line blank below"),
        ("[<Space>", "line blank above"),
        // The tab itself moved along the strip: the shifted letter,
        // as `gT` is `gt` the other way.
        ("]T", "tab move right"),
        ("[T", "tab move left"),
        ("]q", "error next"),
        ("[q", "error prev"),
        ("]d", "lsp diagnostic next"),
        ("[d", "lsp diagnostic prev"),
        // Hunks (docs/design/vcs.md): the changes against the buffer's
        // base, walked, taken back, shown.
        ("]h", "hunk next"),
        ("[h", "hunk prev"),
        ("<leader>hr", "hunk reset"),
        ("<leader>hR", "hunk reset!"),
        ("<leader>hp", "hunk preview"),
        // Staged as `git add` does (`a`), taken back out (`u`); the
        // shifted letter the whole buffer, as `hR` is.
        ("<leader>ha", "hunk stage"),
        ("<leader>hA", "hunk stage!"),
        ("<leader>hu", "hunk unstage"),
        ("<leader>hU", "hunk unstage!"),
        // Merge conflicts: walked, and resolved a side at a time.
        ("]x", "conflict next"),
        ("[x", "conflict prev"),
        ("<leader>hxo", "conflict ours"),
        ("<leader>hxt", "conflict theirs"),
        ("<leader>hxb", "conflict both"),
        ("<leader>hxn", "conflict none"),
        ("<leader>hxO", "conflict ours!"),
        ("<leader>hxT", "conflict theirs!"),
        // Marks (docs/design/marks.md): vim's letters; `]'` `['` the
        // marked lines of the file.
        ("'", "mark line"),
        ("`", "mark go"),
        ("]'", "mark next"),
        ("['", "mark prev"),
        // The syntax's functions (docs/design/nodes.md Decision 9).
        ("]f", "function next"),
        ("[f", "function prev"),
        // The yank-pop: the last put walked through the memory.
        ("[p", "put older"),
        ("]p", "put newer"),
        // `g`: going somewhere. The language server's own under `gr`,
        // as neovim 0.11 has them (keymap-regroup.md): `grr` `grn`
        // `gra` `gri` `grt`, and kawoosh's `grf` `grs` `grS`.
        ("gd", "lsp definition"),
        ("gD", "lsp declaration"),
        ("grr", "lsp references"),
        ("grn", "lsp rename"),
        ("gra", "lsp action"),
        ("gri", "lsp implementation"),
        ("grt", "lsp type definition"),
        // `grf` formats with the buffer's formatter, its server one
        // of them (formatters.md).
        ("grf", "format"),
        ("grs", "picker symbols"),
        ("grS", "picker workspace_symbols"),
        ("K", "lsp hover"),
        ("<C-e>", "lsp diagnostic"),
        ("<CR>", "goto location"),
        ("-", "dir"),
        // oil's `_`: the working directory listed, wherever you are.
        ("_", "dir ."),
        // Surrounds under `gs`, as mini.surround's: add, delete, replace.
        ("gsa", "surround add"),
        // Align, as vim-easy-align's: `gaip=`, or `ga=` over a selection.
        ("ga", "align"),
        ("gsd", "surround delete"),
        ("gsr", "surround replace"),
        ("gx", "open link"),
        // `<C-w>`: the tabs and the layout beside the panes — `c`
        // closes the pane, `C` the tab; `m` the layout's mode.
        ("<C-w>C", "tab close"),
        ("<C-w>m", "layout"),
        // `Z`: write and quit, all of them too.
        ("ZA", "quit all"),
        // `<leader>` groups, one module each (keymap-regroup.md): b
        // buffers, c compile, i help, m memory, o the look, s search,
        // w the workspace, y the path; single letters for the daily
        // few. A key reachable without the leader is not on it again,
        // and keys.md's reserved spellings stay free (`<leader>h` the
        // hunks', `<leader>wd` `<leader>wc` git's).
        ("<leader><leader>", "buffer list"),
        ("<leader>bd", "buffer delete"),
        ("<leader>bD", "buffer delete!"),
        ("<leader>bo", "buffer delete others"),
        ("<leader>ih", "help"),
        ("<leader>im", "messages"),
        ("<leader>ic", "commands"),
        ("<leader>ws", "session save"),
        ("<leader>wr", "session restore"),
        ("<leader>cc", "compile"),
        ("<leader>cC", "compile pick"),
        ("<leader>'", "picker marks"),
        ("<leader>u", "undo history"),
        ("<leader>x", "lua eval"),
        ("<leader>mm", "memory"),
        ("<leader>mp", "memory pins"),
        ("<leader>ma", "memory pin"),
        ("<leader>ml", "memory recent"),
        ("<leader>mj", "memory jumps"),
        ("<leader>mf", "memory files"),
        ("<A-1>", "memory pin 1"),
        ("<A-2>", "memory pin 2"),
        ("<A-3>", "memory pin 3"),
        ("<A-4>", "memory pin 4"),
        ("<A-5>", "memory pin 5"),
        ("<A-6>", "memory pin 6"),
        ("<A-7>", "memory pin 7"),
        ("<A-8>", "memory pin 8"),
        ("<A-9>", "memory pin 9"),
        ("<leader>yp", "path copy relative"),
        ("<leader>yP", "path copy absolute"),
        ("<leader>yd", "path copy dir"),
        ("<leader>yD", "path copy dir absolute"),
        ("<leader>yn", "path copy name"),
        ("<leader>yN", "path copy stem"),
        ("<leader>?", "keys"),
        // `o`: the look (docs/design/themes.md Decision 3) — the base
        // flipped, the OS's again, the themes' pane, the fonts' (fonts.md
        // Decision 3), the lab of both.
        ("<leader>ot", "theme toggle"),
        ("<leader>os", "theme system"),
        ("<leader>oo", "themes"),
        ("<leader>of", "fonts"),
        ("<leader>ol", "theme lab"),
        // The language server's hints and the markdown buffer rendered:
        // how the text looks, so the look's.
        ("<leader>oh", "lsp hints"),
        ("<leader>om", "markdown toggle"),
        // Soft wrap (wrap.md): the focused pane wrapped or not, and the
        // caret a row on screen — `j` `k` stay a line each, as vim's.
        ("<leader>ow", "wrap"),
        // The symbols the caret is in, on the pane's title bar
        // (breadcrumbs.md).
        ("<leader>ob", "breadcrumbs"),
        ("gj", "move down row"),
        ("gk", "move up row"),
        ("g<Down>", "move down row"),
        ("g<Up>", "move up row"),
    ];
    for (k, c) in n {
        km.bind(Normal, k, c);
    }
    // A terminal pane's: the shell's prompts (its OSC 133 marks), ⌘↑ ⌘↓
    // as iTerm and Terminal.app have them and ctrl-shift where there is
    // no ⌘, the last command's output copied, and the clipboard pasted
    // on insert mode's two spellings — chords, so they reach the pane
    // past its pty (`Kawoosh::pane_chord`). Local to a terminal pane
    // (docs/design/local-maps.md): nothing another pane looks up.
    for (k, c) in [
        ("<D-Up>", "terminal prompt prev"),
        ("<D-Down>", "terminal prompt next"),
        ("<C-S-Up>", "terminal prompt prev"),
        ("<C-S-Down>", "terminal prompt next"),
        ("<C-S-o>", "terminal output"),
        // After the terminal's escape, `r` is raw (terminal-keys.md
        // Decision 2): vim's `r` everywhere else.
        ("r", "terminal raw"),
        ("<D-v>", "paste clipboard"),
        ("<C-S-v>", "paste clipboard"),
    ] {
        km.bind_local("terminal", Normal, k, c, &[]);
    }
    // A rendered markdown pane under `markdown.navigation = "row"`:
    // `j` `k` and the arrows a row on screen, `gj` `gk` everywhere else
    // (wrap.md Decision 2). An operator's `j` stays a line: its lookup
    // falls through to normal mode's, where the row move would shadow it.
    for (k, row, line) in [
        ("j", "move down row", "move down"),
        ("k", "move up row", "move up"),
        ("<Down>", "move down row", "move down"),
        ("<Up>", "move up row", "move up"),
    ] {
        km.bind_local("rows", Normal, k, row, &[]);
        km.bind_local("rows", Visual, k, row, &[]);
        km.bind_local("rows", OperatorPending, k, line, &[]);
    }
    // What each prefix is for, as the which-key names it.
    for (keys, name) in [
        ("<leader>", "leader"),
        ("<leader>b", "buffers"),
        ("<leader>c", "compile"),
        ("<leader>h", "hunks, version control"),
        ("<leader>hx", "conflicts"),
        ("<leader>i", "help"),
        ("<leader>m", "memory"),
        ("<leader>s", "search"),
        ("<leader>w", "workspace"),
        ("<leader>y", "copy the path"),
        ("<leader>o", "look"),
        ("<leader>v", "selections"),
        ("g", "goto"),
        ("gr", "language server"),
        ("gs", "surround"),
        ("<C-w>", "panes, tabs, dock"),
        ("]", "next"),
        ("[", "previous"),
        ("Z", "write, quit"),
    ] {
        km.describe(keys, name);
    }
    let v = [
        ("<leader>x", "lua eval"),
        ("o", "cursor swap"),
        // Ctrl counts the selections: in visual mode, one caret a line.
        ("<C-j>", "cursor lines"),
        ("<C-k>", "cursor lines back"),
        ("<C-Down>", "cursor lines"),
        ("<C-Up>", "cursor lines back"),
        ("x", "delete char"),
        ("i", "textobject inner"),
        ("a", "textobject around"),
        ("p", "paste over"),
        ("P", "paste over keep"),
        ("u", "case lower"),
        ("U", "case upper"),
        ("~", "case toggle"),
        ("<D-c>", "yank"),
        ("<Esc>", "normal"),
        ("<C-c>", "normal"),
        // helix's selections by a pattern (docs/design/selections.md):
        // a leader group, so no vim letter is shadowed.
        ("<leader>vs", "select within"),
        ("<leader>vS", "select split"),
        ("<leader>vk", "select keep"),
        ("<leader>vl", "select lines"),
        ("<A-,>", "select drop primary"),
        ("grf", "format selection"),
    ];
    for (k, c) in v {
        km.bind(Visual, k, c);
    }
    // The carets `<C-j>` makes are normal mode's.
    km.bind(Normal, "<A-,>", "select drop primary");
    // The case operators doubled on their last letter, vim's `guu`
    // `gUU` `g~~`, a line each; after any other operator the letter
    // is no operator of its, and nothing runs.
    let op = [
        ("i", "textobject inner"),
        ("a", "textobject around"),
        ("u", "case lower"),
        ("U", "case upper"),
        ("~", "case toggle"),
        // `gcc`; after any other operator `c` is what it was.
        ("c", "comment lines"),
    ];
    for (k, c) in op {
        km.bind(OperatorPending, k, c);
    }
    // After an operator `gj` `gk` are the line moves: a screen row is
    // the shell's, resolved after the key, and no motion an operator
    // can wait for (wrap.md Decision 2).
    for (k, c) in [("gj", "move down"), ("gk", "move up")] {
        km.bind(OperatorPending, k, c);
    }
    // Pane mode (docs/design/keys.md "Panes without a view"): the
    // list keys every listing pane answers through `list …`, and the
    // memory and undo panes' own, gated by their facts. `<C-w>…`,
    // `<leader>…` and the shift chords fall through to normal mode's
    // (`Keymap::shared_from_pane`), so nothing is mirrored here.
    let p = [
        (":", "command"),
        ("j", "list down"),
        ("<Down>", "list down"),
        ("k", "list up"),
        ("<Up>", "list up"),
        ("<C-n>", "list next"),
        ("<C-p>", "list prev"),
        ("gg", "list first"),
        ("G", "list last"),
        ("<C-d>", "list half down"),
        ("<C-u>", "list half up"),
        ("<C-f>", "list page down"),
        ("<PageDown>", "list page down"),
        ("<C-b>", "list page up"),
        ("<PageUp>", "list page up"),
        ("<CR>", "list open"),
        ("<Tab>", "list view"),
        ("<S-Tab>", "list view prev"),
        ("q", "close"),
        ("<Esc>", "pane back"),
        // `g` is a prefix here (`gg`), so the tab keys under it are
        // bound rather than shared; `]x` / `[x` fall through.
        ("gt", "tab next"),
        ("gT", "tab prev"),
        // The Nth column of a strip, the Nth pane of a tree: ⌘ with
        // the digit where there is a ⌘, and ctrl-shift with it
        // everywhere else. Both are spellings no pty can use, so they
        // reach a column from a terminal pane too
        // (`Kawoosh::pane_chord`) — where a plain `<C-3>` is the
        // shell's, which is why it is not bound (2026-09-22).
        ("<D-1>", "pane goto 1"),
        ("<D-2>", "pane goto 2"),
        ("<D-3>", "pane goto 3"),
        ("<D-4>", "pane goto 4"),
        ("<D-5>", "pane goto 5"),
        ("<D-6>", "pane goto 6"),
        ("<D-7>", "pane goto 7"),
        ("<D-8>", "pane goto 8"),
        ("<D-9>", "pane goto 9"),
        ("<C-S-1>", "pane goto 1"),
        ("<C-S-2>", "pane goto 2"),
        ("<C-S-3>", "pane goto 3"),
        ("<C-S-4>", "pane goto 4"),
        ("<C-S-5>", "pane goto 5"),
        ("<C-S-6>", "pane goto 6"),
        ("<C-S-7>", "pane goto 7"),
        ("<C-S-8>", "pane goto 8"),
        ("<C-S-9>", "pane goto 9"),
        // `z`: vim's scrolling, read on the ribbon — the focused
        // column to an edge, or the middle.
        ("zs", "strip left"),
        ("ze", "strip right"),
        ("zz", "strip center"),
        // A pane's key legend whole, or one `⌥/ keys` again: `?`
        // without its shift (docs/design/icons.md Decision 6).
        ("<A-/>", "legend"),
    ];
    for (k, c) in p {
        km.bind(Pane, k, c);
    }
    // A multibuffer's excerpt grown (docs/design/search.md Decision 13):
    // the lines it leaves out read as a closed fold, so vim's fold keys —
    // `zo` open, `zk` `zj` up and down — and Zed's `<S-CR>`.
    for (k, c) in [
        ("zo", "multi more"),
        ("<S-CR>", "multi more"),
        ("zk", "multi more above"),
        ("zj", "multi more below"),
    ] {
        km.bind_local("language:multibuffer", Normal, k, c, &[]);
    }
    // The memory pane's own and the undo pane's, local to each: no
    // other pane finds them (local-maps.md).
    for (k, c) in [
        ("/", "memory filter"),
        ("p", "list open"),
        ("y", "memory recall"),
        ("o", "memory origin"),
        ("x", "memory forget"),
        ("m", "memory pin"),
        // This workspace's rows or every one's, as `<C-a>` flips the
        // buffers picker's.
        ("<C-a>", "memory scope"),
    ] {
        km.bind_local("memory", Pane, k, c, &[]);
    }
    for (k, c) in [
        ("u", "undo pane undo"),
        ("<C-r>", "undo pane redo"),
        ("g-", "undo pane older"),
        ("g+", "undo pane newer"),
    ] {
        km.bind_local("undo", Pane, k, c, &[]);
    }
    let i = [
        ("<Esc>", "normal"),
        ("<C-c>", "normal"),
        ("<CR>", "insert newline"),
        ("<Tab>", "insert tab"),
        ("<BS>", "delete char back"),
        ("<C-h>", "delete char back"),
        ("<Del>", "delete forward"),
        ("<C-w>", "delete word back"),
        ("<Left>", "move left"),
        ("<Right>", "move right"),
        ("<Up>", "move up"),
        ("<Down>", "move down"),
        ("<Home>", "line start"),
        ("<End>", "line end insert"),
        ("<D-v>", "paste clipboard"),
        ("<C-S-v>", "paste clipboard"),
        ("<C-Space>", "lsp complete"),
        ("<C-x>", "lsp candidates"),
        ("<C-u>", "delete to start"),
        ("<C-S-u>", "delete line"),
        ("<C-s>", "write"),
        ("<D-s>", "write"),
        // The line under the caret moves and shifts from insert mode
        // too, per selection.
        ("<A-j>", "move line down"),
        ("<A-k>", "move line up"),
        ("<A-h>", "nudge left"),
        ("<A-l>", "nudge right"),
        ("<A-S-h>", "pane narrower"),
        ("<A-S-l>", "pane wider"),
        ("<A-S-j>", "pane shorter"),
        ("<A-S-k>", "pane taller"),
        // The pane moves from insert mode too: the shifted spelling,
        // since `<C-h>` is a backspace here.
        ("<C-S-h>", "pane left"),
        ("<C-S-j>", "pane down"),
        ("<C-S-k>", "pane up"),
        ("<C-S-l>", "pane right"),
    ];
    for (k, c) in i {
        km.bind(Insert, k, c);
    }
    // The text keys every other line on a Mac answers — a buffer's,
    // the prompt's, a field's, all insert mode — erases as `<C-w>` and
    // `<C-u>` are: by a word on ⌥, the word keys on Ctrl where there is
    // no ⌘, as Windows and Linux spell them; to the line's ends on ⌘,
    // where elsewhere Home and End are the moves and no key the deletes.
    let word = if cfg!(target_os = "macos") { "A" } else { "C" };
    for (k, c) in [
        ("BS", "delete word back"),
        ("Del", "delete word forward"),
        ("Left", "word prev"),
        ("Right", "word end insert"),
    ] {
        km.bind(Insert, &format!("<{word}-{k}>"), c);
    }
    for (k, c) in [
        ("<D-BS>", "delete to start"),
        ("<D-Del>", "delete to end"),
        ("<D-Left>", "line start"),
        ("<D-Right>", "line end insert"),
    ] {
        km.bind(Insert, k, c);
    }

    // The font's size, from every mode and every pane (a ⌘ chord
    // reaches the keymap from a terminal too, `Kawoosh::pane_chord`):
    // ⌘= and ⌘+ bigger, ⌘- and ⌘_ smaller, ⌘0 back to the settings' —
    // Ctrl where there is no ⌘, as every editor there spells it.
    let m = if cfg!(target_os = "macos") { "D" } else { "C" };
    for (k, c) in [
        ("=", "font bigger"),
        ("+", "font bigger"),
        ("-", "font smaller"),
        ("_", "font smaller"),
        ("0", "font reset"),
    ] {
        for mode in [Normal, Visual, Insert, Pane] {
            km.bind(mode, &format!("<{m}-{k}>"), c);
        }
    }
    // The next and the previous tab, from every mode and every pane, as
    // a browser has them. A pty cannot tell `<C-Tab>` from `<Tab>`
    // without an extended key protocol, which the terminal does not
    // speak, so a terminal pane lets it through too
    // (`Kawoosh::pane_chord`).
    for (k, c) in [("<C-Tab>", "tab next"), ("<C-S-Tab>", "tab prev")] {
        for mode in [Normal, Visual, Insert, Pane] {
            km.bind(mode, k, c);
        }
    }
    // `m` marks. A listing's sort keys under it, `ma` `ms` `mm` `me`
    // (dir.lua), are local to it and shadow it there.
    km.bind(Normal, "m", "mark");
    // The prompt's keys, local to it and so before the editor's on the
    // same keys: `<CR>` submits (in either mode), `<Esc>` in normal
    // mode cancels — so `<Esc><Esc>` leaves from insert mode — `<BS>`
    // on an empty line cancels, `<Up>`/`<Down>` and `<C-p>`/`<C-n>`
    // walk the history (the shell binds the latter two to the
    // completion at the command line, whose field is a place inside
    // the prompt's).
    for (mode, k, c) in [
        (Insert, "<CR>", "prompt submit"),
        (Normal, "<CR>", "prompt submit"),
        (Normal, "<Esc>", "prompt cancel"),
        (Normal, "<C-c>", "prompt cancel"),
        (Insert, "<BS>", "prompt backspace"),
        (Insert, "<Up>", "prompt history prev"),
        (Insert, "<Down>", "prompt history next"),
        (Insert, "<C-p>", "prompt history prev"),
        (Insert, "<C-n>", "prompt history next"),
    ] {
        km.bind_local("prompt", mode, k, c, &[]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SyntaxObject;

    /// `align on`'s arguments: a `*` or a count before a pattern, or
    /// before the character; alone with no character, a pattern.
    #[test]
    fn align_which_reads_the_modifier() {
        assert_eq!(align_which("=", false), (Which::First, "=".into()));
        assert_eq!(align_which("* =", false), (Which::Every, "=".into()));
        assert_eq!(
            align_which("2 \\bor_\\w+", false),
            (Which::Nth(2), "\\bor_\\w+".into())
        );
        assert_eq!(align_which("2", false), (Which::First, "2".into()));
        assert_eq!(align_which("*", false), (Which::First, "*".into()));
        assert_eq!(align_which("*", true), (Which::Every, String::new()));
        assert_eq!(align_which("12", true), (Which::Nth(12), String::new()));
        assert_eq!(align_which("", true), (Which::First, String::new()));
        assert_eq!(align_which("0", true), (Which::Nth(1), String::new()));
    }

    fn obj(around: Option<Range<usize>>, inside: Option<Range<usize>>) -> SyntaxObject {
        SyntaxObject { around, inside }
    }

    /// The innermost over the caret, the Nth out for a count, the part
    /// asked for; an inside on a signature is still its function's; a
    /// selection grows past itself; with none over the caret, the next
    /// on its line (docs/design/nodes.md Decision 9).
    #[test]
    fn which_object_a_key_takes() {
        // fn outer { … fn inner { body } … }
        let outer = obj(Some(0..100), Some(10..90));
        let inner = obj(Some(20..60), Some(30..50));
        let found = [outer, inner];
        assert_eq!(
            pick_object(&found, 35..36, true, false, 1, 200),
            Some(20..60)
        );
        assert_eq!(
            pick_object(&found, 35..36, false, false, 1, 200),
            Some(30..50)
        );
        assert_eq!(
            pick_object(&found, 35..36, true, false, 2, 200),
            Some(0..100)
        );
        assert_eq!(pick_object(&found, 35..36, true, false, 3, 200), None);
        // On the inner's signature, outside its body: still its body.
        assert_eq!(
            pick_object(&found, 22..23, false, false, 1, 200),
            Some(30..50)
        );
        // A selection that is the inner already: the outer.
        assert_eq!(
            pick_object(&found, 20..60, true, true, 1, 200),
            Some(0..100)
        );
        assert_eq!(
            pick_object(&found, 30..50, false, true, 1, 200),
            Some(10..90)
        );
        assert_eq!(pick_object(&found, 0..100, true, true, 1, 200), None);
        // An inside its pattern took alone, nvim's way: keyed by the
        // around round it.
        let split = [obj(Some(20..60), None), obj(None, Some(30..50))];
        assert_eq!(
            pick_object(&split, 22..23, false, false, 1, 200),
            Some(30..50)
        );
        // Over nothing: the first after the caret.
        let args = [obj(Some(5..7), Some(5..6)), obj(Some(8..9), Some(8..9))];
        assert_eq!(pick_object(&args, 4..5, false, false, 1, 200), Some(5..6));
        assert_eq!(pick_object(&args, 9..10, false, false, 1, 200), None);
        // The lookahead is held to the line: what starts past `line_end`
        // is not taken, though tree-sitter finished its match.
        assert_eq!(pick_object(&args, 4..5, false, false, 1, 5), None);
        assert_eq!(pick_object(&args, 4..5, false, false, 1, 6), Some(5..6));
    }

    /// A pattern that captures each child of a node as its inside — a
    /// pair's key and value, a field's name and type — finds one object
    /// several times, its around the same: the part under the caret is
    /// the one taken, and a count steps out to the object around, not to
    /// a sibling (docs/design/nodes.md Decision 9).
    #[test]
    fn the_part_under_the_caret() {
        // As json's `(pair (_) @entry.inside) @entry.around` finds them.
        let src = r#"{"o": {"k": "long value"}}"#;
        let at = |s: &str| {
            let i = src.find(s).unwrap();
            i..i + s.len()
        };
        let (outer, inner) = (
            at(r#""o": {"k": "long value"}"#),
            at(r#""k": "long value""#),
        );
        let (value, key) = (at(r#""long value""#), at(r#""k""#));
        let found = [
            obj(Some(outer.clone()), Some(at(r#""o""#))),
            // The `{ }` value, between its brackets.
            obj(Some(outer.clone()), Some(inner.clone())),
            obj(Some(inner.clone()), Some(key.clone())),
            obj(Some(inner.clone()), Some(value.clone())),
        ];
        let on = |s: &str| {
            let i = src.find(s).unwrap();
            i..i + 1
        };
        // On the value: the value, whichever order they came in.
        let pick = |at, around, grow, count| pick_object(&found, at, around, grow, count, 99);
        assert_eq!(pick(on("value"), false, false, 1), Some(value.clone()));
        let mut back = found.clone();
        back.reverse();
        assert_eq!(
            pick_object(&back, on("value"), false, false, 1, 99),
            Some(value.clone())
        );
        // On the key: the key.
        assert_eq!(pick(on("k"), false, false, 1), Some(key.clone()));
        // `2ie`: the enclosing entry's part over the caret, not the key.
        assert_eq!(pick(on("value"), false, false, 2), Some(inner.clone()));
        assert_eq!(pick(on("value"), false, false, 3), None);
        // On the `:` between, over neither: the nearer.
        assert_eq!(pick(on(r#": ""#), false, false, 1), Some(key.clone()));
        // `ae` is the pair whatever the part.
        assert_eq!(pick(on("value"), true, false, 1), Some(inner.clone()));
        assert_eq!(pick(on("value"), true, false, 2), Some(outer));
        // A selection that is the value grows to the outer entry's part.
        assert_eq!(pick(value, false, true, 1), Some(inner));
    }

    /// `daa` leaves the list well-formed: the `,` after an item and the
    /// blanks after that, or the last's `,` before it.
    #[test]
    fn an_item_goes_with_its_comma() {
        let buf = Buffer::new("t", "f(a, b,  c)\n");
        let at = |s: &str| {
            let i = "f(a, b,  c)".find(s).unwrap();
            i..i + s.len()
        };
        let cut = |r: Range<usize>| buf.slice(r);
        assert_eq!(cut(with_separator(&buf, at("a"))), "a, ");
        assert_eq!(cut(with_separator(&buf, at("a,"))), "a, ");
        assert_eq!(cut(with_separator(&buf, at("b"))), "b,  ");
        assert_eq!(cut(with_separator(&buf, at("c"))), ",  c");
        let alone = Buffer::new("t", "[x]\n");
        assert_eq!(with_separator(&alone, 1..2), 1..2);
        // Across lines, the last item takes the `,` that ended the line
        // before it; one with its comma after on its last line, that.
        let src = "[\n  {\n    \"a\": 1\n  },\n  2\n]\n";
        let lines = Buffer::new("t", src);
        let first = src.find('{').unwrap()..src.find('}').unwrap() + 1;
        assert_eq!(
            cut_of(&lines, with_separator(&lines, first)),
            "{\n    \"a\": 1\n  },"
        );
        let two = src.find('2').unwrap();
        assert_eq!(
            cut_of(&lines, with_separator(&lines, two..two + 1)),
            ",\n  2"
        );
        let one = Buffer::new("t", "[\n  1\n]\n");
        assert_eq!(with_separator(&one, 4..5), 4..5);
    }

    fn cut_of(buf: &Buffer, r: Range<usize>) -> String {
        buf.slice(r)
    }

    #[test]
    fn an_object_with_its_lines_to_itself() {
        let src = "fn a() {\n    x();\n}\nlet f = |v| v;\n";
        let buf = Buffer::new("t", src);
        let at = |s: &str| {
            let i = src.find(s).unwrap();
            i..i + s.len()
        };
        assert!(has_its_lines(&buf, &at("fn a() {\n    x();\n}")));
        assert!(has_its_lines(&buf, &at("x();")));
        assert!(!has_its_lines(&buf, &at("|v| v")));
        assert!(!has_its_lines(&buf, &(3..3)));
    }
}
