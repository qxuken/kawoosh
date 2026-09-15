//! The built-in commands and the default keymap. Every command is written
//! over the whole selection set; vim's operator + motion notation drives
//! it (mvp.md Decision 4: "neovim-style keymaps drive a selection-set
//! engine fine"): an operator waits, the next motion or text object
//! extends every selection, and the operator applies to all of them.

use std::ops::Range;

use kawoosh_doc::{Buffer, Run};

use crate::keymap::{Keymap, Mode};
use crate::motions as m;
use crate::{Ctx, Editor, Effect, Kind, MotionKind, Prompt, Selection, ViewId};

pub const SEARCH_LAYER: &str = "search";

/// `:w` → `write`, and the other ex spellings.
pub fn ex_alias(name: &str) -> Option<&'static str> {
    Some(match name {
        "w" | "write" => "write",
        "q" | "quit" => "quit",
        "wq" | "x" => "write_quit",
        "qa" | "qall" | "quitall" => "quit_all",
        "wqa" | "xa" => "write_quit_all",
        "e" | "edit" => "edit",
        "set" | "se" => "set",
        "bn" | "bnext" => "buffer_next",
        "bp" | "bprev" | "bprevious" => "buffer_prev",
        "bd" | "bdelete" => "buffer_delete",
        "b" | "buffer" => "buffer",
        "ls" | "buffers" => "buffer_list",
        "sp" | "split" => "split",
        "vs" | "vsplit" => "vsplit",
        "clo" | "close" => "close",
        "on" | "only" => "only",
        "tabnew" | "tabe" => "tab_new",
        "tabn" | "tabnext" => "tab_next",
        "tabp" | "tabprev" => "tab_prev",
        "tabc" | "tabclose" => "tab_close",
        "term" | "terminal" => "terminal",
        "scrollback" => "scrollback",
        "map" => "map",
        "echo" => "echo",
        "lua" => "lua",
        "lsp" => "lsp_status",
        "tool" => "tool",
        "cd" | "chdir" => "cd",
        "pwd" => "pwd",
        "view" => "view",
        "compile" | "make" => "compile",
        "cn" | "cnext" => "error_next",
        "ol" | "oldfiles" | "bro" | "browse" => "oldfiles",
        "mks" | "mksession" => "session_save",
        "cp" | "cprev" | "cprevious" => "error_prev",
        _ => return None,
    })
}

fn view<'a>(ed: &'a Editor, ctx: &Ctx) -> &'a crate::View {
    &ed.views[ctx.view]
}

fn extend(ed: &Editor) -> bool {
    ed.mode == Mode::Visual || ed.pending_op.is_some()
}

/// Moves every head by `f(buf, head, count)`; the anchor follows unless
/// extending.
fn motion(ed: &mut Editor, ctx: &Ctx, f: impl Fn(&Buffer, usize, usize) -> usize) {
    let ext = extend(ed);
    let id = view(ed, ctx).buffer;
    let buf = &ed.buffers[id];
    let v = &mut ed.views[ctx.view];
    v.sels
        .map(|s| s.with_head(f(buf, s.head, ctx.count).min(buf.len()), ext));
    v.goal_col = None;
}

