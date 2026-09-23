//! Secrets (docs/design/secrets.md): a file a mask rule names is
//! private and its values drawn as `•`; a yank from it is a secret in
//! the register — never on the clipboard, never in the store, put once
//! and forgotten; `zv` shows one mask for a while; nothing of the
//! buffer is a history row or a moment.

mod drive;

use std::time::Duration;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh_systems::store::MomentKey;
use kui::KeyMods;

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
    d.frame(app);
}

fn tmp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-secrets-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    kawoosh_systems::fs::canonicalize(&dir).unwrap()
}

/// Every text a frame drew.
fn drawn(d: &Drive) -> String {
    d.core
        .nodes()
        .iter()
        .filter_map(|n| n.text.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

fn launch(db: &std::path::Path) -> (Drive, Kawoosh) {
    let d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::new("*scratch*", "");
    app.open_store(Some(db));
    (d, app)
}

/// A `.env` opened is private: its values are `•` on the screen, the
/// keys as they are; a yank of a line is a secret — no clipboard, no
/// `text` row — put once and gone; an edit leaves no history row and
/// the file is no moment.
#[test]
fn a_private_file_keeps_nothing_and_draws_its_values_masked() {
    let dir = tmp("env");
    let db = dir.join("state.db");
    let env = dir.join(".env");
    std::fs::write(&env, "TOKEN=hunter22\n# a comment\nexport PORT=3000\n").unwrap();
    let (mut d, mut app) = launch(&db);
    // Writes as soon as it may: a row would be there by the frame after.
    app.histories.quiet = Duration::ZERO;
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("e {}", env.display()));
    let v = app.focused_view().unwrap();
    let id = app.ed.views[v].buffer;
    assert!(app.ed.buffers[id].private, "a rule names .env");
    let shown = drawn(&d);
    assert!(
        !shown.contains("hunter22"),
        "the value is not drawn: {shown}"
    );
    assert!(shown.contains("TOKEN="), "the key is: {shown}");
    assert!(shown.contains("••••••••"), "the value's stand-in: {shown}");
    assert!(shown.contains("# a comment"));
    assert!(!shown.contains("3000"), "an exported value too");

    // A yank: a secret, not on the clipboard.
    let before = app.ed.memory.len();
    d.keys(&mut app, "yy");
    d.frame(&mut app);
    let head = app.ed.memory.head().unwrap();
    assert!(head.secret);
    assert_eq!(head.text, "TOKEN=hunter22\n");
    assert_ne!(
        app.clipboard_last(),
        Some("TOKEN=hunter22\n"),
        "not on the clipboard"
    );
    // Put once, then forgotten.
    d.keys(&mut app, "p");
    d.frame(&mut app);
    assert_eq!(app.ed.memory.len(), before, "the secret went with its put");
    assert!(
        app.ed
            .buffer_of(v)
            .text()
            .starts_with("TOKEN=hunter22\nTOKEN=hunter22\n")
    );

    // An edit, flushed: no history row, no file moment, no text row.
    d.keys(&mut app, "x");
    d.frame(&mut app);
    d.frame(&mut app);
    app.sync_histories(true);
    app.flush_moments();
    let store = app.store.clone().unwrap();
    assert!(
        store
            .load_history(&kawoosh::history::file_key(&env))
            .is_none(),
        "no history row"
    );
    assert!(
        store
            .moment(&MomentKey::new("file", &env.display().to_string(), ""))
            .is_none(),
        "no file moment"
    );
    assert!(
        store
            .moment(&kawoosh::moments::text_key("TOKEN=hunter22\n"))
            .is_none(),
        "no text row"
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// `zv` shows the mask under the caret, that one alone; the caret
/// leaving it hides it again.
#[test]
fn zv_reveals_the_mask_under_the_caret_until_it_leaves() {
    let dir = tmp("reveal");
    let env = dir.join("app.env");
    std::fs::write(&env, "A=first-secret\nB=second-secret\n").unwrap();
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::from_file(&env);
    d.frame(&mut app);
    assert!(!drawn(&d).contains("first-secret"));
    // On the value, `zv`.
    d.keys(&mut app, "3l");
    d.keys(&mut app, "zv");
    d.frame(&mut app);
    let shown = drawn(&d);
    assert!(shown.contains("first-secret"), "revealed: {shown}");
    assert!(
        !shown.contains("second-secret"),
        "only the one under the caret"
    );
    // Away from it, hidden again.
    d.keys(&mut app, "j");
    d.frame(&mut app);
    d.frame(&mut app);
    assert!(!drawn(&d).contains("first-secret"));
    // Not on a mask: said so.
    d.keys(&mut app, "0zv");
    assert_eq!(app.ed.message, "no mask under the caret");
    std::fs::remove_dir_all(&dir).ok();
}

/// A secret nothing puts is forgotten after `secrets.forget_secs`; a
/// text put into a private buffer is a secret too, and goes with its
/// put.
#[test]
fn a_secret_is_forgotten_on_a_timer_and_a_put_into_a_private_buffer_is_one() {
    let dir = tmp("forget");
    let env = dir.join(".env");
    std::fs::write(&env, "KEY=value\n").unwrap();
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::from_file(&env);
    d.frame(&mut app);
    ex(&mut d, &mut app, "set secrets.forget_secs=1");
    d.keys(&mut app, "yy");
    assert!(app.ed.memory.head().is_some_and(|m| m.secret));
    std::thread::sleep(Duration::from_millis(1100));
    d.frame(&mut app);
    assert!(
        app.ed.memory.head().is_none_or(|m| !m.secret),
        "forgotten after a second"
    );

    // A yank from a plain scratch, put into the private file: gone.
    ex(&mut d, &mut app, "enew");
    d.keys(&mut app, "iplain");
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, "yiw");
    assert!(
        app.ed
            .memory
            .head()
            .is_some_and(|m| !m.secret && m.text == "plain")
    );
    ex(&mut d, &mut app, "b #");
    ex(&mut d, &mut app, &format!("e {}", env.display()));
    d.keys(&mut app, "p");
    assert!(
        app.ed.memory.head().is_none_or(|m| m.text != "plain"),
        "put into a private buffer, it went"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A rule is data: a user's `files` rule makes a file private and masks
/// it, and `false` switches a bundled one off.
#[test]
fn a_rule_is_a_setting() {
    let dir = tmp("rule");
    let conf = dir.join("app.conf");
    std::fs::write(&conf, "password = swordfish\nname = kawoosh\n").unwrap();
    let env = dir.join(".env");
    std::fs::write(&env, "K=v4lue\n").unwrap();
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::new("*scratch*", "");
    app.ed.settings.set(
        kawoosh_editor::settings::Layer::User,
        "secrets.masks.conf",
        kawoosh_editor::Setting::Table(
            [
                (
                    "files".to_string(),
                    kawoosh_editor::Setting::Str("*.conf".into()),
                ),
                (
                    "pattern".to_string(),
                    kawoosh_editor::Setting::Str(r"^password\s*=\s*(.+)$".into()),
                ),
            ]
            .into_iter()
            .collect(),
        ),
    );
    app.ed.settings.set(
        kawoosh_editor::settings::Layer::User,
        "secrets.masks.env",
        kawoosh_editor::Setting::Bool(false),
    );
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("e {}", conf.display()));
    assert!(app.ed.buffer_of(app.focused_view().unwrap()).private);
    let shown = drawn(&d);
    assert!(
        !shown.contains("swordfish") && shown.contains("kawoosh"),
        "{shown}"
    );
    ex(&mut d, &mut app, &format!("e {}", env.display()));
    assert!(!app.ed.buffer_of(app.focused_view().unwrap()).private);
    assert!(drawn(&d).contains("v4lue"), "the env rule is off");
    std::fs::remove_dir_all(&dir).ok();
}

/// Frames until `f` holds, for a process's answer to land.
fn until(d: &mut Drive, app: &mut Kawoosh, f: impl Fn(&Kawoosh) -> bool) -> bool {
    for _ in 0..400 {
        if f(app) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
        d.frame(app);
    }
    f(app)
}

/// An Ansible vault opens decrypted in a private scratch, its values
/// masked; `:w` encrypts it back over the file through stdin. The tool
/// here is a stand-in (`secrets.vault_command`) that strips and adds
/// the header, so the test needs no Ansible.
#[test]
fn a_vault_file_opens_decrypted_and_writes_back_encrypted() {
    let dir = tmp("vault");
    let tool = dir.join("fake-vault");
    std::fs::write(
        &tool,
        "#!/bin/sh\ncase \"$1\" in\n  view) tail -n +2 \"$2\" ;;\n  encrypt) { echo '$ANSIBLE_VAULT;1.1;FAKE'; cat; } > \"$3\" ;;\nesac\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    let vault = dir.join("vault.yml");
    std::fs::write(&vault, "$ANSIBLE_VAULT;1.1;FAKE\ndb_password: s3cret\n").unwrap();

    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::new("*scratch*", "");
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    d.frame(&mut app);
    ex(
        &mut d,
        &mut app,
        &format!("set secrets.vault_command={}", tool.display()),
    );
    ex(&mut d, &mut app, &format!("e {}", vault.display()));
    let name = format!("vault: {}", vault.display());
    assert!(
        until(&mut d, &mut app, |a| a.focused_view().is_some_and(|v| a
            .ed
            .buffer_of(v)
            .text()
            .contains("db_password"))),
        "decrypted: {}",
        app.ed.message
    );
    let v = app.focused_view().unwrap();
    let buf = app.ed.buffer_of(v);
    assert_eq!(buf.name, name);
    assert!(buf.private);
    d.frame(&mut app);
    let shown = drawn(&d);
    assert!(
        !shown.contains("s3cret") && shown.contains("db_password:"),
        "{shown}"
    );

    // An edit, written back: encrypted over the file, the buffer clean.
    d.keys(&mut app, "A!");
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "w");
    assert!(
        until(&mut d, &mut app, |_| std::fs::read_to_string(&vault)
            .unwrap()
            == "$ANSIBLE_VAULT;1.1;FAKE\ndb_password: s3cret!\n"),
        "{:?}",
        std::fs::read_to_string(&vault)
    );
    assert!(until(&mut d, &mut app, |a| !a
        .ed
        .buffer_of(a.focused_view().unwrap())
        .modified));
    std::fs::remove_dir_all(&dir).ok();
}
