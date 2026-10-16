//! Completion (docs/design/completion.md): when it asks, how it shows,
//! what it offers.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh_editor::{Layer, Setting};

/// What a keystroke and a frame cost while a completion is live, over
/// a buffer of twenty thousand lines of words — the numbers in the
/// commit messages of completion.md's round 1. Run by hand:
/// `cargo test -p kawoosh --test completion timing -- --ignored --nocapture`.
#[test]
#[ignore]
fn timing_a_keystroke_and_a_frame() {
    let dir =
        std::env::temp_dir().join(format!("kawoosh-completion-timing-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("words.txt");
    let text: String = (0..20_000)
        .map(|i| {
            format!(
                "alpha_{i} beta_{} gamma_{} delta_{i}_epsilon\n",
                i % 97,
                i % 31
            )
        })
        .collect();
    std::fs::write(&file, text).unwrap();
    // The words indexed on their job, as in the window: a word's start
    // ranks the index as it stands, the fresh one lands after.
    let mut app = Kawoosh::from_file(&file);
    // Asked as a word starts, as in code (plain text asks when asked).
    app.ed.settings.set(
        Layer::Session,
        "completion.trigger",
        Setting::Str("typing".into()),
    );
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    d.press(&mut app, "G");
    d.press(&mut app, "o");
    let mut keys = Vec::new();
    let mut word_starts = Vec::new();
    for round in 0..40 {
        for (i, ch) in [" ", "g", "a", "m", "m"].iter().enumerate() {
            let t = std::time::Instant::now();
            d.keys(&mut app, ch);
            let took = t.elapsed();
            d.settle(&mut app);
            if i == 1 {
                word_starts.push(took);
            } else if round > 0 {
                keys.push(took);
            }
        }
    }
    // The nearest `gamma_…`: the last line's.
    assert_eq!(
        app.completion_ghost_text().as_deref(),
        Some("a_4"),
        "a ghost on show"
    );
    let mut frames = Vec::new();
    for _ in 0..200 {
        let t = std::time::Instant::now();
        d.frame(&mut app);
        frames.push(t.elapsed());
    }
    let med = |v: &mut Vec<std::time::Duration>| {
        v.sort();
        (v[v.len() / 2], v[v.len() * 9 / 10])
    };
    let (k, k90) = med(&mut keys);
    let (w, w90) = med(&mut word_starts);
    let (f, f90) = med(&mut frames);
    println!(
        "keystroke {k:?} (p90 {k90:?}); word start {w:?} (p90 {w90:?}); frame {f:?} (p90 {f90:?})"
    );
    std::fs::remove_dir_all(&dir).ok();
}

fn tmp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-completion-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    kawoosh_systems::fs::canonicalize(&dir).unwrap()
}

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.press(app, "<Esc>");
    d.keys(app, ":");
    d.keys(app, line);
    d.press(app, "<CR>");
    app.wait_for_open();
    d.settle(app);
}

/// `file` open, its jobs answered on the spot.
fn open(file: &std::path::Path) -> (Drive, Kawoosh) {
    let mut app = Kawoosh::from_file(file);
    app.jobs_inline = true;
    let mut d = Drive::new(900.0, 500.0);
    d.settle(&mut app);
    (d, app)
}

fn text(app: &Kawoosh) -> String {
    app.ed.buffer_of(app.focused_view().unwrap()).text()
}

