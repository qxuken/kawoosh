//! The second backend (docs/design/vcs.md Decision 4): a fossil
//! checkout gets its base from the checked-in text and its blame from
//! `fossil blame`, through the same doors git uses — and what fossil
//! has no function for is said, not attempted. Skipped where fossil is
//! not installed.

mod drive;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh_editor::Sign;
use kui_native::KeyMods;

fn fossil(dir: &Path, args: &[&str]) -> Option<String> {
    let out = kawoosh_systems::spawn::output(
        std::process::Command::new("fossil")
            .args(args)
            .current_dir(dir)
            .env("FOSSIL_USER", "ann"),
    )
    .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-vcs-fossil-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(dir.join("co")).unwrap();
    kawoosh_systems::fs::canonicalize(&dir).unwrap()
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

#[test]
fn a_fossil_checkout_is_signed_blamed_and_says_what_it_lacks() {
    if kawoosh_systems::spawn::output(std::process::Command::new("fossil").arg("version")).is_err()
    {
        eprintln!("fossil is not installed: skipped");
        return;
    }
    let dir = tmp("co");
    let co = dir.join("co");
    let repo = dir.join("r.fossil");
    if fossil(&co, &["init", repo.to_str().unwrap()]).is_none()
        || fossil(&co, &["open", repo.to_str().unwrap()]).is_none()
    {
        eprintln!("fossil could not make a repository here: skipped");
        return;
    }
    std::fs::write(co.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    fossil(&co, &["add", "a.txt"]).expect("add");
    fossil(&co, &["commit", "-m", "first", "--user", "ann"]).expect("commit");
    std::fs::write(co.join("a.txt"), "one\ntwo!\nthree\nfour\n").unwrap();

    let file = co.join("a.txt");
    let (mut d, mut app) = launch(&file, &co);
    let id = app.ed.views[app.focused_view().unwrap()].buffer;
    until(&mut d, &mut app, "the base from the checkout", |app| {
        app.ed.base(id).is_some_and(|b| b.version.is_some())
    });
    let signs = app.ed.signs_in(id, 0..10);
    assert_eq!(signs.get(&1), Some(&Sign::Modified));
    assert_eq!(signs.get(&3), Some(&Sign::Added));

    ex(&mut d, &mut app, "vcs");
    until(&mut d, &mut app, "the vcs line", |app| {
        app.ed.message.starts_with("fossil at ")
    });
    assert!(
        app.ed.message.contains(", on trunk — "),
        "{}",
        app.ed.message
    );
    assert!(!app.ed.message.contains("merge_base"), "{}", app.ed.message);

    // What fossil has no function for is said.
    d.press(&mut app, "<leader>hm");
    until(&mut d, &mut app, "the refusal", |app| {
        app.ed.message == "fossil here has no merge_base"
    });

    // The review of the working tree, and the blame column.
    d.press(&mut app, "<leader>hd");
    until(&mut d, &mut app, "the review", |app| {
        app.ed.buffers.values().any(|b| b.name == "*vcs diff*")
    });
    let review = app
        .ed
        .buffers
        .iter()
        .find(|(_, b)| b.name == "*vcs diff*")
        .unwrap()
        .0;
    until(&mut d, &mut app, "the review shown", |app| {
        app.focused_view()
            .is_some_and(|v| app.ed.views[v].buffer == review)
    });
    assert_eq!(
        app.ed.buffers[review].text(),
        "a.txt  +2 −1\none\ntwo\ntwo!\nthree\nfour\n"
    );
    d.press(&mut app, "q");
    d.frame(&mut app);
    d.press(&mut app, "<leader>hb");
    until(&mut d, &mut app, "the blame", |app| {
        app.ed.blame_width(id) > 0
    });
    let labels = app.ed.blame_labels(id, 0..10);
    // fossil's blame gives a date, not a time: `ann · 10h` or so.
    let (label, first) = labels.get(&0).cloned().expect("a label on the first line");
    assert!(label.starts_with("ann · "), "{label}");
    assert!(first);
    // fossil blames the checked-in text, not the buffer's: the three
    // committed lines are one run of the first check-in.
    assert_eq!(
        labels.get(&1).map(|(_, first)| *first),
        Some(false),
        "one run"
    );
    std::fs::remove_dir_all(&dir).ok();
}
