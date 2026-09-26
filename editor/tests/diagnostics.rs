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
    ed.diagnostics
        .set(id, None, vec![diag(2, "odd name"), diag(1, "bad\n  why")]);

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
    ed.diagnostics.set_file(p.clone(), Vec::new());
    assert!(ed.diagnostics_listed(None).is_empty());
    assert!(ed.diagnostics.version() > v);
    // Cleared again: nothing moved, nothing to hear.
    let v = ed.diagnostics.version();
    ed.diagnostics.set_file(p, Vec::new());
    assert_eq!(ed.diagnostics.version(), v);
}
