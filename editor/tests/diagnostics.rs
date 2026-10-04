//! The editor's diagnostics (docs/design/lists.md Decisions 1–2): a
//! buffer's list beside its layer, a file's kept by its server's
//! positions, taken by a buffer that opens it and left to it on close.

use std::path::PathBuf;

use kawoosh_doc::diagnostic::{Columns, LAYER, Placed};
use kawoosh_doc::{Buffer, Diagnostic, Run, Update};
use kawoosh_editor::Editor;

fn diag(severity: u32, message: &str) -> Diagnostic {
    Diagnostic {
        severity,
        message: message.into(),
        source: None,
        code: None,
        from: None,
    }
}

fn opened(ed: &mut Editor, path: &str, text: &str) -> kawoosh_doc::BufferId {
    let mut b = Buffer::new(path, text);
    b.path = Some(PathBuf::from(path));
    ed.add_buffer(b)
}

#[test]
fn a_closed_buffer_leaves_its_diagnostics_to_its_file_and_an_open_takes_them() {
    let mut ed = Editor::new();
    // `é` is two bytes and one UTF-16 unit: the file's column is the
    // server's count, not the bytes.
    let id = opened(&mut ed, "/p/a.rs", "fn é() {\n    bad\n}\n");
    let b = &mut ed.buffers[id];
    let v = b.version();
    b.apply(Update {
        layer: LAYER,
        version: v,
        span: 0..b.len(),
        runs: vec![
            Run {
                range: 3..5,
                style: 2,
                tag: 0,
            },
            Run {
                range: 14..17,
                style: 1,
                tag: 1,
            },
        ],
    })
    .unwrap();
    ed.diagnostics.set(
        id,
        None,
        None,
        vec![diag(2, "odd name"), diag(1, "bad\n  why")],
    );

    let listed = ed.diagnostics_listed(Some(id));
    assert_eq!(listed.len(), 2);
    assert_eq!(
        (listed[0].line, listed[0].col, listed[0].end_col),
        (0, 3, 4)
    );
    assert_eq!((listed[1].line, listed[1].col), (1, 4));
    assert_eq!(listed[1].diagnostic.message, "bad\n  why");

    let before = ed.diagnostics.version();
    ed.remove_buffer(id);
    assert!(ed.diagnostics.version() > before);
    let kept = ed.diagnostics.file(std::path::Path::new("/p/a.rs"));
    assert_eq!(
        kept.iter()
            .map(|p| (p.line, p.character, p.end_character))
            .collect::<Vec<_>>(),
        [(0, 3, 4), (1, 4, 7)]
    );
    let all = ed.diagnostics_listed(None);
    assert_eq!(all.len(), 2);
    assert!(all.iter().all(|l| l.buffer.is_none()));

    // Opened again: its own layer, the same bytes.
    let id = opened(&mut ed, "/p/a.rs", "fn é() {\n    bad\n}\n");
    ed.adopt_file_diagnostics();
    assert!(
        ed.diagnostics
            .file(std::path::Path::new("/p/a.rs"))
            .is_empty()
    );
    let runs = ed.buffers[id].runs(LAYER, 0..100);
    assert_eq!(
        runs.iter().map(|r| r.range.clone()).collect::<Vec<_>>(),
        [3..5, 14..17]
    );
    assert_eq!(
        ed.diagnostics.get(id, runs[1].tag).unwrap().message,
        "bad\n  why"
    );
}

#[test]
fn a_file_the_server_cleared_is_gone() {
    let mut ed = Editor::new();
    let p = PathBuf::from("/p/b.rs");
    ed.diagnostics.set_file(
        p.clone(),
        None,
        vec![Placed {
            line: 0,
            character: 0,
            end_line: 0,
            end_character: 1,
            columns: Columns::Utf16,
            diagnostic: diag(1, "x"),
        }],
    );
    assert_eq!(ed.diagnostics_listed(None).len(), 1);
    let v = ed.diagnostics.version();
    ed.diagnostics.set_file(p.clone(), None, Vec::new());
    assert!(ed.diagnostics_listed(None).is_empty());
    assert!(ed.diagnostics.version() > v);
    // Cleared again: nothing moved, nothing to hear.
    let v = ed.diagnostics.version();
    ed.diagnostics.set_file(p, None, Vec::new());
    assert_eq!(ed.diagnostics.version(), v);
}

