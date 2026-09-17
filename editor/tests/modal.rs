//! The modal engine driven by key sequences, no UI anywhere.

use kawoosh_doc::Buffer;
use kawoosh_editor::{ArgKind, Args, Editor, Effect, KeyStroke, Mode, Selection, ViewId};

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
    assert_eq!(t.ed.cmdline, "set tabstop=2");
    t.keys("<Up>");
    assert_eq!(t.ed.cmdline, "set tabstop=8");
    t.keys("<Up>");
    assert_eq!(t.ed.cmdline, "echo hi");
    t.keys("<Up>");
    assert_eq!(t.ed.cmdline, "echo hi", "the oldest stays");
    t.keys("<Down><Down><Down>");
    assert_eq!(t.ed.cmdline, "", "past the newest is what was typed");
    t.keys("<Esc>");
    // A prefix keeps the walk to the lines starting with it.
    t.keys(":ec<Up>");
    assert_eq!(t.ed.cmdline, "echo hi");
    t.keys("<Down>");
    assert_eq!(t.ed.cmdline, "ec");
    t.keys("<C-p><C-p>");
    assert_eq!(
        t.ed.cmdline, "echo hi",
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
    assert_eq!(t.ed.cmdline, "two");
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
        t.ed.take_effects().as_slice(),
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
    assert!(t.ed.take_effects().is_empty());
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

/// A command's `Path` argument reaches it absolute — `~`, `..` and a
/// relative path resolved against the engine's working directory — for
/// the engine's own commands, for one the shell declared, and for one
/// registered with `Args`; `!` is not a path, and an argument of another
/// kind is left alone.
#[test]
fn a_path_argument_is_resolved_before_the_command_runs() {
    let mut t = T::new("x");
    t.ed.cwd = std::path::PathBuf::from("/work/dir");
    t.keys(":e sub/../a.txt<CR>");
    assert!(matches!(
        t.ed.take_effects().as_slice(),
        [Effect::Open(p)] if p == std::path::Path::new("/work/dir/a.txt")
    ));
    t.keys(":e! ~/b.txt<CR>");
    let home = kawoosh_doc::paths::home().unwrap();
    assert!(matches!(
        t.ed.take_effects().as_slice(),
        [Effect::Open(p)] if *p == home.join("b.txt")
    ));
    // The shell's `:cd`, declared here: its argument comes back resolved
    // in the effect, the `!` as it was.
    t.ed.declare("cd", Args::new(&[ArgKind::Path]));
    t.keys(":cd! ../up<CR>");
    assert!(matches!(
        t.ed.take_effects().as_slice(),
        [Effect::Shell { name, args, .. }] if name == "cd" && args == &["/work/up", "!"]
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
    assert_eq!(*seen.borrow(), ["/work/dir/f.txt", "~/not-a-path", "more"]);
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
