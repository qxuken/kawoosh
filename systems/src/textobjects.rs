//! Text objects read off a syntax tree (docs/design/nodes.md Decision
//! 8): a grammar's `textobjects.scm` run over the bytes asked about,
//! each match an object's around and inside as far as it captured them.
//! Which one a key takes is the engine's (`kawoosh_editor`'s
//! `pick_object`); this says what is there.
//!
//! Pure: a tree, its text, the query. The shell keeps the trees
//! (`kawoosh::indent`).

use std::ops::Range;

use kawoosh_languages::{Part, TextObjects};
use tree_sitter::{Node, QueryCursor, StreamingIterator, Tree};

/// One match's object: its around and its inside, where it had them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub around: Option<Range<usize>>,
    pub inside: Option<Range<usize>>,
}

/// The objects named `object` (`function`, `class`) one of whose parts
/// is over `within`, one a match, in the order the query found them.
///
/// Nodes one match captures under one part are one range, the first's
/// start to the last's end: a run of line comments, a parameter and the
/// `,` after it. An inside that is one bracketed node — a block's `{`
/// … `}`, a list's `(` … `)` — is what is between the brackets, from
/// its first child to its last, as nvim-treesitter-textobjects' queries
/// make it with `#make-range!`: `dif` empties a body and keeps its
/// braces, which helix's queries, capturing the block, would not.
pub fn find(
    t: &TextObjects,
    tree: &Tree,
    text: &text_buffer::Buffer,
    object: &str,
    within: Range<usize>,
) -> Vec<Found> {
    let parts: Vec<Option<Part>> = t
        .captures
        .iter()
        .map(|c| c.as_ref().filter(|(o, _)| o == object).map(|(_, p)| *p))
        .collect();
    let made = t.made.iter().any(|m| m.iter().any(|m| m.object == object));
    if !made && parts.iter().all(Option::is_none) {
        return Vec::new();
    }
    let mut cursor = QueryCursor::new();
    // An empty range is no range to tree-sitter (an end of 0 is the
    // text's end): a byte's worth, its nodes over it as well.
    let end = within.end.max(within.start + 1);
    cursor.set_byte_range(within.start..end);
    let mut node_text = |n: Node| std::iter::once(text.collect_range(n.byte_range()));
    let mut it = cursor.matches(&t.query, tree.root_node(), &mut node_text);
    let mut out = Vec::new();
    while let Some(m) = it.next() {
        let mut f = Found::default();
        let mut inside: Vec<Node> = Vec::new();
        for c in m.captures() {
            match parts[c.index as usize] {
                Some(Part::Around) => grow(&mut f.around, c.node.byte_range()),
                Some(Part::Inside) => inside.push(c.node),
                None => {}
            }
        }
        f.inside = match inside.as_slice() {
            [] => None,
            [one] => Some(between_brackets(*one, text)),
            [first, .., last] => Some(first.start_byte()..last.end_byte()),
        };
        for r in t.made[m.pattern_index]
            .iter()
            .filter(|r| r.object == object)
        {
            let node = |i: u32| {
                m.captures()
                    .iter()
                    .filter(move |c| c.index == i)
                    .map(|c| c.node)
            };
            let Some(from) = node(r.from).next() else {
                continue;
            };
            // An optional end not there: the start's own.
            let to = node(r.to).next_back().unwrap_or(from);
            let range = from.start_byte()..to.end_byte().max(from.end_byte());
            match r.part {
                Part::Around => grow(&mut f.around, range),
                Part::Inside => grow(&mut f.inside, range),
            }
        }
        if f.around.is_some() || f.inside.is_some() {
            out.push(f);
        }
    }
    out
}

/// `r` taken into `into`: the range from the first's start to the
/// last's end.
fn grow(into: &mut Option<Range<usize>>, r: Range<usize>) {
    *into = Some(match into.take() {
        Some(had) => had.start.min(r.start)..had.end.max(r.end),
        None => r,
    });
}

