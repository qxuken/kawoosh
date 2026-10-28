//! Files onto the clipboard as the files (kui F247): `<leader>yf` puts
//! an editor's file there, a listing's entries under the caret or the
//! selection, `../` the listed directory, as a file manager's Copy
//! puts them — to paste into Finder, Explorer, a mail or a chat — and
//! their paths into the register. One not on disk is said, and nothing
//! copied.

mod drive;

use std::path::{Path, PathBuf};

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::KeyMods;

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-copy-files-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    kawoosh_systems::fs::canonicalize(&dir).unwrap()
}

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
    d.frame(app);
}

fn app() -> (Drive, Kawoosh) {
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::new("*scratch*", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    (d, app)
}

/// The files kawoosh handed kui's clipboard since the last look.
fn files_out(d: &mut Drive) -> Vec<Vec<String>> {
    d.core
        .take_menu_actions()
        .into_iter()
        .filter_map(|a| match a {
            kui_native::MenuAction::SetClipboardFiles { paths } => Some(paths),
            _ => None,
        })
        .collect()
}

fn whole(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[test]
fn leader_yf_copies_the_file_itself_and_a_listing_s_entries() {
    let root = tmp("yf");
    for f in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(root.join(f), f).unwrap();
    }
    let (mut d, mut app) = app();
    ex(&mut d, &mut app, &format!("cd {}", root.display()));
    ex(&mut d, &mut app, "e a.txt");

    // An editor's file: whole, and its path the register's newest.
    d.press(&mut app, "<leader>yf");
    d.frame(&mut app);
    assert_eq!(files_out(&mut d), [vec![whole(&root.join("a.txt"))]]);
    assert!(
        app.ed.message.starts_with("copied the file "),
        "{}",
        app.ed.message
    );
    let head = app.ed.memory.head().unwrap();
    assert_eq!(head.text, whole(&root.join("a.txt")));
    assert_eq!(app.clipboard_last(), Some(head.text.as_str()));

    // A listing: the entry under the caret, the listed directory on `../`.
    d.press(&mut app, "-");
    d.frame(&mut app);
    d.press(&mut app, "<leader>yf");
    d.frame(&mut app);
    assert_eq!(files_out(&mut d), [vec![whole(&root.join("a.txt"))]]);
    d.press(&mut app, "gg");
    d.press(&mut app, "<leader>yf");
    d.frame(&mut app);
    assert_eq!(files_out(&mut d), [vec![whole(&root)]], "../");

    // A visual selection: every entry it covers, in one copy.
    d.press(&mut app, "G");
    d.press(&mut app, "Vk");
    d.press(&mut app, "<leader>yf");
    d.frame(&mut app);
    assert_eq!(
        files_out(&mut d),
        [vec![whole(&root.join("b.txt")), whole(&root.join("c.txt"))]]
    );
    assert_eq!(app.ed.message, "copied 2 files");
    assert_eq!(
        app.ed.mode(app.focused_view().unwrap()),
        kawoosh_editor::Mode::Normal
    );

    // A file not on disk yet: said, and the clipboard left alone.
    ex(&mut d, &mut app, "e new.txt");
    d.press(&mut app, "<leader>yf");
    d.frame(&mut app);
    assert!(files_out(&mut d).is_empty());
    assert!(
        app.ed.message.ends_with("new.txt: not there"),
        "{}",
        app.ed.message
    );
    std::fs::remove_dir_all(&root).ok();
}
