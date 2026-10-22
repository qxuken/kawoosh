//! Reading a `.kdraw` text (docs/design/diagrams.md Decision 1). A line
//! that does not read is a [`Problem`] and no item; the rest of the file
//! reads as if it were a comment.

use crate::model::*;

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Word(String),
    Str(String),
    Open,
    Close,
}

/// A line's tokens: words, quoted labels, braces.
fn tokens(line: &str) -> Result<Vec<Tok>, String> {
    let mut out = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '{' {
            chars.next();
            out.push(Tok::Open);
        } else if c == '}' {
            chars.next();
            out.push(Tok::Close);
        } else if c == '"' {
            chars.next();
            let mut s = String::new();
            loop {
                match chars.next() {
                    None => return Err("a label with no closing quote".into()),
                    Some('"') => break,
                    Some('\\') => match chars.next() {
                        Some('n') => s.push('\n'),
                        Some('t') => s.push('\t'),
                        Some(c) => s.push(c),
                        None => return Err("a label with no closing quote".into()),
                    },
                    Some(c) => s.push(c),
                }
            }
            out.push(Tok::Str(s));
        } else {
            let mut w = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() || c == '"' || c == '{' || c == '}' {
                    break;
                }
                w.push(c);
                chars.next();
            }
            out.push(Tok::Word(w));
        }
    }
    Ok(out)
}

