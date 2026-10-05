//! `%` on a keyword of a block the language closes with a word — Lua's
//! and Ruby's `end`, a shell's `fi` `done` `esac`: the keyword the key
//! goes to, read off the syntax tree, no query asked. vim's matchit by
//! the tree rather than by a pattern per language.
//!
//! A block is a node whose last token is a closing word and whose first
//! is a keyword: `function` … `end`, `if` … `fi`. Where the closer sits
//! in the node's `body` — Ruby's `while c` … `end`, a shell's `for` …
//! `do` … `done` — the block is the loop around it. Its stops are its
//! opener, its closer, and between them the clause words that are its
//! own and no nested block's (`else`, `elseif`, `rescue`, `when`); `%`
//! on one goes to the next, from the closer back to the opener. Another
//! keyword that is the block's own child (`then`, `do`, the `function`
//! of `local function`) is no stop, and goes to the next one after it.
//!
//! Pure: a tree. The shell keeps the trees (`kawoosh::indent`).

use std::ops::Range;

use tree_sitter::{Node, Tree};

/// The words a block closes with, lower case; any word starting with
/// `end` (`endif`, `endmodule`) is one too.
const CLOSERS: &[&str] = &["end", "fi", "done", "esac", "od"];

/// The words that open a clause of a block, lower case.
const CLAUSES: &[&str] = &[
    "else", "elseif", "elsif", "elif", "when", "case", "rescue", "ensure", "catch", "finally",
    "after", "except",
];

/// A keyword: a token the grammar spells out, all letters.
fn keyword(n: Node) -> bool {
    !n.is_named()
        && !n.is_missing()
        && n.child_count() == 0
        && !n.kind().is_empty()
        && n.kind().bytes().all(|b| b.is_ascii_alphabetic())
}

fn closer(n: Node) -> bool {
    keyword(n) && {
        let k = n.kind().to_ascii_lowercase();
        CLOSERS.contains(&k.as_str()) || k.starts_with("end")
    }
}

fn clause(n: Node) -> bool {
    keyword(n) && CLAUSES.contains(&n.kind().to_ascii_lowercase().as_str())
}

fn first_child(n: Node) -> Option<Node> {
    n.child(0)
}

fn last_child(n: Node) -> Option<Node> {
    n.child(n.child_count().checked_sub(1)?)
}

fn last_leaf(mut n: Node) -> Node {
    while let Some(c) = last_child(n) {
        n = c;
    }
    n
}

/// Whether `n` is `parent`'s `body`.
fn body(parent: Node, n: Node) -> bool {
    let mut c = parent.walk();
    if !c.goto_first_child() {
        return false;
    }
    loop {
        if c.node() == n {
            return c.field_name() == Some("body");
        }
        if !c.goto_next_sibling() {
            return false;
        }
    }
}

/// A block: its node, the node its closer is a child of (the same, or
/// the `body` at its end, however deep), and its two ends.
#[derive(Clone, Copy)]
struct Block<'t> {
    node: Node<'t>,
    inner: Node<'t>,
    open: Node<'t>,
    close: Node<'t>,
}

/// The block `close` closes: its parent, or the loop whose `body` that
/// is — as far up as each is the last of a node a keyword starts.
fn closed_by(close: Node) -> Option<Block> {
    if !closer(close) {
        return None;
    }
    let inner = close.parent()?;
    if last_child(inner)? != close {
        return None;
    }
    let mut node = inner;
    while let Some(up) = node.parent() {
        if last_child(up) == Some(node) && first_child(up).is_some_and(keyword) && body(up, node) {
            node = up;
        } else {
            break;
        }
    }
    let open = first_child(node).filter(|o| keyword(*o) && *o != close)?;
    Some(Block {
        node,
        inner,
        open,
        close,
    })
}

/// The closest block around token `t`.
fn around(t: Node) -> Option<Block> {
    let mut n = t.parent()?;
    loop {
        if let Some(b) = closed_by(last_leaf(n)).filter(|b| b.node == n) {
            return Some(b);
        }
        n = n.parent()?;
    }
}

/// Whether `t` is `b`'s own child, or its body's on the way to the
/// closer.
fn own(b: &Block, t: Node) -> bool {
    let Some(p) = t.parent() else {
        return false;
    };
    let mut n = b.inner;
    loop {
        if n == p {
            return true;
        }
        if n == b.node {
            return false;
        }
        match n.parent() {
            Some(up) => n = up,
            None => return false,
        }
    }
}

/// The clause words in `n` that are block `b`'s, in order.
fn clauses<'t>(b: &Block<'t>, n: Node<'t>, out: &mut Vec<Node<'t>>) {
    let mut c = n.walk();
    if !c.goto_first_child() {
        return;
    }
    loop {
        let child = c.node();
        if child.child_count() == 0 {
            if clause(child) && child != b.open && around(child).is_some_and(|a| a.node == b.node) {
                out.push(child);
            }
        } else if first_child(child).is_none_or(|f| !keyword(f))
            || last_child(child).is_none_or(|l| !closer(l))
        {
            // A node a keyword opens and a closer ends is a block of
            // its own, its clauses its own.
            clauses(b, child, out);
        }
        if !c.goto_next_sibling() {
            return;
        }
    }
}

