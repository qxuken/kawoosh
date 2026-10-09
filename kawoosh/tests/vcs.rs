//! Hunks (docs/design/vcs.md Decisions 1–3): a buffer given a base is
//! diffed against it once still, its lines signed in the gutter, `]h`
//! walks the hunks, `<leader>hp` shows one as a diff, `<leader>hr`
//! takes one back. The base here is given by hand; `vcs_git.rs` has
//! git give it.

mod drive;

use std::sync::Arc;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh_editor::Sign;
use kui_native::KeyMods;

fn tmp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-vcs-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    kawoosh_systems::fs::canonicalize(&dir).unwrap()
}

fn launch(path: &std::path::Path) -> (Drive, Kawoosh) {
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::from_file(path);
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
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

fn caret_line(app: &Kawoosh) -> usize {
    let v = app.focused_view().unwrap();
    let b = app.ed.buffer_of(v);
    b.line_of(app.ed.views[v].sels.primary().head)
}

fn text(app: &Kawoosh) -> String {
    app.ed.buffer_of(app.focused_view().unwrap()).text()
}

#[test]
fn a_base_signs_the_gutter_and_the_hunk_keys_walk_show_and_reset() {
    let dir = tmp("keys");
    let file = dir.join("a.txt");
    std::fs::write(&file, "one\ntwo!\nthree\nfour\nfive\n").unwrap();
    let (mut d, mut app) = launch(&file);
    let id = app.ed.views[app.focused_view().unwrap()].buffer;
    app.ed.set_base(
        id,
        Arc::from("one\ntwo\nthree\ngone\nfour\n"),
        "index".into(),
    );
    d.frame(&mut app);
    d.frame(&mut app);
    let signs = app.ed.signs_in(id, 0..10);
    assert_eq!(signs.get(&1), Some(&Sign::Modified));
    assert_eq!(signs.get(&3), Some(&Sign::Deleted));
    assert_eq!(signs.get(&4), Some(&Sign::Added));
    assert_eq!(signs.len(), 3);
    // The gutter's numbers are as they were: the bars take no column.
    assert_eq!(
        d.gutter_texts(),
        ["1", "2", "3", "4", "5", "~"],
        "numbers, no sign column"
    );

    ex(&mut d, &mut app, "hunk");
    assert_eq!(app.ed.message, "against index: 3 hunks (+1 ~1 −1)");

    // `]h` `[h`, with a count.
    d.press(&mut app, "gg");
    d.press(&mut app, "]h");
    assert_eq!(caret_line(&app), 1);
    d.press(&mut app, "]h");
    assert_eq!(caret_line(&app), 3);
    d.press(&mut app, "]h");
    assert_eq!(caret_line(&app), 4);
    d.press(&mut app, "]h");
    assert_eq!(app.ed.message, "no next hunk");
    d.press(&mut app, "3[h");
    assert_eq!(caret_line(&app), 1);

    // `<leader>hp`: the hunk as a diff, the keys staying.
    d.press(&mut app, "<leader>hp");
    d.frame(&mut app);
    let hunk = app
        .ed
        .buffers
        .iter()
        .find(|(_, b)| b.name == "*hunk*")
        .map(|(_, b)| (b.text(), b.language.to_string()))
        .expect("a *hunk* buffer");
    assert_eq!(hunk.1, "diff");
    assert_eq!(
        hunk.0,
        "--- a.txt (index)\n+++ a.txt\n@@ -1,5 +1,5 @@\n one\n-two\n+two!\n three\n four\n five\n"
    );
    assert_eq!(caret_line(&app), 1, "the keys stayed in the file");
    assert_eq!(text(&app), "one\ntwo!\nthree\nfour\nfive\n");

    // `<leader>hr`: the hunk under the caret made the base's again.
    d.press(&mut app, "<leader>hr");
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(text(&app), "one\ntwo\nthree\nfour\nfive\n");
    assert_eq!(app.ed.message, "1 hunk reset");
    let signs = app.ed.signs_in(id, 0..10);
    assert_eq!(signs.get(&1), None, "diffed again");
    assert_eq!(signs.len(), 2);
    // Visual mode: the selection's hunks.
    d.press(&mut app, "ggVG");
    d.press(&mut app, "<leader>hr");
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(text(&app), "one\ntwo\nthree\ngone\nfour\n");
    assert_eq!(app.ed.message, "2 hunks reset");
    assert!(app.ed.signs_in(id, 0..10).is_empty());
    // One undo node a reset.
    d.press(&mut app, "u");
    assert_eq!(text(&app), "one\ntwo\nthree\nfour\nfive\n");
    d.press(&mut app, "u");
    assert_eq!(text(&app), "one\ntwo!\nthree\nfour\nfive\n");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn signs_are_off_by_a_setting_and_the_hunks_stay() {
    let dir = tmp("setting");
    let file = dir.join("a.txt");
    std::fs::write(&file, "one\ntwo!\n").unwrap();
    let (mut d, mut app) = launch(&file);
    let id = app.ed.views[app.focused_view().unwrap()].buffer;
    app.ed.set_base(id, Arc::from("one\ntwo\n"), "index".into());
    d.frame(&mut app);
    ex(&mut d, &mut app, "set vcs.signs false");
    d.frame(&mut app);
    assert!(
        !app.pane_signs_on(app.layout.focused()),
        "the gutter draws none"
    );
    assert_eq!(
        app.ed.signs_in(id, 0..10).len(),
        1,
        "the engine's are there"
    );
    d.press(&mut app, "gg]h");
    assert_eq!(caret_line(&app), 1, "`]h` still walks them");
    std::fs::remove_dir_all(&dir).ok();
}

/// `ggVG` and `hunk unstage` over a long file newly added — every line
/// staged, one hunk, and every tenth changed again since — asks the
/// staged hunks once for the selection, not once a line: 30 000 lines
/// took 9.5 s of CPU when each line carried every staged line to the
/// buffer through the unstaged hunks. Bounded by how it grows, as
/// `an_edit_per_line_over_a_long_file_is_one_pass` is: four times the
/// lines cost four times as much in one pass, sixteen in the quadratic.
#[test]
fn unstaging_a_selection_over_a_long_file_is_one_pass() {
    let dir = tmp("unstage-long");
    let pass = |n: usize| {
        let file = dir.join(format!("long-{n}.txt"));
        let index: String = (0..n).map(|i| format!("line {i}\n")).collect();
        let text: String = (0..n)
            .map(|i| match i % 10 {
                0 => format!("line {i} again\n"),
                _ => format!("line {i}\n"),
            })
            .collect();
        std::fs::write(&file, &text).unwrap();
        let (mut d, mut app) = launch(&file);
        let id = app.ed.views[app.focused_view().unwrap()].buffer;
        app.ed
            .set_base(id, Arc::from(index.as_str()), "index".into());
        app.ed.set_base_head(id, Some(Arc::from("")));
        d.frame(&mut app);
        d.frame(&mut app);
        assert_eq!(app.ed.hunks(id).len(), n / 10);
        assert_eq!(app.ed.base(id).unwrap().staged.len(), 1);
        app.run_lua_source(
            "stage",
            r#"kawoosh.on_stage(function(_, patch, o)
              kawoosh.echo("unstage " .. o.count .. " " .. select(2, patch:gsub("\n", "")))
            end)"#,
        );
        d.press(&mut app, "gg");
        let start = thread_cpu();
        d.press(&mut app, "VG<leader>hu");
        let took = thread_cpu() - start;
        // One staged hunk; its patch takes every index line out.
        assert_eq!(app.ed.message, format!("unstage 1 {}", n + 1));
        took
    };
    let small = pass(7_500);
    let large = pass(30_000);
    // The thread's clock ticks in 15.6 ms on Windows: a pass that read
    // as none counts as one tick.
    let tick = std::time::Duration::from_millis(16);
    let growth = large.as_secs_f64() / small.max(tick).as_secs_f64();
    assert!(
        growth < 8.0,
        "4x the lines took {growth:.1}x as long ({small:?}, then {large:?})"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The CPU time this thread has had, which other processes do not
/// stretch.
#[cfg(unix)]
fn thread_cpu() -> std::time::Duration {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid timespec for the call to fill.
    let ok = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
    assert_eq!(ok, 0);
    std::time::Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

#[cfg(windows)]
fn thread_cpu() -> std::time::Duration {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentThread, GetThreadTimes};
    let zero = || FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut created, mut exited, mut kernel, mut user) = (zero(), zero(), zero(), zero());
    // SAFETY: the current thread's pseudo-handle, four FILETIMEs to fill.
    let ok = unsafe {
        GetThreadTimes(
            GetCurrentThread(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    };
    assert_ne!(ok, 0);
    let ticks = |f: FILETIME| (f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64;
    // FILETIME counts 100 ns.
    std::time::Duration::from_nanos((ticks(kernel) + ticks(user)) * 100)
}
