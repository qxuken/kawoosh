//! What a gesture does to the file (docs/design/diagrams.md Decision 3):
//! a list of [`Change`]s to the doc's entries, made into byte edits on
//! the text that touch the lines changed and nothing else.

use std::ops::Range;

use crate::model::*;
use crate::write;

#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// The entry's line written again as this item.
    Set(usize, Item),
    /// The entry's line taken out.
    Remove(usize),
    /// A line appended.
    Add(Item),
}

/// `changes` as edits on `text` (the text `doc` was read from):
/// ascending, disjoint, a line taken out with its newline, the new lines
/// at the end. An entry changed twice takes the last.
pub fn edits(doc: &Doc, text: &str, changes: &[Change]) -> Vec<(Range<usize>, String)> {
    let mut per: Vec<Option<Option<&Item>>> = vec![None; doc.entries.len()];
    let mut adds = Vec::new();
    for c in changes {
        match c {
            Change::Set(i, item) if *i < per.len() => {
                // A line taken out stays out.
                if per[*i] != Some(None) {
                    per[*i] = Some(Some(item));
                }
            }
            Change::Remove(i) if *i < per.len() => per[*i] = Some(None),
            Change::Add(item) => adds.push(item),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for (i, what) in per.into_iter().enumerate() {
        let span = doc.entries[i].span.clone();
        match what {
            None => {}
            Some(Some(item)) => {
                let line = write::line(item);
                if text.get(span.clone()) != Some(line.as_str()) {
                    out.push((span, line));
                }
            }
            Some(None) => {
                // The line and the newline after it; the last line, with
                // none after it, the newline before it.
                let rest = &text[span.end..];
                let gone = if let Some(n) = rest.find('\n') {
                    span.start..span.end + n + 1
                } else if span.start > 0 {
                    span.start - 1..span.end
                } else {
                    span
                };
                out.push((gone, String::new()));
            }
        }
    }
    if !adds.is_empty() {
        let mut s = String::new();
        if !doc.ends_in_newline {
            s.push('\n');
        }
        for item in adds {
            s.push_str(&write::line(item));
            s.push('\n');
        }
        out.push((text.len()..text.len(), s));
    }
    out.sort_by_key(|(r, _)| (r.start, r.end));
    // A line taken out at the end with the newline before it may meet the
    // one before it, rewritten: never overlapping, only touching.
    out
}

/// `edits` applied to `text`, for tests and for anything without a
/// buffer.
pub fn apply(text: &str, edits: &[(Range<usize>, String)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (r, s) in edits {
        out.push_str(&text[at..r.start]);
        out.push_str(s);
        at = r.end;
    }
    out.push_str(&text[at..]);
    out
}

/// The boxes the selection moves: its boxes, its groups' boxes, its
/// texts by entry.
fn movers(doc: &Doc, sel: &[usize]) -> (Vec<String>, Vec<usize>) {
    let mut nodes: Vec<String> = Vec::new();
    let mut texts = Vec::new();
    for &i in sel {
        match doc.entries.get(i).map(|e| &e.item) {
            Some(Item::Node(n)) => {
                if !nodes.contains(&n.id) {
                    nodes.push(n.id.clone());
                }
            }
            Some(Item::Group(g)) => {
                for id in doc.group_nodes(&g.id) {
                    if !nodes.contains(&id) {
                        nodes.push(id);
                    }
                }
            }
            Some(Item::Text(_)) => texts.push(i),
            _ => {}
        }
    }
    (nodes, texts)
}

/// The box ids a move of `sel` takes along: what the canvas draws moved
/// while the pointer is down.
pub fn moving_nodes(doc: &Doc, sel: &[usize]) -> Vec<String> {
    movers(doc, sel).0
}

/// The selection moved `dx`, `dy` units: a group moves its boxes, an
/// arrow follows its.
pub fn move_by(doc: &Doc, sel: &[usize], dx: i32, dy: i32) -> Vec<Change> {
    if dx == 0 && dy == 0 {
        return Vec::new();
    }
    let (nodes, texts) = movers(doc, sel);
    let mut out = Vec::new();
    for (i, e) in doc.entries.iter().enumerate() {
        match &e.item {
            Item::Node(n) if nodes.contains(&n.id) => {
                let mut n = n.clone();
                n.x += dx;
                n.y += dy;
                out.push(Change::Set(i, Item::Node(n)));
            }
            Item::Text(t) if texts.contains(&i) => {
                let mut t = t.clone();
                t.x += dx;
                t.y += dy;
                out.push(Change::Set(i, Item::Text(t)));
            }
            _ => {}
        }
    }
    out
}

/// Box `i` put at `x`, `y`, `w` by `h`, a unit each way at least.
pub fn resize(doc: &Doc, i: usize, x: i32, y: i32, w: i32, h: i32) -> Vec<Change> {
    let Some(Item::Node(n)) = doc.entries.get(i).map(|e| &e.item) else {
        return Vec::new();
    };
    let mut n = n.clone();
    (n.x, n.y, n.w, n.h) = (x, y, w.max(1), h.max(1));
    vec![Change::Set(i, Item::Node(n))]
}

/// The selection taken out: a box takes its arrows with it and leaves
/// its groups; a group goes alone, its members staying.
pub fn delete(doc: &Doc, sel: &[usize]) -> Vec<Change> {
    let gone_ids: Vec<&str> = sel
        .iter()
        .filter_map(|&i| doc.entries.get(i).and_then(|e| e.item.id()))
        .collect();
    let mut out = Vec::new();
    for (i, e) in doc.entries.iter().enumerate() {
        if sel.contains(&i) {
            out.push(Change::Remove(i));
            continue;
        }
        match &e.item {
            Item::Edge(edge)
                if gone_ids.contains(&edge.from.as_str()) || gone_ids.contains(&edge.to.as_str()) =>
            {
                out.push(Change::Remove(i));
            }
            Item::Group(g) if g.members.iter().any(|m| gone_ids.contains(&m.as_str())) => {
                let mut g = g.clone();
                g.members.retain(|m| !gone_ids.contains(&m.as_str()));
                out.push(Change::Set(i, Item::Group(g)));
            }
            _ => {}
        }
    }
    out
}

/// Each selected item changed by `f`.
pub fn each(doc: &Doc, sel: &[usize], mut f: impl FnMut(&mut Item)) -> Vec<Change> {
    sel.iter()
        .filter_map(|&i| {
            let mut item = doc.entries.get(i)?.item.clone();
            f(&mut item);
            (item != doc.entries[i].item).then_some(Change::Set(i, item))
        })
        .collect()
}

/// A new box at `x`, `y`, `w` by `h`, named the first `n` free; its id.
pub fn add_node(doc: &Doc, x: i32, y: i32, w: i32, h: i32) -> (Change, String) {
    let id = doc.fresh_id("n");
    let n = Node {
        id: id.clone(),
        label: String::new(),
        x,
        y,
        w: w.max(1),
        h: h.max(1),
        shape: Shape::Rect,
        color: None,
    };
    (Change::Add(Item::Node(n)), id)
}

pub fn add_text(label: &str, x: i32, y: i32) -> Change {
    Change::Add(Item::Text(Text {
        label: label.to_string(),
        x,
        y,
        color: None,
    }))
}

pub fn add_edge(from: &str, to: &str) -> Change {
    Change::Add(Item::Edge(Edge {
        from: from.to_string(),
        to: to.to_string(),
        label: String::new(),
        heads: Heads::End,
        route: Route::Straight,
        dashed: false,
        color: None,
    }))
}

/// A group round the selection's boxes and groups; its id.
pub fn group(doc: &Doc, sel: &[usize]) -> Option<(Change, String)> {
    let members: Vec<String> = sel
        .iter()
        .filter_map(|&i| doc.entries.get(i).and_then(|e| e.item.id()).map(String::from))
        .collect();
    if members.is_empty() {
        return None;
    }
    let id = doc.fresh_id("g");
    Some((
        Change::Add(Item::Group(Group {
            id: id.clone(),
            label: String::new(),
            members,
            color: None,
        })),
        id,
    ))
}

/// Items copied out of `doc` for a paste: the selection, its groups'
/// boxes, and the arrows between the boxes copied.
pub fn copy(doc: &Doc, sel: &[usize]) -> Vec<Item> {
    let (nodes, _) = movers(doc, sel);
    let mut out = Vec::new();
    for (i, e) in doc.entries.iter().enumerate() {
        let keep = match &e.item {
            Item::Node(n) => nodes.contains(&n.id),
            Item::Edge(edge) => sel.contains(&i) || (nodes.contains(&edge.from) && nodes.contains(&edge.to)),
            _ => sel.contains(&i),
        };
        if keep {
            out.push(e.item.clone());
        }
    }
    out
}

/// `items` added to `doc` moved by `dx`, `dy`: each box and group named
/// afresh and what names them renamed with it; an arrow whose box was not
/// among them keeps it, a group's member that was not is left out.
pub fn paste(doc: &Doc, items: &[Item], dx: i32, dy: i32) -> Vec<Change> {
    let mut renamed: Vec<(String, String)> = Vec::new();
    let mut taken: Vec<String> = Vec::new();
    for item in items {
        if let Some(id) = item.id() {
            let prefix = id.trim_end_matches(|c: char| c.is_ascii_digit());
            let prefix = if prefix.is_empty() { "n" } else { prefix };
            let fresh = doc.fresh_id_avoiding(prefix, &taken);
            taken.push(fresh.clone());
            renamed.push((id.to_string(), fresh));
        }
    }
    let name = |id: &str| {
        renamed
            .iter()
            .find(|(old, _)| old == id)
            .map(|(_, new)| new.clone())
    };
    let mut out = Vec::new();
    for item in items {
        let item = match item.clone() {
            Item::Node(mut n) => {
                n.id = name(&n.id).unwrap_or(n.id);
                n.x += dx;
                n.y += dy;
                Item::Node(n)
            }
            Item::Text(mut t) => {
                t.x += dx;
                t.y += dy;
                Item::Text(t)
            }
            Item::Edge(mut e) => {
                e.from = name(&e.from).unwrap_or(e.from);
                e.to = name(&e.to).unwrap_or(e.to);
                if doc.node(&e.from).is_none() && !taken.contains(&e.from)
                    || doc.node(&e.to).is_none() && !taken.contains(&e.to)
                {
                    continue;
                }
                Item::Edge(e)
            }
            Item::Group(mut g) => {
                g.id = name(&g.id).unwrap_or(g.id);
                g.members = g.members.iter().filter_map(|m| name(m)).collect();
                Item::Group(g)
            }
        };
        out.push(Change::Add(item));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    const TEXT: &str = "# the services\nbox a  \"A\"   at 0,0\nbox b \"B\" at 10,0\narrow a -> b\n\ntext \"note\" at 0,6\ngroup g { a b }\n";

    fn run(text: &str, f: impl Fn(&Doc) -> Vec<Change>) -> String {
        let doc = parse(text);
        apply(text, &edits(&doc, text, &f(&doc)))
    }

    #[test]
    fn a_move_writes_the_lines_it_moved_and_no_other() {
        let out = run(TEXT, |d| move_by(d, &[0], 2, 1));
        assert_eq!(
            out,
            "# the services\nbox a \"A\" at 2,1\nbox b \"B\" at 10,0\narrow a -> b\n\ntext \"note\" at 0,6\ngroup g { a b }\n"
        );
        // A group moves its boxes; the text stays.
        let out = run(TEXT, |d| move_by(d, &[4], 0, 3));
        assert!(out.contains("box a \"A\" at 0,3\nbox b \"B\" at 10,3\n"), "{out}");
        assert!(out.contains("text \"note\" at 0,6"));
    }

    #[test]
    fn a_box_deleted_takes_its_arrows_and_leaves_its_groups() {
        let out = run(TEXT, |d| delete(d, &[0]));
        assert_eq!(
            out,
            "# the services\nbox b \"B\" at 10,0\n\ntext \"note\" at 0,6\ngroup g { b }\n"
        );
        // The last line, with no newline after it.
        let out = run("box a at 0,0\nbox b at 1,1", |d| delete(d, &[1]));
        assert_eq!(out, "box a at 0,0");
    }

    #[test]
    fn new_lines_go_at_the_end() {
        let out = run("box a at 0,0", |d| {
            let (c, id) = add_node(d, 4, 4, 8, 3);
            assert_eq!(id, "n1");
            vec![c, add_edge("a", &id)]
        });
        assert_eq!(out, "box a at 0,0\nbox n1 at 4,4\narrow a -> n1\n");
    }

    #[test]
    fn a_paste_names_its_boxes_afresh() {
        let doc = parse(TEXT);
        let items = copy(&doc, &[0, 1]);
        assert_eq!(items.len(), 3, "both boxes and the arrow between them");
        let out = apply(TEXT, &edits(&doc, TEXT, &paste(&doc, &items, 2, 2)));
        assert!(
            out.ends_with("box a1 \"A\" at 2,2\nbox b1 \"B\" at 12,2\narrow a1 -> b1\n"),
            "{out}"
        );
        assert!(parse(&out).problems.is_empty());
    }

    #[test]
    fn a_group_round_the_selection() {
        let out = run(TEXT, |d| vec![group(d, &[0, 1, 2]).unwrap().0]);
        assert!(out.ends_with("group g1 { a b }\n"), "{out}");
    }
}
