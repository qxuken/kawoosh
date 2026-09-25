//! Multibuffers (docs/design/search.md Decisions 1–5): excerpts of
//! other buffers kept equal to them both ways, the gaps refusing edits,
//! undo stepping the sources.

use kawoosh_doc::{Buffer, BufferId};
use kawoosh_editor::{Editor, KeyStroke, Part, ViewId};

struct T {
    ed: Editor,
    v: ViewId,
    m: BufferId,
    a: BufferId,
    b: BufferId,
}

/// Two sources — `a` with a final newline, `b` without — and a
/// multibuffer of lines 1–2 of each under a header.
fn two() -> T {
    let mut ed = Editor::new();
    let a = ed.add_buffer(Buffer::new("a", "a0\na1\na2\na3\n"));
    let b = ed.add_buffer(Buffer::new("b", "b0\nb1\nb2"));
    let m = ed.open_multi(
        "search",
        vec![
            Part::Gap("A\n".into()),
            Part::Lines(a, 1..3),
            Part::Gap("B\n".into()),
            Part::Lines(b, 1..3),
        ],
    );
    let v = ed.add_view(m);
    T { ed, v, m, a, b }
}

impl T {
    fn keys(&mut self, seq: &str) -> &mut Self {
        for k in kawoosh_editor::keymap::parse_notation(seq) {
            self.ed.key(self.v, stroke_of(&k));
        }
        self
    }
    fn text(&self, id: BufferId) -> String {
        self.ed.buffers[id].text()
    }
    fn multi(&self) -> String {
        self.text(self.m)
    }
    /// A view of its own on source `id`, for editing it "elsewhere".
    fn view_on(&mut self, id: BufferId) -> ViewId {
        self.ed.add_view(id)
    }
    fn keys_on(&mut self, v: ViewId, seq: &str) {
        for k in kawoosh_editor::keymap::parse_notation(seq) {
            self.ed.key(v, stroke_of(&k));
        }
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
                "S" => st.shift = true,
                _ => {}
            }
        }
        st.code = match base {
            "Esc" => "escape".into(),
            "CR" => "enter".into(),
            "BS" => "backspace".into(),
            "Del" => "delete".into(),
            other => other.into(),
        };
        st
    } else {
        let mut st = KeyStroke::plain(k);
        st.text = Some(k.to_string());
        st
    }
}

#[test]
fn a_multibuffer_shows_the_lines_asked_for() {
    let t = two();
    // `b`'s last line has no newline: the multibuffer adds its own.
    assert_eq!(t.multi(), "A\na1\na2\nB\nb1\nb2\n");
    assert!(t.ed.is_multi(t.m));
    assert!(!t.ed.buffers[t.m].modified);
    // The gutter's numbers: the file's lines, none on a header.
    let lines = t.ed.multi_lines(t.m, 0..6);
    assert_eq!(
        lines,
        vec![
            None,
            Some((t.a, 1)),
            Some((t.a, 2)),
            None,
            Some((t.b, 1)),
            Some((t.b, 2))
        ]
    );
    assert_eq!(t.ed.multi_at(t.m, 2), Some((t.a, 3)));
    assert_eq!(t.ed.multi_at(t.m, 0), None);
}

#[test]
fn typing_in_an_excerpt_is_in_the_source_at_once() {
    let mut t = two();
    t.keys("jA!<Esc>");
    assert_eq!(t.multi(), "A\na1!\na2\nB\nb1\nb2\n");
    assert_eq!(t.text(t.a), "a0\na1!\na2\na3\n");
    assert!(t.ed.buffers[t.a].modified);
    assert!(t.ed.buffers[t.m].modified, "modified while a source is");
    // The second file, whose last line is bare: it stays bare.
    t.keys("gg5jA?<Esc>");
    assert_eq!(t.text(t.b), "b0\nb1\nb2?");
    // Mid-insert, before `<Esc>`: live, key by key.
    t.keys("ggjjI>");
    assert_eq!(t.text(t.a), "a0\na1!\n>a2\na3\n");
    t.keys("<Esc>");
}

#[test]
fn lines_come_and_go_through_an_excerpt() {
    let mut t = two();
    // A line opened below an excerpt's last line is the file's, after
    // the same line.
    t.keys("jjonew<Esc>");
    assert_eq!(t.multi(), "A\na1\na2\nnew\nB\nb1\nb2\n");
    assert_eq!(t.text(t.a), "a0\na1\na2\nnew\na3\n");
    // A bare last line: one after it gets the newline between them.
    t.keys("gg6jomore<Esc>");
    assert_eq!(t.text(t.b), "b0\nb1\nb2\nmore");
    assert_eq!(t.multi(), "A\na1\na2\nnew\nB\nb1\nb2\nmore\n");
    // `dd` of an excerpt's lines, its last among them.
    t.keys("ggjdd");
    assert_eq!(t.text(t.a), "a0\na2\nnew\na3\n");
    t.keys("jdd");
    assert_eq!(t.text(t.a), "a0\na2\na3\n");
    assert_eq!(t.multi(), "A\na2\nB\nb1\nb2\nmore\n");
    t.keys("ggjdd");
    assert_eq!(t.text(t.a), "a0\na3\n");
    assert_eq!(t.multi(), "A\nB\nb1\nb2\nmore\n");
}