/// Whether `s` can name a box or a group: a letter or `_` first, then
/// letters, digits, `_`, `-`, `.`.
pub fn is_id(s: &str) -> bool {
    let mut cs = s.chars();
    matches!(cs.next(), Some(c) if c.is_alphabetic() || c == '_')
        && cs.all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

fn pair(s: &str, sep: char, what: &str) -> Result<(i32, i32), String> {
    let bad = || format!("`{s}` is not a {what}");
    let (a, b) = s.split_once(sep).ok_or_else(bad)?;
    Ok((
        a.trim().parse().map_err(|_| bad())?,
        b.trim().parse().map_err(|_| bad())?,
    ))
}

struct Reader {
    toks: std::vec::IntoIter<Tok>,
}

impl Reader {
    fn next(&mut self) -> Option<Tok> {
        self.toks.next()
    }

    fn word(&mut self, after: &str) -> Result<String, String> {
        match self.next() {
            Some(Tok::Word(w)) => Ok(w),
            _ => Err(format!("a word wanted after `{after}`")),
        }
    }

    fn id(&mut self, after: &str) -> Result<String, String> {
        let w = self.word(after)?;
        if is_id(&w) {
            Ok(w)
        } else {
            Err(format!(
                "`{w}` is not a name: a letter first, then letters, digits, `_` `-` `.`"
            ))
        }
    }

    fn color(&mut self) -> Result<Color, String> {
        let w = self.word("color")?;
        Color::from_word(&w)
            .ok_or_else(|| format!("no colour `{w}`: {}", Color::ALL.map(Color::word).join(" ")))
    }
}

fn read_line(toks: Vec<Tok>) -> Result<Item, String> {
    let mut r = Reader {
        toks: toks.into_iter(),
    };
    let Some(Tok::Word(kind)) = r.next() else {
        return Err("a line starts with box, text, arrow or group".into());
    };
    match kind.as_str() {
        "box" => {
            let id = r.id("box")?;
            let mut n = Node {
                id,
                label: String::new(),
                x: 0,
                y: 0,
                w: 0,
                h: 0,
                shape: Shape::Rect,
                color: None,
                dashed: false,
                align: LabelAlign::Center,
            };
            let mut size = None;
            while let Some(t) = r.next() {
                match t {
                    Tok::Str(s) => n.label = s,
                    Tok::Word(w) => match w.as_str() {
                        "at" => (n.x, n.y) = pair(&r.word("at")?, ',', "place (X,Y)")?,
                        "size" => size = Some(pair(&r.word("size")?, 'x', "size (WxH)")?),
                        "shape" => {
                            let s = r.word("shape")?;
                            n.shape = Shape::from_word(&s).ok_or_else(|| {
                                format!("no shape `{s}`: {}", Shape::ALL.map(Shape::word).join(" "))
                            })?;
                        }
                        "color" => n.color = Some(r.color()?),
                        "dashed" => n.dashed = true,
                        "solid" => n.dashed = false,
                        "align" => {
                            let a = r.word("align")?;
                            n.align = LabelAlign::from_word(&a)
                                .ok_or_else(|| format!("no align `{a}`: center left right"))?;
                        }
                        _ => return Err(format!("`{w}` means nothing to a box")),
                    },
                    _ => return Err("a brace means nothing to a box".into()),
                }
            }
            let (w, h) = size.unwrap_or(n.shape.default_size());
            if w < 1 || h < 1 {
                return Err("a box is a unit or more each way".into());
            }
            (n.w, n.h) = (w, h);
            Ok(Item::Node(n))
        }
        "text" => {
            // A name first, when a group is to hold it: any word but the
            // ones a text's line takes.
            let id = match r.toks.as_slice().first() {
                Some(Tok::Word(w)) if w != "at" && w != "color" => Some(r.id("text")?),
                _ => None,
            };
            let mut t = Text {
                id,
                label: String::new(),
                x: 0,
                y: 0,
                color: None,
            };
            let mut said = false;
            while let Some(tok) = r.next() {
                match tok {
                    Tok::Str(s) => {
                        t.label = s;
                        said = true;
                    }
                    Tok::Word(w) => match w.as_str() {
                        "at" => (t.x, t.y) = pair(&r.word("at")?, ',', "place (X,Y)")?,
                        "color" => t.color = Some(r.color()?),
                        _ => return Err(format!("`{w}` means nothing to a text")),
                    },
                    _ => return Err("a brace means nothing to a text".into()),
                }
            }
            if !said {
                return Err("a text wants its words, quoted".into());
            }
            Ok(Item::Text(t))
        }
        "arrow" => {
            let from = r.id("arrow")?;
            let op = r.word(&from)?;
            let heads = Heads::from_token(&op)
                .ok_or_else(|| format!("`{op}` is not an arrow: -> <- <-> --"))?;
            let to = r.id(&op)?;
            let mut e = Edge {
                from,
                to,
                label: String::new(),
                heads,
                route: Route::Straight,
                dashed: false,
                color: None,
                via: None,
            };
            while let Some(t) = r.next() {
                match t {
                    Tok::Str(s) => e.label = s,
                    Tok::Word(w) => match w.as_str() {
                        "via" => e.via = Some(pair(&r.word("via")?, ',', "point (X,Y)")?),
                        "elbow" => e.route = Route::Elbow,
                        "straight" => e.route = Route::Straight,
                        "dashed" => e.dashed = true,
                        "solid" => e.dashed = false,
                        "color" => e.color = Some(r.color()?),
                        _ => return Err(format!("`{w}` means nothing to an arrow")),
                    },
                    _ => return Err("a brace means nothing to an arrow".into()),
                }
            }
            Ok(Item::Edge(e))
        }
        "group" => {
            let id = r.id("group")?;
            let mut g = Group {
                id,
                label: String::new(),
                members: Vec::new(),
                color: None,
            };
            let mut opened = false;
            let mut closed = false;
            while let Some(t) = r.next() {
                match t {
                    Tok::Open if !opened => opened = true,
                    Tok::Close if opened && !closed => closed = true,
                    Tok::Word(w) if opened && !closed => {
                        if !is_id(&w) {
                            return Err(format!("`{w}` is not a name"));
                        }
                        if !g.members.contains(&w) {
                            g.members.push(w);
                        }
                    }
                    Tok::Str(s) if !opened => g.label = s,
                    Tok::Word(w) if w == "color" => g.color = Some(r.color()?),
                    Tok::Word(w) => return Err(format!("`{w}` means nothing to a group")),
                    _ => return Err("a group is `group ID \"LABEL\" { A B }`".into()),
                }
            }
            if !closed {
                return Err("a group's members go in braces: `{ a b }`".into());
            }
            Ok(Item::Group(g))
        }
        _ => Err(format!("`{kind}` is not an item: box, text, arrow, group")),
    }
}

/// The text read: each line an item, a comment, nothing, or a problem;
/// then what the items name looked up — an arrow to no box, a second box
/// of a name, a group's member nobody has are problems too.
pub fn parse(text: &str) -> Doc {
    let mut doc = Doc {
        len: text.len(),
        ends_in_newline: text.is_empty() || text.ends_with('\n'),
        ..Default::default()
    };
    let mut start = 0;
    for (line, raw) in text.split('\n').enumerate() {
        let end = start + raw.len();
        let content = raw.strip_suffix('\r').unwrap_or(raw);
        let span = start..start + content.len();
        start = end + 1;
        let trimmed = content.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        match tokens(content).and_then(read_line) {
            Ok(item) => doc.entries.push(Entry { item, line, span }),
            Err(message) => doc.problems.push(Problem { line, message }),
        }
    }
    // A name once: the second box or group of it is a problem.
    let mut seen: Vec<String> = Vec::new();
    let mut keep = Vec::with_capacity(doc.entries.len());
    for e in std::mem::take(&mut doc.entries) {
        if let Some(id) = e.item.id() {
            if seen.iter().any(|s| s == id) {
                doc.problems.push(Problem {
                    line: e.line,
                    message: format!("`{id}` is named twice"),
                });
                continue;
            }
            seen.push(id.to_string());
        }
        keep.push(e);
    }
    doc.entries = keep;
    // An arrow goes between boxes that are there.
    let nodes: Vec<String> = doc
        .entries
        .iter()
        .filter_map(|e| match &e.item {
            Item::Node(n) => Some(n.id.clone()),
            _ => None,
        })
        .collect();
    let mut problems = Vec::new();
    doc.entries.retain(|e| match &e.item {
        Item::Edge(edge) => {
            let missing = [&edge.from, &edge.to]
                .into_iter()
                .find(|id| !nodes.contains(id));
            if let Some(id) = missing {
                problems.push(Problem {
                    line: e.line,
                    message: format!("no box `{id}`"),
                });
                false
            } else {
                true
            }
        }
        _ => true,
    });
    for e in &doc.entries {
        if let Item::Group(g) = &e.item {
            for m in &g.members {
                if !seen.contains(m) {
                    problems.push(Problem {
                        line: e.line,
                        message: format!("no box or group `{m}`"),
                    });
                }
            }
        }
    }
    doc.problems.extend(problems);
    doc.problems.sort_by_key(|p| p.line);
    doc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_reads_into_its_items_and_their_lines() {
        let text = "# services\nbox api \"API gateway\" at 2,2 size 8x3 shape round\r\n\nbox db \"Postgres\" at 16,2 shape cylinder color blue\narrow api -> db \"reads\" elbow dashed\ntext \"say \\\"hi\\\"\\nthere\" at 1,9\ngroup be \"Backend\" { db } color gray\n";
        let doc = parse(text);
        assert_eq!(doc.problems, vec![]);
        assert_eq!(doc.entries.len(), 5);
        let Item::Node(api) = &doc.entries[0].item else {
            panic!()
        };
        assert_eq!(
            (
                api.id.as_str(),
                api.label.as_str(),
                api.x,
                api.y,
                api.w,
                api.h,
                api.shape
            ),
            ("api", "API gateway", 2, 2, 8, 3, Shape::Round)
        );
        // The line's bytes, its `\r` not among them.
        assert_eq!(
            &text[doc.entries[0].span.clone()],
            "box api \"API gateway\" at 2,2 size 8x3 shape round"
        );
        assert_eq!(doc.entries[0].line, 1);
        let Item::Node(db) = &doc.entries[1].item else {
            panic!()
        };
        assert_eq!((db.w, db.h, db.color), (8, 3, Some(Color::Blue)));
        let Item::Edge(e) = &doc.entries[2].item else {
            panic!()
        };
        assert_eq!(
            (
                e.from.as_str(),
                e.to.as_str(),
                e.label.as_str(),
                e.heads,
                e.route,
                e.dashed
            ),
            ("api", "db", "reads", Heads::End, Route::Elbow, true)
        );
        let Item::Text(t) = &doc.entries[3].item else {
            panic!()
        };
        assert_eq!(t.label, "say \"hi\"\nthere");
        let Item::Group(g) = &doc.entries[4].item else {
            panic!()
        };
        assert_eq!(
            (g.label.as_str(), g.members.clone(), g.color),
            ("Backend", vec!["db".to_string()], Some(Color::Gray))
        );
    }

    #[test]
    fn what_does_not_read_is_a_problem_and_the_rest_reads() {
        let doc = parse(
            "box a\nbox a\nbox b shape hex\narrow a -> nope\narrow a => a\ngroup g { a ghost }\nwobble\ntext at 1,1\nbox c size 0x2\n",
        );
        let lines: Vec<(usize, &str)> = doc
            .problems
            .iter()
            .map(|p| (p.line, p.message.as_str()))
            .collect();
        assert_eq!(
            lines,
            vec![
                (1, "`a` is named twice"),
                (
                    2,
                    "no shape `hex`: rect round ellipse diamond cylinder dot final"
                ),
                (3, "no box `nope`"),
                (4, "`=>` is not an arrow: -> <- <-> --"),
                (5, "no box or group `ghost`"),
                (6, "`wobble` is not an item: box, text, arrow, group"),
                (7, "a text wants its words, quoted"),
                (8, "a box is a unit or more each way"),
            ]
        );
        assert_eq!(doc.entries.len(), 2, "box a and the group");
    }

    #[test]
    fn a_dot_is_a_unit_and_a_loop_reads() {
        let doc = parse("box s shape dot at 0,0\narrow s -> s\n");
        assert_eq!(doc.problems, vec![]);
        let Item::Node(n) = &doc.entries[0].item else {
            panic!()
        };
        assert_eq!((n.w, n.h), (1, 1));
    }
}