/// A plugin's diagnostics and the servers' in one layer: each word
/// replaces its publisher's own, the other's carried through the edits
/// between; a plugin's taken back whole; a closed buffer's left to its
/// file still marked, and taken again beside the servers' next word.
#[test]
fn a_plugins_diagnostics_live_beside_the_servers() {
    let mut ed = Editor::new();
    let id = opened(&mut ed, "/p/c.rs", "let a = 1;\nlet b = 2;\n");
    let lsp = |ed: &mut Editor, at: std::ops::Range<usize>, msg: &str| {
        let b = &ed.buffers[id];
        let update = Update {
            layer: LAYER,
            version: b.version(),
            span: 0..b.len(),
            runs: vec![Run {
                range: at,
                style: 1,
                tag: 0,
            }],
        };
        assert!(ed.publish_diagnostics(id, None, update, vec![diag(1, msg)]));
    };
    let mark = |line, character, end_character, msg: &str| Placed {
        line,
        character,
        end_line: line,
        end_character,
        columns: Columns::Chars,
        diagnostic: diag(2, msg),
    };
    let said = |ed: &Editor| {
        let mut v: Vec<(String, Option<String>, std::ops::Range<usize>)> = ed
            .diagnostics_listed(Some(id))
            .into_iter()
            .map(|l| (l.diagnostic.message, l.diagnostic.from, l.range.unwrap()))
            .collect();
        v.sort_by_key(|x| x.2.start);
        v
    };
    lsp(&mut ed, 4..5, "unused a");
    assert!(ed.publish_placed(id, "lint", vec![mark(1, 4, 5, "b, really?")]));
    assert_eq!(
        said(&ed),
        [
            ("unused a".into(), None, 4..5),
            ("b, really?".into(), Some("lint".into()), 15..16),
        ]
    );
    // An edit before both carries both; the servers' next word keeps
    // the plugin's where the edit put it.
    ed.buffers[id].replace(0..0, "// x\n");
    lsp(&mut ed, 9..10, "still unused");
    assert_eq!(
        said(&ed),
        [
            ("still unused".into(), None, 9..10),
            ("b, really?".into(), Some("lint".into()), 20..21),
        ]
    );
    // The plugin's again replaces its own alone.
    assert!(ed.publish_placed(id, "lint", vec![mark(2, 0, 3, "let")]));
    assert_eq!(said(&ed).len(), 2);
    assert_eq!(said(&ed)[1], ("let".into(), Some("lint".into()), 16..19));

    // Closed: the file keeps both, each still its publisher's.
    ed.remove_buffer(id);
    let path = std::path::Path::new("/p/c.rs");
    let mut froms: Vec<Option<String>> = ed
        .diagnostics
        .file(path)
        .iter()
        .map(|p| p.diagnostic.from.clone())
        .collect();
    froms.sort();
    assert_eq!(froms, [None, Some("lint".into())]);
    // A plugin's word for the file replaces its own there too.
    ed.diagnostics
        .set_file(path.to_path_buf(), Some("lint"), vec![mark(0, 0, 2, "//")]);
    assert_eq!(ed.diagnostics.file(path).len(), 2);

    // Taken back whole: the file's, and a buffer's.
    ed.clear_diagnostics_from("lint");
    assert_eq!(ed.diagnostics.file(path).len(), 1);
    let other = opened(&mut ed, "/p/d.rs", "x\n");
    assert!(ed.publish_placed(other, "lint", vec![mark(0, 0, 1, "x")]));
    let v = ed.diagnostics.version();
    ed.clear_diagnostics_from("lint");
    assert!(ed.diagnostics.version() > v);
    assert!(ed.diagnostics_listed(Some(other)).is_empty());
    assert!(ed.buffers[other].runs(LAYER, 0..2).is_empty());

    // Opened again, a plugin's word first: the file's kept one is
    // taken in beside it, not over it.
    let id = opened(&mut ed, "/p/c.rs", "// x\nlet a = 1;\nlet b = 2;\n");
    assert!(ed.publish_placed(id, "lint", vec![mark(2, 4, 5, "b")]));
    ed.adopt_file_diagnostics();
    assert!(ed.diagnostics.file(path).is_empty());
    let mut both: Vec<(String, Option<String>)> = ed
        .diagnostics_listed(Some(id))
        .into_iter()
        .map(|l| (l.diagnostic.message, l.diagnostic.from))
        .collect();
    both.sort();
    assert_eq!(
        both,
        [
            ("b".into(), Some("lint".into())),
            ("still unused".into(), None)
        ]
    );
}

