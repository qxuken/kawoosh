mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh_languages::{LanguageDef, Library, Source};

fn picker(app: &mut Kawoosh, d: &mut Drive, wanted: &[&str]) {
    app.ed.settings.set(
        kawoosh_editor::Layer::Session,
        "symbols.source",
        kawoosh_editor::Setting::Str("syntax".into()),
    );
    app.wait_for_syntax();
    d.frame(app);
    let view = app.focused_view().unwrap();
    let origin = app.ed.views[view].sels.primary().head;
    app.run_lua_source("config-outline", "kawoosh.picker.open('symbols')");
    for _ in 0..200 {
        app.wait_for_jobs();
        d.frame(app);
        app.run_lua_source("config-outline", "local s=kawoosh.picker.state(); kawoosh.echo(s and table.concat(s.rows, '|') or 'no picker')");
        if wanted.iter().all(|name| app.ed.message.contains(name)) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    for name in wanted {
        assert!(
            app.ed.message.contains(name),
            "missing {name}: {}",
            app.ed.message
        );
    }
    app.run_lua_source("config-outline", "kawoosh.run('picker last')");
    for _ in 0..4 {
        d.frame(app);
    }
    assert_ne!(
        app.ed.views[view].sels.primary().head,
        origin,
        "last symbol follows into the file"
    );
    app.run_lua_source("config-outline", "kawoosh.run('picker close')");
    for _ in 0..4 {
        d.frame(app);
    }
    assert_eq!(
        app.ed.views[view].sels.primary().head,
        origin,
        "cancel restores the original caret"
    );
}

#[test]
fn supported_config_dotfiles_offer_symbols_in_the_real_picker() {
    let dir = std::env::temp_dir().join(format!("kawoosh-config-outline-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, source, wanted) in [
        (
            ".zshrc",
            "export EDITOR=kawoosh\nwork() {\n echo ok\n}\n",
            vec!["EDITOR", "work"],
        ),
        (
            ".prettierrc",
            "{\n \"printWidth\": 96,\n \"semi\": false\n}\n",
            vec!["printWidth", "semi"],
        ),
        (
            ".clang-format",
            "BasedOnStyle: LLVM\nColumnLimit: 96\n",
            vec!["BasedOnStyle", "ColumnLimit"],
        ),
        (
            "rustfmt.toml",
            "edition = '2024'\nmax_width = 96\n",
            vec!["edition", "max_width"],
        ),
    ] {
        let file = dir.join(name);
        std::fs::write(&file, source).unwrap();
        let mut app = Kawoosh::from_file(&file);
        app.jobs_inline = true;
        let mut d = Drive::new(1200.0, 600.0);
        let ext = app.attach_lua().unwrap();
        d.extension("lua", ext).unwrap();
        app.wait_for_open();
        d.frame(&mut app);
        picker(&mut app, &mut d, &wanted);
    }
    std::fs::remove_dir_all(dir).ok();
}

/// As with grammars.rs's repository checks, opt in with a built grammar
/// checkout. No network or installation is needed, and no outline is supplied
/// here: the app's fallback must work for archives shipped before it existed.
#[test]
fn installed_config_grammars_have_outlines_even_without_archive_queries() {
    let Some(repo) = std::env::var_os("KAWOOSH_GRAMMARS_REPO") else {
        return;
    };
    let repo = std::path::PathBuf::from(repo);
    let dir = std::env::temp_dir().join(format!(
        "kawoosh-external-config-outline-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let hl = dir.join("highlights.scm");
    std::fs::write(&hl, "").unwrap();
    for (language, name, text, wanted) in [
        (
            "nginx",
            "nginx.conf",
            "events {\n worker_connections 1024;\n}\nhttp {\n server {\n  listen 80;\n  location /api {\n   proxy_pass http://backend;\n  }\n }\n}\n",
            vec![
                "events",
                "worker_connections",
                "http",
                "server",
                "listen",
                "/api",
                "proxy_pass",
            ],
        ),
        (
            "ini",
            ".editorconfig",
            "root = true\n[*]\nindent_size = 2\n",
            vec!["root", "*", "indent_size"],
        ),
        (
            "git_config",
            ".gitconfig",
            "[core]\n editor = kawoosh\n[alias]\n st = status\n",
            vec!["core", "editor", "alias", "st"],
        ),
        (
            "ssh_config",
            "ssh_config",
            "Host example\n HostName example.org\n User me\n",
            vec!["example", "HostName", "User"],
        ),
        (
            "dotenv",
            ".env.production",
            "HOST=example.org\nPORT=443\n",
            vec!["HOST", "PORT"],
        ),
    ] {
        let lib = Library {
            path: repo
                .join("work/build")
                .join(language)
                .join(kawoosh_systems::grammars::target())
                .join(format!("parser.{}", std::env::consts::DLL_EXTENSION)),
            symbol: format!("tree_sitter_{language}"),
            highlights: hl.clone(),
            injections: None,
            outline: None,
            indents: None,
            textobjects: None,
        };
        let mut def = LanguageDef::named(language);
        def.filenames.push(name.into());
        def.grammar = Some(Source::Library(Box::new(lib)));
        let mut app = Kawoosh::new("t", "");
        app.jobs_inline = true;
        app.add_language(def);
        assert!(app.languages.has_grammar(language), "{}", app.ed.message);
        let mut d = Drive::new(1200.0, 600.0);
        let ext = app.attach_lua().unwrap();
        d.extension("lua", ext).unwrap();
        let file = dir.join(name);
        std::fs::write(&file, text).unwrap();
        app.open(&file);
        app.wait_for_open();
        d.frame(&mut app);
        picker(&mut app, &mut d, &wanted);
    }
    std::fs::remove_dir_all(dir).ok();
}
