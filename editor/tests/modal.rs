//! The modal engine driven by key sequences, no UI anywhere.

use kawoosh_doc::Buffer;
use kawoosh_editor::{
    ArgKind, Args, Editor, Effect, KeyStroke, Lookup, Mode, Selection, Spec, Step, ViewId,
};

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
        for k in kawoosh_editor::keymap::parse_notation(seq) {
            let stroke = stroke_of(&k);
            self.ed.key(self.v, stroke);
        }
        self
    }

    /// The prompt's line, empty when no prompt is open.
    fn cmdline(&self) -> String {
        self.ed.prompt_text().unwrap_or_default()
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
    assert_eq!(t.ed.mode(t.v), Mode::Normal);
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

/// A file of CRLF lines — a Windows checkout's — loses whole lines to a
/// linewise operator, `\r\n` and all, and the last line takes the break
/// before it.
#[test]
fn linewise_operators_take_a_crlf_break_whole() {
    let text = "h\r\n\r\npara\r\n\r\n## A";
    let mut t = T::new(text);
    t.keys("jjdd");
    assert_eq!(t.text(), "h\r\n\r\n\r\n## A");
    let mut t = T::new(text);
    t.keys("jjdj");
    assert_eq!(t.text(), "h\r\n\r\n## A");
    let mut t = T::new(text);
    t.keys("jjjdk");
    assert_eq!(t.text(), "h\r\n\r\n## A");
    // The last line: the break before it goes, and `p` on the blank
    // line now last puts it back below, a line.
    let mut t = T::new(text);
    t.keys("Gdd");
    assert_eq!(t.text(), "h\r\n\r\npara\r\n");
    t.keys("p");
    assert_eq!(t.text(), text);
    // A line yanked and put below, and below the last.
    let mut t = T::new(text);
    t.keys("jjyyp");
    assert_eq!(t.text(), "h\r\n\r\npara\r\npara\r\n\r\n## A");
    t.keys("Gp");
    assert_eq!(t.text(), "h\r\n\r\npara\r\npara\r\n\r\n## A\r\npara");
    assert_eq!(t.head(), t.text().len() - 4, "on the put line");
}

/// `r` as neovim's (checked there headless): COUNT characters become
/// CHAR, the caret on the last, nothing when the line is short of them;
/// `r<CR>` makes the lot one line break at the line's indent, the blanks
/// after it gone and the caret stepped back as `<Esc>` steps it.
#[test]
fn replace_char_takes_a_count_and_a_line_break() {
    let mut t = T::new("abcdef");
    t.keys("l3rx");
    assert_eq!((t.text().as_str(), t.head()), ("axxxef", 3));
    t.keys("0l9ry");
    assert_eq!(t.text(), "axxxef", "short of the count: nothing");

    let mut t = T::new("foo bar");
    t.keys("3lr<CR>");
    assert_eq!((t.text().as_str(), t.head()), ("foo\nbar", 4));
    t.keys("u");
    assert_eq!(t.text(), "foo bar", "one undo");

    let mut t = T::new("    foo bar");
    t.keys("7lr<CR>");
    assert_eq!(t.text(), "    foo\n    bar");
    assert_eq!(t.head(), 8 + 3, "on the indent's last blank");

    let mut t = T::new("foo  bar");
    t.keys("3lr<CR>");
    assert_eq!(t.text(), "foo\nbar", "the blanks after it go");
    let mut t = T::new("foo  bar");
    t.keys("4lr<CR>");
    assert_eq!(t.text(), "foo \nbar", "the blanks before it stay");

    let mut t = T::new("abcdef");
    t.keys("l3r<CR>");
    assert_eq!((t.text().as_str(), t.head()), ("a\nef", 2));
    let mut t = T::new("abc");
    t.keys("2lr<CR>");
    assert_eq!(t.text(), "ab\n", "the line's last character");

    let mut t = T::new("a b c d");
    t.keys("lr<CR>j0l.");
    assert_eq!(t.text(), "a\nb\nc d", "`.` again");
}

/// Insert's `<Tab>` and `r<Tab>` count columns as the pane draws them:
/// a wide character two cells, a combining one none, a control its
/// escape's (`^A` two).
#[test]
fn tab_stops_count_the_cells_drawn() {
    let mut t = T::new("日本");
    t.keys("A<Tab>");
    assert_eq!(t.text(), "日本    ", "from cell 4 to 8");
    let mut t = T::new("日x");
    t.keys("lr<Tab>");
    assert_eq!(t.text(), "日  ", "from cell 2 to 4");
    let mut t = T::new("\u{1}x");
    t.keys("lr<Tab>");
    assert_eq!(t.text(), "\u{1}  ", "after `^A`, from cell 2");
    let mut t = T::new("e\u{301}x");
    t.keys("lr<Tab>");
    assert_eq!(t.text(), "e\u{301}   ", "the accent takes no cell");
}

/// Several carets on a line under `expandtab`: each tab reaches a stop
/// from where the carets before it leave it, not from where it stood
/// before they typed.
#[test]
fn tab_stops_of_carets_on_one_line_count_the_ones_before() {
    let carets = |t: &mut T| {
        t.ed.views[t.v].sels = kawoosh_editor::Selections {
            items: vec![Selection::point(1), Selection::point(3)],
            primary: 0,
        };
    };
    let mut t = T::new("abcd");
    t.keys(":set shiftwidth=4<CR>i");
    carets(&mut t);
    t.keys("<Tab>");
    assert_eq!(t.text(), "a   bc  d", "the second from cell 6 to 8");
    let mut t = T::new("abcd");
    t.keys(":set shiftwidth=4<CR>");
    carets(&mut t);
    t.keys("r<Tab>");
    assert_eq!(t.text(), "a   c   ", "the second from cell 5 to 8");
}

/// `r` stops where the line's text does, however the line ends: a
/// `\r\n` line's `\r` is not a character to replace (neovim's `3rx` on
/// `ab` fails there too).
#[test]
fn replace_char_stops_short_of_a_crlf_break() {
    let mut t = T::new("ab\r\ncd");
    t.keys("3rx");
    assert_eq!(t.text(), "ab\r\ncd", "short of the count: nothing");
    t.keys("2rx");
    assert_eq!(t.text(), "xx\r\ncd");
}

/// A field is one line: `r<CR>` there changes nothing.
#[test]
fn replace_char_line_break_in_a_field_changes_nothing() {
    let mut t = T::new("");
    t.keys(":foo bar<Esc>0f<Space>r<CR>");
    assert!(t.ed.prompt_view().is_some());
    assert_eq!(t.cmdline(), "foo bar");
    t.keys("rx");
    assert_eq!(t.cmdline(), "fooxbar", "`r` itself still works there");
}

/// `r<Tab>` as neovim's: what insert's `<Tab>` puts, per character — a
/// tab, or under `expandtab` the spaces to the next `shiftwidth` stop,
/// each from where the last left; the caret on the last.
#[test]
fn replace_char_with_a_tab_is_insert_tab() {
    let mut t = T::new("abcdef");
    t.keys(":set shiftwidth=8<CR>r<Tab>");
    assert_eq!((t.text().as_str(), t.head()), ("        bcdef", 7));
    let mut t = T::new("abcdef");
    t.keys(":set shiftwidth=8<CR>l2r<Tab>");
    assert_eq!(t.text(), format!("a{}def", " ".repeat(15)), "to 8, then 16");
    assert_eq!(t.head(), 15);
    let mut t = T::new("abcdef");
    t.keys("l2r<Tab>");
    assert_eq!(t.text(), format!("a{}def", " ".repeat(7)), "stops of 4");

    let mut t = T::new("abcdef");
    t.keys(":set -expandtab<CR>l2r<Tab>");
    assert_eq!((t.text().as_str(), t.head()), ("a\t\tdef", 2));
    t.keys("u");
    assert_eq!(t.text(), "abcdef", "one undo");
    t.keys("$.");
    assert_eq!(t.text(), "abcdef", "`.` short of its count");
    t.keys("0.");
    assert_eq!(t.text(), "\t\tcdef", "`.` again");
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

/// The buffer is modified by what its text is, not by what happened:
/// an undo back to the text that was loaded, or written, is clean;
/// a redo away from it is not; a write in the middle of the history
/// moves the clean point there.
#[test]
fn undo_back_to_the_saved_text_is_clean() {
    let mut t = T::new("abc");
    assert!(!t.ed.buffer_of(t.v).modified);
    t.keys("x");
    assert!(t.ed.buffer_of(t.v).modified);
    t.keys("ione<Esc>");
    t.keys("u");
    assert!(
        t.ed.buffer_of(t.v).modified,
        "one step back is still off the file"
    );
    t.keys("u");
    assert_eq!(t.text(), "abc");
    assert!(!t.ed.buffer_of(t.v).modified, "back to what was loaded");
    t.keys("<C-r>");
    assert_eq!(t.text(), "bc");
    assert!(t.ed.buffer_of(t.v).modified);
    // Written here: this text is the clean one now, and the loaded
    // text no longer is.
    let dir = std::env::temp_dir().join(format!("kawoosh-modal-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.txt");
    t.keys(&format!(":w {}<CR>", file.display()));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "bc");
    assert!(!t.ed.buffer_of(t.v).modified);
    t.keys("u");
    assert_eq!(t.text(), "abc");
    assert!(t.ed.buffer_of(t.v).modified);
    t.keys("<C-r>");
    assert_eq!(t.text(), "bc");
    assert!(!t.ed.buffer_of(t.v).modified);
    t.keys("<C-r>");
    assert_eq!(t.text(), "onebc");
    assert!(t.ed.buffer_of(t.v).modified);
    // Typing the text back by hand is an edit, not an undo: vim's rule,
    // since the history has moved on.
    t.keys("u");
    assert!(!t.ed.buffer_of(t.v).modified);
    t.keys("ione<Esc>u");
    assert!(!t.ed.buffer_of(t.v).modified);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The history as a list of states: each past the root carries the
/// change that made it — its line, its bytes, its text clipped — and
/// its parent; the text now is marked, so is the saved one, and a seek
/// walks the tree to any of them.
#[test]
fn history_lists_the_states_and_seeks_among_them() {
    let mut t = T::new("abc\ndef");
    let b = t.ed.views[t.v].buffer;
    assert!(t.ed.history(b).is_empty(), "nothing until a command runs");
    t.keys("x");
    t.keys("jione two<Esc>");
    let rows = t.ed.history(b);
    assert_eq!(rows.len(), 3);
    assert!(rows[0].change.is_none() && rows[0].parent.is_none());
    assert_eq!((rows[0].seq, rows[0].at), (0, None));
    assert_eq!(t.ed.history_key(b), (3, 2, false));
    let c1 = rows[1].change.as_ref().unwrap();
    assert_eq!((c1.line, c1.removed, c1.inserted), (1, 1, 0));
    assert_eq!(c1.removed_text, "a");
    assert_eq!((rows[1].parent, rows[1].seq), (Some(0), 1));
    assert!(rows[1].at.is_some());
    let c2 = rows[2].change.as_ref().unwrap();
    assert_eq!((c2.line, c2.removed, c2.inserted), (2, 0, 7));
    assert_eq!(c2.inserted_text, "one two");
    assert_eq!((rows[2].parent, rows[2].seq), (Some(1), 2));
    assert!(rows[2].current && !rows[2].pending);
    assert!(rows[0].saved && !rows[1].saved && !rows[2].saved);
    // Typing in progress is a state of its own, pending, the root
    // still listed under it.
    t.keys("A!");
    let rows = t.ed.history(b);
    assert_eq!(rows.len(), 4);
    assert!(rows[3].current && rows[3].pending && !rows[2].current);
    assert_eq!(rows[3].parent, Some(2));
    assert_eq!(rows[3].change.as_ref().unwrap().inserted_text, "!");
    assert_eq!(t.ed.history_key(b), (3, 2, true));
    t.keys("<Esc>");
    assert!(!t.ed.history(b)[3].pending);
    assert_eq!(t.ed.history_key(b), (4, 3, false));
    // A seek back is undos: the text, the selection and the clean flag
    // are the state's.
    assert!(t.ed.history_seek(t.v, 0));
    assert_eq!(t.text(), "abc\ndef");
    assert!(!t.ed.buffer_of(t.v).modified);
    let rows = t.ed.history(b);
    assert_eq!(rows.len(), 4, "the undone states stay");
    assert!(rows[0].current);
    assert_eq!(rows[3].change.as_ref().unwrap().inserted_text, "!");
    assert_eq!(t.ed.history_key(b), (4, 0, false));
    // Forward again, part way; a seek to where it is moves nothing.
    assert!(t.ed.history_seek(t.v, 2));
    assert_eq!(t.text(), "bc\none twodef");
    assert!(!t.ed.history_seek(t.v, 2));
    // A newline and a tab are one char each in a snippet; a long change
    // is clipped.
    t.keys(":set -expandtab<CR>o\t<Esc>");
    let rows = t.ed.history(b);
    let c = rows.last().unwrap().change.as_ref().unwrap();
    assert_eq!(c.inserted_text, "⏎⇥");
    t.keys(&format!("o{}<Esc>", "x".repeat(60)));
    let rows = t.ed.history(b);
    let c = rows.last().unwrap().change.as_ref().unwrap();
    // The newline, the indent `o` keeps from the tab line, the sixty.
    assert_eq!(c.inserted, 62);
    assert_eq!(c.inserted_text.chars().count(), 41, "{}", c.inserted_text);
    assert!(c.inserted_text.ends_with('…'));
}

/// The history is a tree: an edit after an undo is a branch, and the
/// state undone stays — `u` and `<C-r>` follow the branch last taken,
/// `g-` and `g+` walk every state in the order made, and a seek
/// crosses from one branch to another.
#[test]
fn undo_is_a_tree() {
    let mut t = T::new("");
    let b = t.ed.views[t.v].buffer;
    t.keys("ia<Esc>");
    t.keys("ab<Esc>");
    t.keys("u");
    assert_eq!(t.text(), "a");
    t.keys("ac<Esc>");
    assert_eq!(t.text(), "ac");
    let rows = t.ed.history(b);
    assert_eq!(rows.len(), 4, "the `b` state is kept as a branch");
    assert_eq!(rows[2].change.as_ref().unwrap().inserted_text, "b");
    assert_eq!(rows[3].change.as_ref().unwrap().inserted_text, "c");
    assert_eq!((rows[2].parent, rows[3].parent), (Some(1), Some(1)));
    assert_eq!((rows[2].seq, rows[3].seq), (2, 3));
    assert!(rows[3].current);
    // `u` goes to the parent; `<C-r>` to the branch last taken.
    t.keys("u");
    assert_eq!(t.text(), "a");
    t.keys("<C-r>");
    assert_eq!(t.text(), "ac");
    // `g-` is back in time: to `b`, the state made before `c`, on the
    // other branch; `g+` forward again.
    t.keys("g-");
    assert_eq!(t.text(), "ab");
    t.keys("g-");
    assert_eq!(t.text(), "a");
    t.keys("g-");
    assert_eq!(t.text(), "");
    t.keys("g-");
    assert_eq!(t.ed.message, "already at oldest change");
    t.keys("g+g+");
    assert_eq!(t.text(), "ab");
    t.keys("g+");
    assert_eq!(t.text(), "ac");
    t.keys("g+");
    assert_eq!(t.ed.message, "already at newest change");
    // A seek across branches: `ac` to `ab` is up to `a` and down; the
    // redo pointers now retrace that, so `u` then `<C-r>` lands on `ab`.
    assert!(t.ed.history_seek(t.v, 2));
    assert_eq!(t.text(), "ab");
    t.keys("u<C-r>");
    assert_eq!(t.text(), "ab");
    // The change as lines.
    let hunk = t.ed.history_hunk(b, 3).unwrap();
    assert_eq!(
        (hunk.line, hunk.old, hunk.new),
        (1, vec!["a".into()], vec!["ac".into()])
    );
    assert!(t.ed.history_hunk(b, 0).is_none(), "the root has no change");
    // Typing in progress is a hunk against the text's node.
    t.keys("A!");
    let hunk = t.ed.history_hunk(b, 4).unwrap();
    assert_eq!(
        (hunk.old, hunk.new),
        (vec!["ab".into()], vec!["ab!".into()])
    );
    t.keys("<Esc>");
}

/// A multi-line change is one hunk of whole lines; a huge one is
/// clipped.
#[test]
fn a_hunk_is_the_lines_the_change_touched() {
    let mut t = T::new("one\ntwo\nthree\nfour\n");
    let b = t.ed.views[t.v].buffer;
    t.keys("jcwTWO<Esc>jdd");
    let rows = t.ed.history(b);
    let h = t.ed.history_hunk(b, 1).unwrap();
    assert_eq!(h.line, 2);
    assert_eq!(h.old, vec!["two"]);
    assert_eq!(h.new, vec!["TWO"]);
    // `dd`: the line out, nothing in — not the line after it.
    let h = t.ed.history_hunk(b, 2).unwrap();
    assert_eq!(rows[2].change.as_ref().unwrap().removed, 6);
    assert_eq!(h.old, vec!["three"]);
    assert!(h.new.is_empty());
    assert!(!h.clipped());
    assert_eq!((h.old_total, h.new_total), (1, 0));
    // A line put in whole: nothing out.
    t.keys("Oadded<Esc>");
    let h = t.ed.history_hunk(b, 3).unwrap();
    assert!(h.old.is_empty());
    assert_eq!(h.new, vec!["added"]);
    // A join across a newline: the two lines out, the one they became in.
    t.keys("ggJ");
    let h = t.ed.history_hunk(b, 4).unwrap();
    assert_eq!(h.old, vec!["one", "TWO"]);
    assert_eq!(h.new, vec!["one TWO"]);
    let mut t = T::new("");
    let b = t.ed.views[t.v].buffer;
    t.keys(&format!("i{}<Esc>", "l\n".repeat(300)));
    let h = t.ed.history_hunk(b, 1).unwrap();
    assert_eq!(h.new.len(), 200);
    assert!(h.clipped());
    assert_eq!((h.old_total, h.new_total), (0, 300));
}

/// Past a thousand states the oldest go: the root and the branches
/// off it the text is not under, so the tree stays a tree.
#[test]
fn the_tree_is_pruned_from_the_root() {
    let mut t = T::new("");
    let b = t.ed.views[t.v].buffer;
    t.keys("ia<Esc>");
    t.keys("u");
    t.keys("ib<Esc>");
    // Two branches off the root; then a thousand more on the second.
    for _ in 0..999 {
        t.keys("x");
        t.keys("ib<Esc>");
    }
    let rows = t.ed.history(b);
    assert_eq!(rows.len(), 1000);
    assert!(rows[0].change.is_none() && rows[0].parent.is_none());
    assert!(rows[0].seq > 0, "the root now is a state that was made");
    assert!(
        rows.iter()
            .all(|r| r.change.as_ref().is_none_or(|c| c.inserted_text != "a")),
        "the branch off the old root is gone"
    );
    assert!(rows.last().unwrap().current);
    for (i, r) in rows.iter().enumerate().skip(1) {
        assert_eq!(r.parent, Some(i - 1));
    }
    t.keys("u");
    assert_eq!(t.text(), "");
}

#[test]
fn visual_mode_selects_and_operates() {
    let mut t = T::new("hello world");
    t.keys("vllld");
    assert_eq!(t.text(), "o world");
    assert_eq!(t.ed.mode(t.v), Mode::Normal);
    let mut t = T::new("a\nb\nc");
    t.keys("Vjd");
    assert_eq!(t.text(), "c");
    let mut t = T::new("one two");
    t.keys("wviwy");
    assert_eq!(t.ed.memory.head().unwrap().text, "two");
    assert!(
        t.ed.take_effects()
            .contains(&Effect::SetClipboard("two".into()))
    );
}

#[test]
fn multicursor_edits_every_selection() {
    let mut t = T::new("aa\nbb\ncc");
    t.keys("<C-j><C-j>");
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

/// `<D-d>` (and `<C-n>`): the first press selects the word under the
/// caret, each after adds the next whole-word match and makes it
/// primary, round the end, until every one is selected; `<C-S-n>` takes
/// them all at once. From a selection the text is looked for as is.
#[test]
fn select_next_and_all_matches() {
    let mut t = T::new("foo bar foo\nfoobar foo");
    t.keys("<D-d>");
    assert_eq!(t.ed.mode(t.v), Mode::Visual);
    assert_eq!(t.sels(), [(0, 2)], "the word, the head on its last char");
    t.keys("<D-d>");
    assert_eq!(t.sels(), [(0, 2), (8, 10)]);
    assert_eq!(t.sel(), Selection::new(8, 10), "the newest is primary");
    // `foobar` is not the word; the third `foo` is on the next line.
    t.keys("<C-n>");
    assert_eq!(t.sels(), [(0, 2), (8, 10), (19, 21)]);
    t.keys("<D-d>");
    assert_eq!(t.sels().len(), 3, "round the end: nothing new");
    assert_eq!(t.ed.message, "every match is selected");
    t.keys("cqux<Esc>");
    assert_eq!(t.text(), "qux bar qux\nfoobar qux");
    // From a selection the text is taken as is: `foo` in `foobar` too.
    let mut t = T::new("foo bar foo\nfoobar foo");
    t.keys("vll<D-d>");
    assert_eq!(t.sels(), [(0, 2), (8, 10)]);
    t.keys("<D-d>");
    assert_eq!(t.sels(), [(0, 2), (8, 10), (12, 14)]);
    // The search is set to it: `n` goes on from wherever the caret is.
    t.keys("<Esc>ggn");
    assert_eq!(t.head(), 8);
    // A caret on a blank has no word.
    let mut t = T::new("a  b");
    t.keys("l<D-d>");
    assert_eq!(t.sels(), [(1, 1)]);
    assert_eq!(t.ed.message, "no word under the caret");
    // Every match at once, the primary the one under the caret.
    let mut t = T::new("a b a b a");
    t.keys("4l<C-S-n>");
    assert_eq!(t.sels(), [(0, 0), (4, 4), (8, 8)]);
    assert_eq!(t.sel(), Selection::new(4, 4));
    t.keys("cx<Esc>");
    assert_eq!(t.text(), "x b x b x");
    // `gh` / `gl`, `ZZ`, `<C-s>` in every mode, and `<D-a>` `<C-S-a>` are bound.
    let mut t = T::new("  ab cd");
    t.keys("$gh");
    assert_eq!(t.head(), 2);
    t.keys("gl");
    assert_eq!(t.head(), 6);
    t.keys("<D-a>");
    assert_eq!(t.sels(), [(0, 7)]);
    assert_eq!(t.ed.mode(t.v), Mode::Visual);
    // And on ctrl-shift, for a keyboard without ⌘ (asked 2026-10-04:
    // "add a PC key for select all"); `<C-a>` stays the increment.
    t.keys("<Esc>0<C-S-a>");
    assert_eq!(t.sels(), [(0, 7)]);
    assert_eq!(t.ed.mode(t.v), Mode::Visual);
    t.keys("<C-s>");
    assert_eq!(t.ed.message, "no file name (use :w <path>)");
    assert_eq!(
        t.ed.mode(t.v),
        Mode::Visual,
        "a write leaves the mode alone"
    );
    t.keys("<Esc>ix<C-s>");
    assert_eq!(t.ed.message, "no file name (use :w <path>)");
    assert_eq!(t.ed.mode(t.v), Mode::Insert);
    t.keys("<Esc>ZQ");
    assert!(t.ed.take_effects().contains(&Effect::Quit { force: true }));
}

/// `S` is `cc`; `ip` / `ap` take whole lines, linewise in visual mode;
/// `;` repeats the last `f` or `t` across lines; `gsa` / `gsd` / `gsr`
/// add, take off and swap a pair.
#[test]
fn change_line_paragraphs_find_repeat_and_surrounds() {
    let mut t = T::new("  foo\nbar");
    t.keys("Sx<Esc>");
    assert_eq!(t.text(), "  x\nbar");
    let mut t = T::new("a\nb\n\n\nc\nd\n");
    t.keys("jdap");
    assert_eq!(t.text(), "c\nd\n", "around takes the blank lines after");
    let mut t = T::new("a\nb\n\n\nc\nd\n");
    t.keys("Gkvipy");
    assert_eq!(
        t.ed.memory.head().unwrap().text,
        "c\nd\n",
        "linewise in visual mode"
    );
    let mut t = T::new("a\n\n\nb");
    t.keys("jdip");
    assert_eq!(t.text(), "a\nb", "on a blank, the blanks");
    let mut t = T::new("x = 1\ny = 2\nz == 3");
    t.keys("f=");
    assert_eq!(t.head(), 2);
    t.keys(";");
    assert_eq!(t.head(), 8, "across the line");
    t.keys(";;");
    assert_eq!(t.head(), 15);
    t.keys("ggt=;");
    assert_eq!(t.head(), 7, "a till repeats past the char it is before");
    t.keys("ggf=d;");
    assert_eq!(t.text(), "x  2\nz == 3", "an operator takes it, inclusive");
    let mut t = T::new("foo bar");
    t.keys("gsaiw)");
    assert_eq!(t.text(), "(foo) bar");
    t.keys("$viwgsa\"");
    assert_eq!(t.text(), "(foo) \"bar\"");
    assert_eq!(t.ed.mode(t.v), Mode::Normal);
    t.keys("gsd\"");
    assert_eq!(t.text(), "(foo) bar");
    t.keys("0lgsr)]");
    assert_eq!(t.text(), "[foo] bar");
    t.keys("gsd'");
    assert_eq!(t.ed.message, "no ' around the caret");
    assert_eq!(t.text(), "[foo] bar");
}

/// A digit under a chord is a binding's, never a count's: `<D-2>` is
/// the second column of a scrolling tab (kawoosh's `pane goto`), not
/// "two of the next thing". Ctrl and Alt were excluded from the count
/// already; ⌘ was not, so a ⌘ digit both did nothing and left a count
/// behind for whatever came next (2026-09-22).
#[test]
fn a_digit_under_a_chord_is_not_a_count() {
    let mut t = T::new("abcdef\n");
    t.keys("<D-2>");
    assert_eq!(t.head(), 0, "the chord did nothing here");
    t.keys("x");
    assert_eq!(t.text(), "bcdef\n", "and left no count behind");
    // The plain digit still counts.
    let mut t = T::new("abcdef\n");
    t.keys("2x");
    assert_eq!(t.text(), "cdef\n");
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
    assert!(t.ed.take_effects().contains(&Effect::Quit { force: false }));
    // Unsaved changes are the shell's to keep or refuse: the effect
    // goes out either way, the `!` on it.
    t.keys("x:q<CR>");
    assert!(t.ed.take_effects().contains(&Effect::Quit { force: false }));
    t.keys(":q!<CR>");
    assert!(t.ed.take_effects().contains(&Effect::Quit { force: true }));
    t.keys(":nonsense a b<CR>");
    assert!(matches!(
        effects(&mut t).as_slice(),
        [Effect::Shell { name, ctx }] if name == "nonsense" && ctx.args == ["a", "b"]
    ));
    t.keys(":set tabstop=2<CR>");
    assert_eq!(t.ed.tabstop(), 2);
}

/// `<Up>` at the prompt is the line before, newest first and only the
/// ones starting with what is typed; `<Down>` walks back to what was
/// typed; a line entered twice is remembered once, as the newest; the
/// search prompt has a history of its own.
#[test]
fn the_prompt_walks_its_history() {
    let mut t = T::new("one\ntwo");
    t.keys(":set tabstop=2<CR>");
    t.keys(":echo hi<CR>");
    t.keys(":set tabstop=8<CR>");
    t.keys(":set tabstop=2<CR>");
    assert_eq!(
        t.ed.cmd_history,
        ["echo hi", "set tabstop=8", "set tabstop=2"],
        "no repeats, the newest last"
    );
    t.keys(":<Up>");
    assert_eq!(t.cmdline(), "set tabstop=2");
    t.keys("<Up>");
    assert_eq!(t.cmdline(), "set tabstop=8");
    t.keys("<Up>");
    assert_eq!(t.cmdline(), "echo hi");
    t.keys("<Up>");
    assert_eq!(t.cmdline(), "echo hi", "the oldest stays");
    t.keys("<Down><Down><Down>");
    assert_eq!(t.cmdline(), "", "past the newest is what was typed");
    t.keys("<Esc>");
    // A prefix keeps the walk to the lines starting with it.
    t.keys(":ec<Up>");
    assert_eq!(t.cmdline(), "echo hi");
    t.keys("<Down>");
    assert_eq!(t.cmdline(), "ec");
    t.keys("<C-p><C-p>");
    assert_eq!(
        t.cmdline(),
        "echo hi",
        "the walk starts over from the prefix"
    );
    // Typing ends the walk: the line is the user's.
    t.keys(" there<CR>");
    assert_eq!(t.ed.message, "hi there");
    assert_eq!(
        t.ed.cmd_history.last().map(String::as_str),
        Some("echo hi there")
    );
    // The search prompt's own.
    t.keys("/two<CR>");
    t.keys("/<Up>");
    assert_eq!(t.cmdline(), "two");
    assert_eq!(t.ed.search_history, ["two"]);
    t.keys("<Esc>");
}

/// A search is a walk from the cursor, not a pass over the text: in a
/// buffer past `COUNT_ON_FRAME_BYTES` the next match is found and the
/// count is left to the shell (`Effect::CountMatches`), the message
/// saying so; `*` searches the word under the cursor whole; a bad
/// pattern is refused and the last good one stands.
#[test]
fn search_walks_from_the_cursor_and_counts_off_the_frame_when_big() {
    let line = "alpha,beta,gamma\n";
    let n = kawoosh_editor::commands::COUNT_ON_FRAME_BYTES / line.len() + 3;
    let mut t = T::new(&line.repeat(n));
    t.keys("/gamma<CR>");
    assert_eq!(t.head(), 11);
    assert!(
        t.ed.message.starts_with("/gamma  counting…"),
        "{}",
        t.ed.message
    );
    assert!(matches!(
        effects(&mut t).as_slice(),
        [Effect::CountMatches(_)]
    ));
    t.keys("3n");
    assert_eq!(t.head(), 11 + 3 * line.len());
    t.keys("N");
    assert_eq!(t.head(), 11 + 2 * line.len());
    // The count itself, over the whole text, on every core.
    let re = t.ed.search.as_ref().unwrap().re.clone();
    assert_eq!(
        kawoosh_editor::search::count(t.ed.buffers[t.ed.views[t.v].buffer].tree(), &re),
        n
    );
    // A small buffer counts on the frame.
    let mut t = T::new("one two one\ntwo one");
    t.keys("w*");
    assert_eq!(t.head(), 12, "`*` from `two` finds the next whole `two`");
    assert!(t.ed.message.contains("2 match(es)"), "{}", t.ed.message);
    assert!(effects(&mut t).is_empty());
    t.keys("/(<CR>");
    assert!(t.ed.message.starts_with("bad pattern"), "{}", t.ed.message);
    assert_eq!(
        t.ed.search.as_ref().unwrap().pattern,
        r"\btwo\b",
        "the last good one stands"
    );
    t.keys("/nothing<CR>");
    assert_eq!(t.ed.message, "not found: nothing");
}

/// The search prompt previews its pattern: each key at `/` puts the
/// cursor at the first match from where the prompt opened (`?` the
/// last before it), the view painting the pattern so far; a line that
/// is empty, does not compile yet or matches nothing leaves the cursor
/// at the origin; `<Esc>` puts the cursor, the scroll and the search
/// before back; `<CR>` lands where the preview showed and `n` goes on
/// from there.
#[test]
fn the_search_prompt_previews_the_first_match_as_it_is_typed() {
    let pattern = |t: &T| t.ed.search.as_ref().map(|s| s.pattern.clone());
    let mut t = T::new("one\ntwo\nthree\ntwo again");
    t.keys("/two<CR>gg");
    t.ed.views[t.v].top = 3;
    t.keys("/t");
    assert_eq!(t.head(), 4, "`t` is `two` on the second line");
    assert!(t.ed.prompt_view().is_some(), "the prompt is open");
    t.keys("h");
    assert_eq!(t.head(), 8, "`th` is `three`");
    assert_eq!(pattern(&t).as_deref(), Some("th"), "painted as typed");
    t.keys("<BS><BS>");
    assert_eq!(t.head(), 0, "an empty line is the origin");
    assert_eq!(pattern(&t).as_deref(), Some("two"), "and the search before");
    t.keys("[");
    assert_eq!(t.head(), 0, "not a pattern yet");
    assert_eq!(pattern(&t).as_deref(), Some("two"));
    t.keys("<BS>zzz");
    assert_eq!(t.head(), 0, "no match");
    assert_eq!(pattern(&t).as_deref(), Some("zzz"));
    t.ed.views[t.v].top = 7;
    // `<Esc>` once is normal mode in the field; twice leaves.
    t.keys("<Esc>");
    assert!(t.ed.prompt_view().is_some());
    t.keys("<Esc>");
    assert!(t.ed.prompt_view().is_none());
    assert_eq!(t.ed.mode(t.v), Mode::Normal);
    assert_eq!(t.head(), 0);
    assert_eq!(t.ed.views[t.v].top, 3, "the scroll from before the prompt");
    assert_eq!(
        pattern(&t).as_deref(),
        Some("two"),
        "`<Esc>` leaves no trace"
    );
    assert_eq!(
        t.ed.message, "/two  2 match(es)",
        "the preview says nothing"
    );
    // Backwards, then accepted: the search runs from the origin, so the
    // cursor is where the preview showed, and `n` goes on from there.
    t.keys("G?tw");
    assert_eq!(t.head(), 4);
    t.keys("<CR>");
    assert_eq!(t.head(), 4);
    assert!(t.ed.message.contains("2 match(es)"), "{}", t.ed.message);
    t.keys("n");
    assert_eq!(t.head(), 14);
    // The history walk previews too, and `<BS>` on an empty line is `<Esc>`.
    t.keys("gg/<Up>");
    assert_eq!(t.cmdline(), "tw");
    assert_eq!(t.head(), 4);
    t.keys("<C-u><BS>");
    assert_eq!(t.ed.mode(t.v), Mode::Normal);
    assert_eq!(t.head(), 0);
}

/// `/` and `?` match either case — as typed, previewed and walked by `n`
/// — unless the pattern says `(?-i)`; `*` and `:s` keep to the case they
/// are given, and the same pattern searched both ways is two searches.
#[test]
fn the_search_prompt_ignores_case() {
    let mut t = T::new("one One ONE one");
    t.keys("/ONE");
    assert_eq!(t.head(), 4, "the preview finds `One`");
    t.keys("<CR>");
    assert_eq!(t.head(), 4);
    assert_eq!(t.ed.message, "/ONE  4 match(es)");
    t.keys("n");
    assert_eq!(t.head(), 8);
    t.keys("?one<CR>");
    assert_eq!(t.head(), 4);
    t.keys("gg/(?-i)ONE<CR>");
    assert_eq!(t.head(), 8, "`(?-i)` asks for the case");
    assert_eq!(t.ed.message, "/(?-i)ONE  1 match(es)");
    t.keys("gg*");
    assert_eq!(t.head(), 12, "`*` finds the word as it is");
    assert_eq!(t.ed.message, r"/\bone\b  2 match(es)");
    t.keys("gg/\\bone\\b<CR>");
    assert_eq!(t.head(), 4, "the same pattern from `/` is another search");
    assert_eq!(t.ed.message, r"/\bone\b  4 match(es)");
    t.keys(":s/one/x/g<CR>");
    assert_eq!(t.text(), "x One ONE x");
}

/// `:s`: the current line's first match, `g` every one, `%` every
/// line, `N,M` a range, `&` and `$1` in the replacement, an escaped
/// delimiter, another delimiter, `i`; the cursor lands at the last
/// changed line's start, the pattern becomes the search, one `u` undoes
/// the lot, and a bulk one (past the tree's rebuild threshold) is the
/// same text as the small path gives.
#[test]
fn substitute_is_vims_on_a_line_a_range_and_the_file() {
    let mut t = T::new("aa ab aa\nba aa\naa\n");
    t.keys(":s/aa/X/<CR>");
    assert_eq!(t.text(), "X ab aa\nba aa\naa\n");
    assert_eq!(t.ed.message, "1 substitution(s) on 1 line(s)");
    assert_eq!(t.head(), 0);
    t.keys("u");
    assert_eq!(t.text(), "aa ab aa\nba aa\naa\n");
    t.keys(":s/aa/X/g<CR>");
    assert_eq!(t.text(), "X ab X\nba aa\naa\n");
    t.keys("u:%s/aa/[&]/g<CR>");
    assert_eq!(t.text(), "[aa] ab [aa]\nba [aa]\n[aa]\n");
    assert_eq!(t.ed.message, "4 substitution(s) on 3 line(s)");
    assert_eq!(t.head(), 21, "the last changed line's start");
    t.keys("u:2,3s/(a)(a)/$2-$1/<CR>");
    assert_eq!(t.text(), "aa ab aa\nba a-a\na-a\n");
    t.keys("u:%s#a/#A#g<CR>");
    assert_eq!(
        t.text(),
        "aa ab aa\nba aa\naa\n",
        "no `a/` anywhere: untouched"
    );
    assert_eq!(t.ed.message, "no match: a/");
    t.keys(":%s/AA/z/gi<CR>");
    assert_eq!(t.text(), "z ab z\nba z\nz\n");
    assert_eq!(
        t.ed.search.as_ref().unwrap().pattern,
        "(?i)AA",
        "and the search is the pattern"
    );
    t.keys("u:%s/\\//|/g<CR>");
    assert_eq!(t.ed.message, "no match: /");
    t.keys(":s/aa\\n/Q/<CR>");
    assert_eq!(t.text(), "aa ab Qba aa\naa\n", "a regex over the newline");
    // Bulk: many edits at once take the rebuild path and agree.
    let line = "foo bar foo baz\n";
    let mut t = T::new(&line.repeat(200));
    t.keys(":%s/foo/F/g<CR>");
    assert_eq!(t.text(), "F bar F baz\n".repeat(200));
    assert_eq!(t.ed.message, "400 substitution(s) on 200 line(s)");
    t.keys("u");
    assert_eq!(t.text(), line.repeat(200), "one undo");
    t.keys(":%s/foo/F/<CR>");
    assert_eq!(
        t.text(),
        "F bar foo baz\n".repeat(200),
        "the first per line"
    );
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
    // `cc` on the last line changes that line alone: its linewise
    // range starts with the newline before it, which is not its line.
    t.keys("ccq<Esc>");
    assert_eq!(t.text(), "    z\n    q");
    t.keys("2ccw<Esc>");
    assert_eq!(t.text(), "    z\n    w");
    let mut t = T::new("a\nb\n");
    t.keys("Gccx<Esc>");
    assert_eq!(t.text(), "a\nb\nx");
}

/// Insert's `<CR>` with two carets in one run of blanks: each breaks
/// the line where it is, the blanks between them its own, and the
/// text after them stays.
#[test]
fn line_breaks_from_carets_in_one_run_of_blanks() {
    let mut t = T::new("a  b");
    t.keys("i");
    t.ed.views[t.v].sels = kawoosh_editor::Selections {
        items: vec![Selection::point(1), Selection::point(2)],
        primary: 0,
    };
    t.keys("<CR>");
    assert_eq!(t.text(), "a\n\nb");
}

/// `o` below a line ending in an opening bracket, `O` above one
/// starting with a closer, and `<CR>` after an opener land a level
/// inside the block; `<CR>` between a bracket and its closer opens it.
#[test]
fn new_lines_indent_inside_a_block() {
    let mut t = T::new("    fn f() {\n        x\n    }");
    t.keys("oa<Esc>");
    assert_eq!(
        t.text(),
        "    fn f() {\n        a\n        x\n    }",
        "o after {{"
    );
    t.keys("GOb<Esc>");
    assert_eq!(
        t.text(),
        "    fn f() {\n        a\n        x\n        b\n    }",
        "O before }}"
    );
    t.keys("Goc<Esc>");
    assert_eq!(
        t.text().lines().last(),
        Some("    c"),
        "o after }}: its level"
    );
    t.keys("ggOd<Esc>");
    assert_eq!(
        t.text().lines().next(),
        Some("    d"),
        "O before {{: its level"
    );

    let mut t = T::new("  g(  ");
    t.keys("A<CR>y<Esc>");
    assert_eq!(t.text(), "  g(  \n      y", "<CR> after (");
    let mut t = T::new("  v = [  ]");
    t.keys("f[a<CR>z<Esc>");
    assert_eq!(t.text(), "  v = [\n      z\n  ]", "<CR> between [ and ]");
    let mut t = T::new("  a(b)");
    t.keys("fbi<CR><Esc>");
    assert_eq!(
        t.text(),
        "  a(\n      b)",
        "<CR> after ( with more before the closer"
    );
}

/// `yy` leaves the caret where it is — on the last line too, whose
/// linewise range starts with the newline before it — and a charwise
/// yank puts it at the start of what was yanked.
#[test]
fn a_yank_keeps_the_caret_on_its_line() {
    let mut t = T::new("abc\ndef\nghi");
    t.keys("Gllyy");
    assert_eq!(
        t.ed.views[t.v].sels.primary().head,
        10,
        "still on ghi, col 2"
    );
    t.keys("p");
    assert_eq!(t.text(), "abc\ndef\nghi\nghi");
    t.keys("ggllyb");
    assert_eq!(
        t.ed.views[t.v].sels.primary().head,
        0,
        "at the start of the yank"
    );
}

/// `dd` on the last line takes the newline before it, but what lands
/// in the register is the line with its newline after it, so `p` puts
/// it below the caret's line and `P` above — not an empty line first.
#[test]
fn a_last_line_deleted_pastes_as_a_line() {
    let mut t = T::new("a\nb\nc");
    t.keys("Gdd");
    assert_eq!(t.text(), "a\nb");
    t.keys("kp");
    assert_eq!(t.text(), "a\nc\nb");
    t.keys("Gdd");
    t.keys("ggP");
    assert_eq!(t.text(), "b\na\nc");
}

#[test]
fn insert_mode_keys() {
    let mut t = T::new("");
    t.keys("iab<CR>cd<BS><Tab>x<Esc>");
    assert_eq!(t.text(), "ab\nc   x", "`<Tab>` to the next stop");
    t.keys("o  y<CR>z<Esc>");
    assert_eq!(t.text(), "ab\nc   x\n  y\n  z");
    let mut t = T::new("foo bar");
    t.keys("A<C-w><Esc>");
    assert_eq!(t.text(), "foo ");
}

/// Insert's `<BS>` at a line's start joins it to the line above, the
/// caret where they meet; `X` stops there, and the first line's start
/// has nothing before it.
#[test]
fn insert_backspace_at_a_line_start_joins_the_line_above() {
    let mut t = T::new("ab\ncd");
    t.keys("jI<BS>x<Esc>");
    assert_eq!(t.text(), "abxcd");
    let mut t = T::new("ab\r\ncd");
    t.keys("jI<BS><Esc>");
    assert_eq!(t.text(), "abcd");
    let mut t = T::new("ab\ncd");
    t.keys("j0X");
    assert_eq!(t.text(), "ab\ncd");
    t.keys("ggI<BS><Esc>");
    assert_eq!(t.text(), "ab\ncd");
}

/// The normal-mode caret may stand on a line's newline (`$` then `l`, `j`
/// onto a shorter line), and `x` there deletes it: the next line joined
/// on as it is, no space put in and no indent taken off (vim's `gJ`), the
/// newline in the register as any `x`. A count stops at the line's end,
/// as vim's, so one begun on the text never joins and one begun on the
/// newline joins once. The last line has no newline to take.
#[test]
fn x_on_a_lines_newline_joins_the_next_line_on() {
    let reg = |t: &T| t.ed.memory.head().map(|m| m.text.clone());
    let mut t = T::new("abc\n  def\nghi\n");
    t.keys("$lx");
    assert_eq!(t.text(), "abc  def\nghi\n");
    assert_eq!(t.head(), 3, "where the newline was");
    assert_eq!(reg(&t).as_deref(), Some("\n"));
    t.keys("u");
    assert_eq!(t.text(), "abc\n  def\nghi\n");
    t.keys("j$l.");
    assert_eq!(t.text(), "abc\n  defghi\n", "`.` joins again");
    let mut t = T::new("abc\r\ndef");
    t.keys("$lx");
    assert_eq!(t.text(), "abcdef", "a CRLF break whole");
    assert_eq!(reg(&t).as_deref(), Some("\r\n"));
    t.keys("<Del>");
    assert_eq!(t.text(), "abcef", "normal `<Del>` is `x`");
    // `j` onto a shorter line, and an empty line, whose only cell is its
    // newline.
    let mut t = T::new("abcd\nab\ncd\n\nef");
    t.keys("3lj");
    assert_eq!(t.head(), 7, "on `ab`'s newline");
    t.keys("x");
    assert_eq!(t.text(), "abcd\nabcd\n\nef");
    t.keys("jx");
    assert_eq!(t.text(), "abcd\nabcd\nef");
    // A count: from the text it takes the line's characters and stops,
    // from the newline it joins once.
    let mut t = T::new("abc\ndef\nghi");
    t.keys("l9x");
    assert_eq!(t.text(), "a\ndef\nghi");
    t.keys("l3x");
    assert_eq!(t.text(), "adef\nghi");
    // The last line's end: nothing taken, the register kept.
    let mut t = T::new("abc\ndef");
    t.keys("yiwj$lx");
    assert_eq!(t.text(), "abc\ndef");
    assert_eq!(reg(&t).as_deref(), Some("abc"));
    // `X` takes the character before the newline, and stops at a line's
    // start; `s` changes the newline as `x` deletes it.
    let mut t = T::new("abc\ndef");
    t.keys("$lX");
    assert_eq!(t.text(), "ab\ndef");
    let mut t = T::new("abc\ndef");
    t.keys("$ls-<Esc>");
    assert_eq!(t.text(), "abc-def");
}

/// A visual selection is drawn over a newline its end stands on, and an
/// operator takes that newline with the rest, as `x` takes it under a
/// bare caret — vim's `v$`, and `v` on an empty line. An inclusive
/// motion never does: `d$` on an empty line, CRLF or not, is nothing.
#[test]
fn a_selection_over_a_newline_takes_it() {
    for keys in ["$lvx", "$lvd", "$hvlld"] {
        let mut t = T::new("abc\ndef");
        t.keys(keys);
        let joined = if keys.starts_with("$h") {
            "adef"
        } else {
            "abcdef"
        };
        assert_eq!(t.text(), joined, "{keys}");
    }
    let mut t = T::new("ab\n\ncd");
    t.keys("vjd");
    assert_eq!(t.text(), "cd", "onto an empty line");
    let mut t = T::new("ab\ncd");
    t.keys("$lvy");
    assert_eq!(t.ed.memory.head().unwrap().text, "\n");
    for text in ["a\n\nb", "a\r\n\r\nb"] {
        let mut t = T::new(text);
        t.keys("jd$");
        assert_eq!(t.text(), text);
    }
}

/// Only what an operator took is remembered and put on the clipboard,
/// as vim's: insert's `<BS>`, `<C-h>`, `<Del>` and `<C-w>`, and the
/// prompt's `<BS>`, leave the register and the clipboard alone; `x`,
/// `X` and `dw` fill them.
#[test]
fn plain_deletes_leave_the_register_alone() {
    let mut t = T::new("one two three");
    t.keys("A<BS><C-h><C-w><Esc>0i<Del><Esc>");
    assert_eq!(t.text(), "ne two ");
    assert!(t.ed.memory.head().is_none());
    t.keys(":ab<BS><Esc><Esc>");
    assert!(t.ed.memory.head().is_none());
    assert!(
        !t.ed
            .take_effects()
            .iter()
            .any(|e| matches!(e, Effect::SetClipboard(_)))
    );
    t.keys("x");
    assert_eq!(t.ed.memory.head().unwrap().text, "n");
    t.keys("lX");
    assert_eq!(t.ed.memory.head().unwrap().text, "e");
    t.keys("0dw");
    assert_eq!(t.ed.memory.head().unwrap().text, " ");
    assert!(
        t.ed.take_effects()
            .contains(&Effect::SetClipboard(" ".into()))
    );
}

/// The text keys a Mac types with (Ctrl's word keys where there is no
/// ⌘, as Windows and Linux spell them; keys.md): `<A-BS>` the word
/// before the caret, where `<C-w>` ends it, and `<A-Del>` the word
/// after, its mirror; `<D-BS>` to the line's start as `<C-u>`, `<D-Del>`
/// to its end — erases, the register and the clipboard left alone,
/// none past its line. `<A-Left>` `<A-Right>` by a word, `<D-Left>`
/// `<D-Right>` to the line's ends. In the prompt too; an insert session
/// with them is one undo, and `.` does it again.
#[test]
fn the_word_and_line_keys_erase_and_move_in_insert_mode() {
    let w = if cfg!(target_os = "macos") { "A" } else { "C" };
    let k = |s: &str| s.replace("W-", &format!("{w}-"));
    // Back, a word at a time, where `<C-w>` stops.
    for back in [k("<W-BS>"), "<C-w>".to_string()] {
        let mut t = T::new("one two.three four");
        let mut texts = vec![];
        t.keys("A");
        for _ in 0..3 {
            t.keys(&back);
            texts.push(t.text());
        }
        assert_eq!(texts, ["one two.three ", "one two.", "one two"], "{back}");
    }
    // Forward, the mirror.
    let mut t = T::new("one two.three four");
    let mut texts = vec![];
    t.keys("0i");
    for _ in 0..3 {
        t.keys(&k("<W-Del>"));
        texts.push(t.text());
    }
    assert_eq!(texts, [" two.three four", ".three four", "three four"]);
    // The line's ends.
    let mut t = T::new("one two three");
    t.keys("0fti<D-BS>");
    assert_eq!(t.text(), "two three");
    t.keys("<Esc>wi<D-Del>");
    assert_eq!(t.text(), "two ");
    // None past its line, as `<C-w>`.
    let mut t = T::new("ab\ncd");
    t.keys(&k("A<W-Del><D-Del><Esc>jI<W-BS><D-BS><Esc>"));
    assert_eq!(t.text(), "ab\ncd");
    // Nothing remembered.
    assert!(t.ed.memory.head().is_none());
    assert!(
        !t.ed
            .take_effects()
            .iter()
            .any(|e| matches!(e, Effect::SetClipboard(_)))
    );
    // The moves.
    let mut t = T::new("one two.three four");
    t.keys("A");
    let mut heads = vec![];
    for _ in 0..3 {
        t.keys(&k("<W-Left>"));
        heads.push(t.head());
    }
    t.keys("<D-Right>");
    heads.push(t.head());
    t.keys("<D-Left>");
    heads.push(t.head());
    for _ in 0..3 {
        t.keys(&k("<W-Right>"));
        heads.push(t.head());
    }
    assert_eq!(heads, [14, 8, 7, 18, 0, 3, 7, 8]);
    t.keys("X<Esc>");
    assert_eq!(t.text(), "one two.Xthree four");
    // One undo; `.` again.
    let mut t = T::new("aa bb\ncc dd");
    t.keys(&k("A<W-BS><W-BS>x<Esc>"));
    assert_eq!(t.text(), "x\ncc dd");
    t.keys("u");
    assert_eq!(t.text(), "aa bb\ncc dd");
    t.keys(&k("A<W-BS><Esc>j."));
    assert_eq!(t.text(), "aa \ncc ");
    // The prompt's line.
    let mut t = T::new("");
    t.keys(&k(":echo one two<W-BS>"));
    assert_eq!(t.cmdline(), "echo one ");
    t.keys(&k("<W-Left><W-Left><W-Del>"));
    assert_eq!(t.cmdline(), " one ");
    t.keys("<D-Del>");
    assert_eq!(t.cmdline(), "");
}

/// `"_` names the black hole for the next command: what it takes goes
/// into neither the register nor the clipboard, through an operator's
/// motion, a count on either side of it, visual mode and `.`; the
/// command after it is back to the register, and a put from `_` has
/// nothing to put.
#[test]
fn the_black_hole_register_takes_nothing() {
    let clipboard = |t: &mut T| {
        t.ed.take_effects()
            .into_iter()
            .any(|e| matches!(e, Effect::SetClipboard(_)))
    };
    let mut t = T::new("one two three four five six seven");
    t.keys("yiw");
    let kept = |t: &T| t.ed.memory.head().map(|m| m.text.clone());
    assert_eq!(kept(&t).as_deref(), Some("one"));
    t.ed.take_effects();

    t.keys("w\"_dw");
    assert_eq!(t.text(), "one three four five six seven");
    assert_eq!(kept(&t).as_deref(), Some("one"));
    assert!(!clipboard(&mut t));
    assert_eq!(t.ed.pending_register, None);

    // `.` repeats the whole of it, the register with it.
    t.keys(".");
    assert_eq!(t.text(), "one four five six seven");
    assert_eq!(kept(&t).as_deref(), Some("one"));

    // A count before `"` or after it.
    t.keys("2\"_x\"_2x");
    assert_eq!(t.text(), "one  five six seven");
    assert_eq!(kept(&t).as_deref(), Some("one"));

    t.keys("w\"_cwFIVE<Esc>");
    assert_eq!(t.text(), "one  FIVE six seven");
    t.keys("wviw\"_d");
    assert_eq!(t.text(), "one  FIVE  seven");
    assert_eq!(kept(&t).as_deref(), Some("one"));
    assert!(!clipboard(&mut t));

    // Named, then let go.
    t.keys("\"_<Esc>x");
    assert_eq!(kept(&t).as_deref(), Some(" "));

    // A put from the black hole has nothing to put.
    t.keys("\"_p");
    assert_eq!(t.ed.message, "the _ register is always empty");
    assert_eq!(t.text(), "one  FIVE seven");

    // `""` is the register there always is; others are refused.
    t.keys("0\"\"dw");
    assert_eq!(kept(&t).as_deref(), Some("one  "));
    t.keys("\"a");
    assert_eq!(t.ed.message, "no register a: only _, the black hole");
}

/// Counts on both sides of `"_` multiply, as vim's: `2"_3x` takes six.
/// `.` with a count gives it to the command, not to the naming; and a
/// key bound to nothing after `"_` lets the register go from `.` too.
#[test]
fn black_hole_counts_multiply_and_let_go() {
    let kept = |t: &T| t.ed.memory.head().map(|m| m.text.clone());
    let mut t = T::new("abcdefghij");
    t.keys("2\"_3x");
    assert_eq!(t.text(), "ghij", "two times three");
    assert_eq!(kept(&t), None);
    t.keys("\"_2x");
    assert_eq!(t.text(), "ij", "a count after it alone");
    let mut t = T::new("abcdefghij");
    t.keys("2\"\"x");
    assert_eq!(t.text(), "cdefghij", "`\"\"` keeps the count before it");
    assert_eq!(kept(&t).as_deref(), Some("ab"));
    assert_eq!(t.ed.pending_register, None);

    let mut t = T::new("one two three four five six");
    t.keys("\"_dw3.");
    assert_eq!(t.text(), "five six", "`3.` deletes three words");
    assert_eq!(kept(&t), None);

    let mut t = T::new("abc");
    t.keys("\"_<F12>x");
    assert_eq!(t.text(), "bc");
    assert_eq!(kept(&t).as_deref(), Some("a"), "the register is let go");
    t.keys(".");
    assert_eq!(t.text(), "c");
    assert_eq!(kept(&t).as_deref(), Some("b"), "and `.` lets it go too");
}

/// A command's `Path` argument reaches it absolute — `~`, `..` and a
/// relative path resolved against the engine's working directory — for
/// the engine's own commands, for one the shell declared, and for one
/// registered with `Args`; `!` is not a path, and an argument of another
/// kind is left alone.
/// The effects a key sequence left, minus the prompt's line — every
/// submitted `:` or `/` line is an `Effect::PromptLine` for the shell's
/// memory, beside whatever the line did.
fn effects(t: &mut T) -> Vec<Effect> {
    t.ed.take_effects()
        .into_iter()
        .filter(|e| !matches!(e, Effect::PromptLine { .. }))
        .collect()
}

#[test]
fn a_path_argument_is_resolved_before_the_command_runs() {
    let mut t = T::new("x");
    t.ed.cwd = std::path::PathBuf::from("/work/dir");
    t.keys(":e sub/../a.txt<CR>");
    assert!(matches!(
        effects(&mut t).as_slice(),
        [Effect::Open(p)] if p == std::path::Path::new("/work/dir/a.txt")
    ));
    t.keys(":e! ~/b.txt<CR>");
    let home = kawoosh_doc::paths::home().unwrap();
    assert!(matches!(
        effects(&mut t).as_slice(),
        [Effect::Open(p)] if *p == home.join("b.txt")
    ));
    // The shell's `:cd`, declared here: its argument comes back resolved
    // in the effect, the `!` as it was.
    t.ed.declare(
        Spec::new("cd")
            .args(Args::new(&[ArgKind::Path]))
            .bang("test"),
    );
    t.keys(":cd! ../up<CR>");
    assert!(matches!(
        effects(&mut t).as_slice(),
        [Effect::Shell { name, ctx }] if name == "cd" && ctx.args.len() == 1 && std::path::Path::new(&ctx.args[0]) == std::path::Path::new("/work/up") && ctx.bang()
    ));
    // A plugin's: `args = { "path", "text..." }`.
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let s2 = seen.clone();
    t.ed.register_with_args(
        "plug",
        Args::parse(&["path".into(), "text...".into()]).unwrap(),
        move |_, ctx| s2.borrow_mut().extend(ctx.args.clone()),
    );
    t.keys(":plug ./f.txt ~/not-a-path more<CR>");
    let seen = seen.borrow();
    assert_eq!(
        std::path::Path::new(&seen[0]),
        std::path::Path::new("/work/dir/f.txt")
    );
    assert_eq!(&seen[1..], ["~/not-a-path", "more"]);
    assert_eq!(
        Args::parse(&["text...".into(), "path".into()]).unwrap_err(),
        "`text...` must be the last argument"
    );
    assert!(Args::parse(&["nope".into()]).is_err());
    // `:w path` names the buffer by the resolved path.
    t.keys(":w ../c.txt<CR>");
    assert_eq!(
        t.ed.buffer_of(t.v).path.as_deref(),
        Some(std::path::Path::new("/work/c.txt"))
    );
}

/// A command is a spec and a body (`kawoosh_editor::command`): the ex
/// spelling is an alias on the spec; `!` and `?` are forms the spec
/// has a word for or refuses; `when` names facts, the engine's own or
/// published, and refuses with the reason; a subcommand is a two-word
/// name, walked from the line's words and from a keymap's binding
/// alike, its marker carried on either word.
#[test]
fn commands_are_specs_with_forms_conditions_and_subcommands() {
    use kawoosh_editor::{Ctx, Form};
    use std::cell::RefCell;
    use std::rc::Rc;
    let mut t = T::new("a\n");
    type Ran = Rc<RefCell<Vec<(String, Form, Vec<String>)>>>;
    let ran: Ran = Rc::new(RefCell::new(Vec::new()));
    let note = |ran: &Ran, name: &'static str| {
        let ran = ran.clone();
        move |_: &mut Editor, ctx: &Ctx| {
            ran.borrow_mut()
                .push((name.to_string(), ctx.form, ctx.args.clone()))
        }
    };
    t.ed.register_spec(
        Spec::new("hist").alias(&["hi"]).bang("everything"),
        note(&ran, "hist"),
    );
    t.ed.register_spec(
        Spec::new("hist drop").args(Args::new(&[ArgKind::Text])),
        note(&ran, "hist drop"),
    );
    t.ed.register_spec(
        Spec::new("hist clear").bang("held too").query("how many"),
        note(&ran, "hist clear"),
    );
    // The alias, the subcommand consumed, its arguments past it.
    t.keys(":hi drop file:/a<CR>");
    // A word that is no subcommand is the parent's argument.
    t.keys(":hist nope<CR>");
    // The marker on either word.
    t.keys(":hist clear!<CR>");
    t.keys(":hist! clear<CR>");
    t.keys(":hist clear?<CR>");
    assert_eq!(
        *ran.borrow(),
        [
            (
                "hist drop".to_string(),
                Form::Run,
                vec!["file:/a".to_string()]
            ),
            ("hist".to_string(), Form::Run, vec!["nope".to_string()]),
            ("hist clear".to_string(), Form::Bang, vec![]),
            ("hist clear".to_string(), Form::Bang, vec![]),
            ("hist clear".to_string(), Form::Query, vec![]),
        ]
    );
    ran.borrow_mut().clear();
    // A form the spec has no word for is refused with a message, and
    // the command does not run.
    t.keys(":hist drop!<CR>");
    assert_eq!(t.ed.message, "hist drop takes no !");
    t.keys(":hist?<CR>");
    assert_eq!(t.ed.message, "hist takes no ?");
    assert!(ran.borrow().is_empty());
    // `e!foo` and `e! foo`: the marker splits the name either way.
    t.keys(":hist!clear x<CR>");
    assert_eq!(
        ran.borrow().last().unwrap(),
        &("hist clear".to_string(), Form::Bang, vec!["x".to_string()])
    );

    // `when`: a fact the shell publishes, one the engine answers.
    t.ed.declare(Spec::new("scroll").when(&["terminal"]));
    t.keys(":scroll<CR>");
    assert_eq!(t.ed.message, "scroll: only in a terminal pane");
    assert!(effects(&mut t).is_empty());
    t.ed.fact("terminal", true);
    t.keys(":scroll<CR>");
    assert!(matches!(
        effects(&mut t).as_slice(),
        [Effect::Shell { name, .. }] if name == "scroll"
    ));
    t.ed.fact("terminal", false);
    assert_eq!(
        t.ed.can(Some(t.v), "scroll"),
        Err("scroll: only in a terminal pane".into())
    );
    t.ed.register_spec(
        Spec::new("only_here").when(&["language:oil", "!modified"]),
        note(&ran, "only_here"),
    );
    ran.borrow_mut().clear();
    t.keys(":only_here<CR>");
    assert_eq!(t.ed.message, "only_here: only in an oil buffer");
    let b = t.ed.views[t.v].buffer;
    t.ed.buffers[b].language = "oil".into();
    t.keys(":only_here<CR>");
    assert_eq!(ran.borrow().len(), 1);
    t.keys("x:only_here<CR>");
    assert_eq!(
        t.ed.message,
        "only_here: not in a buffer with unsaved changes"
    );
    assert_eq!(ran.borrow().len(), 1);
    assert!(t.ed.holds(Some(t.v), "buffer:a") || t.ed.holds(Some(t.v), "modified"));

    // A keymap's binding carries a marker and a subcommand like the
    // line does: `:map n Q quit!` binds `Q` to a forced quit.
    t.keys(":map n Q quit!<CR>");
    t.keys("Q");
    assert!(t.ed.take_effects().contains(&Effect::Quit { force: true }));
    // `<lt>` types the `<` itself: the line reads `<leader>d`, and the
    // leader — Space — opens it.
    t.keys(":map n <lt>leader>d hist drop k<CR>");
    ran.borrow_mut().clear();
    t.keys(" d");
    assert_eq!(
        *ran.borrow(),
        [("hist drop".to_string(), Form::Run, vec!["k".to_string()])]
    );

    // A word on the way to subcommands, with no command of its own,
    // asks which — `page`, `delete to` — and the walk goes through it:
    // `:delete to end` is `D`.
    t.keys(":page<CR>");
    assert_eq!(t.ed.message, "page what? (down, half, up)");
    t.keys(":delete to<CR>");
    assert_eq!(t.ed.message, "delete to what? (end, start)");
    t.keys("ggiabc def<Esc>0w:delete to end<CR>");
    assert_eq!(t.text().lines().next(), Some("abc "));
    assert_eq!(
        t.ed.commands.subcommands("delete"),
        ["char", "forward", "line", "to", "word"]
    );
    assert!(t.ed.command_names().contains(&"page"));
    assert!(!t.ed.command_names().contains(&"page down"));

    // The registry as data: names, aliases, subcommands, the specs.
    assert!(t.ed.command_names().contains(&"hist"));
    assert!(!t.ed.command_names().contains(&"hist drop"));
    assert_eq!(t.ed.commands.subcommands("hist"), ["clear", "drop"]);
    assert_eq!(t.ed.commands.canonical("hi"), "hist");
    assert_eq!(t.ed.spec("quit").unwrap().aliases, ["q"]);
    assert_eq!(
        t.ed.spec("edit").unwrap().bang.as_deref(),
        Some("reload the disk's text, as one undoable change")
    );
}

/// A key that types, bound in insert mode under a condition, types
/// where the condition does not hold — the binding is not a place the
/// key goes to die — and runs its command where it does.
#[test]
fn a_gated_insert_binding_on_a_typing_key_types_elsewhere() {
    use kawoosh_editor::Cond;
    let mut t = T::new("");
    t.ed.register("colon", |ed, _| ed.message = "colon".into());
    t.ed.keymap
        .bind_when(Mode::Insert, ":", "colon", &[Cond::parse("field:x")]);
    t.keys("ia:b");
    assert_eq!(t.text(), "a:b");
    assert_eq!(t.ed.message, "");
    t.ed.fact("field:x", true);
    t.keys(":");
    assert_eq!(t.text(), "a:b", "where it holds, the command");
    assert_eq!(t.ed.message, "colon");
}

/// A binding gated off by its own `when` is not bound there: it does
/// not shadow the mode a sequence falls through to — a plugin's
/// pane-mode `<A-S-l>`, in another plugin's pane, is normal mode's —
/// and with nothing under it the key is quiet. A binding whose command
/// cannot run still says why, in words.
#[test]
fn a_gated_off_binding_is_unbound_and_shadows_nothing() {
    use kawoosh_editor::Cond;
    let mut t = T::new("a\n");
    t.ed.register("column wider", |ed, _| ed.message = "wider".into());
    t.ed.register_spec(Spec::new("picker wrap").when(&["lua:picker"]), |ed, _| {
        ed.message = "wrap".into()
    });
    t.ed.register("picker list wider", |ed, _| ed.message = "list".into());
    let on_picker = [Cond::parse("lua:picker")];
    t.ed.keymap.bind(Mode::Normal, "<A-S-l>", "column wider");
    t.ed.keymap
        .bind_when(Mode::Pane, "<A-S-l>", "picker list wider", &on_picker);
    t.ed.keymap
        .bind_when(Mode::Pane, "x", "picker list wider", &on_picker);
    t.ed.keymap.bind(Mode::Pane, "w", "picker wrap");
    t.v = t.ed.pane_view();
    // In another pane: the column's, and a key only the picker has is
    // nothing at all.
    t.keys("<A-S-l>");
    assert_eq!(t.ed.message, "wider");
    t.ed.message.clear();
    t.keys("x");
    assert_eq!(t.ed.message, "", "quiet");
    // A bare binding whose command cannot run here says why.
    t.keys("w");
    assert_eq!(t.ed.message, "picker wrap: only in the picker pane");
    // In the picker's pane: the picker's.
    t.ed.fact("lua:picker", true);
    t.keys("<A-S-l>");
    assert_eq!(t.ed.message, "list");
}

/// A map local to a place (docs/design/local-maps.md) is found only
/// there: before the global one of its keys, which runs everywhere
/// else; a local key shadows the longer global ones under it (the
/// launcher's `g` over `gg`) and a local prefix a shorter global one
/// (the listing's `ma` over `m`); a local command that passes hands
/// the key to the global binding.
#[test]
fn a_local_map_is_the_places_alone() {
    let mut t = T::new("a\nb\nc\n");
    for name in ["plain", "listing", "letter", "sort"] {
        t.ed.register(name, move |ed, _| ed.message = name.into());
    }
    t.ed.register("passes", |ed, _| ed.pass());
    t.ed.keymap.bind(Mode::Normal, "<CR>", "plain");
    let dir = "language:dir";
    t.ed.keymap
        .bind_local(dir, Mode::Normal, "<CR>", "listing", &[]);
    t.ed.keymap
        .bind_local(dir, Mode::Normal, "g", "letter", &[]);
    t.ed.keymap.bind_local(dir, Mode::Normal, "ma", "sort", &[]);
    t.ed.keymap
        .bind_local(dir, Mode::Normal, "x", "passes", &[]);
    t.ed.keymap
        .bind_local("lua:elsewhere", Mode::Normal, "q", "letter", &[]);
    // Anywhere else: the global keys, and no trace of the listing's.
    t.keys("<CR>");
    assert_eq!(t.ed.message, "plain");
    t.keys("jjgg");
    assert_eq!(t.head(), 0, "`gg` is the file's start");
    let m = ["m".to_string()];
    assert!(
        matches!(t.ed.lookup_keys(t.v, Mode::Normal, &m), Lookup::Exact(bs) if bs[0].command == "mark"),
        "`m` marks"
    );
    let b = t.ed.views[t.v].buffer;
    t.ed.buffers[b].language = "dir".into();
    t.keys("<CR>");
    assert_eq!(t.ed.message, "listing");
    t.keys("g");
    assert_eq!(t.ed.message, "letter", "the place's `g` at once");
    assert!(matches!(
        t.ed.lookup_keys(t.v, Mode::Normal, &m),
        Lookup::Prefix
    ));
    t.keys("ma");
    assert_eq!(t.ed.message, "sort", "`m` waits for the place's `ma`");
    t.keys("x");
    assert_eq!(t.text(), "\nb\nc\n", "passed on to the global `x`");
    // A key only another place has is nothing here, and says nothing.
    t.ed.message.clear();
    t.keys("q");
    assert_eq!(t.ed.message, "");
}

/// On a field — the command line over a pane — only what the field
/// answers itself counts: a pane's places (`terminal`, `lua:NAME`) are
/// the pane's, not the line's; the resident pane view is the pane's
/// keys and has them. A buffer's own maps (`:map <buffer>`) go with it.
#[test]
fn a_panes_places_are_not_its_fields() {
    let mut t = T::new("a\n");
    t.ed.keymap
        .bind_local("terminal", Mode::Normal, "r", "echo raw", &[]);
    t.ed.keymap
        .bind_local("prompt", Mode::Normal, "<Esc>", "echo cancel", &[]);
    t.ed.fact("terminal", true);
    let pane = t.ed.pane_view();
    assert_eq!(t.ed.key_scopes(pane), ["terminal"]);
    let field = t.ed.open_field("x", "");
    assert!(
        t.ed.key_scopes(field).is_empty(),
        "{:?}",
        t.ed.key_scopes(field)
    );
    // `:map <buffer>`: this buffer's, found here and gone with it.
    t.ed.execute(t.v, "map <buffer> n Q echo mine");
    let scope = kawoosh_editor::buffer_scope(t.ed.views[t.v].buffer);
    assert_eq!(
        t.ed.key_scopes(t.v),
        [scope.clone(), "terminal".to_string()]
    );
    t.keys("Q");
    assert_eq!(t.ed.message, "mine");
    let other = t.ed.add_buffer(Buffer::new("u", ""));
    let ov = t.ed.add_view(other);
    assert!(!t.ed.key_scopes(ov).contains(&scope));
    let b = t.ed.views[t.v].buffer;
    t.ed.remove_buffer(b);
    assert!(
        !t.ed
            .keymap
            .bindings(Mode::Normal)
            .iter()
            .any(|(k, _)| k == "Q"),
        "dropped with its buffer"
    );
}

/// A key can carry several bindings, newest first: the first whose own
/// `when` holds and whose command can run is the one that runs; none
/// of them is the newest one's reason; a bare binding on a bare command
/// shadows the older ones as it always did.
#[test]
fn a_key_falls_through_its_bindings_by_when() {
    use kawoosh_editor::Cond;
    let mut t = T::new("a\n");
    t.ed.register("plain_enter", |ed, _| ed.message = "plain".into());
    t.ed.register_spec(Spec::new("oil enter").when(&["language:oil"]), |ed, _| {
        ed.message = "oil".into()
    });
    t.ed.keymap.bind_when(
        Mode::Normal,
        "<CR>",
        "plain_enter",
        &[Cond::parse("!language:oil")],
    );
    t.ed.keymap.bind(Mode::Normal, "<CR>", "oil enter");
    t.keys("<CR>");
    assert_eq!(t.ed.message, "plain");
    let b = t.ed.views[t.v].buffer;
    t.ed.buffers[b].language = "oil".into();
    t.keys("<CR>");
    assert_eq!(t.ed.message, "oil");
    // Neither can run: the newest binding's reason.
    t.ed.buffers[b].language = "rust".into();
    t.ed.keymap.bind_when(
        Mode::Normal,
        "<C-g>",
        "plain_enter",
        &[Cond::parse("store")],
    );
    t.ed.keymap.bind(Mode::Normal, "<C-g>", "oil enter");
    t.keys("<C-g>");
    assert_eq!(t.ed.message, "oil enter: only in an oil buffer");
    t.ed.fact("store", true);
    t.keys("<C-g>");
    assert_eq!(t.ed.message, "plain");
    // Bound again with no condition: the older ones are shadowed.
    t.ed.keymap.bind(Mode::Normal, "<CR>", "plain_enter");
    t.ed.buffers[b].language = "oil".into();
    t.keys("<CR>");
    assert_eq!(t.ed.message, "plain");
    // The listing has them all, newest first, an equal one moved to
    // the front, the default keymap's `goto_location` under them, and
    // the prompt's own after the global ones.
    let bs = t.ed.keymap.bindings(Mode::Normal);
    let cr: Vec<String> = bs
        .iter()
        .filter(|(k, _)| k == "<CR>")
        .map(|(_, b)| b.line())
        .collect();
    assert_eq!(
        cr,
        [
            "plain_enter",
            "oil enter",
            "plain_enter",
            "goto location",
            "prompt submit"
        ]
    );
}

/// Every engine command says what it does: the `:commands` pane is the
/// help, and a row without a line is a hole in it.
#[test]
fn every_engine_command_is_documented() {
    let ed = Editor::new();
    let bare: Vec<&str> = ed
        .commands
        .specs()
        .into_iter()
        .filter(|s| s.doc.is_empty())
        .map(|s| s.name.as_str())
        .collect();
    assert!(bare.is_empty(), "undocumented: {bare:?}");
}

/// A count before the operator is the motion's, as vim's: `2dw` is
/// `d2w`, `2d3w` six words, `d2j` three lines — and `2dd` two.
#[test]
fn a_count_before_the_operator_is_the_motions() {
    let w = "one two three four five six seven\n";
    let mut t = T::new(w);
    t.keys("2dw");
    assert_eq!(t.text(), "three four five six seven\n");
    let mut t = T::new(w);
    t.keys("2d3w");
    assert_eq!(t.text(), "seven\n");
    let mut t = T::new(w);
    t.keys("2cwX<Esc>");
    assert_eq!(
        t.text(),
        "X three four five six seven\n",
        "`cw` keeps the space"
    );
    let l = "1\n2\n3\n4\n5\n6\n";
    let mut t = T::new(l);
    t.keys("d2j");
    assert_eq!(t.text(), "4\n5\n6\n");
    let mut t = T::new(l);
    t.keys("2dj");
    assert_eq!(t.text(), "4\n5\n6\n");
    let mut t = T::new(l);
    t.keys("2dd");
    assert_eq!(t.text(), "3\n4\n5\n6\n");
    let mut t = T::new(l);
    t.keys("2>>");
    assert_eq!(t.text(), "    1\n    2\n3\n4\n5\n6\n");
}

/// `.` replays the last change's steps — the command stream, the text
/// typed in insert mode with it — on the selections as they are; a
/// count replaces the change's count and stays its count.
#[test]
fn dot_repeats_the_last_change() {
    let mut t = T::new("one two three four five six\n");
    t.keys(".");
    assert_eq!(t.ed.message, "nothing to repeat");
    t.keys("dw.");
    assert_eq!(t.text(), "three four five six\n");
    t.keys("2.");
    assert_eq!(t.text(), "five six\n", "a count replaces the change's");
    assert_eq!(
        t.ed.repeat.last_change,
        vec![
            Step::Command {
                name: "delete".into(),
                args: vec![],
                count: Some(2),
                arg_char: None,
            },
            Step::Command {
                name: "word next".into(),
                args: vec![],
                count: None,
                arg_char: None,
            },
        ]
    );

    // `x` with a count, undo between: undo is not a change.
    let mut t = T::new("abcdef\n");
    t.keys("x3.");
    assert_eq!(t.text(), "ef\n");
    t.keys("u.");
    assert_eq!(t.text(), "ef\n", "the count is the change's from then on");

    // Insert mode: the text typed goes with the command that opened it,
    // and a motion between is no change.
    let mut t = T::new("foo\nbar\n");
    t.keys("ihi <Esc>");
    assert_eq!(t.text(), "hi foo\nbar\n");
    t.keys("j.");
    assert_eq!(t.text(), "hi foo\nbahi r\n");
    assert_eq!(
        t.ed.repeat.last_change,
        vec![
            Step::Command {
                name: "insert".into(),
                args: vec![],
                count: None,
                arg_char: None,
            },
            Step::Text("hi ".into()),
            Step::Command {
                name: "normal".into(),
                args: vec![],
                count: None,
                arg_char: None,
            },
        ],
        "a run of text is one step"
    );
    let mut t = T::new("foo bar\n");
    t.keys("ciwxx<Esc>w.");
    assert_eq!(t.text(), "xx xx\n", "an operator, its object, and the text");
    t.keys("A;<Esc>0.");
    assert_eq!(t.text(), "xx xx;;\n");

    // A character argument rides with its command.
    let mut t = T::new("abab\n");
    t.keys("rxl.");
    assert_eq!(t.text(), "xxab\n");
    t.keys("0dfa.");
    assert_eq!(t.text(), "\n");

    // A visual change replays from `v`: the same shape from the caret.
    let mut t = T::new("a\nb\nc\nd\ne\n");
    t.keys("Vjd");
    assert_eq!(t.text(), "c\nd\ne\n");
    t.keys(".");
    assert_eq!(t.text(), "e\n");
    t.keys("vy.");
    assert_eq!(
        t.text(),
        "",
        "a yank is not a change: the deletion is still `.`'s"
    );

    // `p`, the numbers, and a `:s` line — each a change.
    let mut t = T::new("ab\n");
    t.keys("ylp.");
    assert_eq!(t.text(), "aaab\n");
    let mut t = T::new("1 1\n");
    t.keys("<C-a>w.");
    assert_eq!(t.text(), "2 2\n");
    t.keys("5.");
    assert_eq!(t.text(), "2 7\n");
    let mut t = T::new("aa\naa\n");
    t.keys(":s/a/b/<CR>");
    assert_eq!(t.text(), "ba\naa\n");
    t.keys("j.");
    assert_eq!(t.text(), "ba\nba\n", "a `:` line that edited is a change");
    t.keys(":foo<Esc><Esc>");
    assert_eq!(t.ed.prompt_text(), None);
    t.keys(".");
    assert_eq!(t.text(), "ba\nbb\n", "a cancelled prompt is no change");
}

/// `q` records the steps into a register and `@` replays them, with a
/// count, `@@` the last one, `@:` the last command line; `.` after a
/// macro is the macro's last change; a macro that plays itself stops.
#[test]
fn macros_record_and_replay_the_stream() {
    let mut t = T::new("a\nb\nc\nd\ne\n");
    t.keys("qa");
    assert_eq!(t.ed.recording(), Some('a'));
    t.keys("I- <Esc>j");
    assert_eq!(t.ed.recording(), Some('a'));
    t.keys("q");
    assert_eq!(t.ed.recording(), None);
    assert_eq!(t.ed.message, "recorded @a");
    assert_eq!(t.text(), "- a\nb\nc\nd\ne\n");
    assert_eq!(
        t.ed.repeat.macros[&'a'],
        vec![
            Step::Command {
                name: "insert line start".into(),
                args: vec![],
                count: None,
                arg_char: None,
            },
            Step::Text("- ".into()),
            Step::Command {
                name: "normal".into(),
                args: vec![],
                count: None,
                arg_char: None,
            },
            Step::Command {
                name: "move down".into(),
                args: vec![],
                count: None,
                arg_char: None,
            },
        ],
        "the `q`s are not in it"
    );
    t.keys("@a");
    assert_eq!(t.text(), "- a\n- b\nc\nd\ne\n");
    t.keys("2@@");
    assert_eq!(t.text(), "- a\n- b\n- c\n- d\ne\n");
    t.keys(".");
    assert_eq!(
        t.text(),
        "- a\n- b\n- c\n- d\n- e\n",
        "`.` after a macro is the macro's last change, on the line its `j` reached"
    );
    t.keys("@z");
    assert_eq!(t.ed.message, "no macro @z");

    // An upper-case register appends.
    t.keys("qAxq");
    assert_eq!(t.ed.repeat.macros[&'a'].len(), 5);
    assert_eq!(
        t.text(),
        "- a\n- b\n- c\n- d\n-e\n",
        "the `x` took the space"
    );
    t.keys("gg@a");
    assert_eq!(t.text(), "- - a\n-b\n- c\n- d\n-e\n");

    // `@:` runs the last command line again.
    let mut t = T::new("aa\naa\n");
    t.keys(":s/a/b/g<CR>j@:");
    assert_eq!(t.text(), "bb\nbb\n");

    // A macro that plays itself: no step fails, so the depth stops it.
    let mut t = T::new("1\n2\n3\n");
    t.keys("qcj@c");
    assert_eq!(t.ed.message, "no macro @c", "the old `c`, while recording");
    t.keys("q");
    assert_eq!(t.ed.message, "recorded @c");
    t.keys("gg@c");
    assert_eq!(
        t.head(),
        6,
        "on the last line, the empty one after the newline"
    );
    assert!(t.ed.message.contains("deep"), "{}", t.ed.message);
}

/// A server's edits land where it says, to the character: inside a
/// grapheme cluster too (a combining mark given its own edit), and an
/// offset inside a character steps back to its start.
#[test]
fn edits_from_outside_land_on_characters_not_graphemes() {
    let mut t = T::new("e\u{301}🦀z");
    let b = t.ed.views[t.v].buffer;
    assert!(t.ed.apply_edits(b, &[(1..3, "\u{300}".into()), (5..5, "!".into())]));
    assert_eq!(t.text(), "e\u{300}!🦀z");
}

/// A formatter's answer goes in as the lines that changed
/// (formatters.md Decision 3): a caret on a line it did not touch stays
/// on its character, one on a reindented line keeps its place in the
/// text, one `u` takes it all back, and a stale answer is refused.
#[test]
fn a_text_put_in_by_its_diff_keeps_the_carets() {
    let mut t = T::new("if a {\nb;\n}\nkeep this\n");
    let id = t.ed.views[t.v].buffer;
    t.keys("3jfh");
    assert_eq!(t.head(), 18);
    let v = t.ed.buffers[id].version();
    let n =
        t.ed.replace_diffed(id, "if a {\n    b;\n}\nkeep this\n", Some(v))
            .unwrap();
    assert_eq!(n, 1);
    assert_eq!(t.text(), "if a {\n    b;\n}\nkeep this\n");
    assert_eq!(t.head(), 22, "on the same `h`, four bytes on");
    t.keys("u");
    assert_eq!(t.text(), "if a {\nb;\n}\nkeep this\n", "one undo node");
    let stale = t.ed.buffers[id].version();
    t.keys("ix<Esc>");
    assert!(t.ed.replace_diffed(id, "y\n", Some(stale)).is_err());
    assert_eq!(t.ed.replace_diffed(id, &t.text(), None), Ok(0));
}

/// An edit per line over a long file is one pass: `>G` over 100,000
/// lines took 20 s — placing each caret walked every caret placed
/// before it, and each edit read its line's graphemes to find a
/// boundary between two ASCII bytes — 1.7 s now. Bounded by how it
/// grows, not by a time: four times the lines cost four times as much
/// in one pass and sixteen in the quadratic, whatever the machine's
/// speed (a 4-core Linux VM took 6 s for 60,000 lines, over the 5 s
/// this bound once was, and 1.5 s for 15,000: linear). The thread's
/// CPU time, not the clock's: under a loaded machine (eight builds at
/// a load of 90) the clock read 6 to 15 s for the same pass.
#[test]
fn an_edit_per_line_over_a_long_file_is_one_pass() {
    let pass = |n: usize| {
        let text = "x\n".repeat(n);
        let mut t = T::new(&text);
        let start = thread_cpu();
        t.keys(">G");
        let took = thread_cpu() - start;
        assert_eq!(t.text().lines().next(), Some("    x"));
        assert_eq!(t.text().len(), text.len() + 4 * n);
        took
    };
    let small = pass(15_000);
    let large = pass(60_000);
    let growth = large.as_secs_f64() / small.as_secs_f64();
    assert!(
        growth < 8.0,
        "4x the lines took {growth:.1}x as long ({small:?}, then {large:?})"
    );
}

/// The CPU time this thread has had, which other processes do not
/// stretch.
#[cfg(unix)]
fn thread_cpu() -> std::time::Duration {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid timespec for the call to fill.
    let ok = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
    assert_eq!(ok, 0);
    std::time::Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

#[cfg(windows)]
fn thread_cpu() -> std::time::Duration {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentThread, GetThreadTimes};
    let zero = || FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut created, mut exited, mut kernel, mut user) = (zero(), zero(), zero(), zero());
    // SAFETY: the current thread's pseudo-handle, four FILETIMEs to fill.
    let ok = unsafe {
        GetThreadTimes(
            GetCurrentThread(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    };
    assert_ne!(ok, 0);
    let ticks = |f: FILETIME| (f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64;
    // FILETIME counts 100 ns.
    std::time::Duration::from_nanos((ticks(kernel) + ticks(user)) * 100)
}
