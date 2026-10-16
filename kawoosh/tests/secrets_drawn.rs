//! A private buffer's text is drawn masked wherever it is drawn
//! (docs/design/secrets.md Decision 9), not only in its own pane: a
//! multibuffer over it (the project search, a review), its undo pane,
//! a hunk's preview, `:memory jumps`, the changed files' diff preview,
//! a commit `vcs show` shows.
//! Each fixture is a `.env`-named file whose value no frame may draw.

mod drive;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::KeyMods;

const SECRET: &str = "hunter22";
const OLD: &str = "oldsecret7";

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "kawoosh-secrets-drawn-{tag}-{}",
        std::process::id()
    ));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    kawoosh_systems::fs::canonicalize(&dir).unwrap()
}

fn launch(path: &Path, cwd: &Path) -> (Drive, Kawoosh) {
    let mut d = Drive::new(1200.0, 800.0);
    let mut app = Kawoosh::from_file(path);
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app.set_cwd(cwd);
    d.frame(&mut app);
    app.wait_for_open();
    d.frame(&mut app);
    (d, app)
}

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
    d.frame(app);
}

/// Frames until `f` holds, a few seconds at most.
fn until(
    d: &mut Drive,
    app: &mut Kawoosh,
    what: &str,
    mut f: impl FnMut(&mut Kawoosh, &Drive) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !f(app, d) {
        assert!(
            Instant::now() < deadline,
            "waited for {what}; the message says {:?}; drawn: {}",
            app.ed.message,
            drawn(d)
        );
        std::thread::sleep(Duration::from_millis(10));
        app.wait_for_jobs();
        d.frame(app);
    }
}

/// Every text a frame drew.
fn drawn(d: &Drive) -> String {
    d.core
        .nodes()
        .iter()
        .filter_map(|n| n.text.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

fn git(dir: &Path, args: &[&str]) {
    let out = kawoosh_systems::spawn::output(
        std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "Ann Author")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "Ann Author")
            .env("GIT_COMMITTER_EMAIL", "t@t"),
    )
    .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn has_git() -> bool {
    kawoosh_systems::spawn::output(std::process::Command::new("git").arg("--version")).is_ok()
}

/// A repository with `prod.env` committed with `OLD` and changed to
/// `SECRET` in the working tree, and a plain file beside.
fn repo(tag: &str) -> PathBuf {
    let dir = tmp(tag);
    git(&dir, &["init", "-q", "-b", "main"]);
    std::fs::write(
        dir.join("prod.env"),
        format!("HOST=db\nTOKEN={OLD}\nPORT=1\n"),
    )
    .unwrap();
    std::fs::write(dir.join("a.txt"), "TOKEN here\n").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "first"]);
    std::fs::write(
        dir.join("prod.env"),
        format!("HOST=db\nTOKEN={SECRET}\nPORT=1\n"),
    )
    .unwrap();
    dir
}

fn focused_buffer(app: &Kawoosh) -> kawoosh_doc::BufferId {
    app.ed.views[app.focused_view().unwrap()].buffer
}

fn buffer_named(app: &Kawoosh, name: &str) -> Option<kawoosh_doc::BufferId> {
    app.ed
        .buffers
        .iter()
        .find(|(_, b)| b.name == name)
        .map(|(id, _)| id)
}

