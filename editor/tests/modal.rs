//! The modal engine driven by key sequences, no UI anywhere.

use kawoosh_doc::Buffer;
use kawoosh_editor::{Editor, Effect, KeyStroke, Mode, Selection, ViewId};

struct T {
    ed: Editor,
    v: ViewId,
}

impl T {
    fn new(text: &str) -> Self {
        let mut ed = Editor::new();
        let b = ed.add_buffer(Buffer::new("t", text));
        let v = ed.add_view(b);
        Self { ed, v }
    }

    /// Keys in map notation, one per char, `<...>` chords whole. In insert
    /// mode a character arrives as text, the way the shell delivers it.
    fn keys(&mut self, seq: &str) -> &mut Self {
        for k in kawoosh_editor::keymap::parse_notation(seq, " ") {
            let stroke = stroke_of(&k);
            self.ed.key(self.v, stroke);
        }
        self
    }

    fn text(&self) -> String {
        self.ed.buffer_of(self.v).text()
    }

    fn sel(&self) -> Selection {
        self.ed.views[self.v].sels.primary()
    }

    fn head(&self) -> usize {
        self.sel().head
    }

    fn sels(&self) -> Vec<(usize, usize)> {
        self.ed.views[self.v]
            .sels
            .iter()
            .map(|s| (s.anchor, s.head))
            .collect()
    }
}

fn stroke_of(k: &str) -> KeyStroke {
    if let Some(inner) = k.strip_prefix('<').and_then(|s| s.strip_suffix('>')) {
        let mut st = KeyStroke::plain("");
        let mut parts: Vec<&str> = inner.split('-').collect();
        let base = parts.pop().unwrap();
        for m in parts {
            match m {
                "C" => st.ctrl = true,
                "A" => st.alt = true,
                "D" => st.sup = true,
                "S" => st.shift = true,
                _ => {}
            }
        }
        st.code = match base {
            "Esc" => "escape".into(),
            "CR" => "enter".into(),
            "Tab" => "tab".into(),
            "BS" => "backspace".into(),
            "Del" => "delete".into(),
            "Space" => "space".into(),
            "Up" | "Down" | "Left" | "Right" | "Home" | "End" | "PageUp" | "PageDown" => {
                base.to_ascii_lowercase()
            }
            other => other.into(),
        };
        if st.code == "space" {
            st.text = Some(" ".into());
        }
        st
    } else {
        let mut st = KeyStroke::plain(k);
        st.text = Some(k.to_string());
        st
    }
}

#[test]
fn motions_move_the_caret() {
    let mut t = T::new("foo bar baz\nsecond line\n\tthird");
    t.keys("w");
    assert_eq!(t.head(), 4);
    t.keys("2w");
    assert_eq!(t.head(), 12);
    t.keys("$");
    assert_eq!(t.head(), 22); // on the last char, as vim
    t.keys("j");
    assert_eq!(t.head(), 30); // end of "\tthird"
    t.keys("0^");
    assert_eq!(t.head(), 25);
    t.keys("ggG");
    assert_eq!(t.head(), 25);
    t.keys("gg");
    assert_eq!(t.head(), 0);
    t.keys("fb");
    assert_eq!(t.head(), 4);
    t.keys(";");
    t.keys("tz");
    assert_eq!(t.head(), 9);
    t.keys("b");
    assert_eq!(t.head(), 8);
}

#[test]
fn j_keeps_the_goal_column() {
    let mut t = T::new("abcdef\nab\nabcdef");
    t.keys("4l");
    assert_eq!(t.head(), 4);
    t.keys("j");
    assert_eq!(t.head(), 9); // clamped to "ab"
    t.keys("j");
    assert_eq!(t.head(), 14); // back to column 4
}

#[test]
fn insert_and_escape() {
    let mut t = T::new("ab");
    t.keys("ihello<Esc>");
    assert_eq!(t.text(), "helloab");
    assert_eq!(t.ed.mode, Mode::Normal);
    assert_eq!(t.head(), 4); // stepped back onto the 'o'
    t.keys("A!<Esc>");
    assert_eq!(t.text(), "helloab!");
    t.keys("o  x<Esc>");
    assert_eq!(t.text(), "helloab!\n  x");
    t.keys("O<Esc>");
    assert_eq!(t.text(), "helloab!\n  \n  x");
}

#[test]
fn operators_compose_with_motions_and_objects() {
    let mut t = T::new("one two three\nfour");
    t.keys("dw");
    assert_eq!(t.text(), "two three\nfour");
    t.keys("de");
    assert_eq!(t.text(), " three\nfour");
    t.keys("d$");
    assert_eq!(t.text(), "\nfour");
    let mut t = T::new("alpha beta gamma");
    t.keys("wdiw");
    assert_eq!(t.text(), "alpha  gamma");
    let mut t = T::new("f(a, (b)) end");
    t.keys("3ldi(");
    assert_eq!(t.text(), "f() end");
    let mut t = T::new("say \"hi there\" now");
    t.keys("6lci\"bye<Esc>");
    assert_eq!(t.text(), "say \"bye\" now");
    let mut t = T::new("a\nb\nc\nd");
    t.keys("jdd");
    assert_eq!(t.text(), "a\nc\nd");
    t.keys("2dd");
    assert_eq!(t.text(), "a");
    let mut t = T::new("a\nb\nc\nd");
    t.keys("dj");
    assert_eq!(t.text(), "c\nd");
    t.keys("yyp");
    assert_eq!(t.text(), "c\nc\nd");
    t.keys("x");
    assert_eq!(t.text(), "c\n\nd");
}

