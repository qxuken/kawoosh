//! Replacing a project search's matches in its results
//! (docs/design/search.md Decision 12): every match the excerpts show,
//! or the one at the caret, as one change the sources each take as one
//! state — a header naming the pattern left alone.

use kawoosh_doc::{Buffer, BufferId};
use kawoosh_editor::replace::{Find, Replace, Replaced};
use kawoosh_editor::{Editor, Part, Selection, Selections, ViewId};

struct T {
    ed: Editor,
    v: ViewId,
    m: BufferId,
    a: BufferId,
    b: BufferId,
}

/// Two files under headers that say the pattern too.
fn two() -> T {
    let mut ed = Editor::new();
    let a = ed.add_buffer(Buffer::new("a", "x\nfoo = foo\nkeep foo\nend\n"));
    let b = ed.add_buffer(Buffer::new("b", "foo_bar()\nlast foo"));
    let m = ed.open_multi(
        "*search*",
        vec![
            Part::Gap("a foo  3\n".into()),
            Part::Lines(a, 1..3),
            Part::Gap("\nb foo  2\n".into()),
            Part::Lines(b, 0..2),
        ],
    );
    let v = ed.add_view(m);
    T { ed, v, m, a, b }
}

fn find(p: &str) -> Find {
    Find {
        pattern: p.into(),
        ..Default::default()
    }
}

impl T {
    fn text(&self, id: BufferId) -> String {
        self.ed.buffers[id].text()
    }
    fn replace(&mut self, r: Replace) -> Option<Replaced> {
        self.ed.multi_replace(self.m, self.v, &r)
    }
    fn keys(&mut self, seq: &str) {
        for k in kawoosh_editor::keymap::parse_notation(seq) {
            self.ed.key(self.v, kawoosh_editor::KeyStroke::plain(&k));
        }
    }
}

#[test]
fn replace_all_is_one_change_and_leaves_the_headers() {
    let mut t = two();
    let done = t.replace(Replace {
        find: find("foo"),
        with: "baz".into(),
        ..Default::default()
    });
    assert_eq!(
        done,
        Some(Replaced {
            matches: 5,
            files: 2,
            read_only: 0
        })
    );
    assert_eq!(t.text(t.a), "x\nbaz = baz\nkeep baz\nend\n");
    assert_eq!(t.text(t.b), "baz_bar()\nlast baz");
    assert_eq!(
        t.text(t.m),
        "a foo  3\nbaz = baz\nkeep baz\n\nb foo  2\nbaz_bar()\nlast baz\n",
        "the headers are the plugin's text, not a file's"
    );
    assert!(
        t.ed.message.contains("5 matches replaced in 2 files"),
        "{}",
        t.ed.message
    );
    // One `u` in the results: both files back.
    t.keys("u");
    assert_eq!(t.text(t.a), "x\nfoo = foo\nkeep foo\nend\n");
    assert_eq!(t.text(t.b), "foo_bar()\nlast foo");
    // And `<C-r>` forward again.
    let mut redo = kawoosh_editor::KeyStroke::plain("r");
    redo.ctrl = true;
    t.ed.key(t.v, redo);
    assert_eq!(t.text(t.b), "baz_bar()\nlast baz");
}

#[test]
fn a_regex_replacement_takes_the_groups() {
    let mut t = two();
    t.replace(Replace {
        find: Find {
            pattern: r"(\w+)_(\w+)\(\)".into(),
            regex: true,
            ..Default::default()
        },
        with: r"${2}_$1()\tdone".into(),
        ..Default::default()
    });
    assert_eq!(t.text(t.b), "bar_foo()\tdone\nlast foo");
    // A literal find takes the field's text as it is.
    let mut t = two();
    t.replace(Replace {
        find: find("foo_bar()"),
        with: "$1".into(),
        ..Default::default()
    });
    assert_eq!(t.text(t.b), "$1\nlast foo");
}

#[test]
fn the_lines_a_drop_stage_left_out_keep_their_matches() {
    let mut t = two();
    // `foo › drop keep`: the line saying `keep` was dropped, though the
    // results show it as another's context.
    t.replace(Replace {
        find: find("foo"),
        with: "X".into(),
        lines: vec![(find("keep"), false)],
        ..Default::default()
    });
    assert_eq!(t.text(t.a), "x\nX = X\nkeep foo\nend\n");
    assert_eq!(t.text(t.b), "X_bar()\nlast X");
    // `foo › keep last`: only those.
    let mut t = two();
    t.replace(Replace {
        find: find("foo"),
        with: "X".into(),
        lines: vec![(find("last"), true)],
        ..Default::default()
    });
    assert_eq!(t.text(t.a), "x\nfoo = foo\nkeep foo\nend\n");
    assert_eq!(t.text(t.b), "foo_bar()\nlast X");
}