fn vertical(ed: &mut Editor, ctx: &Ctx, dy: i64) {
    let ext = extend(ed);
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
                ed.mode = Mode::Insert;
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
            let unit = if ed.option("expandtab") == Some("true") {
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
    if ed.mode == Mode::Visual {
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let linewise = ed.visual_linewise;
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
        ed.mode = Mode::Normal;
        ed.visual_linewise = false;
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

fn search(ed: &mut Editor, ctx: &Ctx, forward: bool) {
    let Some(pat) = ed.last_search.clone() else {
        ed.message = "no previous search".into();
        return;
    };
    let re = match regex::RegexBuilder::new(&pat).build() {
        Ok(re) => re,
        Err(e) => {
            ed.message = format!("bad pattern: {e}");
            return;
        }
    };
    let id = view(ed, ctx).buffer;
    let text = ed.buffers[id].text();
    let hits: Vec<(usize, usize)> = re.find_iter(&text).map(|m| (m.start(), m.end())).collect();
    ed.buffers[id].set_layer(
        SEARCH_LAYER,
        hits.iter()
            .map(|(a, b)| Run {
                range: *a..*b,
                style: 0,
                tag: 0,
            })
            .collect(),
    );
    if hits.is_empty() {
        ed.message = format!("not found: {pat}");
        return;
    }
    let ext = extend(ed);
    let v = &mut ed.views[ctx.view];
    let mut wrapped = false;
    for _ in 0..ctx.count.max(1) {
        v.sels.map(|s| {
            let target = if forward {
                hits.iter().find(|(a, _)| *a > s.head).or_else(|| {
                    wrapped = true;
                    hits.first()
                })
            } else {
                hits.iter().rev().find(|(a, _)| *a < s.head).or_else(|| {
                    wrapped = true;
                    hits.last()
                })
            };
            match target {
                Some((a, _)) => s.with_head(*a, ext),
                None => s,
            }
        });
    }
    ed.message = if wrapped {
        format!("search wrapped: {pat}")
    } else {
        format!("/{pat}  {} match(es)", hits.len())
    };
}

fn write(ed: &mut Editor, ctx: &Ctx) -> bool {
    let id = view(ed, ctx).buffer;
    if let Some(p) = ctx.args.first().filter(|a| *a != "!") {
        ed.buffers[id].path = Some(std::path::PathBuf::from(p));
        ed.buffers[id].name = std::path::Path::new(p)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.clone());
    }
    let buf = &ed.buffers[id];
    if buf.path.is_none() && buf.hook.is_some() {
        ed.buffers[id].modified = false;
        ed.effects.push(Effect::Write(id));
        return true;
    }
    let Some(path) = buf.path.clone() else {
        ed.message = "no file name (use :w <path>)".into();
        return false;
    };
    let text = buf.text();
    match std::fs::write(&path, &text) {
        Ok(()) => {
            let b = &mut ed.buffers[id];
            b.modified = false;
            b.disk_len = Some(text.len());
            ed.message = format!(
                "\"{}\" {}L, {}B written",
                path.display(),
                b.line_count(),
                text.len()
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
    ed.motion("move_left", Exclusive, |b, o, n| {
        let mut o = o;
        let ls = b.line_start(b.line_of(o));
        for _ in 0..n {
            if o > ls {
                o = b.prev_char(o);
            }
        }
        o
    });
    ed.motion("move_right", Exclusive, |b, o, n| {
        let mut o = o;
        let le = b.line_range(b.line_of(o)).end;
        for _ in 0..n {
            if o < le {
                o = b.next_char(o);
            }
        }
        o
    });
    ed.register_kind("move_down", Kind::Motion(Linewise), |ed, ctx| {
        vertical(ed, ctx, 1)
    });
    ed.register_kind("move_up", Kind::Motion(Linewise), |ed, ctx| {
        vertical(ed, ctx, -1)
    });
    ed.motion("line_start", Exclusive, |b, o, _| {
        b.line_start(b.line_of(o))
    });
    ed.motion("first_nonblank", Exclusive, |b, o, _| {
        m::first_nonblank(b, b.line_of(o))
    });
    ed.motion("line_end", Inclusive, |b, o, n| {
        // Onto the last char, as vim's `$`; an empty line stays put.
        let ln = (b.line_of(o) + n - 1).min(b.line_count() - 1);
        let r = b.line_range(ln);
        if r.end > r.start {
            b.prev_char(r.end)
        } else {
            r.end
        }
    });
    ed.motion("line_end_insert", Exclusive, |b, o, _| {
        b.line_range(b.line_of(o)).end
    });
    ed.register_kind("word_next", Kind::Motion(Exclusive), |ed, ctx| {
        // Under an operator, `w` stops at the end of its line (`dw` on the
        // last word never joins lines) — vim's one special case.
        let op = ed.pending_op.is_some();
        motion(ed, ctx, |b, o, n| {
            let target = (0..n).fold(o, |o, _| m::next_word_start(b, o));
            let le = b.line_range(b.line_of(o)).end;
            if op && target > le { le } else { target }
        });
    });
    ed.motion("word_prev", Exclusive, |b, o, n| {
        (0..n).fold(o, |o, _| m::prev_word_start(b, o))
    });
    ed.motion("word_end", Inclusive, |b, o, n| {
        (0..n).fold(o, |o, _| m::next_word_end(b, o))
    });
    ed.register_kind("goto_file_start", Kind::Motion(Linewise), |ed, ctx| {
        let n = if ctx.has_count { ctx.count } else { 1 };
        goto_line(ed, ctx.view, n);
    });
    ed.register_kind("goto_file_end", Kind::Motion(Linewise), |ed, ctx| {
        let n = if ctx.has_count {
            ctx.count
        } else {
            ed.buffer_of(ctx.view).line_count()
        };
        goto_line(ed, ctx.view, n);
    });
    ed.register_kind("goto_line", Kind::Motion(Linewise), |ed, ctx| {
        goto_line(ed, ctx.view, ctx.count)
    });
    ed.register_kind("half_page_down", Kind::Motion(Linewise), |ed, ctx| {
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
    ed.register_kind("half_page_up", Kind::Motion(Linewise), |ed, ctx| {
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
    ed.register_kind("page_down", Kind::Motion(Linewise), |ed, ctx| {
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
    ed.register_kind("page_up", Kind::Motion(Linewise), |ed, ctx| {
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
        ("find_char", true, false),
        ("find_char_back", false, false),
        ("till_char", true, true),
        ("till_char_back", false, true),
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
    ed.register_kind("search_next", Kind::Motion(Exclusive), |ed, ctx| {
        search(ed, ctx, true)
    });
    ed.register_kind("search_prev", Kind::Motion(Exclusive), |ed, ctx| {
        search(ed, ctx, false)
    });
    ed.register("search_word", |ed, ctx| {
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let (a, b) = m::word_at(buf, ed.views[ctx.view].sels.primary().head);
        if a == b {
            return;
        }
        let word = buf.slice(a..b);
        ed.last_search = Some(format!(r"\b{}\b", regex::escape(&word)));
        search(ed, ctx, true);
    });

    // ---- text objects (operator-pending and visual)
    ed.register_kind_char("textobject_inner", Kind::TextObject, |ed, ctx| {
        textobject(ed, ctx, false)
    });
    ed.register_kind_char("textobject_around", Kind::TextObject, |ed, ctx| {
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
        let ranges: Vec<(Range<usize>, bool)> = if ed.mode == Mode::Visual {
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
        if ed.mode == Mode::Visual {
            ed.mode = Mode::Normal;
        }
        apply_operator(ed, ctx.view, "join", ranges);
    });
    ed.register("delete_char", |ed, ctx| {
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let ranges: Vec<(Range<usize>, bool)> = ed.views[ctx.view]
            .sels
            .iter()
            .map(|s| {
                if ed.mode == Mode::Visual && !s.is_empty() {
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
        if ed.mode == Mode::Visual {
            ed.mode = Mode::Normal;
        }
        apply_operator(ed, ctx.view, "delete", ranges);
    });
    ed.register("delete_char_back", |ed, ctx| {
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
    ed.register("change_char", |ed, ctx| {
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let ranges: Vec<(Range<usize>, bool)> = ed.views[ctx.view]
            .sels
            .iter()
            .map(|s| {
                if ed.mode == Mode::Visual && !s.is_empty() {
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
        ed.mode = Mode::Normal;
        apply_operator(ed, ctx.view, "change", ranges);
    });
    ed.register("delete_to_end", |ed, ctx| {
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let ranges = ed.views[ctx.view]
            .sels
            .iter()
            .map(|s| (s.head..buf.line_range(buf.line_of(s.head)).end, false))
            .collect();
        apply_operator(ed, ctx.view, "delete", ranges);
    });
    ed.register("change_to_end", |ed, ctx| {
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let ranges = ed.views[ctx.view]
            .sels
            .iter()
            .map(|s| (s.head..buf.line_range(buf.line_of(s.head)).end, false))
            .collect();
        apply_operator(ed, ctx.view, "change", ranges);
    });
    ed.register_with_char("replace_char", |ed, ctx| {
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
    ed.register("insert_mode", |ed, _| ed.mode = Mode::Insert);
    ed.register("append", |ed, ctx| {
        motion(ed, ctx, |b, o, _| {
            let le = b.line_range(b.line_of(o)).end;
            if o < le { b.next_char(o) } else { o }
        });
        ed.mode = Mode::Insert;
    });
    ed.register("insert_line_start", |ed, ctx| {
        motion(ed, ctx, |b, o, _| m::first_nonblank(b, b.line_of(o)));
        ed.mode = Mode::Insert;
    });
    ed.register("append_line_end", |ed, ctx| {
        motion(ed, ctx, |b, o, _| b.line_range(b.line_of(o)).end);
        ed.mode = Mode::Insert;
    });
    ed.register("open_below", |ed, ctx| open_line(ed, ctx, true));
    ed.register("open_above", |ed, ctx| open_line(ed, ctx, false));
    ed.register("normal_mode", |ed, ctx| {
        let was_insert = ed.mode == Mode::Insert;
        ed.mode = Mode::Normal;
        ed.visual_linewise = false;
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
    ed.register("insert_newline", |ed, ctx| {
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let ln = buf.line_of(ed.views[ctx.view].sels.primary().head);
        let indent = m::indent_of(buf, ln);
        ed.insert_text(ctx.view, &format!("\n{indent}"));
    });
    ed.register("insert_tab", |ed, ctx| ed.insert_text(ctx.view, "\t"));
    ed.register("delete_word_back", |ed, ctx| {
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
    ed.register("delete_forward", |ed, ctx| {
        let id = view(ed, ctx).buffer;
        let buf = &ed.buffers[id];
        let ranges = ed.views[ctx.view]
            .sels
            .iter()
            .map(|s| (s.head..buf.next_char(s.head), false))
            .collect();
        apply_operator(ed, ctx.view, "delete", ranges);
    });
    ed.register("paste_after", |ed, ctx| paste(ed, ctx, true));
    ed.register("paste_before", |ed, ctx| paste(ed, ctx, false));
    ed.register("paste_clipboard", |ed, _| {
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

    // ---- visual and selections
    ed.register("visual_mode", |ed, _| {
        if ed.mode == Mode::Visual && !ed.visual_linewise {
            ed.mode = Mode::Normal;
        } else {
            ed.mode = Mode::Visual;
            ed.visual_linewise = false;
        }
    });
    ed.register("visual_line_mode", |ed, _| {
        if ed.mode == Mode::Visual && ed.visual_linewise {
            ed.mode = Mode::Normal;
            ed.visual_linewise = false;
        } else {
            ed.mode = Mode::Visual;
            ed.visual_linewise = true;
        }
    });
    ed.register("swap_ends", |ed, ctx| {
        ed.views[ctx.view]
            .sels
            .map(|s| Selection::new(s.head, s.anchor));
    });
    ed.register("keep_primary", |ed, ctx| {
        ed.views[ctx.view].sels.keep_primary()
    });
    ed.register("select_all", |ed, ctx| {
        let len = ed.buffer_of(ctx.view).len();
        ed.views[ctx.view].sels = crate::Selections::single(Selection::new(0, len));
        ed.mode = Mode::Visual;
    });
    ed.register("add_cursor_below", |ed, ctx| add_cursor(ed, ctx, 1));
    ed.register("add_cursor_above", |ed, ctx| add_cursor(ed, ctx, -1));

    // ---- prompts and ex commands
    ed.register("command_mode", |ed, _| {
        ed.mode = Mode::Command;
        ed.prompt = Prompt::Command;
        ed.cmdline.clear();
    });
    ed.register("search_mode", |ed, _| {
        ed.mode = Mode::Command;
        ed.prompt = Prompt::Search { backwards: false };
        ed.cmdline.clear();
    });
    ed.register("search_mode_back", |ed, _| {
        ed.mode = Mode::Command;
        ed.prompt = Prompt::Search { backwards: true };
        ed.cmdline.clear();
    });
    ed.register("write", |ed, ctx| {
        write(ed, ctx);
    });
    ed.register("quit", |ed, ctx| {
        let force = ctx.args.iter().any(|a| a == "!");
        let id = view(ed, ctx).buffer;
        if ed.buffers[id].modified && !force {
            ed.message = "unsaved changes (:q! to discard, :wq to write)".into();
            return;
        }
        ed.effects.push(Effect::Quit);
    });
    ed.register("quit_all", |ed, ctx| {
        let force = ctx.args.iter().any(|a| a == "!");
        if !force && ed.buffers.values().any(|b| b.modified) {
            ed.message = "unsaved changes (:qa! to discard)".into();
            return;
        }
        ed.effects.push(Effect::QuitAll);
    });
    ed.register("write_quit", |ed, ctx| {
        if write(ed, ctx) {
            ed.effects.push(Effect::Quit);
        }
    });
    ed.register("write_quit_all", |ed, ctx| {
        let ids: Vec<_> = ed
            .buffers
            .iter()
            .filter(|(_, b)| b.modified && b.path.is_some())
            .map(|(id, _)| id)
            .collect();
        for id in ids {
            let text = ed.buffers[id].text();
            if let Some(p) = ed.buffers[id].path.clone()
                && std::fs::write(&p, text).is_ok()
            {
                ed.buffers[id].modified = false;
            }
        }
        let _ = ctx;
        ed.effects.push(Effect::QuitAll);
    });
    ed.register("edit", |ed, ctx| match ctx.args.first() {
        Some(p) => ed.effects.push(Effect::Open(p.into())),
        None => ed.message = "edit what?".into(),
    });
    ed.register("set", |ed, ctx| {
        let Some(a) = ctx.args.first() else {
            ed.message = "set what?".into();
            return;
        };
        let (k, v) = match a.split_once('=') {
            Some((k, v)) => (k.to_string(), v.to_string()),
            None if a.starts_with("no") => (a[2..].to_string(), "false".to_string()),
            None => (a.to_string(), "true".to_string()),
        };
        ed.options.insert(k, v);
    });
    ed.register("echo", |ed, ctx| ed.message = ctx.args.join(" "));
}

fn goto_line(ed: &mut Editor, view: ViewId, n: usize) {
    let ext = extend(ed);
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
    ed.mode = Mode::Insert;
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
        ("h", "move_left"),
        ("<Left>", "move_left"),
        ("<BS>", "move_left"),
        ("l", "move_right"),
        ("<Right>", "move_right"),
        ("j", "move_down"),
        ("<Down>", "move_down"),
        ("k", "move_up"),
        ("<Up>", "move_up"),
        ("0", "line_start"),
        ("<Home>", "line_start"),
        ("^", "first_nonblank"),
        ("$", "line_end"),
        ("<End>", "line_end"),
        ("w", "word_next"),
        ("b", "word_prev"),
        ("e", "word_end"),
        ("gg", "goto_file_start"),
        ("G", "goto_file_end"),
        ("<C-d>", "half_page_down"),
        ("<C-u>", "half_page_up"),
        ("<C-f>", "page_down"),
        ("<PageDown>", "page_down"),
        ("<C-b>", "page_up"),
        ("<PageUp>", "page_up"),
        ("%", "match_bracket"),
        ("f", "find_char"),
        ("F", "find_char_back"),
        ("t", "till_char"),
        ("T", "till_char_back"),
        ("n", "search_next"),
        ("N", "search_prev"),
        ("*", "search_word"),
        ("/", "search_mode"),
        ("?", "search_mode_back"),
        (":", "command_mode"),
        ("d", "delete"),
        ("c", "change"),
        ("y", "yank"),
        (">", "indent"),
        ("<", "dedent"),
        ("J", "join"),
        ("x", "delete_char"),
        ("<Del>", "delete_char"),
        ("X", "delete_char_back"),
        ("D", "delete_to_end"),
        ("C", "change_to_end"),
        ("s", "change_char"),
        ("r", "replace_char"),
        ("i", "insert_mode"),
        ("a", "append"),
        ("I", "insert_line_start"),
        ("A", "append_line_end"),
        ("o", "open_below"),
        ("O", "open_above"),
        ("p", "paste_after"),
        ("P", "paste_before"),
        ("u", "undo"),
        ("<C-r>", "redo"),
        ("U", "redo"),
        ("v", "visual_mode"),
        ("V", "visual_line_mode"),
        (",", "keep_primary"),
        ("<A-j>", "add_cursor_below"),
        ("<A-k>", "add_cursor_above"),
        ("<Esc>", "normal_mode"),
        ("<C-c>", "normal_mode"),
        // Panes, tabs, the dock: the shell's commands (Effect::Shell).
        ("<C-w>v", "vsplit"),
        ("<C-w>s", "split"),
        ("<C-w>q", "close"),
        ("<C-w>c", "close"),
        ("<C-w>o", "only"),
        ("<C-w>w", "pane_next"),
        ("<C-w>h", "pane_left"),
        ("<C-w>j", "pane_down"),
        ("<C-w>k", "pane_up"),
        ("<C-w>l", "pane_right"),
        ("<C-w><Left>", "pane_left"),
        ("<C-w><Down>", "pane_down"),
        ("<C-w><Up>", "pane_up"),
        ("<C-w><Right>", "pane_right"),
        ("<C-w>t", "tab_new"),
        ("gt", "tab_next"),
        ("gT", "tab_prev"),
        ("<C-w>d", "dock_toggle"),
        ("gd", "lsp_definition"),
        ("K", "lsp_hover"),
        ("<CR>", "goto_location"),
        ("]q", "error_next"),
        ("[q", "error_prev"),
        ("-", "oil"),
    ];
    for (k, c) in n {
        km.bind(Normal, k, c);
    }
    let v = [
        ("o", "swap_ends"),
        ("x", "delete_char"),
        ("i", "textobject_inner"),
        ("a", "textobject_around"),
        ("<Esc>", "normal_mode"),
        ("<C-c>", "normal_mode"),
    ];
    for (k, c) in v {
        km.bind(Visual, k, c);
    }
    let op = [("i", "textobject_inner"), ("a", "textobject_around")];
    for (k, c) in op {
        km.bind(OperatorPending, k, c);
    }
    let i = [
        ("<Esc>", "normal_mode"),
        ("<C-c>", "normal_mode"),
        ("<CR>", "insert_newline"),
        ("<Tab>", "insert_tab"),
        ("<BS>", "delete_char_back"),
        ("<C-h>", "delete_char_back"),
        ("<Del>", "delete_forward"),
        ("<C-w>", "delete_word_back"),
        ("<Left>", "move_left"),
        ("<Right>", "move_right"),
        ("<Up>", "move_up"),
        ("<Down>", "move_down"),
        ("<Home>", "line_start"),
        ("<End>", "line_end_insert"),
        ("<D-v>", "paste_clipboard"),
        ("<C-S-v>", "paste_clipboard"),
        ("<C-Space>", "lsp_complete"),
    ];
    for (k, c) in i {
        km.bind(Insert, k, c);
    }
}
