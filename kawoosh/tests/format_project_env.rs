//! Real formatter selection, execution and indentation through a fake
//! mise whose PATH contains a formatter absent from the editor's PATH.
mod drive;

use drive::Drive;
use kawoosh::{Kawoosh, format::Choice};
use kawoosh_editor::{Layer, Setting};
use std::path::{Path, PathBuf};

const TOOL: &str = "kawoosh-project-only-formatter";

fn executable(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn fixture(tag: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("kawoosh-format-env-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
    std::fs::write(dir.join("mise.toml"), "# fake project\n").unwrap();
    std::fs::write(dir.join(".format-project"), "").unwrap();
    std::fs::write(dir.join("a.ts"), "unformatted\n").unwrap();
    std::fs::write(
        dir.join("env.json"),
        serde_json::to_vec(&serde_json::json!({
            "PATH": dir.join("bin"), "PROJECT_VALUE": "from-project"
        }))
        .unwrap(),
    )
    .unwrap();
    let mise = dir.join(if cfg!(windows) {
        "fake-mise.cmd"
    } else {
        "fake-mise"
    });
    let script = if cfg!(windows) {
        "@echo off\r\nif \"%~1\"==\"env\" goto env\r\nif \"%~1\"==\"tasks\" (echo [] & exit /b 0)\r\necho unexpected>\"%~dp0task-ran\"\r\nexit /b 1\r\n:env\r\necho started>\"%~dp0env-started\"\r\n:wait\r\nif exist \"%~dp0gate\" if not exist \"%~dp0release\" goto wait\r\ntype \"%~dp0env.json\"\r\n"
    } else {
        "#!/bin/sh\ndir=${0%/*}\ncase $1 in\nenv) : > \"$dir/env-started\"; while [ -f \"$dir/gate\" ] && [ ! -f \"$dir/release\" ]; do /bin/sleep 0.01; done; /bin/cat \"$dir/env.json\" ;;\ntasks) printf '[]' ;;\n*) : > \"$dir/task-ran\"; exit 1 ;;\nesac\n"
    };
    executable(&mise, script);
    let tool = dir.join("bin").join(if cfg!(windows) {
        format!("{TOOL}.cmd")
    } else {
        TOOL.into()
    });
    executable(
        &tool,
        if cfg!(windows) {
            "@echo off\r\necho if (a) {\r\necho    %PROJECT_VALUE%\r\necho }\r\n"
        } else {
            "#!/bin/sh\nprintf 'if (a) {\n   %s\n}\n' \"$PROJECT_VALUE\"\n"
        },
    );
    (dir, mise)
}

fn until(d: &mut Drive, app: &mut Kawoosh, mut condition: impl FnMut(&Kawoosh) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        d.frame(app);
        if condition(app) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out: {}; LSP: {}; status: {:?}",
            app.ed.message,
            app.lsp.logs.render(&kawoosh::lsp_logs::Shown::All),
            app.lsp.status,
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn app(dir: &Path, mise: &Path) -> (Kawoosh, Drive, kawoosh_doc::BufferId) {
    let mut app = Kawoosh::new("*scratch*", "");
    let mut d = Drive::new(900.0, 500.0);
    let extension = app.attach_lua().unwrap();
    d.extension("lua", extension).unwrap();
    app.set_cwd(dir);
    app.run_lua_source("formatter environment fixture", &format!(
        "kawoosh.opt('mise.command', {:?}); kawoosh.opt('lsp.typescript.enabled', false); kawoosh.formatter('project', {{ cmd = {:?}, languages = {{ 'typescript' }}, when = {{ '.format-project' }}, probe = {{ typescript = 'if (a) {{\\nb;\\n}}\\n' }} }})",
        mise.display().to_string(), TOOL
    ));
    d.frame(&mut app);
    app.open(&dir.join("a.ts"));
    d.frame(&mut app);
    let id = app.ed.views[app.focused_view().unwrap()].buffer;
    (app, d, id)
}

fn format(app: &mut Kawoosh, id: kawoosh_doc::BufferId) {
    app.run_lua_source(
        "format fixture",
        &format!("kawoosh.format({})", kawoosh_lua::handle_of(id)),
    );
}

#[test]
fn a_project_only_formatter_formats_and_probes_through_mise() {
    assert_ne!(kawoosh_systems::io::on_path(TOOL), Some(true));
    let (dir, mise) = fixture("format-probe");
    let (mut app, mut d, id) = app(&dir, &mise);
    until(&mut d, &mut app, |a| a.ed.shiftwidth_in(id) == 3);
    assert!(matches!(app.formatter_for(id, None), Choice::Tool(p) if p.def.name == "project"));
    format(&mut app, id);
    until(&mut d, &mut app, |a| {
        a.ed.buffers[id].text().contains("from-project")
    });
    assert_eq!(
        app.ed.buffers[id].text().replace("\r\n", "\n"),
        "if (a) {\n   from-project\n}\n"
    );
    assert!(
        !dir.join("task-ran").exists(),
        "environment discovery must not run tasks"
    );
    drop(app);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_pending_format_keeps_its_project_recipe_and_save_completes() {
    let (dir, mise) = fixture("pending-recipe");
    std::fs::write(dir.join("gate"), "").unwrap();
    let (mut app, mut d, id) = app(&dir, &mise);
    until(&mut d, &mut app, |_| dir.join("env-started").exists());
    app.ed
        .settings
        .set(Layer::Session, "format_on_save", Setting::Bool(true));
    app.run_lua_source("save", "kawoosh.run('w')");
    d.frame(&mut app);
    assert_eq!(app.ed.buffers[id].text(), "unformatted\n");
    assert_eq!(
        std::fs::read_to_string(dir.join("a.ts")).unwrap(),
        "unformatted\n"
    );
    let other = dir.join("other");
    std::fs::create_dir_all(&other).unwrap();
    app.set_cwd(&other);
    app.ed.settings.set(
        Layer::Session,
        "format.project.cmd",
        Setting::Str("wrong-project-formatter".into()),
    );
    std::fs::write(dir.join("release"), "").unwrap();
    until(&mut d, &mut app, |_| {
        std::fs::read_to_string(dir.join("a.ts"))
            .unwrap()
            .contains("from-project")
    });
    assert!(!other.join("a.ts").exists());
    assert!(!dir.join("task-ran").exists());
    drop(app);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn missing_project_tools_keep_list_lsp_fallback_and_config_errors() {
    let (dir, mise) = fixture("fallback");
    let (mut app, mut d, id) = app(&dir, &mise);
    let (python, args) = drive::python();
    let output = kawoosh_systems::spawn::output(
        std::process::Command::new(python)
            .args(args)
            .args(["-c", "import sys; print(sys.executable)"]),
    )
    .unwrap();
    assert!(output.status.success());
    let mut server = drive::fake_lsp("typescript");
    server.command = String::from_utf8(output.stdout).unwrap().trim().into();
    server.args = vec![
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/fake_lsp.py")
            .display()
            .to_string(),
    ];
    app.ed.settings.set(
        Layer::Session,
        "lsp.typescript.enabled",
        Setting::Bool(true),
    );
    app.ed.settings.set(
        Layer::Session,
        "lsp.languages.typescript",
        Setting::List(vec![Setting::Str("typescript".into())]),
    );
    app.add_lsp_server(server);
    until(&mut d, &mut app, |a| {
        a.lsp.caps.get("typescript").is_some_and(|c| c.format)
    });
    app.ed.settings.set(
        Layer::Session,
        "format.project.cmd",
        Setting::Str("missing-project-formatter".into()),
    );
    app.ed.settings.set(
        Layer::Session,
        "formatter",
        Setting::List(vec![
            Setting::Str("project".into()),
            Setting::Str("lsp".into()),
        ]),
    );
    format(&mut app, id);
    until(&mut d, &mut app, |a| {
        a.ed.buffers[id].text().starts_with("// formatted\n")
    });
    app.ed
        .settings
        .set(Layer::Session, "formatter", Setting::Str("auto".into()));
    assert!(
        matches!(app.formatter_for(id, None), Choice::None(why) if why.contains(".format-project says the project formats with it"))
    );
    app.ed
        .settings
        .set(Layer::Session, "formatter", Setting::Str("lsp".into()));
    assert!(matches!(app.formatter_for(id, None), Choice::Lsp(_)));
    assert!(!dir.join("task-ran").exists());
    drop(app);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_format_rechecks_a_cached_missing_tool_after_local_configuration_changes() {
    let (dir, mise) = fixture("negative-cache");
    std::fs::create_dir_all(dir.join("empty-bin")).unwrap();
    std::fs::write(
        dir.join("env.json"),
        serde_json::to_vec(&serde_json::json!({
            "PATH": dir.join("empty-bin"), "PROJECT_VALUE": "before"
        }))
        .unwrap(),
    )
    .unwrap();
    let (mut app, mut d, id) = app(&dir, &mise);
    until(
        &mut d,
        &mut app,
        |a| matches!(a.formatter_for(id, None), Choice::None(why) if why.contains("is not found")),
    );
    std::fs::write(
        dir.join("env.json"),
        serde_json::to_vec(&serde_json::json!({
            "PATH": dir.join("bin"), "PROJECT_VALUE": "recovered-project"
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(dir.join("mise.toml"), "# tools became available\n").unwrap();
    format(&mut app, id);
    until(&mut d, &mut app, |a| {
        a.ed.buffers[id].text().contains("recovered-project")
    });
    assert!(!dir.join("task-ran").exists());
    drop(app);
    std::fs::remove_dir_all(dir).ok();
}