/// A node between its brackets — its first child a `{` `(` `[` token
/// and its last the closer (a closer the parser guessed is not one) —
/// from the first child after the opener to the last before the closer,
/// or the empty range between them — the blanks at either end left out
/// (go's statement list ends with its newline); any other node whole.
fn between_brackets(n: Node, text: &text_buffer::Buffer) -> Range<usize> {
    let count = n.child_count();
    if count >= 2
        && let (Some(open), Some(close)) = (n.child(0), n.child(count - 1))
        && matches!(
            (open.kind(), close.kind()),
            ("{", "}") | ("(", ")") | ("[", "]")
        )
        && !open.is_named()
        && !close.is_named()
        && !close.is_missing()
    {
        return match (n.child(1), n.child(count - 2)) {
            (Some(first), Some(last)) if count > 2 => {
                let (a, b) = (first.start_byte(), last.end_byte());
                let bytes = text.collect_range(a..b);
                let lead = bytes.iter().take_while(|c| c.is_ascii_whitespace()).count();
                let trail = bytes[lead..]
                    .iter()
                    .rev()
                    .take_while(|c| c.is_ascii_whitespace())
                    .count();
                a + lead..b - trail
            }
            _ => open.end_byte()..close.start_byte(),
        };
    }
    n.byte_range()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kawoosh_languages::{Grammar, LANGUAGES};

    fn grammar(name: &str) -> Grammar {
        let l = LANGUAGES.iter().find(|l| l.name == name).unwrap();
        (l.grammar.unwrap())().unwrap()
    }

    /// Each object `object` of `src` in `language` over the whole text,
    /// as the text of its around and of its inside, sorted.
    fn objects(language: &str, src: &str, object: &str) -> Vec<(String, String)> {
        let g = grammar(language);
        let t = g
            .textobjects
            .as_ref()
            .unwrap_or_else(|| panic!("{language}: no text objects"));
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&g.language).unwrap();
        let tree = parser.parse(src, None).unwrap();
        let text = text_buffer::Buffer::with_text(src.as_bytes());
        let cut =
            |r: &Option<Range<usize>>| r.clone().map_or(String::new(), |r| src[r].to_string());
        let mut out: Vec<(String, String)> = find(t, &tree, &text, object, 0..src.len())
            .iter()
            .map(|f| (cut(&f.around), cut(&f.inside)))
            .collect();
        out.sort();
        out.dedup();
        out
    }

    fn has(got: &[(String, String)], around: &str, inside: &str) -> bool {
        got.iter().any(|(a, i)| a == around && i == inside)
    }

    /// Every query shipped finds what its language calls a function, a
    /// class, an argument, a comment, an entry on a small sample — the
    /// test that a query and the grammar revision pinned agree on the
    /// nodes' names beyond compiling.
    #[test]
    fn each_shipped_query_finds_its_objects() {
        let rs = "// one\n// two\nstruct S { a: u8 }\nfn f(a: u8, b: u8) -> u8 {\n    a\n}\n";
        let got = objects("rust", rs, "function");
        assert!(
            has(&got, "fn f(a: u8, b: u8) -> u8 {\n    a\n}", "a"),
            "{got:?}"
        );
        let got = objects("rust", rs, "class");
        assert!(has(&got, "struct S { a: u8 }", "a: u8"), "{got:?}");
        let got = objects("rust", rs, "parameter");
        assert!(
            has(&got, "a: u8,", "a: u8") && has(&got, "b: u8", "b: u8"),
            "{got:?}"
        );
        let got = objects("rust", rs, "comment");
        assert!(
            has(&got, "// one\n// two", ""),
            "a run of line comments: {got:?}"
        );
        assert!(
            !has(&got, "// one", "") && !has(&got, "// two", ""),
            "{got:?}"
        );

        let js = "// c\nclass A { m(x) { return x } }\nfunction g(a, b) { return 1 }\n";
        for lang in ["javascript", "typescript", "tsx"] {
            let got = objects(lang, js, "function");
            assert!(
                has(&got, "function g(a, b) { return 1 }", "return 1"),
                "{lang}: {got:?}"
            );
            assert!(
                has(&got, "m(x) { return x }", "return x"),
                "{lang}: {got:?}"
            );
            let got = objects(lang, js, "class");
            assert!(
                has(&got, "class A { m(x) { return x } }", "m(x) { return x }"),
                "{lang}: {got:?}"
            );
            let got = objects(lang, js, "parameter");
            assert!(has(&got, "a,", "a"), "{lang}: {got:?}");
        }
        let got = objects("typescript", "interface I { a: number }\n", "class");
        assert!(
            has(&got, "interface I { a: number }", "a: number"),
            "{got:?}"
        );

        let go = "package p\n// c\ntype T struct {\n\tA int\n}\nfunc F(a int, b int) int {\n\treturn a\n}\n";
        let got = objects("go", go, "function");
        assert!(
            has(
                &got,
                "func F(a int, b int) int {\n\treturn a\n}",
                "return a"
            ),
            "{got:?}"
        );
        let got = objects("go", go, "class");
        assert!(has(&got, "type T struct {\n\tA int\n}", "A int"), "{got:?}");
        let got = objects("go", go, "parameter");
        assert!(has(&got, "a int,", "a int"), "{got:?}");

        let lua = "-- c\nlocal function f(a, b)\n  return a\nend\n";
        let got = objects("lua", lua, "function");
        assert!(
            has(&got, "local function f(a, b)\n  return a\nend", "return a"),
            "{got:?}"
        );
        let got = objects("lua", lua, "parameter");
        assert!(has(&got, "a,", "a"), "{got:?}");
        assert!(!objects("lua", lua, "comment").is_empty());

        let sh = "# c\nf() {\n  echo $1\n}\n";
        let got = objects("bash", sh, "function");
        assert!(has(&got, "f() {\n  echo $1\n}", "echo $1"), "{got:?}");
        assert!(!objects("bash", sh, "comment").is_empty());

        let c = "/* c */\nstruct S { int a; };\nint f(int a, int b) {\n  return a;\n}\n";
        for lang in ["c", "cpp"] {
            let got = objects(lang, c, "function");
            assert!(
                has(&got, "int f(int a, int b) {\n  return a;\n}", "return a;"),
                "{lang}: {got:?}"
            );
            let got = objects(lang, c, "class");
            assert!(
                has(&got, "struct S { int a; }", "int a;"),
                "{lang}: {got:?}"
            );
            let got = objects(lang, c, "parameter");
            assert!(has(&got, "int a,", "int a"), "{lang}: {got:?}");
        }
        let got = objects("cpp", "class K { int a; };\n", "class");
        assert!(has(&got, "class K { int a; }", "int a;"), "{got:?}");

        let py = "# c\nclass K:\n    def m(self, a):\n        return a\n";
        let got = objects("python", py, "function");
        assert!(
            has(&got, "def m(self, a):\n        return a", "return a"),
            "{got:?}"
        );
        let got = objects("python", py, "class");
        assert!(
            has(
                &got,
                "class K:\n    def m(self, a):\n        return a",
                "def m(self, a):\n        return a"
            ),
            "{got:?}"
        );
        let got = objects("python", py, "parameter");
        assert!(has(&got, "self,", "self"), "{got:?}");

        let got = objects("json", "{\"a\": [1, 2]}", "entry");
        assert!(has(&got, "\"a\": [1, 2]", "\"a\""), "{got:?}");
        assert!(has(&got, "1", ""), "{got:?}");
        let got = objects("jsonc", "{\"a\": 1}", "entry");
        assert!(has(&got, "\"a\": 1", "\"a\""), "{got:?}");
        let got = objects("toml", "# c\na = [1, 2]\n", "entry");
        assert!(has(&got, "a = [1, 2]", "a"), "{got:?}");
        assert!(!objects("toml", "# c\na = 1\n", "comment").is_empty());
        let got = objects("yaml", "# c\na: 1\n", "entry");
        assert!(has(&got, "a: 1", "a"), "{got:?}");
        assert!(!objects("yaml", "# c\na: 1\n", "comment").is_empty());
        assert!(!objects("sql", "-- c\nselect 1;\n", "comment").is_empty());

        let nu = "# c\ndef f [a: int] {\n  $a\n}\n";
        let got = objects("nu", nu, "function");
        // nvim's way: the around and the inside each a pattern of its
        // own, paired by the engine.
        assert!(
            has(&got, "def f [a: int] {\n  $a\n}", "") && has(&got, "", "$a"),
            "{got:?}"
        );
        assert!(!objects("nu", nu, "comment").is_empty());
        assert!(!objects("nu", nu, "parameter").is_empty());
    }

    /// Only what is over the bytes asked about: a function before them
    /// is not found, the one around them is.
    #[test]
    fn only_the_objects_over_the_range() {
        let src = "fn a() {}\nfn b() {\n    x();\n}\n";
        let g = grammar("rust");
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&g.language).unwrap();
        let tree = parser.parse(src, None).unwrap();
        let text = text_buffer::Buffer::with_text(src.as_bytes());
        let x = src.find("x()").unwrap();
        let got = find(
            g.textobjects.as_ref().unwrap(),
            &tree,
            &text,
            "function",
            x..x + 1,
        );
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(
            got[0].around,
            Some(src.find("fn b").unwrap()..src.len() - 1)
        );
        assert_eq!(got[0].inside, Some(x..x + "x();".len()));
        // An empty body: the empty range between its braces.
        let got = find(
            g.textobjects.as_ref().unwrap(),
            &tree,
            &text,
            "function",
            3..4,
        );
        assert_eq!(got[0].inside, Some(8..8));
        // An object the query does not name: nothing, and no error.
        assert!(find(g.textobjects.as_ref().unwrap(), &tree, &text, "nope", 0..1).is_empty());
    }

    /// nvim-treesitter-textobjects' spelling reads as helix's, and its
    /// `#make-range!` names an object from two captures.
    #[test]
    fn nvim_s_spelling_and_make_range() {
        let g = grammar("rust");
        let q = r#"
(function_item) @function.outer
(function_item body: (block . "{" . (_) @_start @_end (_)? @_end . "}"
  (#make-range! "function.inner" @_start @_end)))
"#;
        let t = TextObjects::new(&g.language, q).unwrap();
        assert_eq!(t.objects(), ["function"]);
        let src = "fn f() {\n    a();\n    b();\n}\n";
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&g.language).unwrap();
        let tree = parser.parse(src, None).unwrap();
        let text = text_buffer::Buffer::with_text(src.as_bytes());
        let got = find(&t, &tree, &text, "function", 0..src.len());
        let inner = got.iter().find_map(|f| f.inside.clone()).unwrap();
        assert_eq!(&src[inner], "a();\n    b();");
    }
}