/// The keyword over byte `at` and the one `%` goes to from it, each its
/// bytes; `None` where `at` is on no keyword of a block.
pub fn partner(tree: &Tree, at: usize) -> Option<(Range<usize>, Range<usize>)> {
    let t = tree.root_node().descendant_for_byte_range(at, at + 1)?;
    if !keyword(t) {
        return None;
    }
    let b = match closed_by(t) {
        Some(b) => b,
        None => around(t)?,
    };
    let mut stops = vec![b.open];
    clauses(&b, b.node, &mut stops);
    stops.push(b.close);
    // A clause's own keyword (`elseif c then`) is the block's as well.
    let of_clause = t
        .parent()
        .and_then(first_child)
        .is_some_and(|f| stops.contains(&f));
    if !stops.contains(&t) && !of_clause && !own(&b, t) {
        return None;
    }
    let to = stops
        .iter()
        .find(|s| s.start_byte() > t.start_byte())
        .unwrap_or(&b.open);
    Some((t.byte_range(), to.byte_range()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kawoosh_languages::LANGUAGES;

    fn tree(language: &str, src: &str) -> Tree {
        let l = LANGUAGES.iter().find(|l| l.name == language).unwrap();
        let g = (l.grammar.unwrap())().unwrap();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&g.language).unwrap();
        parser.parse(src, None).unwrap()
    }

    /// The `n`th `word` of `src`, from 0.
    fn nth(src: &str, word: &str, n: usize) -> usize {
        src.match_indices(word)
            .filter(|(i, _)| {
                let b = src.as_bytes();
                let before = i
                    .checked_sub(1)
                    .is_none_or(|j| !b[j].is_ascii_alphanumeric());
                let after = b
                    .get(i + word.len())
                    .is_none_or(|c| !c.is_ascii_alphanumeric());
                before && after
            })
            .nth(n)
            .unwrap_or_else(|| panic!("no {word} #{n}"))
            .0
    }

    /// `%` on the `n`th `word` lands on the `m`th `to`.
    #[track_caller]
    fn goes(language: &str, src: &str, (word, n): (&str, usize), (to, m): (&str, usize)) {
        let t = tree(language, src);
        let at = nth(src, word, n);
        let got = partner(&t, at + word.len() - 1).map(|(from, to)| (from.start, to));
        let want = nth(src, to, m);
        assert_eq!(
            got,
            Some((at, want..want + to.len())),
            "{word} #{n} to {to} #{m}"
        );
    }

    #[track_caller]
    fn stays(language: &str, src: &str, (word, n): (&str, usize)) {
        let t = tree(language, src);
        assert_eq!(partner(&t, nth(src, word, n)), None, "{word} #{n}");
    }

    #[test]
    fn lua() {
        let src = "\
local function f(a)
  if a then
    return function() end
  elseif b then
    while a do break end
  else
    for i = 1, 2 do print(i) end
  end
  repeat a = 1 until a
end
";
        // The outer function: its first keyword and its `end`.
        goes("lua", src, ("local", 0), ("end", 4));
        goes("lua", src, ("function", 0), ("end", 4));
        goes("lua", src, ("end", 4), ("local", 0));
        // The `if` goes round its clauses, the nested blocks' not its.
        goes("lua", src, ("if", 0), ("elseif", 0));
        goes("lua", src, ("elseif", 0), ("else", 0));
        goes("lua", src, ("else", 0), ("end", 3));
        goes("lua", src, ("end", 3), ("if", 0));
        // `then` is no stop: on from it.
        goes("lua", src, ("then", 0), ("elseif", 0));
        goes("lua", src, ("then", 1), ("else", 0));
        // A function that is a value, after a keyword of another node.
        goes("lua", src, ("function", 1), ("end", 0));
        goes("lua", src, ("end", 0), ("function", 1));
        stays("lua", src, ("return", 0));
        goes("lua", src, ("while", 0), ("end", 1));
        goes("lua", src, ("do", 0), ("end", 1));
        goes("lua", src, ("end", 1), ("while", 0));
        stays("lua", src, ("break", 0));
        goes("lua", src, ("for", 0), ("end", 2));
        // No closing word, no block.
        stays("lua", src, ("repeat", 0));
        stays("lua", src, ("until", 0));
        stays("lua", src, ("print", 0));
    }

    #[test]
    fn bash() {
        let src = "\
if a; then
  for x in 1 2; do echo $x; done
elif b; then
  case $x in
    a) echo;;
  esac
else
  while true; do break; done
fi
";
        goes("bash", src, ("if", 0), ("elif", 0));
        goes("bash", src, ("elif", 0), ("else", 0));
        goes("bash", src, ("else", 0), ("fi", 0));
        goes("bash", src, ("fi", 0), ("if", 0));
        // A loop is its `do` … `done` and the word before them.
        goes("bash", src, ("for", 0), ("done", 0));
        goes("bash", src, ("do", 0), ("done", 0));
        goes("bash", src, ("done", 0), ("for", 0));
        goes("bash", src, ("while", 0), ("done", 1));
        goes("bash", src, ("done", 1), ("while", 0));
        goes("bash", src, ("case", 0), ("esac", 0));
        goes("bash", src, ("esac", 0), ("case", 0));
        stays("bash", src, ("echo", 0));
    }

    /// A language that closes nothing with a word has no blocks.
    #[test]
    fn none_in_braces() {
        let src = "fn f() { if a { return; } else { loop { break } } }";
        for w in ["fn", "if", "return", "else", "loop", "break"] {
            stays("rust", src, (w, 0));
        }
        let src = "for x in y:\n    pass\nelse:\n    pass\n";
        for w in ["for", "in", "else"] {
            stays("python", src, (w, 0));
        }
    }
}
