//! Multibuffers (docs/design/search.md Decisions 1–5): excerpts of
//! other buffers kept equal to them both ways, the gaps refusing edits,
//! undo stepping the sources.

use kawoosh_doc::{Buffer, BufferId};
use kawoosh_editor::{Editor, KeyStroke, MultiLine, Part, ViewId};

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
    // Each header, the file it opens.
    use MultiLine::{File, Header};
    assert_eq!(
        lines,
        vec![
            Header(t.a),
            File(t.a, 1),
            File(t.a, 2),
            Header(t.b),
            File(t.b, 1),
            File(t.b, 2)
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
    assert_eq!(t.ed.multi_lines(t.m, 1..2), vec![MultiLine::File(t.a, 2)]);
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
    assert_eq!(
        ed.multi_lines(m, 5..7),
        vec![MultiLine::File(a, 5), MultiLine::File(a, 6)]
    );
    // A `⋯` between two of one file's excerpts is no header.
    assert_eq!(ed.multi_lines(m, 0..1), vec![MultiLine::Header(a)]);
    assert_eq!(ed.multi_lines(m, 4..5), vec![MultiLine::Gap]);
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

/// A list made again (docs/design/lists.md Decision 5): the caret stays
/// on its file's place, and what was changed through the list is still
/// undone by `u`.
#[test]
fn made_again_the_caret_and_the_undo_stay() {
    let mut t = two();
    // On `b1`, and a change made through the multibuffer.
    t.keys("4j");
    assert_eq!(
        t.ed.multi_at(t.m, t.ed.views[t.v].sels.primary().head),
        Some((t.b, 3))
    );
    t.keys("xu<C-r>");
    assert_eq!(t.text(t.b), "b0\n1\nb2");
    // Made again with `a`'s excerpt gone and a header of another size.
    t.ed.fill_multi(
        t.m,
        vec![Part::Gap("the b file\n".into()), Part::Lines(t.b, 0..3)],
    );
    assert_eq!(t.multi(), "the b file\nb0\n1\nb2\n");
    let head = t.ed.views[t.v].sels.primary().head;
    assert_eq!(
        t.ed.multi_at(t.m, head),
        Some((t.b, 3)),
        "on the same place"
    );
    assert_eq!(head, t.ed.multi_offset(t.m, t.b, 3).unwrap());
    t.keys("u");
    assert_eq!(
        t.text(t.b),
        "b0\nb1\nb2",
        "undone through the list made again"
    );
    // A place no excerpt shows any more: the top.
    t.ed.fill_multi(t.m, vec![Part::Lines(t.a, 0..1)]);
    assert_eq!(t.ed.views[t.v].sels.primary().head, 0);
    assert_eq!(t.ed.multi_offset(t.m, t.b, 3), None);
}

/// A gap in a colour: where it is in the text, found again after an
/// edit in an excerpt before it moved it.
#[test]
fn painted_gaps_are_found_where_they_are() {
    let mut ed = Editor::new();
    let a = ed.add_buffer(Buffer::new("a", "a0\na1\na2\n"));
    let m = ed.open_multi(
        "list",
        vec![
            Part::Gap("A\n".into()),
            Part::Lines(a, 0..1),
            Part::Painted("error: boom\n".into(), "error".into()),
            Part::Lines(a, 1..2),
            Part::Painted("the end\n".into(), "dim".into()),
        ],
    );
    assert_eq!(ed.buffers[m].text(), "A\na0\nerror: boom\na1\nthe end\n");
    let text = ed.buffers[m].text();
    let spans: Vec<(String, String)> = ed
        .multi_paints(m)
        .into_iter()
        .map(|(r, c)| (text[r].to_string(), c.to_string()))
        .collect();
    assert_eq!(
        spans,
        [
            ("error: boom\n".to_string(), "error".to_string()),
            ("the end\n".to_string(), "dim".to_string())
        ]
    );
    // Typed into the first excerpt: the paint moves with its gap.
    let v = ed.add_view(m);
    for k in ["j", "A", "x", "x"] {
        let mut st = KeyStroke::plain(k);
        st.text = Some(k.to_string());
        ed.key(v, st);
    }
    ed.sync_multis();
    assert_eq!(ed.buffers[a].text(), "a0xx\na1\na2\n");
    let text = ed.buffers[m].text();
    let (r, _) = ed.multi_paints(m)[0].clone();
    assert_eq!(&text[r], "error: boom\n");
}

/// A source's layer as the multibuffer shows it: each run where its
/// excerpt puts it.
#[test]
fn a_sources_runs_are_found_in_its_excerpts() {
    let mut t = two();
    let b = &mut t.ed.buffers[t.b];
    let v = b.version();
    b.apply(kawoosh_doc::Update {
        layer: "places",
        version: v,
        span: 0..b.len(),
        runs: vec![
            kawoosh_doc::Run {
                range: 0..2,
                style: 0,
                tag: 0,
            },
            kawoosh_doc::Run {
                range: 4..5,
                style: 0,
                tag: 1,
            },
        ],
    })
    .unwrap();
    // `b0` is not shown; `b1`'s `1` is, at the multibuffer's 11.
    let runs = t.ed.multi_runs(t.m, "places");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].0, 11..12);
    assert_eq!(runs[0].1, t.b);
    assert_eq!(&t.multi()[11..12], "1");
}