/// The project search's results show a private file's line as its pane
/// does: a multibuffer is drawn with its sources' masks.
#[test]
fn a_search_result_masks_a_private_file() {
    let dir = tmp("search");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    std::fs::write(dir.join("prod.env"), format!("HOST=db\nTOKEN={SECRET}\n")).unwrap();
    std::fs::write(dir.join("a.txt"), "TOKEN here\n").unwrap();
    let (mut d, mut app) = launch(&dir.join("a.txt"), &dir);
    ex(&mut d, &mut app, "search project TOKEN");
    until(&mut d, &mut app, "the results", |app, _| {
        buffer_named(app, "*search*").is_some_and(|id| app.ed.buffers[id].text().contains("TOKEN="))
    });
    d.frame(&mut app);
    let shown = drawn(&d);
    assert!(shown.contains("TOKEN="), "the key is drawn: {shown}");
    assert!(!shown.contains(SECRET), "the value is not: {shown}");
    // A yank there of the private file's line is a secret, as in its
    // own pane: never on the clipboard.
    let id = buffer_named(&app, "*search*").unwrap();
    assert!(
        app.ed.buffers[id].private,
        "a mirror of a private file is private"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// `:memory jumps` shows each jump's line as its pane does.
#[test]
fn a_jump_row_masks_its_line() {
    let dir = tmp("jumps");
    let env = dir.join("prod.env");
    let text: String = std::iter::once(format!("TOKEN={SECRET}\n"))
        .chain((0..40).map(|i| format!("# line {i}\n")))
        .collect();
    std::fs::write(&env, text).unwrap();
    let (mut d, mut app) = launch(&env, &dir);
    assert!(app.ed.buffers[focused_buffer(&app)].private);
    d.keys(&mut app, "G");
    d.keys(&mut app, "gg");
    d.frame(&mut app);
    ex(&mut d, &mut app, "memory jumps");
    d.frame(&mut app);
    let shown = drawn(&d);
    assert!(shown.contains("TOKEN="), "the jump's row: {shown}");
    assert!(!shown.contains(SECRET), "masked: {shown}");
    std::fs::remove_dir_all(&dir).ok();
}

/// The undo pane shows a private buffer's states masked.
#[test]
fn the_undo_pane_masks_a_private_buffer() {
    let dir = tmp("undo");
    let env = dir.join("prod.env");
    std::fs::write(&env, format!("TOKEN={SECRET}\n")).unwrap();
    let (mut d, mut app) = launch(&env, &dir);
    d.keys(&mut app, "A9");
    d.key(&mut app, "escape", KeyMods::default());
    d.frame(&mut app);
    ex(&mut d, &mut app, "undo history");
    d.frame(&mut app);
    d.frame(&mut app);
    let shown = drawn(&d);
    assert!(shown.contains("TOKEN="), "the hunk's lines: {shown}");
    assert!(!shown.contains(SECRET), "masked: {shown}");
    std::fs::remove_dir_all(&dir).ok();
}

/// `hunk preview` of a private file: its base and its lines masked.
#[test]
fn a_hunk_preview_masks_a_private_file() {
    if !has_git() {
        eprintln!("git is not installed: skipped");
        return;
    }
    let dir = repo("hunk");
    let (mut d, mut app) = launch(&dir.join("prod.env"), &dir);
    let id = focused_buffer(&app);
    until(&mut d, &mut app, "the base", |app, _| {
        app.ed.base(id).is_some_and(|b| b.version.is_some())
    });
    d.keys(&mut app, "j");
    ex(&mut d, &mut app, "hunk preview");
    until(&mut d, &mut app, "the preview", |app, _| {
        buffer_named(app, "*hunk*").is_some()
    });
    d.frame(&mut app);
    let shown = drawn(&d);
    assert!(
        shown.contains("-TOKEN=") && shown.contains("+TOKEN="),
        "the hunk: {shown}"
    );
    assert!(
        !shown.contains(SECRET) && !shown.contains(OLD),
        "masked: {shown}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The changed files' picker previews a private file's diff masked,
/// the buffer open and edited or not.
#[test]
fn the_changed_files_preview_masks_a_private_file() {
    if !has_git() {
        eprintln!("git is not installed: skipped");
        return;
    }
    let dir = repo("changes");
    let (mut d, mut app) = launch(&dir.join("prod.env"), &dir);
    // The base read on a thread: inline, its answer would land before
    // the preview said it was reading, and the picker ask again.
    app.jobs_inline = false;
    // Edited and not saved: the preview reads the buffer.
    d.keys(&mut app, "ggA");
    d.keys(&mut app, "x");
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "vcs status");
    until(&mut d, &mut app, "the diff preview", |_, d| {
        drawn(d).contains("+TOKEN=")
    });
    let shown = drawn(&d);
    assert!(shown.contains("-TOKEN="), "the base's line: {shown}");
    assert!(
        !shown.contains(SECRET) && !shown.contains(OLD),
        "masked: {shown}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A review (`vcs diff`) of a private file: its lines masked, as the
/// multibuffer it is draws its sources.
#[test]
fn a_review_masks_a_private_file() {
    if !has_git() {
        eprintln!("git is not installed: skipped");
        return;
    }
    let dir = repo("review");
    let (mut d, mut app) = launch(&dir.join("a.txt"), &dir);
    ex(&mut d, &mut app, "vcs diff");
    until(&mut d, &mut app, "the review", |app, _| {
        buffer_named(app, "*vcs diff*")
            .is_some_and(|id| app.ed.buffers[id].text().contains("TOKEN="))
    });
    d.frame(&mut app);
    let shown = drawn(&d);
    assert!(shown.contains("TOKEN="), "the line: {shown}");
    assert!(
        !shown.contains(SECRET) && !shown.contains(OLD),
        "masked: {shown}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// `*` on a masked value: the search it starts does not draw the value
/// (a prompt, a message, a count).
#[test]
fn a_star_search_on_a_mask_does_not_draw_it() {
    let dir = tmp("star");
    let env = dir.join("prod.env");
    std::fs::write(&env, format!("TOKEN={SECRET}\nOTHER={SECRET}\n")).unwrap();
    let (mut d, mut app) = launch(&env, &dir);
    d.keys(&mut app, "f=l*");
    d.frame(&mut app);
    let shown = drawn(&d);
    assert!(!shown.contains(SECRET), "masked: {shown}");
    // It searched all the same: the other line's value is where `*`
    // went.
    assert!(
        app.ed.message.contains("•••••••• ") && app.ed.message.contains("2 match"),
        "{}",
        app.ed.message
    );
    let v = app.focused_view().unwrap();
    let head = app.ed.views[v].sels.primary().head;
    assert_eq!(head, "TOKEN=hunter22\nOTHER=".len(), "on the second");
    std::fs::remove_dir_all(&dir).ok();
}

/// A visual selection over a masked value does not become the project
/// search's pattern, which its field would draw.
#[test]
fn a_masked_selection_is_no_search_pattern() {
    let dir = tmp("seed");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let env = dir.join("prod.env");
    std::fs::write(&env, format!("TOKEN={SECRET}\n")).unwrap();
    let (mut d, mut app) = launch(&env, &dir);
    d.keys(&mut app, "f=lve");
    ex(&mut d, &mut app, "search project");
    d.frame(&mut app);
    let shown = drawn(&d);
    assert!(!shown.contains(SECRET), "masked: {shown}");
    std::fs::remove_dir_all(&dir).ok();
}

/// `:hex` on a private file: its bytes are its text, and no stand-in
/// fits a byte's cell — the pane is not opened, and says why.
#[test]
fn hex_refuses_a_private_file() {
    let dir = tmp("hex");
    let env = dir.join("prod.env");
    std::fs::write(&env, format!("TOKEN={SECRET}\n")).unwrap();
    let (mut d, mut app) = launch(&env, &dir);
    ex(&mut d, &mut app, "hex");
    for _ in 0..3 {
        d.frame(&mut app);
    }
    let shown = drawn(&d);
    assert!(!shown.contains(SECRET), "the text column: {shown}");
    assert!(app.lua_view_pane("hex").is_none(), "no bytes pane");
    assert!(app.ed.message.contains("private"), "{}", app.ed.message);
    std::fs::remove_dir_all(&dir).ok();
}

/// `m` in a private file sets no mark: a mark is a moment whose record
/// is the line's text, drawn by `:memory marks` and kept in the store,
/// and a private buffer is no one's moment (secrets.md Decision 1).
#[test]
fn a_private_file_keeps_no_marks() {
    let dir = tmp("mark");
    let env = dir.join("prod.env");
    std::fs::write(&env, format!("TOKEN={SECRET}\n")).unwrap();
    let db = dir.join("state.db");
    let mut d = Drive::new(1200.0, 800.0);
    let mut app = Kawoosh::new("*scratch*", "");
    app.open_store(Some(&db));
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("e {}", env.display()));
    d.keys(&mut app, "ma");
    d.frame(&mut app);
    assert!(app.ed.message.contains("no marks"), "{}", app.ed.message);
    ex(&mut d, &mut app, "memory marks");
    d.frame(&mut app);
    let shown = drawn(&d);
    assert!(!shown.contains(SECRET), "masked: {shown}");
    std::fs::remove_dir_all(&dir).ok();
}

/// `vcs show` of a commit that changed a private file: the diff's
/// lines masked by the rules of the file its section names, each side
/// read whole, so a block (a private key added to a plain file) is
/// found across its lines.
#[test]
fn a_shown_commit_masks_a_private_file() {
    if !has_git() {
        eprintln!("git is not installed: skipped");
        return;
    }
    let dir = repo("show");
    std::fs::write(
        dir.join("notes.txt"),
        "a key:\n-----BEGIN RSA PRIVATE KEY-----\nMIIEkeybody9\n-----END RSA PRIVATE KEY-----\n",
    )
    .unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "second"]);
    let (mut d, mut app) = launch(&dir.join("a.txt"), &dir);
    ex(&mut d, &mut app, "vcs show HEAD");
    until(&mut d, &mut app, "the commit", |app, _| {
        buffer_named(app, "*show HEAD*").is_some()
    });
    d.frame(&mut app);
    let shown = drawn(&d);
    assert!(
        shown.contains("-TOKEN=") && shown.contains("+TOKEN=") && shown.contains("+a key:"),
        "the diff: {shown}"
    );
    assert!(
        !shown.contains(SECRET) && !shown.contains(OLD) && !shown.contains("MIIEkeybody9"),
        "masked: {shown}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A key in a plain file, committed, then its body replaced: what a
/// review of the masks found drawn raw (2026-10-16).
const KEY_OLD: &str = "MIIEoldbody1";
const KEY_NEW: &str = "MIIEnewbody2";

fn key_text(body: &str) -> String {
    format!(
        "a key:\n-----BEGIN RSA PRIVATE KEY-----\n{body}\n-----END RSA PRIVATE KEY-----\ntail\n"
    )
}

/// `hunk preview` of a key's body replaced in a plain file: the
/// `BEGIN` line is the hunk's context, and the side taken away is
/// masked as the side put in — it was scanned without its context, and
/// the old key drawn.
#[test]
fn a_hunk_preview_masks_a_key_body_replaced() {
    if !has_git() {
        eprintln!("git is not installed: skipped");
        return;
    }
    let dir = tmp("hunk-key");
    git(&dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("notes.txt"), key_text(KEY_OLD)).unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "first"]);
    std::fs::write(dir.join("notes.txt"), key_text(KEY_NEW)).unwrap();
    let (mut d, mut app) = launch(&dir.join("notes.txt"), &dir);
    let id = focused_buffer(&app);
    until(&mut d, &mut app, "the base", |app, _| {
        app.ed.base(id).is_some_and(|b| b.version.is_some())
    });
    d.keys(&mut app, "jj");
    ex(&mut d, &mut app, "hunk preview");
    until(&mut d, &mut app, "the preview", |app, _| {
        buffer_named(app, "*hunk*").is_some()
    });
    d.frame(&mut app);
    let shown = drawn(&d);
    assert!(
        shown.contains("-••••••••") && shown.contains("+••••••••"),
        "the hunk: {shown}"
    );
    assert!(
        !shown.contains(KEY_OLD) && !shown.contains(KEY_NEW),
        "masked: {shown}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The undo pane over a key's body replaced: each state's lines are
/// masked by the block they are in, though the edit's lines hold no
/// `BEGIN`; the change's snippet too.
#[test]
fn the_undo_pane_masks_a_key_body_edited() {
    let dir = tmp("undo-key");
    let file = dir.join("notes.txt");
    std::fs::write(&file, key_text(KEY_OLD)).unwrap();
    let (mut d, mut app) = launch(&file, &dir);
    d.keys(&mut app, "jjcc");
    d.keys(&mut app, KEY_NEW);
    d.key(&mut app, "escape", KeyMods::default());
    d.frame(&mut app);
    ex(&mut d, &mut app, "undo history");
    d.frame(&mut app);
    d.frame(&mut app);
    let shown = drawn(&d);
    assert!(shown.contains("••••••••"), "the hunk's lines: {shown}");
    assert!(
        !shown.contains(KEY_OLD) && !shown.contains(KEY_NEW),
        "masked: {shown}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// `*` on a word under a mask in a file that is not private (a key in
/// a plain file): a secret search, the pattern never said.
#[test]
fn a_star_search_on_a_key_in_a_plain_file_does_not_draw_it() {
    let dir = tmp("star-key");
    let file = dir.join("notes.txt");
    std::fs::write(&file, key_text(KEY_OLD)).unwrap();
    let (mut d, mut app) = launch(&file, &dir);
    assert!(!app.ed.buffers[focused_buffer(&app)].private);
    d.keys(&mut app, "jj*");
    d.frame(&mut app);
    assert!(!app.ed.message.contains(KEY_OLD), "{}", app.ed.message);
    assert!(!drawn(&d).contains(KEY_OLD));
    std::fs::remove_dir_all(&dir).ok();
}

/// A secret search that finds nothing off the frame (a buffer past the
/// frame's budget is walked on a thread) says the stand-in, as the
/// frame's own "not found" does.
#[test]
fn a_secret_search_not_found_off_the_frame_says_no_pattern() {
    let dir = tmp("star-thread");
    let env = dir.join("prod.env");
    std::fs::write(&env, format!("TOKEN={SECRET}\n")).unwrap();
    let (mut d, mut app) = launch(&env, &dir);
    d.keys(&mut app, "f=l*");
    d.frame(&mut app);
    let v = app.focused_view().unwrap();
    let id = focused_buffer(&app);
    let (pattern, ic) = {
        let s = app.ed.search.as_ref().unwrap();
        assert!(s.secret);
        (s.pattern.clone(), s.ignore_case)
    };
    let version = app.ed.buffers[id].version();
    let head = app.ed.views[v].sels.primary().head;
    app.ed
        .search_landed(id, version, &pattern, ic, v, head, None);
    assert!(
        app.ed.message.starts_with("not found"),
        "{}",
        app.ed.message
    );
    assert!(!app.ed.message.contains(SECRET), "{}", app.ed.message);
    std::fs::remove_dir_all(&dir).ok();
}

/// `vcs show` of a commit that renamed `prod.env` and changed it, in a
/// repository whose config colours every diff: the section is masked
/// by the old name's rules as by the new name's (`prod.env.bak` is no
/// `.env`), and the colour does not hide its headers.
#[test]
fn a_shown_commit_masks_a_renamed_file_whatever_the_colour() {
    if !has_git() {
        eprintln!("git is not installed: skipped");
        return;
    }
    let dir = repo("show-rename");
    git(&dir, &["config", "color.ui", "always"]);
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "second"]);
    git(&dir, &["mv", "prod.env", "prod.env.bak"]);
    // One line of three changed: git finds the rename.
    std::fs::write(
        dir.join("prod.env.bak"),
        format!("HOST=db\nTOKEN={SECRET}\nPORT=2\n"),
    )
    .unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "third"]);
    let (mut d, mut app) = launch(&dir.join("a.txt"), &dir);
    ex(&mut d, &mut app, "vcs show HEAD");
    until(&mut d, &mut app, "the commit", |app, _| {
        buffer_named(app, "*show HEAD*").is_some()
    });
    d.frame(&mut app);
    let shown = drawn(&d);
    assert!(
        shown.contains("rename to prod.env.bak") && shown.contains("+PORT="),
        "the diff: {shown}"
    );
    assert!(!shown.contains(SECRET), "masked: {shown}");
    std::fs::remove_dir_all(&dir).ok();
}