#[test]
fn the_gaps_take_no_edits() {
    let mut t = two();
    let before = t.multi();
    t.keys("ggdd");
    assert_eq!(t.multi(), before, "a header is not deleted");
    assert!(t.ed.message.contains("excerpt"), "{}", t.ed.message);
    t.keys("x");
    assert_eq!(t.multi(), before);
    // `J` on an excerpt's last line would join it to the header after.
    t.keys("jjJ");
    assert_eq!(t.multi(), before);
    assert!(t.ed.message.contains("line break"), "{}", t.ed.message);
    // `<BS>` at an excerpt's start would join it to the header before.
    t.keys("ggji<BS><Esc>");
    assert_eq!(t.multi(), before);
    // A whole-buffer delete takes none of it.
    t.keys("ggdG");
    assert_eq!(t.multi(), before);
    assert_eq!(t.text(t.a), "a0\na1\na2\na3\n");
    // A multicursor edit with one caret on a header is refused whole.
    t.keys("gg");
    let v = t.v;
    t.ed.views[v].sels = kawoosh_editor::Selections {
        items: vec![
            kawoosh_editor::Selection::point(0),
            kawoosh_editor::Selection::point(2),
        ],
        primary: 0,
    };
    t.keys("iz<Esc>");
    assert_eq!(t.multi(), before);
}

#[test]
fn an_edit_elsewhere_is_in_the_excerpt() {
    let mut t = two();
    let va = t.view_on(t.a);
    // On a line the excerpt shows.
    t.keys_on(va, "jA+<Esc>");
    assert_eq!(t.multi(), "A\na1+\na2\nB\nb1\nb2\n");
    // Above it: the excerpt's lines move, its text does not.
    t.keys_on(va, "ggOtop<Esc>");
    assert_eq!(t.text(t.a), "top\na0\na1+\na2\na3\n");
    assert_eq!(t.multi(), "A\na1+\na2\nB\nb1\nb2\n");
    assert_eq!(t.ed.multi_lines(t.m, 1..2), vec![Some((t.a, 2))]);
    // Joined to the line after its last: the excerpt takes that line in.
    t.keys_on(va, "ggjjjJ");
    assert_eq!(t.text(t.a), "top\na0\na1+\na2 a3\n");
    assert_eq!(t.multi(), "A\na1+\na2 a3\nB\nb1\nb2\n");
    // A plugin's edits, outside any command.
    assert!(t.ed.apply_edits(t.b, &[(0..2, "B0".into()), (3..5, "B1".into())]));
    assert_eq!(t.multi(), "A\na1+\na2 a3\nB\nB1\nb2\n");
    // The multibuffer's own caret rides along.
    t.keys("gg5j$");
    let head = t.ed.views[t.v].sels.primary().head;
    t.ed.apply_edits(t.b, &[(0..0, "zz".into())]);
    assert_eq!(t.ed.views[t.v].sels.primary().head, head);
}

#[test]
fn undo_steps_the_sources_back() {
    let mut t = two();
    // One change reaching both files.
    t.keys(":%s/1/X/g<CR>");
    assert_eq!(t.text(t.a), "a0\naX\na2\na3\n");
    assert_eq!(t.text(t.b), "b0\nbX\nb2");
    t.keys("u");
    assert_eq!(t.text(t.a), "a0\na1\na2\na3\n");
    assert_eq!(t.text(t.b), "b0\nb1\nb2");
    assert_eq!(t.multi(), "A\na1\na2\nB\nb1\nb2\n");
    assert!(!t.ed.buffers[t.m].modified, "back to what was saved");
    t.keys("<C-r>");
    assert_eq!(t.text(t.a), "a0\naX\na2\na3\n");
    assert_eq!(t.multi(), "A\naX\na2\nB\nbX\nb2\n");
    // An insert session is one change, however many keys.
    t.keys("ggjjofoo<CR>bar<Esc>");
    assert_eq!(t.text(t.a), "a0\naX\na2\nfoo\nbar\na3\n");
    t.keys("u");
    assert_eq!(t.text(t.a), "a0\naX\na2\na3\n");
    // The file's own pane: the change made through the multibuffer is
    // one state of its tree.
    let va = t.view_on(t.a);
    t.keys_on(va, "u");
    assert_eq!(t.text(t.a), "a0\na1\na2\na3\n");
    assert_eq!(t.multi(), "A\na1\na2\nB\nbX\nb2\n");
    // Edited since, a source is left as it is; the others step back.
    t.keys("u");
    assert!(t.ed.message.contains("a"), "{}", t.ed.message);
    assert_eq!(t.text(t.b), "b0\nb1\nb2");
}