/// A file of `n` lines `c0`…, and a multibuffer of `parts` over it.
fn long(n: usize, parts: impl Fn(BufferId) -> Vec<Part>) -> T {
    let mut ed = Editor::new();
    let text: String = (0..n).map(|i| format!("c{i}\n")).collect();
    let a = ed.add_buffer(Buffer::new("c", &text));
    let b = ed.add_buffer(Buffer::new("b", "b0\nb1\nb2"));
    let m = ed.open_multi("search", parts(a));
    let v = ed.add_view(m);
    T { ed, v, m, a, b }
}

/// `c5 c6 ⋯ c12 c13` under a header.
fn split_file() -> T {
    long(20, |c| {
        vec![
            Part::Gap("C\n".into()),
            Part::Lines(c, 5..7),
            Part::Gap("⋯\n".into()),
            Part::Lines(c, 12..14),
        ]
    })
}

fn lines(from: usize, to: usize) -> String {
    (from..to).map(|i| format!("c{i}\n")).collect()
}

/// Growing an excerpt (search.md Decision 12): `zk` `zj` show more of
/// its file above and below, COUNT lines, and an excerpt that meets the
/// next of its file is one with it, the `⋯` gone.
#[test]
fn an_excerpt_grows_above_and_below() {
    let mut t = split_file();
    assert_eq!(t.multi(), "C\nc5\nc6\n⋯\nc12\nc13\n");
    t.keys("j2zk");
    assert_eq!(t.multi(), format!("C\n{}⋯\n{}", lines(3, 7), lines(12, 14)));
    assert_eq!(t.ed.message, "2 more lines");
    // The caret stays on its line of the file.
    let head = t.ed.views[t.v].sels.primary().head;
    let c5 = t.ed.buffers[t.a].line_start(5);
    assert_eq!(t.ed.multi_at(t.m, head), Some((t.a, c5)));
    // The gutter numbers the new lines as the file's.
    assert_eq!(
        t.ed.multi_lines(t.m, 1..3),
        vec![MultiLine::File(t.a, 3), MultiLine::File(t.a, 4)]
    );
    t.keys("2zj");
    assert_eq!(t.multi(), format!("C\n{}⋯\n{}", lines(3, 9), lines(12, 14)));
    // `multi.expand`'s 5 reach the next excerpt's first line: one
    // excerpt, the `⋯` gone.
    t.keys("zj");
    assert_eq!(t.multi(), format!("C\n{}", lines(3, 14)));
    assert_eq!(t.ed.message, "3 more lines");
    // To the file's start, and no further.
    t.keys("zk");
    assert_eq!(t.multi(), format!("C\n{}", lines(0, 14)));
    t.keys("zk");
    assert!(t.ed.message.contains("start"), "{}", t.ed.message);
    // To its end, from the command line too.
    let v = t.v;
    t.ed.execute(v, "multi more below 100");
    assert_eq!(t.multi(), format!("C\n{}", lines(0, 20)));
    t.keys("zj");
    assert!(t.ed.message.contains("end"), "{}", t.ed.message);
    assert_eq!(t.text(t.a), lines(0, 20), "the file is not touched");
    assert!(!t.ed.buffers[t.a].modified);
}

/// The lines a growth brought in are the excerpt's like any other: an
/// edit there is in the file, and the file's is in them.
#[test]
fn grown_lines_are_mirrored() {
    let mut t = split_file();
    t.keys("jjzj");
    // c7 c8 c9 c10 c11 came in, and met c12: one excerpt.
    assert_eq!(t.multi(), format!("C\n{}", lines(5, 14)));
    // On c9, a line that came in.
    t.keys("gg5jA!<Esc>");
    assert_eq!(t.ed.buffers[t.a].line_text(9), "c9!");
    // And c12, which was the other excerpt's.
    t.keys("3jA?<Esc>");
    assert_eq!(t.ed.buffers[t.a].line_text(12), "c12?");
    // `u` in the multibuffer steps the file back; the growth stays.
    t.keys("u");
    assert_eq!(t.ed.buffers[t.a].line_text(12), "c12");
    assert_eq!(
        t.multi(),
        format!("C\n{}", lines(5, 14)).replace("c9\n", "c9!\n")
    );
    // An edit in the file is in the grown lines.
    let va = t.view_on(t.a);
    t.keys_on(va, "8GA+<Esc>");
    assert!(t.multi().starts_with("C\nc5\nc6\nc7+\n"), "{}", t.multi());
}

