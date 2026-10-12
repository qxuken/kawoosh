//! An item as its line, the canonical form: what a line the canvas
//! changed is written as (docs/design/diagrams.md Decision 3).

use crate::model::*;

/// `s` quoted, `\` `"` and the line breaks escaped.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn color(out: &mut String, c: Option<Color>) {
    if let Some(c) = c {
        out.push_str(" color ");
        out.push_str(c.word());
    }
}

pub fn line(item: &Item) -> String {
    match item {
        Item::Node(n) => {
            let mut s = format!("box {}", n.id);
            if !n.label.is_empty() {
                s.push(' ');
                s.push_str(&quote(&n.label));
            }
            s.push_str(&format!(" at {},{}", n.x, n.y));
            if (n.w, n.h) != n.shape.default_size() {
                s.push_str(&format!(" size {}x{}", n.w, n.h));
            }
            if n.shape != Shape::Rect {
                s.push_str(" shape ");
                s.push_str(n.shape.word());
            }
            color(&mut s, n.color);
            s
        }
        Item::Text(t) => {
            let mut s = format!("text {} at {},{}", quote(&t.label), t.x, t.y);
            color(&mut s, t.color);
            s
        }
        Item::Edge(e) => {
            let mut s = format!("arrow {} {} {}", e.from, e.heads.token(), e.to);
            if !e.label.is_empty() {
                s.push(' ');
                s.push_str(&quote(&e.label));
            }
            if e.route == Route::Elbow {
                s.push_str(" elbow");
            }
            if e.dashed {
                s.push_str(" dashed");
            }
            color(&mut s, e.color);
            s
        }
        Item::Group(g) => {
            let mut s = format!("group {}", g.id);
            if !g.label.is_empty() {
                s.push(' ');
                s.push_str(&quote(&g.label));
            }
            s.push_str(" {");
            for m in &g.members {
                s.push(' ');
                s.push_str(m);
            }
            s.push_str(" }");
            color(&mut s, g.color);
            s
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    #[test]
    fn a_line_written_reads_back_as_the_item() {
        let text = "box api \"API \\\"gw\\\"\\nv2\" at -2,3 size 9x4 shape round color blue\nbox s at 0,0 shape dot\ntext \"note\" at 1,9 color red\narrow api <-> s \"both\" elbow dashed color green\narrow s -- s\ngroup g \"G\" { api s } color gray\n";
        let doc = parse(text);
        assert_eq!(doc.problems, vec![]);
        let written: String = doc.items().map(|i| line(i) + "\n").collect();
        assert_eq!(written, text, "canonical in, the same out");
        assert_eq!(parse(&written).entries.len(), doc.entries.len());
    }
}
