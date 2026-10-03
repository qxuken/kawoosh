//! The editor's diagnostics (docs/design/lists.md Decisions 1–2): a
//! buffer's list beside its layer, a file's kept by its server's
//! positions, taken by a buffer that opens it and left to it on close.

use std::path::PathBuf;

use kawoosh_doc::diagnostic::{LAYER, Placed};
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