/// On a `⋯`, `zo` (and a click there, the shell's) grows the excerpts on
/// either side toward it; in an excerpt, `zo` and `<S-CR>` (Zed's) grow
/// it both ways.
#[test]
fn on_the_dots_both_sides_grow_toward_them() {
    let mut t = split_file();
    let dots = t.multi().find('⋯').unwrap();
    assert!(t.ed.multi_elided(t.m, dots));
    assert!(!t.ed.multi_elided(t.m, 0), "a header is not");
    assert!(!t.ed.multi_elided(t.m, 3), "nor a file's line");
    t.keys("3j2zo");
    assert_eq!(t.multi(), format!("C\n{}⋯\n{}", lines(5, 9), lines(10, 14)));
    assert_eq!(t.ed.message, "4 more lines");
    // On c10: 5 below, and up only as far as c9, where it meets the
    // excerpt before.
    t.keys("gg6j<S-CR>");
    assert_eq!(t.multi(), format!("C\n{}", lines(5, 19)));
    assert_eq!(t.ed.message, "6 more lines");
    assert!(!t.ed.multi_elided(t.m, dots));
}

/// A last line with no newline: grown to, it gets the multibuffer's
/// own, which is not written back; and a growth stops at its file.
#[test]
fn growth_stops_at_the_file() {
    let mut t = two();
    // `a`'s excerpt is a1 a2, the header `B` after it.
    t.keys("jzj");
    assert_eq!(t.multi(), "A\na1\na2\na3\nB\nb1\nb2\n");
    t.keys("zj");
    assert!(t.ed.message.contains("end"), "{}", t.ed.message);
    t.keys("zk");
    assert_eq!(t.multi(), "A\na0\na1\na2\na3\nB\nb1\nb2\n");
    let mut t = long(3, |_| Vec::new());
    let b = t.b;
    t.m =
        t.ed.open_multi("bare", vec![Part::Gap("B\n".into()), Part::Lines(b, 0..1)]);
    t.v = t.ed.add_view(t.m);
    t.keys("jzj");
    assert_eq!(t.multi(), "B\nb0\nb1\nb2\n");
    t.keys("jjA?<Esc>");
    assert_eq!(t.text(b), "b0\nb1\nb2?", "still bare");
    assert_eq!(t.multi(), "B\nb0\nb1\nb2?\n");
}

/// A diagnostic's message under its line cuts its run in two; the run
/// grows as one, and a gap the caller painted is kept when an excerpt
/// meets the next across it.
#[test]
fn a_run_cut_by_a_note_grows_as_one() {
    let mut t = long(20, |c| {
        vec![
            Part::Gap("C\n".into()),
            Part::Lines(c, 3..6),
            Part::Painted("  error: boom\n".into(), "error".into()),
            Part::Lines(c, 6..8),
            Part::Gap("⋯\n".into()),
            Part::Lines(c, 15..16),
            Part::Painted("  note\n".into(), "info".into()),
            Part::Lines(c, 18..19),
        ]
    });
    // On c6, under the message: above is the run's first excerpt's.
    t.keys("5j2zk");
    assert_eq!(
        t.multi(),
        format!(
            "C\n{}  error: boom\n{}⋯\nc15\n  note\nc18\n",
            lines(1, 6),
            lines(6, 8)
        )
    );
    // Below is the run's last's: it meets c15 across the `⋯`, joined.
    t.keys("10zj");
    assert_eq!(
        t.multi(),
        format!(
            "C\n{}  error: boom\n{}  note\nc18\n",
            lines(1, 6),
            lines(6, 16)
        )
    );
    // Across the painted note: the two touch, the note kept.
    t.keys("G");
    let note = t.multi().find("  note").unwrap();
    assert!(!t.ed.multi_elided(t.m, note), "a note is not `⋯`");
    t.keys("gg16jzj");
    assert_eq!(
        t.multi(),
        format!(
            "C\n{}  error: boom\n{}  note\nc18\n",
            lines(1, 6),
            lines(6, 18)
        )
    );
    // The run now reaches c18: the whole of it grows to the file's end.
    t.keys("zj");
    assert!(t.multi().ends_with("  note\nc18\nc19\n"), "{}", t.multi());
}

/// `zo` on the `⋯` after a run cut by a note: the two sides meet.
#[test]
fn the_dots_after_a_note_close() {
    let mut t = long(30, |c| {
        vec![
            Part::Gap("C\n".into()),
            Part::Lines(c, 2..5),
            Part::Painted("  error: boom\n".into(), "error".into()),
            Part::Lines(c, 5..7),
            Part::Gap("⋯\n".into()),
            Part::Lines(c, 17..22),
        ]
    });
    t.keys("7jzo");
    assert_eq!(
        t.multi(),
        format!("C\n{}  error: boom\n{}", lines(2, 5), lines(5, 22))
    );
    // Grown up to the excerpt over a note: they touch, the note kept.
    let mut t = long(30, |c| {
        vec![
            Part::Gap("C\n".into()),
            Part::Lines(c, 2..4),
            Part::Painted("  note\n".into(), "info".into()),
            Part::Lines(c, 8..10),
        ]
    });
    t.keys("4jzk");
    assert_eq!(
        t.multi(),
        format!("C\n{}  note\n{}", lines(2, 4), lines(4, 10))
    );
    assert_eq!(t.ed.message, "4 more lines");
    // One run now: `zk` from its foot grows its head.
    t.keys("Gkzk");
    assert_eq!(
        t.multi(),
        format!("C\n{}  note\n{}", lines(0, 4), lines(4, 10))
    );
}
