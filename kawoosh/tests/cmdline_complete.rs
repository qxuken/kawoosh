//! A shell line's arguments completed by a program outside
//! (docs/design/compile.md Decision 23): `cmdline.complete_with`, a
//! fake carapace on a PATH of the test's own here — answering, hanging,
//! printing garbage or too much — asked off the frame, its answers kept
//! by question for the prompt's life and shown only for the line that
//! asked them.
//!
//! Every test sets PATH, HOME and TMPDIR, which are the process's: they
//! take one lock, so under `cargo test` (a thread a test) one runs at a
//! time; under nextest each is a process of its own anyway.
#![cfg(unix)]

mod drive;

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::settings::{PROJECT_DIR, SETTINGS_FILE};
use kui_native::KeyMods;

static ENV: Mutex<()> = Mutex::new(());

/// The fake: what it was asked goes to `calls` beside it, a line each;
/// it answers by the program asked about.
const FAKE: &str = r#"#!/bin/sh
here=$(/usr/bin/dirname "$0")
echo "$*" >> "$here/calls"
case "$1" in
  git)
    case "$4$5" in
      commit-) printf '{"values":[{"value":"--amend","description":"amend previous commit"},{"value":"--author"},{"value":"-m"}]}' ;;
      *) printf '{"version":"v1","messages":[],"values":[{"value":"commit","display":"commit","description":"Record changes to the repository"},{"value":"checkout","description":"Switch branches"},{"value":"clean"}]}' ;;
    esac ;;
  lag) /bin/sleep 0.3; printf '{"values":[{"value":"v%s"}]}' "$#" ;;
  slow) echo $$ > "$here/slow.pid"; exec /bin/sleep 30 ;;
  garbage) echo 'Error: not a completion, and {not json' ;;
  huge) exec /usr/bin/yes '{"values":[{"value":"x"}]}' ;;
  *) ;;
esac
"#;

/// A sandbox of the test's own: HOME, TMPDIR and PATH in it (PATH its
/// `bin`, where the fake is when `fake`, then the system's own), and a
/// project with `settings` as its `.kawoosh/settings.lua`.
struct Sandbox {
    root: PathBuf,
    project: PathBuf,
    bin: PathBuf,
    _env: MutexGuard<'static, ()>,
}