/// A word that says again what was said moves no version: a linter
/// republishing on every reparse — or from `on_diagnostics`, which the
/// version wakes — wakes no listener (lists.md Decision 7). An empty
/// word on a buffer that has none, a server's same answer, and a
/// file's same list are each nothing new; a change is.
#[test]
fn saying_again_what_was_said_moves_nothing() {
    let mut ed = Editor::new();
    let id = opened(&mut ed, "/p/e.rs", "let a = 1;\nlet b = 2;\n");
    let mark = |line, character, end_character, msg: &str| Placed {
        line,
        character,
        end_line: line,
        end_character,
        columns: Columns::Chars,
        diagnostic: diag(2, msg),
    };
    let v = ed.diagnostics.version();
    assert!(ed.publish_placed(id, "lint", Vec::new()));
    assert_eq!(ed.diagnostics.version(), v, "none, and none again");

    assert!(ed.publish_placed(id, "lint", vec![mark(0, 4, 5, "a?")]));
    let v = ed.diagnostics.version();
    assert!(v > 0);
    assert!(ed.publish_placed(id, "lint", vec![mark(0, 4, 5, "a?")]));
    assert_eq!(ed.diagnostics.version(), v, "the same again");

    // Carried by an edit, then said again where the edit put it.
    ed.buffers[id].replace(0..0, "\n");
    assert!(ed.publish_placed(id, "lint", vec![mark(1, 4, 5, "a?")]));
    assert_eq!(ed.diagnostics.version(), v, "where the layer has it");

    // A server's same answer beside it.
    let server = |ed: &mut Editor| {
        let b = &ed.buffers[id];
        let update = Update {
            layer: LAYER,
            version: b.version(),
            span: 0..b.len(),
            runs: vec![Run {
                range: 13..14,
                style: 1,
                tag: 0,
            }],
        };
        assert!(ed.publish_diagnostics(id, None, update, vec![diag(1, "b!")]));
    };
    server(&mut ed);
    let v = ed.diagnostics.version();
    server(&mut ed);
    assert_eq!(ed.diagnostics.version(), v);

    // Something else said moves it.
    assert!(ed.publish_placed(id, "lint", vec![mark(1, 4, 5, "a, really?")]));
    assert!(ed.diagnostics.version() > v);
    let v = ed.diagnostics.version();
    assert!(ed.publish_placed(id, "lint", Vec::new()));
    assert!(ed.diagnostics.version() > v, "taken back");

    // A file's same list.
    let p = PathBuf::from("/p/f.rs");
    ed.diagnostics
        .set_file(p.clone(), Some("lint"), vec![mark(0, 0, 1, "f")]);
    let v = ed.diagnostics.version();
    ed.diagnostics
        .set_file(p.clone(), Some("lint"), vec![mark(0, 0, 1, "f")]);
    assert_eq!(ed.diagnostics.version(), v);
    ed.diagnostics
        .set_file(p, Some("lint"), vec![mark(0, 0, 2, "f")]);
    assert!(ed.diagnostics.version() > v);
}

