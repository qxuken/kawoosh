//! Exercise the bundled plugin against an actual fake mise process:
//! discovery, the live compile offer and cwd-scoped child environment.
mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use std::path::{Path, PathBuf};

fn fixture(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-mise-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
    std::fs::write(dir.join("mise.toml"), "# project\n").unwrap();
    std::fs::write(
        dir.join("env.json"),
        r#"{"PROJECT_VALUE":"first","EXPLICIT":"provider"}"#,
    )
    .unwrap();
    std::fs::write(dir.join("tasks.json"), serde_json::to_string(&serde_json::json!([
        { "name": "check", "description": "Check the project", "source": dir.join("mise.toml") },
        { "name": "name with spaces", "source": dir.join("mise.toml") },
        { "name": "needs argument", "usage": "arg \"<value>\"", "source": dir.join("mise.toml") },
        { "name": "hidden", "hide": true }, { "name": "global", "global": true }
    ])).unwrap()).unwrap();
    let script = if cfg!(windows) {
        "@echo off\r\nif \"%~1\"==\"env\" (\r\n type \"env.json\"\r\n exit /b 0\r\n)\r\nif \"%~1\"==\"tasks\" (\r\n type \"%~dp0tasks.json\"\r\n exit /b 0\r\n)\r\nif \"%~1\"==\"probe\" (\r\n echo {\"PROJECT_VALUE\":\"%PROJECT_VALUE%\",\"EXPLICIT\":\"%EXPLICIT%\"}\r\n exit /b 0\r\n)\r\nif \"%~1\"==\"run\" (\r\n echo \"%~2\">\"%~dp0task-ran\"\r\n echo \"%~3\">\"%~dp0task-argument\"\r\n echo selected task ran\r\n exit /b 0\r\n)\r\necho unexpected>\"%~dp0task-ran\"\r\nexit /b 1\r\n"
    } else {
        "#!/bin/sh\ncase \"$1\" in\nenv) cat env.json ;;\ntasks) cat \"$(dirname \"$0\")/tasks.json\" ;;\nprobe) printf '{\"PROJECT_VALUE\":\"%s\",\"EXPLICIT\":\"%s\"}' \"$PROJECT_VALUE\" \"$EXPLICIT\" ;;\nrun) printf '%s' \"$2\" > \"$(dirname \"$0\")/task-ran\"; printf '%s' \"$3\" > \"$(dirname \"$0\")/task-argument\"; echo 'selected task ran' ;;\n*) touch \"$(dirname \"$0\")/task-ran\"; exit 1 ;;\nesac\n"
    };
    let binary = dir.join(if cfg!(windows) {
        "fake-mise.cmd"
    } else {
        "fake-mise"
    });
    std::fs::write(&binary, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    dir
}

fn until(d: &mut Drive, app: &mut Kawoosh, mut condition: impl FnMut(&Kawoosh) -> bool) {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        d.frame(app);
        if condition(app) {
            return;
        }
        assert!(
            std::time::Instant::now() < end,
            "timed out: {}; messages: {}; servers: {:?}; terminals: {:?}",
            app.ed.message,
            app.notes.render_log(),
            app.lsp.status,
            app.terms
                .map
                .values()
                .map(|t| t.scrollback_text())
                .collect::<Vec<_>>()
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn probe(app: &mut Kawoosh, binary: &Path, dir: &Path) {
    app.run_lua_source("probe", &format!(
        "probe_answer = nil; kawoosh.spawn({{ {:?}, 'probe' }}, {{ cwd = {:?}, env = {{ EXPLICIT = 'caller' }}, on_done = function(out, code) probe_answer = code == 0 and kawoosh.json.decode(out) or {{}} end }})",
        binary.display().to_string(), dir.display().to_string()
    ));
}

#[test]
fn mise_discovers_without_running_tasks_and_children_use_the_project_environment() {
    let dir = fixture("discovery command with spaces");
    let binary = dir.join(if cfg!(windows) {
        "fake-mise.cmd"
    } else {
        "fake-mise"
    });
    let mut app = Kawoosh::new("*scratch*", "");
    let mut d = Drive::new(900.0, 600.0);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app.set_cwd(&dir);
    app.run_lua_source(
        "mise-test",
        &format!(
            "kawoosh.opt('mise.command', {:?})",
            binary.display().to_string()
        ),
    );
    d.frame(&mut app);
    d.frame(&mut app);
    app.offer_compile();
    until(&mut d, &mut app, |a| {
        a.compile.offer.iter().any(|o| o.cmd.contains(" run check"))
    });
    let offer = app
        .compile
        .offer
        .iter()
        .find(|o| o.cmd.contains(" run check"))
        .unwrap();
    assert_eq!(offer.cwd, dir);
    assert_eq!(offer.why, "Check the project");
    assert!(
        !app.compile
            .offer
            .iter()
            .any(|o| o.cmd.ends_with(" run hidden") || o.cmd.ends_with(" run global"))
    );
    assert!(
        !dir.join("task-ran").exists(),
        "listing tasks never runs them"
    );

    probe(&mut app, &binary, &dir);
    until(&mut d, &mut app, |a| {
        a.scripting
            .rt
            .as_ref()
            .unwrap()
            .eval("probe_answer ~= nil")
            .unwrap()
            == "true"
    });
    assert_eq!(
        app.scripting
            .rt
            .as_ref()
            .unwrap()
            .eval("probe_answer.PROJECT_VALUE == 'first'")
            .unwrap(),
        "true"
    );
    assert_eq!(
        app.scripting
            .rt
            .as_ref()
            .unwrap()
            .eval("probe_answer.EXPLICIT == 'caller'")
            .unwrap(),
        "true"
    );

    std::fs::write(
        dir.join("env.json"),
        r#"{"PROJECT_VALUE":"second-value","EXPLICIT":"provider"}"#,
    )
    .unwrap();
    std::fs::write(dir.join("mise.toml"), "# configuration changed\n").unwrap();
    probe(&mut app, &binary, &dir);
    until(&mut d, &mut app, |a| {
        a.scripting
            .rt
            .as_ref()
            .unwrap()
            .eval("probe_answer ~= nil")
            .unwrap()
            == "true"
    });
    assert_eq!(
        app.scripting
            .rt
            .as_ref()
            .unwrap()
            .eval("probe_answer.PROJECT_VALUE == 'second-value'")
            .unwrap(),
        "true"
    );
    assert!(!dir.join("task-ran").exists());
    // Selection uses argv, so a custom binary path and a task argument
    // containing spaces work independently of Nu/PowerShell/POSIX syntax.
    let task = app
        .compile
        .offer
        .iter()
        .position(|o| o.cmd.contains("name with spaces"))
        .unwrap()
        + 1;
    app.compile_offered(task, false);
    until(&mut d, &mut app, |a| {
        a.compile.buffer().is_some() && !a.compile.running()
    });
    let output = app.ed.buffers[app.compile.buffer().unwrap()].text();
    assert!(output.contains("selected task ran"), "{output}");
    assert_eq!(
        std::fs::read_to_string(dir.join("task-ran"))
            .unwrap()
            .trim()
            .trim_matches('"'),
        "name with spaces"
    );
    assert!(app.compile.runs.iter().any(|r| {
        r.steps[0]
            .argv
            .as_ref()
            .is_some_and(|a| a[2] == "name with spaces")
    }));

    let required = app
        .compile
        .offer
        .iter()
        .position(|o| o.cmd.contains("needs argument"))
        .unwrap()
        + 1;
    app.compile_offered(required, false);
    assert!(
        app.ed.prompt_view().is_some(),
        "required arguments open the prompt"
    );
    d.keys(&mut app, "\"argument with spaces\"");
    d.key(&mut app, "enter", Default::default());
    until(&mut d, &mut app, |a| !a.compile.running());
    assert_eq!(
        std::fs::read_to_string(dir.join("task-argument"))
            .unwrap()
            .trim()
            .trim_matches('"'),
        "argument with spaces"
    );
    assert!(
        app.compile.runs.last().unwrap().steps[0]
            .argv
            .as_ref()
            .unwrap()
            .ends_with(&["argument with spaces".to_string()])
    );

    app.compile_offered(task, true);
    d.keys(&mut app, "\"edited argument\"");
    d.key(&mut app, "enter", Default::default());
    until(&mut d, &mut app, |a| !a.compile.running());
    assert_eq!(
        std::fs::read_to_string(dir.join("task-argument"))
            .unwrap()
            .trim()
            .trim_matches('"'),
        "edited argument"
    );
    std::fs::remove_file(dir.join("task-argument")).unwrap();
    app.run_lua_source("again", "kawoosh.run('compile')");
    until(&mut d, &mut app, |a| {
        !a.compile.running() && dir.join("task-argument").exists()
    });
    assert_eq!(
        std::fs::read_to_string(dir.join("task-argument"))
            .unwrap()
            .trim()
            .trim_matches('"'),
        "edited argument",
        "recent rerun retains argv"
    );

    // LSP and terminal launches share the same scoped resolver, and the
    // first terminal launch waits for its environment off the UI thread.
    let log = dir.join("lsp-environment.txt");
    let wrapper = dir.join("env_lsp.py");
    let fake_lsp = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_lsp.py");
    std::fs::write(&wrapper, format!(
        "import os, pathlib, runpy\npathlib.Path({:?}).write_text(os.environ.get('PROJECT_VALUE', 'missing'))\nrunpy.run_path({:?}, run_name='__main__')\n",
        log.display().to_string(), fake_lsp.display().to_string()
    )).unwrap();
    let (command, mut args) = drive::python();
    args.push(wrapper.display().to_string());
    app.add_lsp_server(kawoosh_systems::lsp::ServerDef {
        language: "rust".into(),
        command,
        args,
        roots: vec!["mise.toml".into()],
        ..Default::default()
    });
    let source = dir.join("probe.rs");
    std::fs::write(&source, "fn main() {}\n").unwrap();
    app.run_lua_source(
        "open",
        &format!(
            "kawoosh.picker.close(); kawoosh.open({:?})",
            source.display().to_string()
        ),
    );
    until(&mut d, &mut app, |_| log.exists());
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "second-value");

    let shell = if cfg!(windows) { "cmd.exe" } else { "/bin/sh" };
    let line = if cfg!(windows) {
        "!echo %%PROJECT_VALUE%% > terminal-value.txt"
    } else {
        "!echo \"$PROJECT_VALUE\" > terminal-value.txt"
    };
    app.run_lua_source(
        "terminal",
        &format!(
            "kawoosh.opt('terminal.shell', {:?}); kawoosh.run({:?})",
            shell, line
        ),
    );
    let other = dir.join("other");
    std::fs::create_dir_all(&other).unwrap();
    app.set_cwd(&other);
    until(&mut d, &mut app, |_| {
        dir.join("terminal-value.txt").exists()
    });
    assert_eq!(
        std::fs::read_to_string(dir.join("terminal-value.txt"))
            .unwrap()
            .trim(),
        "second-value"
    );
    assert!(
        !other.join("terminal-value.txt").exists(),
        "the pending terminal keeps its launch directory"
    );

    app.set_cwd(&dir);
    app.run_lua_source(
        "cancel",
        "kawoosh.run('!echo cancelled > cancelled-marker.txt')",
    );
    let pending_pane = app.layout.focused();
    assert!(matches!(
        app.layout.content(pending_pane),
        Some(kawoosh::layout::Content::Terminal(_))
    ));
    app.layout.close(pending_pane);
    for _ in 0..30 {
        d.frame(&mut app);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        !dir.join("cancelled-marker.txt").exists(),
        "closing a pane during environment preparation cannot launch its command later"
    );

    drop(app);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn the_listing_cache_and_mise_async_cache_lua_regressions_pass() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("lua/tests");
    for file in [
        "vcs_listing_cache.lua",
        "mise_discovery.lua",
        "spawn_quiet.lua",
    ] {
        kawoosh::harness::run_file(&root.join(file)).unwrap();
    }
}

#[test]
fn input_before_a_project_environment_arrives_reaches_its_terminal_once() {
    let dir = fixture("pending-input");
    std::fs::remove_file(dir.join("mise.toml")).unwrap();
    let gate = dir.join("release-provider");
    let output = dir.join("terminal-input.json");
    std::fs::remove_file(&gate).ok();
    std::fs::remove_file(&output).ok();
    let provider = dir.join("provider.py");
    std::fs::write(&provider,
        "import pathlib, time\nwhile not pathlib.Path('release-provider').exists(): time.sleep(.01)\nprint('{\"PENDING_TEST\":\"ready\"}')\n"
    ).unwrap();
    let consumer = dir.join("consumer.py");
    std::fs::write(&consumer,
        "import json, os, pathlib\nlines = [input() for _ in range(4)]\npathlib.Path('terminal-input.json').write_text(json.dumps([os.environ.get('PENDING_TEST'), lines]))\n"
    ).unwrap();
    let (python, mut args) = drive::python();
    let mut command = vec![python.clone()];
    command.extend(args.clone());
    command.push(provider.display().to_string());
    let mut app = Kawoosh::new("*scratch*", "");
    let mut d = Drive::new(900.0, 600.0);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app.set_cwd(&dir);
    app.io.environments.register(
        "delayed-test".into(),
        Some(kawoosh_systems::project_env::Provider {
            command,
            ..Default::default()
        }),
    );
    app.ed.settings.set(
        kawoosh_editor::Layer::Session,
        "terminal.shell",
        kawoosh_editor::Setting::Str(if cfg!(windows) { "cmd.exe" } else { "/bin/sh" }.into()),
    );
    args.push(consumer.display().to_string());
    let argv = std::iter::once(python).chain(args).collect::<Vec<_>>();
    let id = app.spawn_bang_argv(&argv, &dir).unwrap();
    app.layout
        .panes
        .insert(app.layout.focused(), kawoosh::layout::Content::Terminal(id));
    d.frame(&mut app);
    assert!(!app.terms.map.contains_key(&id), "provider is still gated");
    // A reconnect retains the finished screen under this number while
    // preparing its replacement. Its old normal-mode keys must not
    // consume the replacement's input.
    app.terms.map.insert(
        id,
        kawoosh_term::Terminal::headless(kawoosh_term::TermSize { rows: 24, cols: 80 }),
    );
    app.terms.done.insert(id, None);
    d.keys(&mut app, "keyboard");
    d.press(&mut app, "<CR>");
    d.commit(&mut app, "pasted\n");
    app.shell_command("terminal", &["send".into(), "notation<CR>".into()], None);
    app.run_lua_source("queued-send", "kawoosh.term.send('lua\\r')");
    assert_eq!(app.term_of_focused(), Some(id));
    assert!(
        app.terms.done.contains_key(&id),
        "all four inputs precede adoption"
    );
    assert!(
        app.terms.map.get_mut(&id).unwrap().take_sent().is_empty(),
        "the old terminal receives none of the new input"
    );
    app.run_lua_source("overflow", "kawoosh.term.send(string.rep('x', 65537))");
    assert!(app.ed.message.contains("pending environment input limit"));
    let original = app.layout.focused();
    let canceled = app
        .spawn_terminal(Some("echo cancelled > cancelled-input.txt"), Some(&dir))
        .unwrap();
    let pane = app.layout.split(
        kawoosh::layout::SplitDir::H,
        kawoosh::layout::Content::Terminal(canceled),
    );
    d.frame(&mut app);
    d.keys(&mut app, "discarded");
    app.layout.close(pane);
    app.layout.focus(original);
    std::fs::write(&gate, "go").unwrap();
    until(&mut d, &mut app, |_| output.exists());
    let answer: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&output).unwrap()).unwrap();
    assert_eq!(
        answer,
        serde_json::json!(["ready", ["keyboard", "pasted", "notation", "lua"]])
    );
    assert!(
        !dir.join("cancelled-input.txt").exists(),
        "closing a loading pane discards its input and launch"
    );
    drop(app);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_pending_shell_keeps_its_directory_shell_and_recording() {
    let dir = fixture("pending-shell");
    std::fs::remove_file(dir.join("mise.toml")).unwrap();
    let gate = dir.join("release-provider");
    std::fs::remove_file(&gate).ok();
    let cast = dir.join("first bytes.cast");
    std::fs::remove_file(&cast).ok();
    let provider = dir.join("provider.py");
    std::fs::write(&provider,
        "import pathlib, time\nwhile not pathlib.Path('release-provider').exists(): time.sleep(.01)\nprint('{\"PENDING_TEST\":\"ready\"}')\n"
    ).unwrap();
    let (python, args) = drive::python();
    let command = std::iter::once(python)
        .chain(args)
        .chain(std::iter::once(provider.display().to_string()))
        .collect();
    let mut app = Kawoosh::new("*scratch*", "");
    let mut d = Drive::new(900.0, 600.0);
    app.set_cwd(&dir);
    app.io.environments.register(
        "delayed-test".into(),
        Some(kawoosh_systems::project_env::Provider {
            command,
            ..Default::default()
        }),
    );
    app.ed.settings.set(
        kawoosh_editor::Layer::Session,
        "terminal.shell",
        kawoosh_editor::Setting::Str(if cfg!(windows) { "cmd.exe" } else { "/bin/sh" }.into()),
    );
    app.shell_command("terminal record into", &[cast.display().to_string()], None);
    let id = app.term_of_focused().expect("a recording terminal");
    d.frame(&mut app);
    assert!(!app.terms.map.contains_key(&id));
    assert!(app.term_title(id).ends_with(" · recording"));
    let line = if cfg!(windows) {
        "echo %PENDING_TEST% FIRST-OUTPUT"
    } else {
        "echo \"$PENDING_TEST\" FIRST-OUTPUT"
    };
    d.keys(&mut app, line);
    d.press(&mut app, "<CR>");
    let other = dir.join("other");
    std::fs::create_dir_all(&other).unwrap();
    app.set_cwd(&other);
    app.ed.settings.set(
        kawoosh_editor::Layer::Session,
        "terminal.shell",
        kawoosh_editor::Setting::Str("kawoosh-no-such-shell-after-switch".into()),
    );
    fn keeps_terminal_cwd(value: &serde_json::Value, cwd: &serde_json::Value) -> bool {
        match value {
            serde_json::Value::Object(o) => {
                (o.get("content").is_some_and(|v| v == "terminal") && o.get("cwd") == Some(cwd))
                    || o.values().any(|v| keeps_terminal_cwd(v, cwd))
            }
            serde_json::Value::Array(a) => a.iter().any(|v| keeps_terminal_cwd(v, cwd)),
            _ => false,
        }
    }
    assert!(
        keeps_terminal_cwd(
            &serde_json::to_value(app.session_data()).unwrap(),
            &serde_json::to_value(&dir).unwrap()
        ),
        "a session retains a waiting shell's own cwd"
    );
    std::fs::write(&gate, "go").unwrap();
    // ConPTY can publish startup controls before the shell consumes its
    // queued line. Let that real command answer before asking it to exit.
    until(&mut d, &mut app, |a| {
        a.terms
            .map
            .get(&id)
            .is_some_and(|t| t.scrollback_text().contains("ready FIRST-OUTPUT"))
    });
    d.keys(&mut app, "exit");
    d.press(&mut app, "<CR>");
    until(&mut d, &mut app, |a| a.ed.message.starts_with("recorded "));
    let recording = kawoosh_term::cast::Cast::read(&cast).unwrap();
    let input = recording
        .events
        .iter()
        .filter_map(|e| match &e.kind {
            kawoosh_term::cast::Kind::Input(s) => Some(s.as_str()),
            _ => None,
        })
        .collect::<String>();
    let output = recording
        .events
        .iter()
        .filter_map(|e| match &e.kind {
            kawoosh_term::cast::Kind::Output(s) => Some(s.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert!(
        input.starts_with(line),
        "queued first input is recorded: {input:?}"
    );
    assert!(
        output.contains("ready FIRST-OUTPUT"),
        "first output uses the original shell/environment: {output:?}"
    );
    drop(app);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_failed_language_server_does_not_poison_another_project_or_a_changed_environment() {
    let dir = fixture("failed-server");
    let first = dir.join("a");
    let second = dir.join("b");
    let bin = dir.join("bin");
    for root in [&first, &second, &bin] {
        std::fs::create_dir_all(root).unwrap();
    }
    for root in [&first, &second] {
        // Workspace discovery chooses the outermost marker within a
        // repository; these are deliberately two independent projects.
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join("mise.toml"), "# project\n").unwrap();
        std::fs::write(root.join("probe.rs"), "fn main() {}\n").unwrap();
    }
    let original_path = std::env::var_os("PATH").unwrap_or_default();
    let project_path = std::env::join_paths(
        std::iter::once(bin.clone()).chain(std::env::split_paths(&original_path)),
    )
    .unwrap();
    let first_log = first.join("server-env.txt");
    let second_log = second.join("server-env.txt");
    let write_env = |root: &Path, path: &std::ffi::OsStr, value: &str, log: &Path| {
        std::fs::write(
            root.join("env.json"),
            serde_json::to_string(&serde_json::json!({
                "PATH": path.to_string_lossy(), "PROJECT_VALUE": value, "PROJECT_LOG": log,
            }))
            .unwrap(),
        )
        .unwrap();
    };
    write_env(&first, &original_path, "absent", &first_log);
    write_env(&second, &project_path, "second", &second_log);

    let wrapper = dir.join("project_lsp.py");
    let fake_lsp = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_lsp.py");
    std::fs::write(&wrapper, format!(
        "import os, pathlib, runpy\npathlib.Path(os.environ['PROJECT_LOG']).write_text(os.environ.get('PROJECT_VALUE', 'missing'))\nrunpy.run_path({:?}, run_name='__main__')\n", fake_lsp.display().to_string()
    )).unwrap();
    let (python, arguments) = drive::python();
    let name = "kawoosh-project-env-test-lsp";
    let executable = bin.join(if cfg!(windows) {
        format!("{name}.cmd")
    } else {
        name.to_string()
    });
    let launcher = if cfg!(windows) {
        format!(
            "@echo off\r\n\"{}\" {} \"{}\" %*\r\n",
            python,
            arguments.join(" "),
            wrapper.display()
        )
    } else {
        let words = std::iter::once(python)
            .chain(arguments)
            .chain(std::iter::once(wrapper.display().to_string()))
            .map(|w| kawoosh_systems::io::shell_quote(&w))
            .collect::<Vec<_>>()
            .join(" ");
        format!("#!/bin/sh\nexec {words} \"$@\"\n")
    };
    std::fs::write(&executable, launcher).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let mut app = Kawoosh::new("*scratch*", "");
    let mut d = Drive::new(900.0, 600.0);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app.set_cwd(&first);
    let fake_mise = dir.join(if cfg!(windows) {
        "fake-mise.cmd"
    } else {
        "fake-mise"
    });
    app.run_lua_source(
        "mise-test",
        &format!(
            "kawoosh.opt('mise.command', {:?})",
            fake_mise.display().to_string()
        ),
    );
    d.frame(&mut app);
    d.frame(&mut app);
    app.add_lsp_server(kawoosh_systems::lsp::ServerDef {
        language: "rust".into(),
        command: name.into(),
        roots: vec!["mise.toml".into()],
        ..Default::default()
    });
    app.ed.settings.set(
        kawoosh_editor::settings::Layer::Session,
        "lsp.rust.enabled",
        kawoosh_editor::Setting::Bool(true),
    );
    app.ed.settings.set(
        kawoosh_editor::settings::Layer::Session,
        "lsp.languages.rust",
        kawoosh_editor::Setting::List(vec![kawoosh_editor::Setting::Str("rust".into())]),
    );
    let open = |app: &mut Kawoosh, file: &Path| {
        app.run_lua_source(
            "open",
            &format!("kawoosh.open({:?})", file.display().to_string()),
        )
    };
    open(&mut app, &first.join("probe.rs"));
    until(&mut d, &mut app, |a| {
        a.notes
            .render_log()
            .contains(&format!("`{name}` not found"))
    });
    assert!(!first_log.exists());

    open(&mut app, &second.join("probe.rs"));
    until(&mut d, &mut app, |_| second_log.exists());
    assert_eq!(std::fs::read_to_string(&second_log).unwrap(), "second");

    write_env(&first, &project_path, "first-recovered", &first_log);
    std::fs::write(first.join("mise.toml"), "# changed tool environment\n").unwrap();
    let another = first.join("another.rs");
    std::fs::write(&another, "fn another() {}\n").unwrap();
    open(&mut app, &another);
    until(&mut d, &mut app, |_| first_log.exists());
    assert_eq!(
        std::fs::read_to_string(&first_log).unwrap(),
        "first-recovered"
    );
    drop(app);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn symbols_auto_uses_the_grammar_after_a_server_failed_in_this_root() {
    let dir = fixture("missing-json-server");
    let file = dir.join("document.json");
    std::fs::write(&file, "{\n  \"alpha\": 1\n}\n").unwrap();
    let missing = "kawoosh-unique-missing-json-lsp";
    let mut app = Kawoosh::new("*scratch*", "");
    let mut d = Drive::new(900.0, 600.0);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app.set_cwd(&dir);
    app.run_lua_source("test", "kawoosh.opt('mise.enabled', false)");
    app.add_lsp_server(kawoosh_systems::lsp::ServerDef {
        language: "json".into(),
        command: missing.into(),
        roots: vec!["mise.toml".into()],
        ..Default::default()
    });
    app.ed.settings.set(
        kawoosh_editor::settings::Layer::Session,
        "lsp.languages.json",
        kawoosh_editor::Setting::List(vec![kawoosh_editor::Setting::Str("json".into())]),
    );
    app.ed.settings.set(
        kawoosh_editor::settings::Layer::Session,
        "lsp.json.enabled",
        kawoosh_editor::Setting::Bool(true),
    );
    app.ed.settings.set(
        kawoosh_editor::settings::Layer::Session,
        "symbols.source",
        kawoosh_editor::Setting::Str("auto".into()),
    );
    app.run_lua_source(
        "open",
        &format!("kawoosh.open({:?})", file.display().to_string()),
    );
    until(&mut d, &mut app, |a| {
        a.notes
            .render_log()
            .contains(&format!("`{missing}` not found"))
    });
    app.wait_for_syntax();
    app.run_lua_source("symbols", "kawoosh.picker.open('symbols')");
    until(&mut d, &mut app, |a| {
        a.scripting.rt.as_ref().unwrap().eval("local s = kawoosh.picker.state(); return s ~= nil and table.concat(s.rows, '|'):find('alpha', 1, true) ~= nil").unwrap() == "true"
    });
    drop(app);
    std::fs::remove_dir_all(dir).ok();
}
