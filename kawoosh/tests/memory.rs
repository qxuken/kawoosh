//! The working memory (`:memory`, `<leader>p`): every yank, delete,
//! change and clipboard paste is a moment, newest first, the `"`
//! register its head; the pane puts an older one again, recalls it,
//! goes to where it came from, forgets it.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::layout::Content;
use kawoosh_editor::Took;
use kui::KeyMods;

fn text_of(app: &Kawoosh) -> String {
    app.ed.buffer_of(app.focused_view().unwrap()).text()
}

fn texts(d: &Drive) -> Vec<String> {
    d.core
        .nodes()
        .iter()
        .filter_map(|n| n.text.clone())
        .collect()
}

fn moments(app: &Kawoosh) -> Vec<(Took, String)> {
    app.ed
        .memory
        .moments()
        .iter()
        .map(|m| (m.took, m.text.clone()))
        .collect()
}

#[test]
fn the_memory_keeps_what_passed_and_the_pane_puts_it_again() {
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::new("t", "one\ntwo\nthree\nfour\n");
    d.frame(&mut app);
    // A yank, a delete, a change, each a moment; the same text yanked
    // again while it is the head is not remembered twice.
    d.keys(&mut app, "yy");
    d.keys(&mut app, "yy");
    d.keys(&mut app, "jdd");
    d.keys(&mut app, "jcwFOUR");
    d.key(&mut app, "escape", KeyMods::default());
    assert_eq!(text_of(&app), "one\nthree\nFOUR\n");
    assert_eq!(
        moments(&app),
        [
            (Took::Yank, "one\n".into()),
            (Took::Delete, "two\n".into()),
            (Took::Change, "four".into()),
        ]
    );
    // `p` puts the head; the clipboard's text is a moment too.
    d.keys(&mut app, "p");
    assert_eq!(text_of(&app), "one\nthree\nFOURfour\n");
    let v = app.focused_view().unwrap();
    app.ed.paste_text(v, "clip\n");
    assert_eq!(
        moments(&app).last().unwrap(),
        &(Took::Clipboard, "clip\n".into())
    );
    assert_eq!(text_of(&app), "one\nthree\nFOURfour\nclip\n");
    // The pane: newest at the top, the cursor on it; `j` goes down,
    // older. `⏎` puts the cursor's moment after the caret's line in
    // the editor pane, which takes the keyboard, and the moment is the
    // register from then on.
    d.keys(&mut app, "gg");
    d.keys(&mut app, " p");
    d.frame(&mut app);
    assert_eq!(app.layout.focused_content(), Some(Content::Memory));
    let t = texts(&d);
    assert!(t.iter().any(|s| s.starts_with("4 moments")), "{t:?}");
    assert!(
        t.contains(&"clip".to_string()) && t.contains(&"two".to_string()),
        "{t:?}"
    );
    d.keys(&mut app, "jj");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert!(
        app.focused_view().is_some(),
        "the editor pane has the keyboard"
    );
    assert_eq!(text_of(&app), "one\ntwo\nthree\nFOURfour\nclip\n");
    assert_eq!(app.ed.memory.head().unwrap().text, "two\n");
    assert_eq!(app.ed.memory.len(), 4, "recalled, not copied");
    // `y` recalls without putting (`G` is the oldest); `x` forgets;
    // `o` goes to where a moment came from, its bytes carried through
    // the edits since — `three` was yanked on line 3 and is on line 3
    // still.
    d.keys(&mut app, "jyy");
    d.keys(&mut app, " p");
    d.frame(&mut app);
    d.keys(&mut app, "G");
    d.keys(&mut app, "y");
    assert_eq!(app.ed.memory.head().unwrap().text, "one\n");
    assert!(
        app.ed.message.starts_with("recalled: one"),
        "{}",
        app.ed.message
    );
    assert_eq!(app.layout.focused_content(), Some(Content::Memory));
    assert_eq!(
        moments(&app)
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>(),
        ["four", "clip\n", "two\n", "three\n", "one\n"]
    );
    d.keys(&mut app, "jjj");
    d.keys(&mut app, "x");
    assert_eq!(
        moments(&app)
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>(),
        ["four", "two\n", "three\n", "one\n"]
    );
    d.keys(&mut app, "k");
    d.keys(&mut app, "o");
    d.frame(&mut app);
    let v = app.focused_view().expect("the editor pane again");
    let head = app.ed.views[v].sels.primary().head;
    let buf = app.ed.buffer_of(v);
    assert_eq!(buf.line_of(head), 2, "on `three`");
    // `q` closes the pane.
    d.keys(&mut app, " p");
    d.frame(&mut app);
    d.keys(&mut app, "q");
    d.frame(&mut app);
    assert_ne!(app.layout.focused_content(), Some(Content::Memory));
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn the_memory_is_capped_and_an_origin_can_be_gone() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("t", "a\nb\n");
    d.frame(&mut app);
    for i in 0..(kawoosh_editor::MEMORY_MAX + 5) {
        d.keys(&mut app, &format!("ccx{i}"));
        d.key(&mut app, "escape", KeyMods::default());
    }
    assert_eq!(app.ed.memory.len(), kawoosh_editor::MEMORY_MAX);
    // The line the head came from deleted: its bytes are gone, and `o`
    // says so.
    d.keys(&mut app, "yy");
    d.keys(&mut app, "dd");
    d.keys(&mut app, " p");
    d.frame(&mut app);
    d.keys(&mut app, "o");
    assert!(
        app.ed.message.starts_with("its text is gone from"),
        "{}",
        app.ed.message
    );
    assert_eq!(app.layout.focused_content(), Some(Content::Memory));
}
