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
    ArgKind, Args, Cond, Ctx, Editor, Effect, Kind, Layer, MotionKind, Prompt, Selection, Setting,
    Spec, ViewId,
};

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
    // Take the newline too, unless it is the last line, then the one
    // before it.
    if end < buf.len() {
        start..end + 1
    } else if start > 0 && a > 0 {
        start - 1..end
    } else {
        start..end
    }
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
            // Inclusive of the char under the head, never of a newline.
            let end = if buf.char_at(r.end) == Some('\n') || r.end >= buf.len() {
                r.end
            } else {
                buf.next_char(r.end)
            };
            (r.start..end, false)
        }
        MotionKind::Linewise => (line_range_of_sel(buf, s, count.saturating_sub(1)), true),
    }
}

fn set_register(ed: &mut Editor, texts: &[String], linewise: bool) {
    let joined = if texts.len() == 1 {
        texts[0].clone()
    } else {
        texts.join("\n")
    };
    ed.registers.insert('"', joined.clone());
    ed.register_linewise = linewise;
    ed.effects.push(Effect::SetClipboard(joined));
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
            set_register(ed, &texts, linewise);
            let starts: Vec<usize> = ranges.iter().map(|(r, _)| r.start).collect();
            let v = &mut ed.views[view];
            let mut i = 0;
            v.sels.map(|_| {
                let s = Selection::point(starts[i.min(starts.len() - 1)]);
                i += 1;
                s
            });
            ed.message = format!(
                "yanked {} line(s)",
                texts
                    .iter()
                    .map(|t| t.matches('\n').count().max(1))
                    .sum::<usize>()
            );
        }
        "delete" | "change" => {
            set_register(ed, &texts, linewise);
            let edits = ranges
                .iter()
                .enumerate()
                .map(|(i, (r, lw))| {
                    // `cc` keeps the line's indent and its newline.
                    if op == "change" && *lw {
                        let buf = &ed.buffers[id];
                        let ln = buf.line_of(r.start);
                        let indent = m::indent_of(buf, ln);
                        let inner = buf.line_range(ln).start
                            ..buf
                                .line_range(buf.line_of(r.end.saturating_sub(1).max(r.start)))
                                .end;
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
        "indent" | "dedent" => {
            let ts = ed.tabstop();
            let unit = if ed.expandtab() {
                " ".repeat(ts)
            } else {
                "\t".into()
            };
            let buf = &ed.buffers[id];
            let mut edits = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for (i, (r, _)) in ranges.iter().enumerate() {
                let a = buf.line_of(r.start);
                let b = buf.line_of(r.end.saturating_sub(1).max(r.start));
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
            let heads: Vec<usize> = ranges.iter().map(|(r, _)| r.start).collect();
            ed.edit_each(view, edits, |start, _| Selection::point(start));
            let buf = &ed.buffers[id];
            let v = &mut ed.views[view];
            let mut i = 0;
            v.sels.map(|_| {
                let h = heads[i.min(heads.len() - 1)];
                i += 1;
                Selection::point(m::first_nonblank(buf, buf.line_of(h.min(buf.len()))))
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
                    op_range(buf, s, MotionKind::Inclusive, 1)
                }
            })
            .collect();
        ed.set_mode(ctx.view, Mode::Normal);
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
    let id = view(ed, ctx).buffer;
    let buf = &ed.buffers[id];
    let v = &mut ed.views[ctx.view];
    let mut ok = true;
    v.sels.map(|s| {
        let range = match c {
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
            '(' | ')' | 'b' => bracket_object(buf, s.head, '(', ')', around),
            '[' | ']' => bracket_object(buf, s.head, '[', ']', around),
            '{' | '}' | 'B' => bracket_object(buf, s.head, '{', '}', around),
            '<' | '>' => bracket_object(buf, s.head, '<', '>', around),
            '"' | '\'' | '`' => quote_object(buf, s.head, c, around),
            _ => None,
        };
        match range {
            Some(r) => Selection::new(r.start, r.end),
            None => {
                ok = false;
                s
            }
        }
    });
    if !ok {
        ed.message = format!("no text object for {c}");
    }
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
    let end = m::matching_bracket(buf, start)?;
    if around {
        Some(start..buf.next_char(end))
    } else {
        Some(buf.next_char(start)..end)
    }
}

fn quote_object(buf: &Buffer, o: usize, q: char, around: bool) -> Option<Range<usize>> {
    let ln = buf.line_of(o);
    let range = buf.line_range(ln);
    let text = buf.slice(range.clone());
    let rel = o - range.start;
    let positions: Vec<usize> = text.match_indices(q).map(|(i, _)| i).collect();
    let mut pairs = positions.chunks(2).filter(|c| c.len() == 2);
    let (a, b) = pairs
        .find(|c| c[0] <= rel && rel <= c[1])
        .map(|c| (c[0], c[1]))?;
    let (s, e) = if around { (a, b + 1) } else { (a + 1, b) };
    Some(range.start + s..range.start + e)
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
    if let Err(e) = ed.set_search(&pattern) {
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
    if edits.is_empty() {
        ed.message = format!("no match: {pat}");
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
        "{count} substitution(s) on {lines} line(s){}",
        if took.as_millis() >= 100 {
            format!(" in {:.1}s", took.as_secs_f64())
        } else {
            String::new()
        }
    );
}

/// Writes `buf` to `path` through a file beside it, renamed over the
/// target once whole: the text streams out piece by piece (a big file is
/// never one string), a crash mid-write leaves the old file, and a
/// buffer whose text is the file's own mapping keeps reading the old
/// inode rather than the bytes being written over it.
fn save_beside(buf: &kawoosh_doc::Buffer, path: &std::path::Path) -> std::io::Result<()> {
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

fn write(ed: &mut Editor, ctx: &Ctx) -> bool {
    let id = view(ed, ctx).buffer;
    if let Some(p) = ctx.args.first() {
        ed.buffers[id].path = Some(std::path::PathBuf::from(p));
        ed.buffers[id].name = std::path::Path::new(p)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.clone());
    }
    let buf = &ed.buffers[id];
    if buf.path.is_none() && buf.hook.is_some() {
        ed.buffers[id].mark_saved();
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
    match save_beside(buf, &path) {
        Ok(()) => {
            let b = &mut ed.buffers[id];
            b.mark_saved();
            b.disk_len = Some(b.len());
            ed.message = format!(
                "\"{}\" {}L, {}B written",
                path.display(),
                b.line_count(),
                b.len()
            );
            ed.effects.push(Effect::Wrote(id));
            true
        }
        Err(e) => {
            ed.message = format!("write failed: {e}");
            false
        }
    }
}

/// `:e!`: the file's text as it is on disk, put in as one journaled
/// edit — undoable — and the buffer clean on it. A file that is gone
/// is said so, the buffer left as it is.
fn reload(ed: &mut Editor, ctx: &Ctx) {
    let id = view(ed, ctx).buffer;
    let buf = &ed.buffers[id];
    let Some(path) = buf.path.clone() else {
        ed.message = "no file to reload from".into();
        return;
    };
    if buf.loading.is_some() {
        ed.message = "still opening".into();
        return;
    }
    let disk = match kawoosh_doc::Buffer::from_file(&path) {
        Ok(b) => b,
        Err(e) => {
            ed.message = format!("cannot read {}: {e}", path.display());
            return;
        }
    };
    let len = disk.len();
    let b = &mut ed.buffers[id];
    b.restore(disk.text_root());
    b.mark_saved();
    b.disk_len = Some(len);
    ed.message = format!(
        "\"{}\" {}L, {}B loaded from disk (u brings the changes back)",
        path.display(),
        ed.buffers[id].line_count(),
        len
    );
    let v = &mut ed.views[ctx.view];
    v.sels
        .map(|s| Selection::new(s.anchor.min(len), s.head.min(len)));
}

fn paste(ed: &mut Editor, ctx: &Ctx, after: bool) {
    let Some(text) = ed.registers.get(&'"').cloned() else {
        ed.message = "nothing to paste".into();
        return;
    };
    let text = text.repeat(ctx.count.max(1));
    let linewise = ed.register_linewise;
    let id = view(ed, ctx).buffer;
    let sels = ed.views[ctx.view].sels.items.clone();
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
                if after {
                    let end = buf.line_range(ln).end;
                    if end >= buf.len() {
                        (i, end..end, format!("\n{}", t.trim_end_matches('\n')))
                    } else {
                        (i, end + 1..end + 1, t)
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
    ed.edit_each(ctx.view, edits, move |start, len| {
        if linewise {
            Selection::point(start)
        } else {
            Selection::point(start + len.saturating_sub(1))
        }
    });
    if linewise {
        let buf = &ed.buffers[id];
        let v = &mut ed.views[ctx.view];
        v.sels.map(|s| {
            let ln = buf.line_of(s.head);
            let ln = if after && s.head > 0 && buf.byte_at(s.head) == Some(b'\n') {
                ln + 1
            } else {
                ln
            };
            Selection::point(m::first_nonblank(buf, ln.min(buf.line_count() - 1)))
        });
    }
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
        // last word never joins lines) — vim's one special case.
        let op = ed.pending_op.is_some();
        motion(ed, ctx, |b, o, n| {
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
    ed.register_kind("match_bracket", Kind::Motion(Inclusive), |ed, ctx| {
        motion(ed, ctx, |b, o, _| m::matching_bracket(b, o).unwrap_or(o));
    });
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
    ed.register_kind("search next", Kind::Motion(Exclusive), |ed, ctx| {
        search(ed, ctx, true)
    });
    ed.register_kind("search prev", Kind::Motion(Exclusive), |ed, ctx| {
        search(ed, ctx, false)
    });
    ed.register_with_args("substitute", Args::rest(&[ArgKind::Text]), substitute);
    ed.register("search word", |ed, ctx| {
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let (a, b) = m::word_at(buf, ed.views[ctx.view].sels.primary().head);
        if a == b {
            return;
        }
        let word = buf.slice(a..b);
        if let Err(e) = ed.set_search(&format!(r"\b{}\b", regex::escape(&word))) {
            ed.message = e;
            return;
        }
        search(ed, ctx, true);
    });

    // ---- text objects (operator-pending and visual)
    ed.register_kind_char("textobject inner", Kind::TextObject, |ed, ctx| {
        textobject(ed, ctx, false)
    });
    ed.register_kind_char("textobject around", Kind::TextObject, |ed, ctx| {
        textobject(ed, ctx, true)
    });

    // ---- operators
    for op in ["delete", "change", "yank", "indent", "dedent"] {
        ed.register_kind(op, Kind::Operator, move |ed, ctx| operator(ed, ctx, op));
    }
    ed.register("join", |ed, ctx| {
        let n = ctx.count.max(2) - 1;
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let ranges: Vec<(Range<usize>, bool)> = if ed.mode(ctx.view) == Mode::Visual {
            ed.views[ctx.view]
                .sels
                .iter()
                .map(|s| (line_range_of_sel(buf, s, 0), true))
                .collect()
        } else {
            ed.views[ctx.view]
                .sels
                .iter()
                .map(|s| (line_range_of_sel(buf, s, n), true))
                .collect()
        };
        if ed.mode(ctx.view) == Mode::Visual {
            ed.set_mode(ctx.view, Mode::Normal);
        }
        apply_operator(ed, ctx.view, "join", ranges);
    });
    ed.register("delete char", |ed, ctx| {
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let ranges: Vec<(Range<usize>, bool)> = ed.views[ctx.view]
            .sels
            .iter()
            .map(|s| {
                if ed.mode(ctx.view) == Mode::Visual && !s.is_empty() {
                    return op_range(buf, s, MotionKind::Inclusive, 1);
                }
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
        if ed.mode(ctx.view) == Mode::Visual {
            ed.set_mode(ctx.view, Mode::Normal);
        }
        apply_operator(ed, ctx.view, "delete", ranges);
    });
    ed.register("delete char back", |ed, ctx| {
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let ranges: Vec<(Range<usize>, bool)> = ed.views[ctx.view]
            .sels
            .iter()
            .map(|s| {
                let ls = buf.line_start(buf.line_of(s.head));
                let mut a = s.head;
                for _ in 0..ctx.count.max(1) {
                    if a > ls {
                        a = buf.prev_char(a);
                    }
                }
                (a..s.head, false)
            })
            .collect();
        apply_operator(ed, ctx.view, "delete", ranges);
    });
    ed.register("change char", |ed, ctx| {
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let ranges: Vec<(Range<usize>, bool)> = ed.views[ctx.view]
            .sels
            .iter()
            .map(|s| {
                if ed.mode(ctx.view) == Mode::Visual && !s.is_empty() {
                    return op_range(buf, s, MotionKind::Inclusive, 1);
                }
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
        ed.set_mode(ctx.view, Mode::Normal);
        apply_operator(ed, ctx.view, "change", ranges);
    });
    ed.register("delete to end", |ed, ctx| {
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let ranges = ed.views[ctx.view]
            .sels
            .iter()
            .map(|s| (s.head..buf.line_range(buf.line_of(s.head)).end, false))
            .collect();
        apply_operator(ed, ctx.view, "delete", ranges);
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
    ed.register_with_char("replace char", |ed, ctx| {
        let Some(c) = ctx.arg_char else { return };
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let edits = ed.views[ctx.view]
            .sels
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let e = buf.next_char(s.head);
                (e > s.head && buf.char_at(s.head) != Some('\n'))
                    .then(|| (i, s.head..e, c.to_string()))
            })
            .collect();
        ed.edit_each(ctx.view, edits, |start, _| Selection::point(start));
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
        ed.set_mode(ctx.view, Mode::Normal);
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
            ed.last_insert.clear();
        } else {
            v.sels.map(Selection::collapse);
        }
    });
    ed.register("insert newline", |ed, ctx| {
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let ln = buf.line_of(ed.views[ctx.view].sels.primary().head);
        let indent = m::indent_of(buf, ln);
        ed.insert_text(ctx.view, &format!("\n{indent}"));
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
        apply_operator(ed, ctx.view, "delete", ranges);
    });
    ed.register("delete forward", |ed, ctx| {
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let ranges = ed.views[ctx.view]
            .sels
            .iter()
            .map(|s| (s.head..buf.next_char(s.head), false))
            .collect();
        apply_operator(ed, ctx.view, "delete", ranges);
    });
    ed.register("paste after", |ed, ctx| paste(ed, ctx, true));
    ed.register("paste before", |ed, ctx| paste(ed, ctx, false));
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
    ed.register("select all", |ed, ctx| {
        let len = ed.buffer_of(ctx.view).len();
        ed.views[ctx.view].sels = crate::Selections::single(Selection::new(0, len));
        ed.set_mode(ctx.view, Mode::Visual);
    });
    ed.register("cursor below", |ed, ctx| add_cursor(ed, ctx, 1));
    ed.register("cursor above", |ed, ctx| add_cursor(ed, ctx, -1));

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
            .bang("nothing yet: taken for the fingers that type :w!")
            .doc("write the buffer to its file, or to PATH"),
        |ed, ctx| {
            write(ed, ctx);
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
            .bang("nothing yet: taken for the fingers that type :wq!")
            .doc("write, then quit"),
        |ed, ctx| {
            if write(ed, ctx) {
                ed.effects.push(Effect::Quit { force: false });
            }
        },
    );
    ed.register_spec(
        Spec::new("write quit all")
            .alias(&["wqa", "xa"])
            .doc("write every file, then quit"),
        |ed, _| {
            let ids: Vec<_> = ed
                .buffers
                .iter()
                .filter(|(_, b)| b.modified && b.path.is_some())
                .map(|(id, _)| id)
                .collect();
            for id in ids {
                if let Some(p) = ed.buffers[id].path.clone()
                    && save_beside(&ed.buffers[id], &p).is_ok()
                {
                    ed.buffers[id].mark_saved();
                }
            }
            ed.effects.push(Effect::QuitAll { force: false });
        },
    );
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
    // `:set PATH=VALUE`, `:set FLAG`, `:set noFLAG` set into the session
    // layer, shaped like the value already there; `:set PATH?` says what
    // it is and where it came from; `:set PATH!` takes the session's
    // value back out.
    ed.register_spec(
        Spec::new("set")
            .alias(&["se"])
            .args(Args::rest(&[ArgKind::Option]))
            .doc("set an option for the session (PATH=VALUE, FLAG, noFLAG, PATH?, PATH!)"),
        |ed, ctx| {
            // One setting per line: what follows a `=` is the value, spaces
            // and all (`:set compile.command=cargo test`).
            let a = ctx.args.join(" ");
            if a.is_empty() {
                ed.message = "set what? (:set PATH=VALUE, :set PATH?)".into();
                return;
            }
            let a = a.as_str();
            if let Some(path) = a.strip_suffix('?') {
                ed.message = match ed.settings.get(path) {
                    Some(v) => match ed.settings.origin(path) {
                        Some(from) => format!("{path} = {v}  ({from})"),
                        None => format!("{path} = {v}"),
                    },
                    None => format!("{path} is not set"),
                };
                return;
            }
            if let Some(path) = a.strip_suffix('!') {
                ed.settings.unset(Layer::Session, path);
                return;
            }
            let (path, value) = match a.split_once('=') {
                Some((k, v)) => (k.to_string(), Setting::parse_like(v, ed.settings.get(k))),
                None if a.starts_with("no") => (a[2..].to_string(), Setting::Bool(false)),
                None => (a.to_string(), Setting::Bool(true)),
            };
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
    // plugin's `kawoosh.map` makes one.
    ed.register_spec(
        Spec::new("map")
            .args(Args::rest(&[
                ArgKind::Text,
                ArgKind::Text,
                ArgKind::Command,
            ]))
            .doc("bind KEYS in MODE to COMMAND"),
        |ed, ctx| match ctx.args.as_slice() {
            [mode, keys, cmd @ ..] if !cmd.is_empty() => match Mode::from_short(mode) {
                Some(m) => ed.keymap.bind(m, keys, &cmd.join(" ")),
                None => ed.message = format!("map: unknown mode {mode}"),
            },
            _ => ed.message = "map what? (:map MODE KEYS COMMAND)".into(),
        },
    );

    for (name, doc) in DOCS {
        match ed.commands.spec_mut(name) {
            Some(spec) => spec.doc = doc.to_string(),
            None => debug_assert!(false, "DOCS names no command: {name}"),
        }
    }
}

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
    ("goto file start", "the first line, or line COUNT"),
    ("goto file end", "the last line, or line COUNT"),
    ("goto line", "line COUNT (`:42` too)"),
    ("page half down", "half a screen down"),
    ("page half up", "half a screen up"),
    ("page down", "a screen down"),
    ("page up", "a screen up"),
    (
        "match_bracket",
        "the bracket matching the one under the caret",
    ),
    ("find char", "onto the next CHAR in the line"),
    ("find char back", "onto the previous CHAR in the line"),
    ("till char", "before the next CHAR in the line"),
    ("till char back", "after the previous CHAR in the line"),
    ("search next", "the next match of the search, wrapping"),
    ("search prev", "the previous match of the search, wrapping"),
    (
        "search word",
        "search for the word under the caret, forward",
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
        "select inside a pair or word: iw, i(, i\", ...",
    ),
    (
        "textobject around",
        "select a pair or word with what surrounds it: aw, a(, ...",
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
        "join",
        "join COUNT lines (the selection's, in visual) with a space between",
    ),
    ("delete char", "delete the character under the caret (`x`)"),
    (
        "delete char back",
        "delete the character before the caret (`X`, insert's Backspace)",
    ),
    ("delete to end", "delete to the end of the line (`D`)"),
    (
        "delete forward",
        "delete the character after the caret (insert's Delete)",
    ),
    (
        "delete word back",
        "delete the word before the caret (insert's <C-w>)",
    ),
    (
        "delete to start",
        "delete to the start of the line (insert's <C-u>)",
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
        "replace the character under the caret with CHAR (`r`)",
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
    ("insert tab", "insert a tab (spaces under `expandtab`)"),
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
    // undo
    ("undo", "back to the state before"),
    ("redo", "forward again, along the branch last taken"),
    (
        "undo older",
        "the state made before this one, on any branch (`g-`)",
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
    ("cursor below", "add a caret on the line below (<A-j>)"),
    ("cursor above", "add a caret on the line above (<A-k>)"),
    ("select all", "select the whole buffer"),
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

fn open_line(ed: &mut Editor, ctx: &Ctx, below: bool) {
    let id = view(ed, ctx).buffer;
    let buf = &ed.buffers[id];
    let edits = ed.views[ctx.view]
        .sels
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let ln = buf.line_of(s.head);
            let indent = m::indent_of(buf, ln);
            if below {
                let end = buf.line_range(ln).end;
                (i, end..end, format!("\n{indent}"))
            } else {
                let start = buf.line_start(ln);
                (i, start..start, format!("{indent}\n"))
            }
        })
        .collect();
    ed.edit_each(ctx.view, edits, move |start, len| {
        Selection::point(if below { start + len } else { start + len - 1 })
    });
    ed.set_mode(ctx.view, Mode::Insert);
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
        ("w", "word next"),
        ("b", "word prev"),
        ("e", "word end"),
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
        ("J", "join"),
        ("x", "delete char"),
        ("<Del>", "delete char"),
        ("X", "delete char back"),
        ("D", "delete to end"),
        ("C", "change to end"),
        ("s", "change char"),
        ("r", "replace char"),
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
        ("v", "visual"),
        ("V", "visual line"),
        (",", "cursor primary"),
        ("<A-j>", "cursor below"),
        ("<A-k>", "cursor above"),
        ("<Esc>", "normal"),
        ("<C-c>", "normal"),
        // Panes, tabs, the dock: the shell's commands (Effect::Shell).
        ("<C-w>v", "vsplit"),
        ("<C-w>s", "split"),
        ("<C-w>q", "close"),
        ("<C-w>c", "close"),
        ("<C-w>o", "only"),
        ("<C-w>w", "pane next"),
        ("<C-w>x", "pane swap"),
        ("<C-w>h", "pane left"),
        ("<C-w>j", "pane down"),
        ("<C-w>k", "pane up"),
        ("<C-w>l", "pane right"),
        ("<C-w><Left>", "pane left"),
        ("<C-w><Down>", "pane down"),
        ("<C-w><Up>", "pane up"),
        ("<C-w><Right>", "pane right"),
        ("<C-w>t", "tab new"),
        ("gt", "tab next"),
        ("gT", "tab prev"),
        ("<C-w>d", "dock"),
        ("<C-w>n", "toast"),
        ("gd", "lsp definition"),
        ("K", "lsp hover"),
        ("<CR>", "goto location"),
        ("]q", "error next"),
        ("[q", "error prev"),
        ("-", "oil"),
    ];
    for (k, c) in n {
        km.bind(Normal, k, c);
    }
    let v = [
        ("o", "cursor swap"),
        ("x", "delete char"),
        ("i", "textobject inner"),
        ("a", "textobject around"),
        ("<Esc>", "normal"),
        ("<C-c>", "normal"),
    ];
    for (k, c) in v {
        km.bind(Visual, k, c);
    }
    let op = [("i", "textobject inner"), ("a", "textobject around")];
    for (k, c) in op {
        km.bind(OperatorPending, k, c);
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
        ("<C-u>", "delete to start"),
    ];
    for (k, c) in i {
        km.bind(Insert, k, c);
    }
    // The prompt's keys, bound after the editor's on the same keys so
    // they come first and fall through when no prompt is open: `<CR>`
    // submits (in either mode), `<Esc>` in normal mode cancels — so
    // `<Esc><Esc>` leaves from insert mode — `<BS>` on an empty line
    // cancels, `<Up>`/`<Down>` and `<C-p>`/`<C-n>` walk the history
    // (the shell binds the latter two to the completion at the command
    // line, over these).
    let prompt = [Cond::parse("prompt")];
    km.bind_when(Insert, "<CR>", "prompt submit", &prompt);
    km.bind_when(Normal, "<CR>", "prompt submit", &prompt);
    km.bind_when(Normal, "<Esc>", "prompt cancel", &prompt);
    km.bind_when(Normal, "<C-c>", "prompt cancel", &prompt);
    km.bind_when(Insert, "<BS>", "prompt backspace", &prompt);
    km.bind_when(Insert, "<Up>", "prompt history prev", &prompt);
    km.bind_when(Insert, "<Down>", "prompt history next", &prompt);
    km.bind_when(Insert, "<C-p>", "prompt history prev", &prompt);
    km.bind_when(Insert, "<C-n>", "prompt history next", &prompt);
}