#[test]
fn undo_redo_are_per_command_and_per_insert_session() {
    let mut t = T::new("abc");
    t.keys("x");
    assert_eq!(t.text(), "bc");
    t.keys("ione two<Esc>");
    assert_eq!(t.text(), "one twobc");
    t.keys("u");
    assert_eq!(t.text(), "bc");
    t.keys("u");
    assert_eq!(t.text(), "abc");
    t.keys("u");
    assert_eq!(t.text(), "abc");
    t.keys("<C-r>");
    assert_eq!(t.text(), "bc");
    t.keys("U");
    assert_eq!(t.text(), "one twobc");
}

#[test]
fn visual_mode_selects_and_operates() {
    let mut t = T::new("hello world");
    t.keys("vllld");
    assert_eq!(t.text(), "o world");
    assert_eq!(t.ed.mode, Mode::Normal);
    let mut t = T::new("a\nb\nc");
    t.keys("Vjd");
    assert_eq!(t.text(), "c");
    let mut t = T::new("one two");
    t.keys("wviwy");
    assert_eq!(t.ed.registers[&'"'], "two");
    assert!(
        t.ed.take_effects()
            .contains(&Effect::SetClipboard("two".into()))
    );
}

#[test]
fn multicursor_edits_every_selection() {
    let mut t = T::new("aa\nbb\ncc");
    t.keys("<A-j><A-j>");
    assert_eq!(t.sels().len(), 3);
    t.keys("ix<Esc>");
    assert_eq!(t.text(), "xaa\nxbb\nxcc");
    t.keys("A!<Esc>");
    assert_eq!(t.text(), "xaa!\nxbb!\nxcc!");
    t.keys("0dw");
    assert_eq!(t.text(), "!\n!\n!");
    t.keys(",");
    assert_eq!(t.sels().len(), 1);
}

#[test]
fn counts_and_paste() {
    let mut t = T::new("abc\n");
    t.keys("3x");
    assert_eq!(t.text(), "\n");
    t.keys("u");
    t.keys("yyp");
    assert_eq!(t.text(), "abc\nabc\n");
    t.keys("ggylP");
    assert_eq!(t.text(), "aabc\nabc\n");
    let mut t = T::new("ab");
    t.keys("lrx");
    assert_eq!(t.text(), "ax");
    t.keys("J");
    let mut t = T::new("a\n  b\nc");
    t.keys("JJ");
    assert_eq!(t.text(), "a b c");
}

#[test]
fn command_line_and_search() {
    let mut t = T::new("one\ntwo\nthree\ntwo again");
    t.keys(":3<CR>");
    assert_eq!(t.head(), 8);
    t.keys("gg/two<CR>");
    assert_eq!(t.head(), 4);
    t.keys("n");
    assert_eq!(t.head(), 14);
    t.keys("n");
    assert_eq!(t.head(), 4);
    assert!(t.ed.message.contains("wrapped"));
    t.keys("N");
    assert_eq!(t.head(), 14);
    t.keys(":q<CR>");
    assert!(t.ed.take_effects().contains(&Effect::Quit));
    t.keys("x:q<CR>");
    assert!(!t.ed.take_effects().contains(&Effect::Quit));
    t.keys(":q!<CR>");
    assert!(t.ed.take_effects().contains(&Effect::Quit));
    t.keys(":nonsense a b<CR>");
    assert!(matches!(
        t.ed.take_effects().as_slice(),
        [Effect::Shell { name, args, .. }] if name == "nonsense" && args == &["a", "b"]
    ));
    t.keys(":set tabstop=2<CR>");
    assert_eq!(t.ed.tabstop(), 2);
}

#[test]
fn indent_and_change_line() {
    let mut t = T::new("a\nb");
    t.keys(">>");
    assert_eq!(t.text(), "    a\nb");
    t.keys("<lt><lt>");
    assert_eq!(t.text(), "a\nb");
    t.keys("Vj>");
    assert_eq!(t.text(), "    a\n    b");
    t.keys("ccz<Esc>");
    assert_eq!(t.text(), "    z\n    b");
    t.keys("jC!<Esc>");
    assert_eq!(t.text(), "    z\n    !");
}

#[test]
fn insert_mode_keys() {
    let mut t = T::new("");
    t.keys("iab<CR>cd<BS><Tab>x<Esc>");
    assert_eq!(t.text(), "ab\nc    x");
    t.keys("o  y<CR>z<Esc>");
    assert_eq!(t.text(), "ab\nc    x\n  y\n  z");
    let mut t = T::new("foo bar");
    t.keys("A<C-w><Esc>");
    assert_eq!(t.text(), "foo ");
}
