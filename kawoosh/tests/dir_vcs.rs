//! `dir` and version control (roadmap step 22's follow-up): a listing
//! in a git repository has its entries painted by what git says of
//! them — ignored, untracked, added, modified — through the plugins'
//! paint door (`kawoosh.buf.paint`).

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::KeyMods;

fn git(dir: &std::path::Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    assert!(ok, "git {args:?}");
}

#[test]
fn a_listing_in_a_repository_paints_what_git_says() {
    if std::process::Command::new("git")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("git is not installed: skipped");
        return;
    }
    let dir = std::env::temp_dir().join(format!("kawoosh-dir-vcs-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(dir.join("build")).unwrap();
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
    git(&dir, &["init", "-q"]);
    std::fs::write(dir.join(".gitignore"), "build/\n").unwrap();
    std::fs::write(dir.join("kept.txt"), "k\n").unwrap();
    std::fs::write(dir.join("changed.txt"), "c\n").unwrap();
    std::fs::write(dir.join("build/out.o"), "o\n").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "first"]);
    std::fs::write(dir.join("changed.txt"), "c2\n").unwrap();
    std::fs::write(dir.join("new.txt"), "n\n").unwrap();

    let mut app = Kawoosh::new("*scratch*", "");
    let mut d = Drive::new(900.0, 500.0);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    d.frame(&mut app);
    d.keys(&mut app, &format!(":dir {}", dir.display()));
    d.key(&mut app, "enter", KeyMods::default());
    // The states, by the painted line's text.
    let states = |app: &Kawoosh| -> Vec<(String, String)> {
        let Some(v) = app.focused_view() else {
            return Vec::new();
        };
        let id = app.ed.views[v].buffer;
        let buf = app.ed.buffer_of(v);
        let text = buf.text();
        let mut out: Vec<(String, String)> = app
            .scripting
            .paints
            .get(&id)
            .and_then(|s| s.get("vcs"))
            .map(|p| {
                p.spans
                    .iter()
                    .map(|(r, c)| (text[r.clone()].to_string(), c.clone()))
                    .collect()
            })
            .unwrap_or_default();
        out.sort();
        out
    };
    let mut got = Vec::new();
    for _ in 0..300 {
        d.frame(&mut app);
        got = states(&app);
        if got.len() >= 3 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        got,
        [
            ("build/".to_string(), "ignored".to_string()),
            ("changed.txt".to_string(), "modified".to_string()),
            ("new.txt".to_string(), "untracked".to_string()),
        ]
    );
    std::fs::remove_dir_all(&dir).ok();
}
