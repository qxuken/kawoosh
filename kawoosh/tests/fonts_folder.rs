//! The user's fonts folder (fonts.md Decision 7): loaded at start and
//! watched — a font file dropped in is a family the completion, the
//! pane and `font.family` know within a second, with a note; a file
//! taken out goes the same way.

mod drive;

use std::time::{Duration, Instant};

use drive::Drive;
use kawoosh::Kawoosh;

/// The `:font ` completion's families.
fn families(d: &mut Drive, app: &mut Kawoosh) -> Vec<String> {
    d.keys(app, ":font ");
    let c = app.cmd_completion.clone().unwrap().candidates;
    d.key(app, "escape", Default::default());
    d.key(app, "escape", Default::default());
    c
}

/// Frames until `ok` holds, or three seconds: the watch looks twice a
/// second.
fn until(
    d: &mut Drive,
    app: &mut Kawoosh,
    mut ok: impl FnMut(&mut Drive, &mut Kawoosh) -> bool,
) -> bool {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(3) {
        d.frame(app);
        if ok(d, app) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

#[test]
fn a_font_dropped_in_the_users_folder_is_a_family() {
    let dir = std::env::temp_dir().join(format!("kawoosh-fonts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("fonts")).unwrap();
    // SAFETY: this test binary's only test, set before any thread reads it.
    unsafe { std::env::set_var("KAWOOSH_FONTS", dir.join("fonts")) };
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../assets/fonts/IntelOneMono/IntelOneMono-Regular.otf");
    let mut app = Kawoosh::from_file(&dir.join("a.txt"));
    let ext = app.attach_lua().unwrap();
    let mut d = Drive::new(900.0, 500.0);
    d.extension("lua", ext);
    app.set_cwd(&dir);
    d.frame(&mut app);
    let name = "Intel One Mono";
    if families(&mut d, &mut app).iter().any(|f| f == name) {
        // Installed on this machine already: nothing to tell apart.
        return;
    }

    // In a folder of its own, as a download unpacks.
    std::fs::create_dir_all(dir.join("fonts/intel")).unwrap();
    std::fs::copy(&src, dir.join("fonts/intel/IntelOneMono-Regular.otf")).unwrap();
    assert!(
        until(&mut d, &mut app, |d, app| families(d, app)
            .iter()
            .any(|f| f == name)),
        "the family appears"
    );
    assert!(
        app.notes
            .shown
            .iter()
            .any(|s| s.text.contains("fonts: added Intel One Mono")),
        "{:?}",
        app.notes.shown.iter().map(|s| &s.text).collect::<Vec<_>>()
    );
    d.keys(&mut app, &format!(":font {name}"));
    d.key(&mut app, "enter", Default::default());
    d.frame(&mut app);
    assert_eq!(app.ed.settings.str("font.family"), Some(name));

    // Taken out: gone again.
    std::fs::remove_file(dir.join("fonts/intel/IntelOneMono-Regular.otf")).unwrap();
    assert!(
        until(&mut d, &mut app, |d, app| !families(d, app)
            .iter()
            .any(|f| f == name)),
        "the family goes"
    );
    std::fs::remove_dir_all(&dir).ok();
}
