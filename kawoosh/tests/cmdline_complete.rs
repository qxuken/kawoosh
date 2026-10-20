//! A shell line's arguments completed by a program outside
//! (docs/design/compile.md Decision 23): `cmdline.complete_with`, a
//! fake carapace on a PATH of the test's own here — answering, hanging,
//! printing garbage or too much — asked off the frame, its answers kept
//! by question for the prompt's life and shown only for the line that
//! asked them; and a nushell parameter's completer, a fake `nu` there,
//! asked the same way (Decision 16).
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
here=${0%/*}
echo "$*" >> "$here/calls"
case "$1" in
  git)
    case "$4$5" in
      commit-) printf '{"values":[{"value":"--amend","description":"amend previous commit"},{"value":"--author"},{"value":"-m"}]}' ;;
      *) printf '{"version":"v1","messages":[],"values":[{"value":"commit","display":"commit","description":"Record changes to the repository"},{"value":"checkout","description":"Switch branches"},{"value":"clean"}]}' ;;
    esac ;;
  lag) /bin/sleep 0.3; printf '{"values":[{"value":"v%s"}]}' "$#" ;;
  slow) echo $$ > "$here/slow.pid"; exec /bin/sleep 30 ;;
  orphan) /bin/sleep 30 & echo $! > "$here/orphan.pid"; printf '{"values":[{"value":"o"}]}' ;;
  garbage) echo 'Error: not a completion, and {not json' ;;
  where) printf '{"values":[{"value":"%s"}]}' "$(basename "$PWD")" ;;
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

    // Asked as the word starts, on a job; the line completed again as
    // it answers (whether it beats the next frame is the machine's).
    d.keys(&mut app, ":compile git ");
    answered(&mut d, &mut app);
    let c = candidates(&app);
    assert_eq!(
        c[..3],
        ["commit", "checkout", "clean"],
        "{c:?} {:?}",
        s.calls()
    );
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
    assert_eq!(candidates(&app), ["--amend", "--author"], "{:?}", s.calls());
    assert_eq!(detail(&app).as_deref(), Some("amend previous commit"));
    // Asked from `-`; the space before asked too, its answer out at
    // the same time, so in either order.
    assert!(
        s.calls().iter().any(|c| c == "git export git commit -"),
        "{:?}",
        s.calls()
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
    assert!(gone(pid), "the completer was stopped");
    leave(&mut d, &mut app);

    // What it started outlives it, holding the pipe: stopped with it,
    // its whole group — none left behind.
    d.keys(&mut app, ":compile orphan ");
    answered(&mut d, &mut app);
    let pid: i32 = std::fs::read_to_string(s.bin.join("orphan.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(gone(pid), "the completer's child was stopped too");
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

/// A fake `nu` for a nushell parameter's completer (compile.md Decision
/// 16): the script it was given goes to `nu-calls` beside it, a line
/// each; it answers by the completer the script names.
const FAKE_NU: &str = r#"#!/bin/sh
here=${0%/*}
echo "$3" >> "$here/nu-calls"
case "$3" in
  *"; files |"*) echo 'the file printing at its top level'; printf '["alpha.txt","beta.txt"]\n' ;;
  *"; lag |"*) /bin/sleep 0.3; printf '["late"]\n' ;;
  *"; hang |"*) exec /bin/sleep 30 ;;
esac
"#;

/// The completers a fake `nu` answers, as code (so nu is asked) behind
/// a script's three positionals.
const BUILD_NU: &str = r#"
def files [] { ["alpha.txt", "beta.txt"] | each {|f| $f } }
def lag [] { ["late"] | each {|f| $f } }
def hang [] { [] | each {|f| $f } }
def kinds [] { ["debug", "release"] }
def "main build" [file: string@files, other?: string@lag, third?: string@hang, kind?: string@kinds] { }
"#;

/// `:trust allow PATH`, typed.
fn trust(d: &mut Drive, app: &mut Kawoosh, path: &std::path::Path) {
    d.keys(app, &format!(":trust allow {}", path.display()));
    d.key(app, "enter", KeyMods::default());
    d.frame(app);
}

/// One count bounds the completers running, carapace's and nu's, across
/// prompts: with four carapace questions out (each hanging past a prompt
/// that closed), a nushell parameter's completer waits — and is asked
/// once one of them ends, the line completed again without a key.
#[test]
fn carapace_and_nu_share_one_bound() {
    let s = Sandbox::new("bound", true, "return {}");
    let nu = s.bin.join("nu");
    std::fs::write(&nu, FAKE_NU).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&nu, std::fs::Permissions::from_mode(0o755)).unwrap();
    let build = s.project.join("build.nu");
    std::fs::write(&build, BUILD_NU).unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = s.open(&mut d);
    trust(&mut d, &mut app, &build);

    // Four questions, each the words before the caret's: four running.
    d.keys(&mut app, ":compile slow a b c ");
    leave(&mut d, &mut app);
    d.keys(&mut app, ":compile nu build.nu build al");
    // Long past what the fake takes to start, well short of the slow
    // ones' patience.
    std::thread::sleep(Duration::from_millis(500));
    d.frame(&mut app);
    assert!(!s.bin.join("nu-calls").exists(), "nu waits for a place");
    // The slow ones are given up on; the place freed asks nu.
    answered(&mut d, &mut app);
    answered(&mut d, &mut app);
    assert_eq!(app.cmdline_ghost().as_deref(), Some("pha.txt"));
    leave(&mut d, &mut app);
}

/// compile.md Decision 16 on Decision 23's footing: a nushell
/// parameter's completer is asked on a job, no key waiting for nu; the
/// answer kept by file, completer and the file's stamp, across prompts;
/// one that lands after its prompt closed dropped and asked again.
#[test]
fn a_nushell_completer_is_asked_off_the_frame() {
    let s = Sandbox::new("nu", false, "return {}");
    let nu = s.bin.join("nu");
    std::fs::write(&nu, FAKE_NU).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&nu, std::fs::Permissions::from_mode(0o755)).unwrap();
    let build = s.project.join("build.nu");
    std::fs::write(&build, BUILD_NU).unwrap();
    let asked = |name: &str| {
        std::fs::read_to_string(s.bin.join("nu-calls"))
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(&format!("; {name} |")))
            .count()
    };
    let mut d = Drive::new(900.0, 500.0);
    let mut app = s.open(&mut d);

    // Untrusted, the file's code runs nowhere: no completer is asked of
    // nu (`source` would run its top level), and a completer whose body
    // is a list is still its values, read from the text.
    d.keys(&mut app, ":compile nu build.nu build al");
    answered(&mut d, &mut app);
    assert_eq!(app.cmdline_ghost(), None);
    leave(&mut d, &mut app);
    d.keys(&mut app, ":compile nu build.nu build x y z d");
    answered(&mut d, &mut app);
    assert_eq!(app.cmdline_ghost().as_deref(), Some("ebug"));
    leave(&mut d, &mut app);
    assert!(!s.bin.join("nu-calls").exists(), "nu was never run");
    d.keys(&mut app, ":trust?");
    d.key(&mut app, "enter", KeyMods::default());
    assert!(
        app.ed.message.contains("build.nu (not trusted)"),
        "{}",
        app.ed.message
    );
    trust(&mut d, &mut app, &build);

    // Trusted: asked as the parameter starts; the line completed as nu
    // answers, read from its last line.
    d.keys(&mut app, ":compile nu build.nu build al");
    answered(&mut d, &mut app);
    assert_eq!(app.cmdline_ghost().as_deref(), Some("pha.txt"));
    assert_eq!(asked("files"), 1);
    leave(&mut d, &mut app);
    // Kept while the file is unchanged: the next prompt has it at once.
    d.keys(&mut app, ":compile nu build.nu build be");
    assert_eq!(app.cmdline_ghost().as_deref(), Some("ta.txt"));
    assert_eq!(asked("files"), 1);
    leave(&mut d, &mut app);

    // Out as its prompt closes: the answer is the file's, kept as it
    // lands for the next prompt — and while it runs, no prompt starts
    // nu again.
    d.keys(&mut app, ":compile nu build.nu build x la");
    leave(&mut d, &mut app);
    d.keys(&mut app, ":compile nu build.nu build x la");
    leave(&mut d, &mut app);
    app.wait_for_jobs();
    d.keys(&mut app, ":compile nu build.nu build x la");
    assert_eq!(app.cmdline_ghost().as_deref(), Some("te"));
    assert_eq!(asked("lag"), 1);
    leave(&mut d, &mut app);

    // One that hangs: the keys do not wait for it, and it is given up
    // on after its patience, once — no values kept for it, and it is
    // not asked again while the file holds.
    let at = Instant::now();
    d.keys(&mut app, ":compile nu build.nu build x y h");
    let typed = at.elapsed();
    assert!(typed < Duration::from_millis(1500), "{typed:?}");
    answered(&mut d, &mut app);
    assert!(at.elapsed() >= Duration::from_millis(1500));
    assert_eq!(app.cmdline_ghost(), None);
    leave(&mut d, &mut app);
    d.keys(&mut app, ":compile nu build.nu build x y h");
    answered(&mut d, &mut app);
    assert_eq!(asked("hang"), 1);
    leave(&mut d, &mut app);

    // The file changed: its trust was of the text before, so nothing
    // runs until it is trusted again — then asked again.
    std::fs::write(&build, format!("# changed\n{BUILD_NU}")).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&build)
        .unwrap()
        .set_modified(std::time::SystemTime::now() + Duration::from_secs(5))
        .unwrap();
    d.keys(&mut app, ":compile nu build.nu build al");
    answered(&mut d, &mut app);
    assert_eq!(app.cmdline_ghost(), None);
    assert_eq!(asked("files"), 1);
    leave(&mut d, &mut app);
    trust(&mut d, &mut app, &build);
    d.keys(&mut app, ":compile nu build.nu build al");
    answered(&mut d, &mut app);
    assert_eq!(app.cmdline_ghost().as_deref(), Some("pha.txt"));
    assert_eq!(asked("files"), 2);
    leave(&mut d, &mut app);
}

/// Whether process `pid` is gone within a few seconds: a signal to a
/// group is delivered, and an orphan reaped by the system, a moment
/// after the kill — longer on a loaded machine.
#[cfg(unix)]
fn gone(pid: i32) -> bool {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < until {
        if unsafe { libc::kill(pid, 0) } != 0 {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    false
}

/// A completer reads where it runs (a make completer the Makefile,
/// whose `$(shell …)` is code): in a project not trusted it runs in an
/// empty directory of kawoosh's own, and in the project once
/// `:trust allow DIR` trusts it (compile.md Decision 23).
#[test]
fn a_completer_runs_in_the_project_only_once_it_is_trusted() {
    let s = Sandbox::new("trust-dir", true, "return {}");
    let mut d = Drive::new(900.0, 500.0);
    let mut app = s.open(&mut d);
    d.keys(&mut app, ":compile where ");
    answered(&mut d, &mut app);
    let c = candidates(&app);
    assert!(
        c.iter().any(|c| c == "kawoosh-neutral"),
        "{c:?} {:?}",
        s.calls()
    );
    assert!(!c.iter().any(|c| c == "project"), "{c:?}");
    leave(&mut d, &mut app);

    d.keys(&mut app, ":");
    d.keys(&mut app, &format!("trust allow {}", s.project.display()));
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert!(app.ed.message.starts_with("trusted"), "{}", app.ed.message);
    d.keys(&mut app, ":compile where ");
    answered(&mut d, &mut app);
    let c = candidates(&app);
    assert!(c.iter().any(|c| c == "project"), "{c:?} {:?}", s.calls());
    leave(&mut d, &mut app);

    d.keys(&mut app, ":");
    d.keys(&mut app, &format!("trust revoke {}", s.project.display()));
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    d.keys(&mut app, ":compile where ");
    answered(&mut d, &mut app);
    let c = candidates(&app);
    assert!(c.iter().any(|c| c == "kawoosh-neutral"), "revoked: {c:?}");
    leave(&mut d, &mut app);
}

/// A shell line's program — `:!`'s first word, the first after a `|`,
/// `;`, `&&` or `||`, past assignments — completes to the executables on
/// the PATH, listed on a job once a prompt; a word that reads as a path
/// to the paths there. A file that does not run, a directory, is none;
/// a program installed since is offered at the next prompt.
#[test]
fn a_shell_line_s_program_completes_from_the_path() {
    use std::os::unix::fs::PermissionsExt;
    let s = Sandbox::new("programs", false, "return {}");
    let exe = |name: &str, mode: u32| {
        let p = s.bin.join(name);
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
    };
    exe("kwprog", 0o755);
    exe("kwplain", 0o644);
    std::fs::create_dir_all(s.bin.join("kwdir")).unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = s.open(&mut d);

    d.keys(&mut app, ":!kw");
    answered(&mut d, &mut app);
    assert_eq!(candidates(&app), ["kwprog"]);
    d.key(&mut app, "tab", KeyMods::default());
    assert_eq!(app.ed.prompt_text().as_deref(), Some("!kwprog"));
    leave(&mut d, &mut app);
    for line in [
        ":!echo a | kwp",
        ":!true && kwp",
        ":!FOO=1 kwp",
        ":shell kwp",
        ":terminal kwp",
    ] {
        d.keys(&mut app, line);
        answered(&mut d, &mut app);
        assert_eq!(candidates(&app), ["kwprog"], "{line}");
        leave(&mut d, &mut app);
    }
    // An argument is no program; an empty word offers none of thousands.
    d.keys(&mut app, ":!echo kwp");
    answered(&mut d, &mut app);
    assert_eq!(candidates(&app), Vec::<String>::new());
    leave(&mut d, &mut app);
    d.keys(&mut app, ":shell ");
    answered(&mut d, &mut app);
    assert_eq!(candidates(&app), Vec::<String>::new());
    leave(&mut d, &mut app);
    // A path to one.
    d.keys(&mut app, ":!./s");
    answered(&mut d, &mut app);
    assert_eq!(
        candidates(&app),
        [format!(".{0}src{0}", std::path::MAIN_SEPARATOR)]
    );
    leave(&mut d, &mut app);

    exe("kwnew", 0o755);
    d.keys(&mut app, ":!kwn");
    answered(&mut d, &mut app);
    assert_eq!(candidates(&app), ["kwnew"]);
    leave(&mut d, &mut app);
}

/// `:terminal shell`'s NAME completes to the shells on the PATH — at an
/// empty word too, there being a handful — and no other program; a word
/// that reads as a path to the paths there.
#[test]
fn terminal_shell_completes_the_shells_on_the_path() {
    use std::os::unix::fs::PermissionsExt;
    let s = Sandbox::new("shells", false, "return {}");
    for name in ["pwsh", "fish", "kwprog"] {
        let p = s.bin.join(name);
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mut d = Drive::new(900.0, 500.0);
    let mut app = s.open(&mut d);

    d.keys(&mut app, ":terminal shell ");
    answered(&mut d, &mut app);
    let c = candidates(&app);
    for shell in ["fish", "pwsh", "sh"] {
        assert!(c.iter().any(|c| c == shell), "{shell}: {c:?}");
    }
    assert!(!c.iter().any(|c| c == "kwprog"), "no other program: {c:?}");
    leave(&mut d, &mut app);
    d.keys(&mut app, ":terminal shell pw");
    answered(&mut d, &mut app);
    assert_eq!(candidates(&app), ["pwsh"]);
    d.key(&mut app, "tab", KeyMods::default());
    assert_eq!(app.ed.prompt_text().as_deref(), Some("terminal shell pwsh"));
    leave(&mut d, &mut app);
    d.keys(&mut app, ":terminal shell ./s");
    answered(&mut d, &mut app);
    assert_eq!(
        candidates(&app),
        [format!(".{0}src{0}", std::path::MAIN_SEPARATOR)]
    );
    leave(&mut d, &mut app);
}

/// The repository's packages `:compile` offers (compile.md Decision 9)
/// are walked on a job — a walk reads up to every entry — the prompt's
/// lines and the picker's rows read again as it lands.
#[test]
fn the_repository_s_packages_are_walked_off_the_frame() {
    let s = Sandbox::new("packages", false, "return {}");
    let pkg = s.project.join("web");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(
        pkg.join("package.json"),
        r#"{ "scripts": { "build": "vite build" } }"#,
    )
    .unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = s.open(&mut d);

    d.keys(&mut app, ":compile ");
    answered(&mut d, &mut app);
    assert!(
        candidates(&app).iter().any(|c| c == "npm run build"),
        "{:?}",
        candidates(&app)
    );
    d.keys(&mut app, "npm run b");
    assert_eq!(candidates(&app), ["npm run build"]);
    leave(&mut d, &mut app);

    // A script added since: the picker opens with the last walk's rows,
    // and takes the new one as the next walk lands.
    std::fs::write(
        pkg.join("package.json"),
        r#"{ "scripts": { "build": "vite build", "check": "tsc" } }"#,
    )
    .unwrap();
    app.offer_compile();
    answered(&mut d, &mut app);
    let rows: Vec<&str> = app.compile.offer.iter().map(|o| o.cmd.as_str()).collect();
    assert!(rows.contains(&"npm run build"), "{rows:?}");
    assert!(
        rows.contains(&"npm run check"),
        "{rows:?} {}",
        app.ed.message
    );
    assert_eq!(
        rows.iter().filter(|r| **r == "npm run build").count(),
        1,
        "a row there already is not made twice"
    );
}