/// `completion.trigger` (Decision 1): code asks as a word starts, prose
/// only when asked; a language's own and a pane's own say otherwise;
/// `endline_only` asks nowhere but at a line's end; `off` not even when
/// asked.
#[test]
fn the_trigger_is_a_languages_and_a_panes() {
    let dir = tmp("trigger");
    // A word, not a symbol: symbols rank over words wherever they are.
    std::fs::write(dir.join("main.rs"), "fn main() {\n    // helper_value\n}\n").unwrap();
    std::fs::write(dir.join("notes.md"), "helper words here\n").unwrap();
    let (mut d, mut app) = open(&dir.join("main.rs"));
    d.press(&mut app, "Go");
    d.keys(&mut app, "hel");
    assert_eq!(
        app.completion_ghost_text().as_deref(),
        Some("per_value"),
        "code asks"
    );

    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("notes.md").display()),
    );
    d.press(&mut app, "Go");
    d.keys(&mut app, "hel");
    assert!(app.completion.session.is_none(), "markdown asks when asked");
    d.press(&mut app, "<C-Space>");
    assert_eq!(app.completion_ghost_text().as_deref(), Some("per"));
    // The language's own, said for the session.
    ex(
        &mut d,
        &mut app,
        "set language.markdown.completion.trigger=typing",
    );
    d.press(&mut app, "o");
    d.keys(&mut app, "hel");
    assert_eq!(
        app.completion_ghost_text().as_deref(),
        Some("per"),
        "markdown types now"
    );

    // One pane's own: off, and `<C-Space>` says so.
    ex(&mut d, &mut app, "setlocal completion.trigger=off");
    d.press(&mut app, "o");
    d.keys(&mut app, "hel");
    d.press(&mut app, "<C-Space>");
    assert!(app.completion.session.is_none());
    assert_eq!(
        app.ed.message,
        "completion is off here (completion.trigger)"
    );

    // `endline_only`: a word typed before text on its line asks nothing.
    ex(&mut d, &mut app, "setlocal completion.trigger=endline_only");
    d.press(&mut app, "o");
    d.keys(&mut app, "words");
    d.press(&mut app, "<Home>");
    d.keys(&mut app, "x");
    assert!(app.completion.session.is_none(), "mid-line");
    d.press(&mut app, "<End>");
    d.keys(&mut app, " hel");
    assert_eq!(
        app.completion_ghost_text().as_deref(),
        Some("per"),
        "at the end"
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// `completion.ghost` (Decision 2): `endline` draws nothing mid-line —
/// the session lives, its keys do not, and `<Tab>` is a tab — `always`
/// draws it anywhere, `never` nowhere.
#[test]
fn the_ghost_is_drawn_where_completion_ghost_says() {
    let dir = tmp("ghost");
    std::fs::write(dir.join("main.rs"), "let helper_value = 1;\nlet y = ();\n").unwrap();
    let (mut d, mut app) = open(&dir.join("main.rs"));
    d.press(&mut app, "jf(a");
    d.keys(&mut app, "hel");
    assert!(app.completion.session.is_some(), "asked");
    assert_eq!(app.completion_ghost_text(), None, "mid-line: no ghost");
    let v = app.focused_view().unwrap();
    assert!(!app.ed.holds(Some(v), "completion"), "nothing on show");
    d.press(&mut app, "<Tab>");
    assert!(text(&app).contains("(hel    )"), "a tab: {:?}", text(&app));

    ex(&mut d, &mut app, "setlocal completion.ghost=always");
    d.press(&mut app, "S");
    d.keys(&mut app, "x = ()");
    d.press(&mut app, "<Left>");
    d.keys(&mut app, "hel");
    // Mid-line, drawn.
    assert_eq!(app.completion_ghost_text().as_deref(), Some("per_value"));
    assert!(
        d.row_extras().iter().any(|t| t == "per_value"),
        "{:?}",
        d.row_extras()
    );
    d.press(&mut app, "<Right>");
    assert!(app.completion.session.is_none(), "the caret left the word");
    d.press(&mut app, "<Left>");
    d.keys(&mut app, "p");
    assert_eq!(
        app.completion_ghost_text().as_deref(),
        Some("er_value"),
        "asked again"
    );
    d.press(&mut app, "<Tab>");
    assert!(
        text(&app).contains("x = (helper_value)"),
        "{:?}",
        text(&app)
    );

    ex(&mut d, &mut app, "setlocal completion.ghost=never");
    d.press(&mut app, "o");
    d.keys(&mut app, "hel");
    assert!(app.completion.session.is_some());
    assert_eq!(app.completion_ghost_text(), None, "never");
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// The keys are the keymap's (Decision 3): local to the `completion`
/// fact, so they hold only while one is on show, rebound and unbound
/// with `kawoosh.map`; `<CR>` with a ghost on show is a newline.
#[test]
fn the_keys_are_local_maps_and_enter_is_a_newline() {
    let dir = tmp("keys");
    std::fs::write(dir.join("main.rs"), "helium helper\n").unwrap();
    let (mut d, mut app) = open(&dir.join("main.rs"));
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.settle(&mut app);
    // With nothing on show, the keys are insert mode's own.
    d.press(&mut app, "Go");
    d.press(&mut app, "<C-n><C-e>");
    assert_eq!(
        text(&app),
        "helium helper\n\n",
        "nothing taken, nothing typed"
    );

    d.keys(&mut app, "hel");
    assert_eq!(app.completion_ghost_text().as_deref(), Some("ium"));
    let v = app.focused_view().unwrap();
    assert!(app.ed.holds(Some(v), "completion"));
    d.press(&mut app, "<C-n>");
    assert_eq!(app.completion_ghost_text().as_deref(), Some("per"));
    d.press(&mut app, "<C-p>");
    assert_eq!(app.completion_ghost_text().as_deref(), Some("ium"));
    // `<CR>` is a newline, the ghost left behind.
    d.press(&mut app, "<CR>");
    assert_eq!(text(&app), "helium helper\n\nhel\n");
    assert!(app.completion.session.is_none());

    // Rebound: `<C-l>` accepts, `<Tab>` no longer does.
    app.run_lua_source(
        "t",
        r#"kawoosh.map("i", "<C-l>", "completion accept", { scope = "completion" })
           kawoosh.unmap("i", "<Tab>", { scope = "completion" })"#,
    );
    d.keys(&mut app, "hel");
    assert_eq!(app.completion_ghost_text().as_deref(), Some("ium"));
    d.press(&mut app, "<Tab>");
    assert!(
        text(&app).ends_with("\nhel "),
        "to the next stop: {:?}",
        text(&app)
    );
    d.press(&mut app, "<BS>");
    d.press(&mut app, "<C-Space>");
    assert_eq!(app.completion_ghost_text().as_deref(), Some("ium"));
    d.press(&mut app, "<C-l>");
    assert!(text(&app).ends_with("\nhelium"), "{:?}", text(&app));
    // `<C-e>` closes, and typing on in the word asks nothing again.
    d.keys(&mut app, " hel");
    assert!(app.completion_ghost_text().is_some());
    d.press(&mut app, "<C-e>");
    d.keys(&mut app, "i");
    assert_eq!(app.completion_ghost_text(), None, "closed for the word");
    d.keys(&mut app, " hel");
    assert_eq!(
        app.completion_ghost_text().as_deref(),
        Some("ium"),
        "a new word asks"
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// Every source asked, merged and ranked (Decisions 4 and 5): the
/// grammar's symbols before the words, one candidate per text, the
/// same buffer's words before another's; a private buffer's words are
/// never offered.
#[test]
fn sources_are_merged_and_a_private_buffer_is_none() {
    let dir = tmp("sources");
    std::fs::write(
        dir.join("main.rs"),
        "fn helper_fn() {}\n// help_near\nfn main() {\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("far.txt"), "helium_far\n").unwrap();
    std::fs::write(dir.join(".env"), "HELLO_SECRET=hunter2\n").unwrap();
    let (mut d, mut app) = open(&dir.join("far.txt"));
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join(".env").display()),
    );
    assert!(app.ed.buffer_of(app.focused_view().unwrap()).private);
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("main.rs").display()),
    );
    d.settle(&mut app);
    d.press(&mut app, "Go");
    d.keys(&mut app, "hel");
    d.settle(&mut app);
    assert_eq!(
        app.completion.candidates_from(),
        [
            ("helper_fn".to_string(), "buffer_symbols"),
            ("help_near".to_string(), "buffer_words"),
            ("helium_far".to_string(), "buffer_words"),
        ]
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// `path` (Decision 4): after `./` the entries of the file's directory,
/// directories first with a `/` that asks again, a hidden one left out.
#[test]
fn a_path_completes_from_the_files_directory() {
    let dir = tmp("path");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/lib.rs"), "").unwrap();
    std::fs::write(dir.join("readme.md"), "").unwrap();
    std::fs::write(dir.join(".hidden"), "").unwrap();
    std::fs::write(dir.join("main.rs"), "\n").unwrap();
    let (mut d, mut app) = open(&dir.join("main.rs"));
    d.press(&mut app, "i");
    d.keys(&mut app, "// see ./");
    assert_eq!(
        app.completion.candidates(),
        ["src/", "main.rs", "readme.md"]
    );
    assert_eq!(app.completion_ghost_text().as_deref(), Some("src/"));
    d.press(&mut app, "<Tab>");
    assert_eq!(
        app.completion.candidates(),
        ["lib.rs"],
        "the directory asks again"
    );
    d.keys(&mut app, "l");
    assert_eq!(app.completion_ghost_text().as_deref(), Some("ib.rs"));
    d.press(&mut app, "<Tab>");
    assert_eq!(text(&app), "// see ./src/lib.rs\n");
    // A word with a slash in it is no path.
    d.keys(&mut app, " a/");
    assert!(app.completion.session.is_none());
    std::fs::remove_dir_all(&dir).ok();
}

/// Frecency (Decision 5): a candidate accepted ranks over its equals
/// next time, and in the next launch — the state db keeps it.
#[test]
fn an_accepted_candidate_ranks_higher_next_time() {
    let dir = tmp("frecency");
    let db = dir.join("state.db");
    std::fs::write(dir.join("main.rs"), "helium helper\n").unwrap();
    {
        let (mut d, mut app) = open(&dir.join("main.rs"));
        app.open_store(Some(&db));
        d.press(&mut app, "Go");
        d.keys(&mut app, "hel");
        assert_eq!(
            app.completion.candidates(),
            ["helium", "helper"],
            "the text's order"
        );
        d.press(&mut app, "<C-n><Tab>");
        d.press(&mut app, "<CR>");
        d.keys(&mut app, "hel");
        assert_eq!(
            app.completion.candidates(),
            ["helper", "helium"],
            "accepted before"
        );
        assert_eq!(app.completion_ghost_text().as_deref(), Some("per"));
    }
    let (mut d, mut app) = open(&dir.join("main.rs"));
    app.open_store(Some(&db));
    d.press(&mut app, "Go");
    d.keys(&mut app, "hel");
    assert_eq!(app.completion.candidates()[0], "helper", "remembered");
    std::fs::remove_dir_all(&dir).ok();
}

/// An answer for an anchor that is gone is dropped as it lands
/// (Decision 6): a listing for a session that ended, or for an earlier
/// one, changes nothing.
#[test]
fn a_stale_answer_is_dropped() {
    let dir = tmp("stale");
    std::fs::write(dir.join("readme.md"), "").unwrap();
    std::fs::write(dir.join("main.rs"), "\n").unwrap();
    let mut app = Kawoosh::from_file(&dir.join("main.rs"));
    let mut d = Drive::new(900.0, 500.0);
    d.settle(&mut app);
    d.press(&mut app, "i");
    // Asked on its job; the word left before it lands.
    d.keys(&mut app, "// ./");
    d.keys(&mut app, " ");
    d.settle(&mut app);
    assert!(app.completion.session.is_none());
    // A listing for an earlier session than the live one.
    d.keys(&mut app, "./");
    d.settle(&mut app);
    assert_eq!(app.completion.candidates(), ["main.rs", "readme.md"]);
    let serial = app.completion.session.as_ref().unwrap().serial;
    let entries = kawoosh_systems::fs::list(&dir.join("..")).ok();
    app.completion_listed(serial - 1, dir.clone(), entries);
    assert_eq!(
        app.completion.candidates(),
        ["main.rs", "readme.md"],
        "unchanged"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The memory's signals (Decision 5): a word yanked lately ranks over
/// its equals.
#[test]
fn a_word_yanked_lately_ranks_higher() {
    let dir = tmp("signals");
    std::fs::write(dir.join("main.rs"), "helium helper\n").unwrap();
    let (mut d, mut app) = open(&dir.join("main.rs"));
    d.press(&mut app, "wyiw");
    d.press(&mut app, "Go");
    d.keys(&mut app, "hel");
    assert_eq!(app.completion.candidates(), ["helper", "helium"]);
    std::fs::remove_dir_all(&dir).ok();
}
