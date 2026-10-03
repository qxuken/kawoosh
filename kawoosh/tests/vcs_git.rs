//! Version control with git behind it (docs/design/vcs.md Decisions
//! 4–9): a file in a repository gets its base from the index, its
//! changes signed; `vcs diff` reviews the working tree, `vcs diff main`
//! what a branch did; `vcs blame` puts the column on; `vcs worktree
//! add` makes a worktree and a tab on it. Skipped where git is not
//! installed.

mod drive;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh_editor::Sign;
use kui_native::KeyMods;

fn git(dir: &Path, args: &[&str]) -> String {
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
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn has_git() -> bool {
    kawoosh_systems::spawn::output(std::process::Command::new("git").arg("--version")).is_ok()
}

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-vcs-git-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    kawoosh_systems::fs::canonicalize(&dir).unwrap()
}

/// A repository with one commit on `main`, `a.txt` changed in the
/// working tree since.
fn repo(tag: &str) -> PathBuf {
    let dir = tmp(tag);
    git(&dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\ngone\nfour\n").unwrap();
    std::fs::write(dir.join("b.txt"), "b\n").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "first"]);
    std::fs::write(dir.join("a.txt"), "one\ntwo!\nthree\nfour\nfive\n").unwrap();
    dir
}

fn launch(path: &Path, cwd: &Path) -> (Drive, Kawoosh) {
    let mut d = Drive::new(1000.0, 600.0);
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
fn until(d: &mut Drive, app: &mut Kawoosh, what: &str, mut f: impl FnMut(&Kawoosh) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !f(app) {
        assert!(
            Instant::now() < deadline,
            "waited for {what}; the message says {:?}",
            app.ed.message
        );
        std::thread::sleep(Duration::from_millis(10));
        d.frame(app);
    }
}

fn focused_buffer(app: &Kawoosh) -> Option<kawoosh_doc::BufferId> {
    app.focused_view().map(|v| app.ed.views[v].buffer)
}

fn buffer_named(app: &Kawoosh, name: &str) -> Option<kawoosh_doc::BufferId> {
    app.ed
        .buffers
        .iter()
        .find(|(_, b)| b.name == name)
        .map(|(id, _)| id)
}

#[test]
fn a_file_in_a_repository_is_signed_against_the_index_and_reviewed() {
    if !has_git() {
        eprintln!("git is not installed: skipped");
        return;
    }
    let dir = repo("signs");
    let file = dir.join("a.txt");
    let (mut d, mut app) = launch(&file, &dir);
    let id = focused_buffer(&app).unwrap();
    until(&mut d, &mut app, "the base from the index", |app| {
        app.ed.base(id).is_some_and(|b| b.version.is_some())
    });
    assert_eq!(app.ed.base(id).unwrap().label, "index");
    let signs = app.ed.signs_in(id, 0..10);
    assert_eq!(signs.get(&1), Some(&Sign::Modified));
    assert_eq!(signs.get(&3), Some(&Sign::Deleted));
    assert_eq!(signs.get(&4), Some(&Sign::Added));

    // `:vcs` names the backend and the branch.
    ex(&mut d, &mut app, "vcs");
    until(&mut d, &mut app, "the vcs line", |app| {
        app.ed.message.starts_with("git at ")
    });
    assert!(
        app.ed.message.contains(", on main — base status"),
        "{}",
        app.ed.message
    );

    // `<leader>hd`: the working tree against the index, as a review.
    d.press(&mut app, "<leader>hd");
    until(&mut d, &mut app, "the review", |app| {
        buffer_named(app, "*vcs diff*").is_some()
    });
    let review = buffer_named(&app, "*vcs diff*").unwrap();
    until(&mut d, &mut app, "the review shown", |app| {
        focused_buffer(app) == Some(review)
    });
    let text = app.ed.buffers[review].text();
    assert_eq!(
        text, "a.txt  +2 −2\none\ntwo\ntwo!\nthree\ngone\nfour\nfive\n",
        "the base's lines as gaps before the new ones"
    );
    assert!(
        app.ed
            .message
            .starts_with("1 file against the index: +2 −2"),
        "{}",
        app.ed.message
    );
    // The excerpt lines carry the file's signs; `]h` walks them.
    let signs = app.signs_of(
        review,
        0,
        &app.ed
            .multi_lines(review, 0..app.ed.buffers[review].line_count()),
    );
    assert_eq!(signs.get(&3), Some(&Sign::Modified), "two! in the review");
    assert_eq!(signs.get(&7), Some(&Sign::Added), "five in the review");
    d.press(&mut app, "gg]h");
    let v = app.focused_view().unwrap();
    let b = app.ed.buffer_of(v);
    assert_eq!(b.line_of(app.ed.views[v].sels.primary().head), 3);
    // `q` closes it, back to the file.
    d.press(&mut app, "q");
    d.frame(&mut app);
    assert_eq!(focused_buffer(&app), Some(id));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_branch_is_reviewed_against_main_and_blamed() {
    if !has_git() {
        eprintln!("git is not installed: skipped");
        return;
    }
    let dir = repo("branch");
    // The working change committed on a branch; main moves on too, so
    // the merge base is what the review reads against.
    git(&dir, &["checkout", "-q", "-b", "feature"]);
    git(&dir, &["commit", "-q", "-am", "on feature"]);
    git(&dir, &["checkout", "-q", "main"]);
    std::fs::write(dir.join("b.txt"), "b\nmain moved\n").unwrap();
    git(&dir, &["commit", "-q", "-am", "on main"]);
    git(&dir, &["checkout", "-q", "feature"]);
    let file = dir.join("a.txt");
    let (mut d, mut app) = launch(&file, &dir);
    let id = focused_buffer(&app).unwrap();
    until(&mut d, &mut app, "the base", |app| {
        app.ed.base(id).is_some_and(|b| b.version.is_some())
    });
    assert!(
        app.ed.hunks(id).is_empty(),
        "committed: nothing against the index"
    );

    d.press(&mut app, "<leader>hm");
    until(&mut d, &mut app, "the branch review", |app| {
        buffer_named(app, "*vcs diff main*").is_some()
    });
    let review = buffer_named(&app, "*vcs diff main*").unwrap();
    until(&mut d, &mut app, "the review shown", |app| {
        focused_buffer(app) == Some(review)
    });
    // The file's lines are as the checkout wrote them — CRLF where
    // `core.autocrlf` says so — and the base's are the review's own.
    assert_eq!(
        app.ed.buffers[review].text().replace("\r\n", "\n"),
        "a.txt  +2 −2\none\ntwo\ntwo!\nthree\ngone\nfour\nfive\n",
        "what the branch did, not what main did since"
    );
    assert!(
        app.ed.message.starts_with("1 file against main: +2 −2"),
        "{}",
        app.ed.message
    );
    // The file's own base is the merge base now: its gutter shows the
    // branch's work.
    until(&mut d, &mut app, "the file's base", |app| {
        app.ed
            .base(id)
            .is_some_and(|b| b.version.is_some() && b.label != "index")
    });
    assert_eq!(app.ed.signs_in(id, 0..10).get(&1), Some(&Sign::Modified));
    d.press(&mut app, "q");
    d.frame(&mut app);

    // Two revisions: main against feature, the new side in scratches.
    ex(&mut d, &mut app, "vcs diff main feature");
    until(&mut d, &mut app, "the revision review", |app| {
        buffer_named(app, "*vcs diff main..feature*").is_some()
    });
    let review = buffer_named(&app, "*vcs diff main..feature*").unwrap();
    until(&mut d, &mut app, "the review shown", |app| {
        focused_buffer(app) == Some(review)
    });
    let text = app.ed.buffers[review].text();
    assert!(text.starts_with("a.txt  +2 −2\n"), "{text}");
    assert!(
        text.contains("b.txt  +0 −1\nb\nmain moved\n"),
        "main's line gone on feature: {text}"
    );
    let scratch = buffer_named(&app, "vcs:feature:a.txt").expect("the scratch");
    assert!(app.ed.buffers[scratch].read_only);
    assert_eq!(app.ed.base(scratch).unwrap().label, "main");
    d.press(&mut app, "q");
    d.frame(&mut app);

    // `<leader>hb`: the blame column, its labels the author and when.
    d.press(&mut app, "<leader>hb");
    until(&mut d, &mut app, "the blame", |app| {
        app.ed.blame_width(id) > 0
    });
    let labels = app.ed.blame_labels(id, 0..10);
    assert_eq!(
        labels.get(&0),
        Some(&("Ann Author · now".to_string(), true))
    );
    // `three` and `four` are the first commit's, one run: the label on
    // its first line only. `two!` and `five` are the branch's.
    assert_eq!(
        labels.get(&2),
        Some(&("Ann Author · now".to_string(), true))
    );
    assert_eq!(
        labels.get(&3),
        Some(&("Ann Author · now".to_string(), false)),
        "one run"
    );
    let rev = |ln: usize| app.ed.blame_at(id, ln).map(|r| r.summary.clone());
    assert_eq!(rev(0).as_deref(), Some("first"));
    assert_eq!(rev(1).as_deref(), Some("on feature"));
    assert_eq!(rev(4).as_deref(), Some("on feature"));
    assert!(
        d.gutter_texts().iter().any(|t| t == "Ann Author · now"),
        "{:?}",
        d.gutter_texts()
    );
    // `<leader>hs`: the caret line's commit shown — `two!`, the branch's.
    d.press(&mut app, "ggj");
    d.press(&mut app, "<leader>hs");
    until(&mut d, &mut app, "the show buffer", |app| {
        app.ed
            .buffers
            .values()
            .any(|b| b.name.starts_with("*show "))
    });
    let shown = app
        .ed
        .buffers
        .values()
        .find(|b| b.name.starts_with("*show "))
        .unwrap();
    assert_eq!(shown.language.as_ref(), "diff");
    assert!(shown.text().contains("on feature"), "{}", shown.text());
    assert!(shown.text().contains("+two!"), "{}", shown.text());
    d.press(&mut app, "q");
    d.frame(&mut app);
    let names: Vec<String> = app
        .ed
        .buffers
        .iter()
        .map(|(i, b)| format!("{i:?}={}", b.name))
        .collect();
    let panes: Vec<String> = app
        .ed
        .views
        .values()
        .map(|v| app.ed.buffers[v.buffer].name.clone())
        .collect();
    assert_eq!(
        focused_buffer(&app),
        Some(id),
        "back in the file after `q`: {names:?} panes {panes:?}"
    );
    d.press(&mut app, "<leader>hb");
    until(&mut d, &mut app, "the blame off", |app| {
        app.ed.blame_width(id) == 0
    });
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_worktree_is_made_and_opened_as_a_tab() {
    if !has_git() {
        eprintln!("git is not installed: skipped");
        return;
    }
    let dir = repo("worktree");
    git(&dir, &["commit", "-q", "-am", "second"]);
    let file = dir.join("a.txt");
    let (mut d, mut app) = launch(&file, &dir);
    ex(&mut d, &mut app, "vcs worktree add spike");
    let wt = dir.join(".worktrees").join("spike");
    until(&mut d, &mut app, "the worktree", |app| app.cwd == wt);
    assert!(wt.join("a.txt").is_file());
    assert_eq!(app.layout.tabs.len(), 2, "a tab on it");
    let branches = git(&dir, &["branch", "--list", "spike"]);
    assert!(branches.contains("spike"), "{branches}");
    let exclude = std::fs::read_to_string(dir.join(".git/info/exclude")).unwrap_or_default();
    assert!(exclude.contains("/.worktrees/"), "{exclude}");
    let status = git(&dir, &["status", "--porcelain"]);
    assert!(
        status.is_empty(),
        "the worktrees' directory is excluded: {status}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A repository whose `core.autocrlf` is the test's, not the
/// machine's: `a.txt` committed with LF lines, `one two three gone
/// four`.
fn staging_repo(tag: &str, autocrlf: bool) -> PathBuf {
    let dir = tmp(tag);
    git(&dir, &["init", "-q", "-b", "main"]);
    git(
        &dir,
        &[
            "config",
            "core.autocrlf",
            if autocrlf { "true" } else { "false" },
        ],
    );
    std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\ngone\nfour\n").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "first"]);
    dir
}

/// The index's text of `rel`, as git keeps it.
fn index_text(dir: &Path, rel: &str) -> String {
    git(dir, &["show", &format!(":{rel}")])
}

/// Waits for the message `said`, then for buffer `id`'s base to be
/// read again with `staged` hunks under it.
fn staged_after(
    d: &mut Drive,
    app: &mut Kawoosh,
    id: kawoosh_doc::BufferId,
    said: &str,
    staged: usize,
) {
    until(d, app, said, |app| app.ed.message == said);
    until(d, app, "the base read again", |app| {
        app.ed
            .base(id)
            .is_some_and(|b| b.version.is_some() && b.staged.len() == staged)
    });
}

/// `hunk stage` and `hunk unstage` (docs/design/vcs.md Decision 12):
/// the caret's hunk taken into the index and back out, the staged
/// lines' signs faint in the gutter; the selection's hunks; the whole
/// buffer; and a hunk staged from the review.
#[test]
fn a_hunk_is_staged_and_unstaged_from_the_gutter() {
    if !has_git() {
        eprintln!("git is not installed: skipped");
        return;
    }
    let dir = staging_repo("stage", false);
    let head = "one\ntwo\nthree\ngone\nfour\n";
    std::fs::write(dir.join("a.txt"), "one\ntwo!\nthree\nfour\nfive\n").unwrap();
    let file = dir.join("a.txt");
    let (mut d, mut app) = launch(&file, &dir);
    let id = focused_buffer(&app).unwrap();
    until(&mut d, &mut app, "the base from the index", |app| {
        app.ed.base(id).is_some_and(|b| b.version.is_some())
    });
    assert!(app.ed.base(id).unwrap().head.is_some(), "HEAD under it");
    assert!(app.ed.staged_signs_in(id, 0..10).is_empty());
    // Nothing staged: nothing to unstage.
    d.press(&mut app, "gg<leader>hu");
    assert_eq!(app.ed.message, "no staged hunk here");

    // `<leader>ha` on `two!`: that line in the index, the rest not.
    d.press(&mut app, "]h<leader>ha");
    staged_after(&mut d, &mut app, id, "1 hunk staged", 1);
    assert_eq!(index_text(&dir, "a.txt"), "one\ntwo!\nthree\ngone\nfour\n");
    let signs = app.ed.signs_in(id, 0..10);
    assert_eq!(signs.get(&1), None, "two! is the index's now");
    assert_eq!(signs.get(&3), Some(&Sign::Deleted));
    assert_eq!(signs.get(&4), Some(&Sign::Added));
    let staged = app.staged_signs_of(id, 0, &[]);
    assert_eq!(staged.get(&1), Some(&Sign::Modified), "staged, drawn faint");
    assert_eq!(staged.len(), 1);

    // `<leader>hu` on it: back out, the index HEAD's again.
    d.press(&mut app, "<leader>hu");
    staged_after(&mut d, &mut app, id, "1 hunk unstaged", 0);
    assert_eq!(index_text(&dir, "a.txt"), head);
    assert_eq!(app.ed.signs_in(id, 0..10).get(&1), Some(&Sign::Modified));

    // In visual mode, the hunks the selection touches: `gone`'s
    // deletion (on `four`) and `five`.
    d.press(&mut app, "4GVj<leader>ha");
    staged_after(&mut d, &mut app, id, "2 hunks staged", 2);
    assert_eq!(index_text(&dir, "a.txt"), "one\ntwo\nthree\nfour\nfive\n");
    assert_eq!(
        app.ed.mode(app.focused_view().unwrap()),
        kawoosh_editor::Mode::Normal
    );
    let staged = app.ed.staged_signs_in(id, 0..10);
    assert_eq!(staged.get(&3), Some(&Sign::Deleted));
    assert_eq!(staged.get(&4), Some(&Sign::Added));
    ex(&mut d, &mut app, "hunk");
    assert_eq!(app.ed.message, "against index: 1 hunk (+0 ~1 −0), 2 staged");
    let status = git(&dir, &["status", "--porcelain"]);
    assert_eq!(status.trim_end(), "MM a.txt", "staged, and changed since");
    // The backend's `status` says so too, which `vcs status` shows.
    app.run_lua_source(
        "status",
        r#"kawoosh.vcs.root(kawoosh.fs.cwd(), function(r)
          r.backend.status(r.root, function(files)
            for _, f in ipairs(files) do
              if kawoosh.fs.basename(f.path) == "a.txt" then
                kawoosh.echo("a.txt " .. tostring(f.staged) .. " " .. tostring(f.unstaged))
              end
            end
          end)
        end)"#,
    );
    until(&mut d, &mut app, "the status", |app| {
        app.ed.message.starts_with("a.txt ")
    });
    assert_eq!(app.ed.message, "a.txt true true");

    // `<leader>hA`: every hunk; `<leader>hU`: every staged one back.
    d.press(&mut app, "<leader>hA");
    staged_after(&mut d, &mut app, id, "1 hunk staged", 3);
    assert_eq!(index_text(&dir, "a.txt"), "one\ntwo!\nthree\nfour\nfive\n");
    until(&mut d, &mut app, "no hunks", |app| {
        app.ed.hunks(id).is_empty()
    });
    d.press(&mut app, "<leader>hU");
    staged_after(&mut d, &mut app, id, "3 hunks unstaged", 0);
    assert_eq!(index_text(&dir, "a.txt"), head);

    // From the review: the caret on `two!` there stages the file's
    // hunk, and the review's wash of it follows.
    d.press(&mut app, "<leader>hd");
    until(&mut d, &mut app, "the review", |app| {
        buffer_named(app, "*vcs diff*").is_some_and(|r| focused_buffer(app) == Some(r))
    });
    let review = buffer_named(&app, "*vcs diff*").unwrap();
    d.press(&mut app, "gg]h");
    let v = app.focused_view().unwrap();
    let b = app.ed.buffer_of(v);
    assert_eq!(b.line_of(app.ed.views[v].sels.primary().head), 3, "on two!");
    d.press(&mut app, "<leader>ha");
    staged_after(&mut d, &mut app, id, "1 hunk staged", 1);
    assert_eq!(index_text(&dir, "a.txt"), "one\ntwo!\nthree\ngone\nfour\n");
    let lines = app
        .ed
        .multi_lines(review, 0..app.ed.buffers[review].line_count());
    assert_eq!(app.signs_of(review, 0, &lines).get(&3), None);
    assert_eq!(
        app.staged_signs_of(review, 0, &lines).get(&3),
        Some(&Sign::Modified),
        "two! staged in the review"
    );
    d.press(&mut app, "q");
    d.frame(&mut app);
    std::fs::remove_dir_all(&dir).ok();
}

/// A CRLF checkout (`core.autocrlf`) stages LF lines, as `git add`
/// would, and a file without a last newline stages as it is — the
/// patch says `\ No newline at end of file` either way round.
#[test]
fn a_crlf_checkout_and_a_file_without_a_last_newline_stage_as_git_add_would() {
    if !has_git() {
        eprintln!("git is not installed: skipped");
        return;
    }
    let dir = staging_repo("stage-crlf", true);
    std::fs::write(dir.join("a.txt"), "one\r\ntwo!\r\nthree\r\nfour\r\nfive").unwrap();
    let file = dir.join("a.txt");
    let (mut d, mut app) = launch(&file, &dir);
    let id = focused_buffer(&app).unwrap();
    until(&mut d, &mut app, "the base from the index", |app| {
        app.ed.base(id).is_some_and(|b| b.version.is_some())
    });
    assert!(app.ed.base(id).unwrap().crlf, "the index's LF read as CRLF");
    assert_eq!(app.ed.hunks(id).len(), 3, "two!, gone, the unended five");
    d.press(&mut app, "gg]h<leader>ha");
    staged_after(&mut d, &mut app, id, "1 hunk staged", 1);
    assert_eq!(
        index_text(&dir, "a.txt"),
        "one\ntwo!\nthree\ngone\nfour\n",
        "LF in the index, the other lines untouched"
    );
    // The last line, `five` with no newline after it, and `four`
    // before it.
    d.press(&mut app, "G<leader>ha");
    until(&mut d, &mut app, "five staged", |app| {
        app.ed.message == "1 hunk staged" && app.ed.base(id).is_some_and(|b| b.staged.len() == 2)
    });
    assert_eq!(
        index_text(&dir, "a.txt"),
        "one\ntwo!\nthree\ngone\nfour\nfive"
    );
    let diff = git(&dir, &["diff"]);
    assert!(
        diff.contains("-gone") && !diff.contains("+five") && !diff.contains("+two!"),
        "only gone left unstaged: {diff}"
    );
    // Taken back: `four` ends with a newline in the index again.
    d.press(&mut app, "G<leader>hu");
    until(&mut d, &mut app, "five unstaged", |app| {
        app.ed.message == "1 hunk unstaged" && app.ed.base(id).is_some_and(|b| b.staged.len() == 1)
    });
    assert_eq!(index_text(&dir, "a.txt"), "one\ntwo!\nthree\ngone\nfour\n");

    // A file committed without a last newline, given one and a line.
    std::fs::write(dir.join("c.txt"), "x\ny").unwrap();
    git(&dir, &["add", "c.txt"]);
    git(&dir, &["commit", "-q", "-m", "c"]);
    let c = dir.join("c.txt");
    std::fs::write(&c, "x\r\ny\r\nz\r\n").unwrap();
    ex(&mut d, &mut app, &format!("e {}", c.display()));
    let cid = until_buffer(&mut d, &mut app, &c);
    until(&mut d, &mut app, "c's base", |app| {
        app.ed
            .base(cid)
            .is_some_and(|b| b.version.is_some() && !b.hunks.is_empty())
    });
    d.press(&mut app, "<leader>hA");
    staged_after(&mut d, &mut app, cid, "1 hunk staged", 1);
    assert_eq!(index_text(&dir, "c.txt"), "x\ny\nz\n");
    until(&mut d, &mut app, "no hunks", |app| {
        app.ed.hunks(cid).is_empty()
    });
    assert_eq!(
        git(&dir, &["diff", "--", "c.txt"]),
        "",
        "the index is the file"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The buffer of file `path`, once it is open and loaded.
fn until_buffer(d: &mut Drive, app: &mut Kawoosh, path: &Path) -> kawoosh_doc::BufferId {
    until(d, app, "the buffer", |app| {
        app.ed
            .buffers
            .values()
            .any(|b| b.path.as_deref() == Some(path) && b.loading.is_none())
    });
    app.ed
        .buffers
        .iter()
        .find(|(_, b)| b.path.as_deref() == Some(path))
        .map(|(id, _)| id)
        .unwrap()
}