impl Sandbox {
    fn new(tag: &str, fake: bool, settings: &str) -> Self {
        let env = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!(
            "kawoosh-complete-with-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let (home, tmp, bin) = (root.join("home"), root.join("tmp"), root.join("bin"));
        let project = root.join("project");
        for d in [
            &home,
            &tmp,
            &bin,
            &project.join(".git"),
            &project.join("src"),
        ] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::create_dir_all(project.join(PROJECT_DIR)).unwrap();
        std::fs::write(project.join(PROJECT_DIR).join(SETTINGS_FILE), settings).unwrap();
        std::fs::write(project.join("src/a.rs"), "one\n").unwrap();
        if fake {
            let path = bin.join("carapace");
            std::fs::write(&path, FAKE).unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        // The process's environment, which the lock above keeps to this
        // test while it runs.
        unsafe {
            std::env::set_var("HOME", &home);
            std::env::set_var("XDG_CONFIG_HOME", home.join(".config"));
            std::env::set_var("XDG_CACHE_HOME", home.join(".cache"));
            std::env::set_var("TMPDIR", &tmp);
            std::env::set_var("PATH", format!("{}:/usr/bin:/bin", bin.display()));
        }
        Sandbox {
            root,
            project,
            bin,
            _env: env,
        }
    }

    /// What the fake was asked, a line a question.
    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.bin.join("calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// The app on the project's file, its completer allowed as a loaded
    /// config allows it.
    fn open(&self, d: &mut Drive) -> Kawoosh {
        let mut app = Kawoosh::from_file(&self.project.join("src/a.rs"));
        let ext = app.attach_lua().unwrap();
        d.extension("lua", ext).unwrap();
        app.set_cwd(&self.project);
        app.config.completer = true;
        d.frame(&mut app);
        app
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).ok();
    }
}

fn candidates(app: &Kawoosh) -> Vec<String> {
    app.cmd_completion
        .as_ref()
        .map(|c| c.candidates.clone())
        .unwrap_or_default()
}

fn detail(app: &Kawoosh) -> Option<String> {
    app.cmd_completion
        .as_ref()
        .and_then(|c| c.detail())
        .map(str::to_string)
}

/// The completer's answers in, a frame drawn with them.
fn answered(d: &mut Drive, app: &mut Kawoosh) {
    app.wait_for_jobs();
    d.frame(app);
}

fn leave(d: &mut Drive, app: &mut Kawoosh) {
    d.key(app, "escape", KeyMods::default());
    d.key(app, "escape", KeyMods::default());
    assert!(app.ed.prompt_view().is_none());
}

fn texts(d: &Drive) -> Vec<String> {
    d.core
        .nodes()
        .iter()
        .filter_map(|n| n.text.clone())
        .collect()
}

#[test]
fn a_line_s_arguments_complete_from_its_completer() {
    let s = Sandbox::new("answers", true, "return {}");
    let mut d = Drive::new(900.0, 500.0);
    let mut app = s.open(&mut d);

    // Asked as the word starts, off the frame: nothing until it answers.
    d.keys(&mut app, ":compile git ");
    assert!(!candidates(&app).contains(&"commit".to_string()));
    answered(&mut d, &mut app);
    let c = candidates(&app);
    assert_eq!(c[..3], ["commit", "checkout", "clean"], "{c:?}");
    assert_eq!(
        detail(&app).as_deref(),
        Some("Record changes to the repository")
    );
    assert!(
        texts(&d).contains(&"Record changes to the repository".to_string()),
        "the description is drawn beside the candidates"
    );
    // Typed through, narrowed here: asked once.
    d.keys(&mut app, "co");
    assert_eq!(candidates(&app), ["commit"]);
    assert_eq!(app.cmdline_ghost().as_deref(), Some("mmit"));
    assert_eq!(
        s.calls(),
        ["git export git "],
        "PROG export PROG WORDS… CURRENT"
    );
    d.key(&mut app, "tab", KeyMods::default());
    assert_eq!(app.ed.prompt_text().as_deref(), Some("compile git commit"));

    // A flag: asked from `-`, the flags narrowed here.
    d.keys(&mut app, " --a");
    answered(&mut d, &mut app);
    assert_eq!(candidates(&app), ["--amend", "--author"]);
    assert_eq!(detail(&app).as_deref(), Some("amend previous commit"));
    assert_eq!(
        s.calls().last().map(String::as_str),
        Some("git export git commit -")
    );
    leave(&mut d, &mut app);

    // A named command's line is the name's: no completer.
    let before = s.calls().len();
    std::fs::write(
        s.project.join(PROJECT_DIR).join(SETTINGS_FILE),
        "return { compile = { commands = { git = \"echo named\" } } }",
    )
    .unwrap();
    app.reload_project_settings();
    d.keys(&mut app, ":compile git ");
    answered(&mut d, &mut app);
    assert!(!candidates(&app).contains(&"commit".to_string()));
    assert_eq!(s.calls().len(), before);
    leave(&mut d, &mut app);

    // `:!`, `:shell` and `:terminal` complete their program's arguments
    // the same way, from the working directory.
    d.keys(&mut app, ":!git ch");
    answered(&mut d, &mut app);
    assert_eq!(candidates(&app), ["checkout"]);
    d.key(&mut app, "tab", KeyMods::default());
    assert_eq!(app.ed.prompt_text().as_deref(), Some("!git checkout"));
    leave(&mut d, &mut app);
    d.keys(&mut app, ":terminal git cl");
    answered(&mut d, &mut app);
    assert_eq!(candidates(&app), ["clean"]);
    leave(&mut d, &mut app);
    d.keys(&mut app, ":shell git com");
    answered(&mut d, &mut app);
    assert_eq!(candidates(&app), ["commit"]);
    leave(&mut d, &mut app);
}

#[test]
fn an_answer_is_shown_only_for_the_line_that_asked_it() {
    let s = Sandbox::new("stale", true, "return {}");
    let mut d = Drive::new(900.0, 500.0);
    let mut app = s.open(&mut d);

    // Typed on before the answer: the line's own question is asked, and
    // the first answer, when it lands, is not its.
    d.keys(&mut app, ":compile lag ");
    d.keys(&mut app, "one ");
    answered(&mut d, &mut app);
    // `lag export lag one ""` is five words: the fake answers `v5`.
    assert!(
        candidates(&app).contains(&"v5".to_string()),
        "{:?}",
        candidates(&app)
    );
    assert!(!candidates(&app).contains(&"v4".to_string()));
    // Kept by its question: back there, it is shown, not asked again.
    let asked = s.calls().len();
    d.press(&mut app, "<BS><BS><BS><BS>");
    assert_eq!(app.ed.prompt_text().as_deref(), Some("compile lag "));
    assert!(candidates(&app).contains(&"v4".to_string()));
    assert_eq!(s.calls().len(), asked);
    leave(&mut d, &mut app);

    // One that lands after its prompt closed is dropped — not taken for
    // the next prompt's same question, which is asked again.
    d.keys(&mut app, ":compile lag ");
    leave(&mut d, &mut app);
    d.keys(&mut app, ":compile lag ");
    answered(&mut d, &mut app);
    assert!(candidates(&app).contains(&"v4".to_string()));
    assert_eq!(s.calls().len(), asked + 2);
    leave(&mut d, &mut app);
}

#[test]
fn a_completer_that_hangs_raves_or_floods_gives_nothing() {
    let s = Sandbox::new("bad", true, "return {}");
    let mut d = Drive::new(900.0, 500.0);
    let mut app = s.open(&mut d);
    let only_paths = |app: &Kawoosh| {
        candidates(app)
            .iter()
            .all(|c| c.ends_with('/') || c.contains('.'))
    };

    // Hangs: given up on after its patience, and stopped.
    let at = Instant::now();
    d.keys(&mut app, ":compile slow ");
    answered(&mut d, &mut app);
    let took = at.elapsed();
    assert!(took >= Duration::from_millis(1500), "{took:?}");
    assert!(took < Duration::from_secs(10), "{took:?}");
    assert!(only_paths(&app), "{:?}", candidates(&app));
    let pid: i32 = std::fs::read_to_string(s.bin.join("slow.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_ne!(
        unsafe { libc::kill(pid, 0) },
        0,
        "the completer was stopped"
    );
    leave(&mut d, &mut app);

    // Garbage: nothing, quietly.
    d.keys(&mut app, ":compile garbage ");
    answered(&mut d, &mut app);
    assert!(only_paths(&app), "{:?}", candidates(&app));
    leave(&mut d, &mut app);

    // Without end: cut at the cap, stopped, nothing — long before its
    // patience runs out.
    let at = Instant::now();
    d.keys(&mut app, ":compile huge ");
    answered(&mut d, &mut app);
    assert!(
        at.elapsed() < Duration::from_millis(1500),
        "{:?}",
        at.elapsed()
    );
    assert!(only_paths(&app), "{:?}", candidates(&app));
    leave(&mut d, &mut app);

    // A program it does not know: nothing.
    d.keys(&mut app, ":compile unknown ");
    answered(&mut d, &mut app);
    assert!(only_paths(&app), "{:?}", candidates(&app));
    leave(&mut d, &mut app);
    assert!(
        !app.ed.message.contains("complete"),
        "no word of it: {}",
        app.ed.message
    );
}

#[test]
fn no_completer_on_the_path_or_turned_off_asks_nothing() {
    // None on the PATH: nothing, no error.
    {
        let s = Sandbox::new("none", false, "return {}");
        let mut d = Drive::new(900.0, 500.0);
        let mut app = s.open(&mut d);
        let said = app.ed.message.clone();
        d.keys(&mut app, ":compile git co");
        answered(&mut d, &mut app);
        assert_eq!(candidates(&app), Vec::<String>::new());
        assert_eq!(app.ed.message, said, "nothing said");
        leave(&mut d, &mut app);
    }
    // Turned off: never asked.
    {
        let s = Sandbox::new("off", true, "return {}");
        let mut d = Drive::new(900.0, 500.0);
        let mut app = s.open(&mut d);
        d.keys(&mut app, ":set cmdline.complete_with=false");
        d.key(&mut app, "enter", KeyMods::default());
        d.keys(&mut app, ":compile git co");
        answered(&mut d, &mut app);
        assert_eq!(candidates(&app), Vec::<String>::new());
        assert_eq!(s.calls(), Vec::<String>::new());
        leave(&mut d, &mut app);
        // Named another: that one is asked.
        d.keys(
            &mut app,
            &format!(
                ":set cmdline.complete_with={}",
                s.bin.join("carapace").display()
            ),
        );
        d.key(&mut app, "enter", KeyMods::default());
        d.keys(&mut app, ":compile git co");
        answered(&mut d, &mut app);
        assert_eq!(candidates(&app), ["commit"]);
        leave(&mut d, &mut app);
    }
    // A project may turn it off ...
    {
        let s = Sandbox::new(
            "project-off",
            true,
            "return { cmdline = { complete_with = false } }",
        );
        let mut d = Drive::new(900.0, 500.0);
        let mut app = s.open(&mut d);
        d.keys(&mut app, ":compile git co");
        answered(&mut d, &mut app);
        assert_eq!(s.calls(), Vec::<String>::new());
        leave(&mut d, &mut app);
    }
    // ... but not name a program to run at every key: its settings are
    // data. The default stands.
    {
        let s = Sandbox::new("project-named", false, "");
        let evil = s.root.join("evil");
        std::fs::write(&evil, "#!/bin/sh\ntouch \"$0.ran\"\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&evil, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(
            s.project.join(PROJECT_DIR).join(SETTINGS_FILE),
            format!(
                "return {{ cmdline = {{ complete_with = {:?} }} }}",
                evil.display().to_string()
            ),
        )
        .unwrap();
        let mut d = Drive::new(900.0, 500.0);
        let mut app = s.open(&mut d);
        d.keys(&mut app, ":compile git co");
        answered(&mut d, &mut app);
        assert!(
            !s.root.join("evil.ran").exists(),
            "a project's program never runs"
        );
        leave(&mut d, &mut app);
    }
    // And an app that loaded no config runs none.
    {
        let s = Sandbox::new("no-config", true, "return {}");
        let mut d = Drive::new(900.0, 500.0);
        let mut app = s.open(&mut d);
        app.config.completer = false;
        d.keys(&mut app, ":compile git co");
        answered(&mut d, &mut app);
        assert_eq!(s.calls(), Vec::<String>::new());
        leave(&mut d, &mut app);
    }
}