/// A plugin's columns are characters in and out, whether a buffer
/// holds the file or not (lists.md Decision 7): kept by a path, placed
/// by the buffer that opens it in characters — past the BMP, where a
/// server's UTF-16 count differs — and a closed buffer's read back as
/// they were read open.
#[test]
fn a_plugins_columns_are_characters_on_a_file_too() {
    let mut ed = Editor::new();
    let path = std::path::Path::new("/p/emoji.txt");
    let text = "😀😀x\n";
    // `x` is the third character: bytes 8..9, UTF-16 units 4..5.
    let at = |columns, character, end_character, msg: &str| Placed {
        line: 0,
        character,
        end_line: 0,
        end_character,
        columns,
        diagnostic: diag(1, msg),
    };
    ed.diagnostics.set_file(
        path.to_path_buf(),
        Some("lint"),
        vec![at(Columns::Chars, 2, 3, "plugin's x")],
    );
    ed.diagnostics.set_file(
        path.to_path_buf(),
        None,
        vec![at(Columns::Utf16, 4, 5, "server's x")],
    );
    let listed = |ed: &Editor| {
        let mut v: Vec<(String, usize, usize)> = ed
            .diagnostics_listed(None)
            .into_iter()
            .map(|l| (l.diagnostic.message, l.col, l.end_col))
            .collect();
        v.sort();
        v
    };
    // Unopened: each read back as given — the server's in its count.
    assert_eq!(
        listed(&ed),
        [("plugin's x".into(), 2, 3), ("server's x".into(), 4, 5)]
    );

    let id = opened(&mut ed, "/p/emoji.txt", text);
    ed.adopt_file_diagnostics();
    let runs = ed.buffers[id].runs(LAYER, 0..100);
    assert_eq!(
        runs.iter().map(|r| r.range.clone()).collect::<Vec<_>>(),
        [8..9, 8..9],
        "both on the x"
    );
    assert_eq!(
        listed(&ed),
        [("plugin's x".into(), 2, 3), ("server's x".into(), 2, 3)]
    );

    // Closed: kept in characters, read as they were open, and placed
    // there again.
    ed.remove_buffer(id);
    assert!(
        ed.diagnostics
            .file(path)
            .iter()
            .all(|p| p.columns == Columns::Chars)
    );
    assert_eq!(
        listed(&ed),
        [("plugin's x".into(), 2, 3), ("server's x".into(), 2, 3)]
    );
    let id = opened(&mut ed, "/p/emoji.txt", text);
    ed.adopt_file_diagnostics();
    assert_eq!(
        ed.buffers[id]
            .runs(LAYER, 0..100)
            .iter()
            .map(|r| r.range.clone())
            .collect::<Vec<_>>(),
        [8..9, 8..9]
    );
}

/// On Windows a file's name is matched case aside: kept under one
/// spelling, taken by a buffer opened under another.
#[cfg(windows)]
#[test]
fn a_files_diagnostics_are_its_however_its_case_is_spelled() {
    let mut ed = Editor::new();
    ed.diagnostics.set_file(
        PathBuf::from(r"C:\Proj\Main.rs"),
        Some("lint"),
        vec![Placed {
            line: 0,
            character: 0,
            end_line: 0,
            end_character: 1,
            columns: Columns::Chars,
            diagnostic: diag(1, "m"),
        }],
    );
    assert_eq!(
        ed.diagnostics
            .file(std::path::Path::new("c:/proj/main.rs"))
            .len(),
        1
    );
    let id = opened(&mut ed, r"c:\proj\main.rs", "m\n");
    ed.adopt_file_diagnostics();
    assert_eq!(ed.diagnostics_listed(Some(id)).len(), 1);
    assert_eq!(ed.diagnostics.files().count(), 0);
}