#[test]
fn replace_one_takes_the_match_at_the_caret_and_moves_to_the_next() {
    let mut t = two();
    // The caret on the second `foo` of `foo = foo`.
    let at = t.text(t.m).find("= foo").unwrap() + 2;
    t.ed.views[t.v].sels = Selections::single(Selection::point(at));
    let one = Replace {
        find: find("foo"),
        with: "Y".into(),
        one: true,
        ..Default::default()
    };
    t.replace(one.clone());
    assert_eq!(t.text(t.a), "x\nfoo = Y\nkeep foo\nend\n");
    let head = t.ed.views[t.v].sels.primary().head;
    assert_eq!(
        head,
        t.text(t.m).find("keep foo").unwrap() + 5,
        "on the next match"
    );
    assert!(t.ed.message.contains("4 matches left"), "{}", t.ed.message);
    // From the last match, round to the first.
    let at = t.text(t.m).find("last foo").unwrap() + 5;
    t.ed.views[t.v].sels = Selections::single(Selection::point(at));
    t.replace(one.clone());
    assert_eq!(t.text(t.b), "foo_bar()\nlast Y");
    let head = t.ed.views[t.v].sels.primary().head;
    assert_eq!(
        head,
        t.text(t.m).find("foo = Y").unwrap(),
        "round to the first"
    );
    // Each a change of its own: `u` takes the last back alone.
    t.keys("u");
    assert_eq!(t.text(t.b), "foo_bar()\nlast foo");
    assert_eq!(t.text(t.a), "x\nfoo = Y\nkeep foo\nend\n");
}

#[test]
fn nothing_to_replace_says_so_and_read_only_files_are_left() {
    let mut t = two();
    assert_eq!(
        t.replace(Replace {
            find: find("absent"),
            with: "z".into(),
            ..Default::default()
        }),
        None
    );
    assert!(t.ed.message.contains("no match"), "{}", t.ed.message);
    t.ed.buffers[t.b].read_only = true;
    let done = t.replace(Replace {
        find: find("foo"),
        with: "z".into(),
        ..Default::default()
    });
    assert_eq!(
        done,
        Some(Replaced {
            matches: 3,
            files: 1,
            read_only: 2
        })
    );
    assert_eq!(t.text(t.b), "foo_bar()\nlast foo");
    assert!(
        t.ed.message.contains("2 in read-only files left"),
        "{}",
        t.ed.message
    );
}

#[test]
fn a_crlf_file_keeps_its_line_ends() {
    let mut ed = Editor::new();
    let a = ed.add_buffer(Buffer::new("a", "one end\r\ntwo end\r\n"));
    let m = ed.open_multi(
        "*search*",
        vec![Part::Gap("a\n".into()), Part::Lines(a, 0..2)],
    );
    let v = ed.add_view(m);
    ed.multi_replace(
        m,
        v,
        &Replace {
            find: Find {
                pattern: "end$".into(),
                regex: true,
                ..Default::default()
            },
            with: "fin".into(),
            ..Default::default()
        },
    );
    assert_eq!(ed.buffers[a].text(), "one fin\r\ntwo fin\r\n");
}

#[test]
fn a_line_two_excerpts_show_is_replaced_once() {
    let mut ed = Editor::new();
    let a = ed.add_buffer(Buffer::new("a", "k0\nk1 k\nk2\n"));
    let m = ed.open_multi(
        "*search*",
        vec![
            Part::Gap("a\n".into()),
            Part::Lines(a, 0..2),
            Part::Gap("⋯\n".into()),
            Part::Lines(a, 1..3),
        ],
    );
    let v = ed.add_view(m);
    let done = ed.multi_replace(
        m,
        v,
        &Replace {
            find: find("k"),
            with: "kk".into(),
            ..Default::default()
        },
    );
    // The file's text, each line once. (What the second excerpt then
    // shows is the sync's: overlapping excerpts, which the search never
    // makes — its runs that meet are one — are carried loosely.)
    assert_eq!(done.map(|d| d.matches), Some(4));
    assert_eq!(ed.buffers[a].text(), "kk0\nkk1 kk\nkk2\n");
}

/// A pattern that matches nothing — `x*` — matches between characters,
/// not inside one: a character of two bytes takes its replacement once.
#[test]
fn an_empty_match_is_between_characters() {
    let mut ed = Editor::new();
    let a = ed.add_buffer(Buffer::new("a", "αβ\n"));
    let m = ed.open_multi(
        "*search*",
        vec![Part::Gap("a\n".into()), Part::Lines(a, 0..1)],
    );
    let v = ed.add_view(m);
    let done = ed.multi_replace(
        m,
        v,
        &Replace {
            find: Find {
                pattern: "x*".into(),
                regex: true,
                ..Default::default()
            },
            with: "-".into(),
            ..Default::default()
        },
    );
    assert_eq!(ed.buffers[a].text(), "-α-β-\n");
    assert_eq!(done.map(|d| d.matches), Some(3));
}