#[test]
fn substitute_leaves_the_headers() {
    let mut ed = Editor::new();
    let a = ed.add_buffer(Buffer::new("a", "x foo\n"));
    let m = ed.open_multi(
        "s",
        vec![Part::Gap("foo.rs\n".into()), Part::Lines(a, 0..1)],
    );
    let v = ed.add_view(m);
    for k in kawoosh_editor::keymap::parse_notation(":%s/foo/bar/g<CR>") {
        ed.key(v, stroke_of(&k));
    }
    assert_eq!(ed.buffers[a].text(), "x bar\n");
    assert_eq!(ed.buffers[m].text(), "foo.rs\nx bar\n");
    assert!(
        ed.message.contains("outside the excerpts"),
        "{}",
        ed.message
    );
}

#[test]
fn two_excerpts_of_one_file_move_together() {
    let mut ed = Editor::new();
    let a = ed.add_buffer(Buffer::new("a", "l0\nl1\nl2\nl3\nl4\n"));
    let m = ed.open_multi(
        "s",
        vec![
            Part::Gap("a\n".into()),
            Part::Lines(a, 0..1),
            Part::Gap("⋯\n".into()),
            Part::Lines(a, 3..5),
        ],
    );
    let v = ed.add_view(m);
    let keys = |ed: &mut Editor, s: &str| {
        for k in kawoosh_editor::keymap::parse_notation(s) {
            ed.key(v, stroke_of(&k));
        }
    };
    assert_eq!(ed.buffers[m].text(), "a\nl0\n⋯\nl3\nl4\n");
    // Two lines put in the first excerpt: the second one's lines are
    // two further down in the file, and still its own.
    keys(&mut ed, "jox<CR>y<Esc>");
    assert_eq!(ed.buffers[a].text(), "l0\nx\ny\nl1\nl2\nl3\nl4\n");
    assert_eq!(ed.buffers[m].text(), "a\nl0\nx\ny\n⋯\nl3\nl4\n");
    keys(&mut ed, "gg6jA!<Esc>");
    assert_eq!(ed.buffers[a].text(), "l0\nx\ny\nl1\nl2\nl3\nl4!\n");
    assert_eq!(ed.multi_lines(m, 5..7), vec![Some((a, 5)), Some((a, 6))]);
}

#[test]
fn a_file_still_opening_is_filled_in_when_it_lands() {
    let mut ed = Editor::new();
    let path = std::path::Path::new("/nowhere/f.txt");
    let a = ed.add_buffer(Buffer::opening(path, 10));
    let m = ed.open_multi("s", vec![Part::Gap("f\n".into()), Part::Lines(a, 1..2)]);
    assert_eq!(ed.buffers[m].text(), "f\n");
    ed.buffers[a].attach(text_buffer::Buffer::with_text(b"zero\none\ntwo\n"));
    ed.sync_multis();
    assert_eq!(ed.buffers[m].text(), "f\none\n");
    let v = ed.add_view(m);
    for k in kawoosh_editor::keymap::parse_notation("jA!<Esc>") {
        ed.key(v, stroke_of(&k));
    }
    assert_eq!(ed.buffers[a].text(), "zero\none!\ntwo\n");
}

#[test]
fn write_writes_the_files_it_shows() {
    let dir = std::env::temp_dir().join(format!("kawoosh-multi-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (pa, pb) = (dir.join("a.txt"), dir.join("b.txt"));
    std::fs::write(&pa, "one\ntwo\n").unwrap();
    std::fs::write(&pb, "three\n").unwrap();
    let mut ed = Editor::new();
    let a = ed.add_buffer(Buffer::from_file(&pa).unwrap());
    let b = ed.add_buffer(Buffer::from_file(&pb).unwrap());
    let m = ed.open_multi(
        "s",
        vec![
            Part::Gap("a\n".into()),
            Part::Lines(a, 1..2),
            Part::Gap("b\n".into()),
            Part::Lines(b, 0..1),
        ],
    );
    // Borrowed: not listed until a pane shows them or they are edited.
    assert!(!ed.listed_buffers().contains(&a));
    let v = ed.add_view(m);
    for k in kawoosh_editor::keymap::parse_notation("jA!<Esc>:w<CR>") {
        ed.key(v, stroke_of(&k));
    }
    assert_eq!(std::fs::read_to_string(&pa).unwrap(), "one\ntwo!\n");
    assert_eq!(std::fs::read_to_string(&pb).unwrap(), "three\n");
    assert!(ed.listed_buffers().contains(&a), "edited, so listed");
    assert!(!ed.buffers[m].modified);
    assert!(ed.message.contains("1 file written"), "{}", ed.message);
    // The multibuffer gone: `b`, borrowed and untouched, is released.
    ed.remove_buffer(m);
    assert_eq!(ed.take_released(), vec![b]);
    let _ = std::fs::remove_dir_all(&dir);
}
