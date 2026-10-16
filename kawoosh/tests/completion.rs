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
    app.wait_for_open();
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
        // Written with the memory's flush, not at the key.
        let ws = app.moments.workspace().to_string();
        let rows = |app: &Kawoosh| app.store.as_ref().unwrap().completion_accepts(&ws);
        assert!(rows(&app).is_empty(), "nothing written at the key");
        app.flush_moments();
        assert_eq!(rows(&app).len(), 1, "one row, its score");
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

/// `path` opened in the pane and the pane back on `back`: an open
/// buffer that is not on show.
fn open_beside(d: &mut Drive, app: &mut Kawoosh, path: &std::path::Path, back: &std::path::Path) {
    ex(d, app, &format!("e {}", path.display()));
    ex(d, app, &format!("e {}", back.display()));
}

/// A landing between two keys — an index, an outline, an answer —
/// re-ranks under the user: the candidate `<C-n>` selected is still the
/// one `<Tab>` takes.
#[test]
fn the_selection_stays_through_a_landing() {
    let dir = tmp("landing");
    std::fs::write(dir.join("main.rs"), "helium helper\n").unwrap();
    std::fs::write(dir.join("far.txt"), "help_far\n").unwrap();
    let (mut d, mut app) = open(&dir.join("main.rs"));
    open_beside(&mut d, &mut app, &dir.join("far.txt"), &dir.join("main.rs"));
    let far = app.ed.buffer_at(&dir.join("far.txt")).unwrap();
    d.press(&mut app, "Go");
    d.keys(&mut app, "hel");
    d.press(&mut app, "<C-n>");
    assert_eq!(app.completion_ghost_text().as_deref(), Some("per"));
    // Another buffer's words land, as from their job.
    let snapshot = app.ed.buffers[far].snapshot();
    let index = kawoosh_systems::words::index(far, &snapshot, &[]);
    app.completion_words_indexed(index, (0, 0));
    assert_eq!(app.completion_ghost_text().as_deref(), Some("per"), "kept");
    d.press(&mut app, "<Tab>");
    assert!(text(&app).ends_with("\nhelper"), "{:?}", text(&app));
    std::fs::remove_dir_all(&dir).ok();
}

/// A session waits on its words: the index built on its job lands
/// after the key, and the session is ranked again with it. And the
/// servers told the buffer anew as the word is typed — their table
/// changed, as when the shell's PATH arrives at startup and they start
/// again — ends nothing: forgetting the buffer for the servers ended
/// the session under the keys (the load test's empty list).
#[test]
fn a_session_is_ranked_again_when_its_words_land() {
    let dir = tmp("land");
    std::fs::write(dir.join("main.rs"), "helium helper\n").unwrap();
    let mut app = Kawoosh::from_file(&dir.join("main.rs"));
    let mut d = Drive::new(900.0, 500.0);
    app.wait_for_open();
    d.press(&mut app, "Go");
    d.keys(&mut app, "hel");
    let serial = app.completion.session.as_ref().map(|s| s.serial);
    assert!(serial.is_some());
    // The words on their job, not landed yet; the servers' table moves.
    app.add_lsp_server(kawoosh_systems::lsp::ServerDef {
        language: "rust".into(),
        command: "kawoosh-another-server".into(),
        ..Default::default()
    });
    d.settle(&mut app);
    assert_eq!(
        app.completion.session.as_ref().map(|s| s.serial),
        serial,
        "the same session"
    );
    assert_eq!(app.completion.candidates(), ["helium", "helper"]);
    std::fs::remove_dir_all(&dir).ok();
}

/// A yank of a long text of letters past ASCII, then a word started:
/// the memory's signal cuts the text where a character starts.
#[test]
fn a_long_text_yanked_is_cut_on_a_character() {
    let dir = tmp("cut");
    std::fs::write(dir.join("main.rs"), format!("x{}\n", "я".repeat(40_000))).unwrap();
    let (mut d, mut app) = open(&dir.join("main.rs"));
    d.press(&mut app, "ggyG");
    d.press(&mut app, "Go");
    d.keys(&mut app, "a");
    assert!(app.completion.session.is_some());
    std::fs::remove_dir_all(&dir).ok();
}

/// A word inside a mask (secrets.md Decision 3) — a PEM key pasted in a
/// README, which is no private buffer — is never offered, from its
/// buffer or anywhere.
#[test]
fn a_masked_word_is_never_offered() {
    let dir = tmp("masked");
    std::fs::write(
        dir.join("README.md"),
        "the key:\n-----BEGIN PRIVATE KEY-----\nMIIEvSecretKeyMaterial\n-----END PRIVATE KEY-----\nMIIEvPublicWord\n",
    )
    .unwrap();
    std::fs::write(dir.join("main.rs"), "\n").unwrap();
    let (mut d, mut app) = open(&dir.join("main.rs"));
    open_beside(
        &mut d,
        &mut app,
        &dir.join("README.md"),
        &dir.join("main.rs"),
    );
    let readme = app.ed.buffer_at(&dir.join("README.md")).unwrap();
    assert!(!app.ed.buffers[readme].private);
    d.press(&mut app, "i");
    d.keys(&mut app, "MII");
    d.settle(&mut app);
    assert_eq!(app.completion.candidates(), ["MIIEvPublicWord"]);
    std::fs::remove_dir_all(&dir).ok();
}

/// Typing at several carets is one session: it follows the primary
/// caret's word, not ended and asked again at each key.
#[test]
fn several_carets_keep_one_session() {
    let dir = tmp("carets");
    std::fs::write(dir.join("main.rs"), "helium helper\n\n\n").unwrap();
    let (mut d, mut app) = open(&dir.join("main.rs"));
    d.press(&mut app, "j<C-j>i");
    assert_eq!(app.ed.views[app.focused_view().unwrap()].sels.len(), 2);
    d.keys(&mut app, "h");
    let serial = app.completion.session.as_ref().map(|s| s.serial);
    assert!(serial.is_some());
    let indexed = app.completion.indexed;
    d.keys(&mut app, "el");
    assert_eq!(app.completion.session.as_ref().map(|s| s.serial), serial);
    assert_eq!(app.completion.indexed, indexed, "nothing indexed again");
    assert_eq!(app.completion_ghost_text().as_deref(), Some("ium"));
    std::fs::remove_dir_all(&dir).ok();
}

/// The search signal moves with each search, the history at its cap
/// too.
#[test]
fn a_search_counts_with_the_history_full() {
    let dir = tmp("searched");
    std::fs::write(dir.join("main.rs"), "helium helper\n").unwrap();
    let (mut d, mut app) = open(&dir.join("main.rs"));
    app.ed.search_history = (0..200).map(|i| format!("old{i}")).collect();
    d.press(&mut app, "Go");
    d.keys(&mut app, "hel");
    assert_eq!(app.completion.candidates(), ["helium", "helper"]);
    d.press(&mut app, "<Esc>u");
    d.keys(&mut app, "/helper");
    d.press(&mut app, "<CR>");
    assert_eq!(app.ed.search_history.len(), 200, "at its cap");
    d.press(&mut app, "Go");
    d.keys(&mut app, "hel");
    assert_eq!(
        app.completion.candidates(),
        ["helper", "helium"],
        "searched lately"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A path's hidden entries are the answer's too: a word that starts
/// with `.` is offered them, after `./` asked with nothing typed.
#[test]
fn a_dot_brings_the_hidden_entries() {
    let dir = tmp("dot");
    std::fs::write(dir.join(".gitignore"), "").unwrap();
    std::fs::write(dir.join("main.rs"), "\n").unwrap();
    let (mut d, mut app) = open(&dir.join("main.rs"));
    d.press(&mut app, "i");
    d.keys(&mut app, "// ./");
    assert_eq!(app.completion.candidates(), ["main.rs"]);
    d.keys(&mut app, ".");
    assert_eq!(app.completion.candidates(), [".gitignore", "main.rs"]);
    assert_eq!(app.completion_ghost_text().as_deref(), Some("gitignore"));
    std::fs::remove_dir_all(&dir).ok();
}

/// A buffer made private is no source from then on: its words leave a
/// live session at once.
#[test]
fn a_buffer_made_private_leaves_the_session() {
    let dir = tmp("private");
    std::fs::write(dir.join("main.rs"), "\n").unwrap();
    std::fs::write(dir.join("notes.txt"), "helium_secret\n").unwrap();
    let (mut d, mut app) = open(&dir.join("main.rs"));
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.settle(&mut app);
    open_beside(
        &mut d,
        &mut app,
        &dir.join("notes.txt"),
        &dir.join("main.rs"),
    );
    d.press(&mut app, "i");
    d.keys(&mut app, "hel");
    assert_eq!(app.completion.candidates(), ["helium_secret"]);
    app.run_lua_source("t", r#"kawoosh.buf.set_private(true, "notes.txt")"#);
    d.frame(&mut app);
    let notes = app.ed.buffer_at(&dir.join("notes.txt")).unwrap();
    assert!(app.ed.buffers[notes].private);
    assert!(app.completion.session.is_some(), "the session goes on");
    assert!(
        app.completion.candidates().is_empty(),
        "{:?}",
        app.completion.candidates()
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// One index a buffer, whatever shortest word each language reads it
/// at: going between two languages with their own does not index every
/// buffer again.
#[test]
fn languages_with_their_own_shortest_share_an_index() {
    let dir = tmp("shortest");
    std::fs::write(dir.join("a.rs"), "heliumx hex\n").unwrap();
    std::fs::write(dir.join("b.py"), "helpful he\n").unwrap();
    std::fs::write(dir.join("c.txt"), "quiet words\n").unwrap();
    let (mut d, mut app) = open(&dir.join("a.rs"));
    open_beside(&mut d, &mut app, &dir.join("c.txt"), &dir.join("a.rs"));
    ex(
        &mut d,
        &mut app,
        "set language.python.completion.words.min_length=5",
    );
    let typed = |d: &mut Drive, app: &mut Kawoosh| {
        d.press(app, "Go");
        d.keys(app, "he");
        let c = app.completion.candidates();
        d.press(app, "<Esc>u");
        c
    };
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("b.py").display()),
    );
    assert_eq!(
        typed(&mut d, &mut app),
        ["helpful", "heliumx"],
        "five and more"
    );
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("a.rs").display()),
    );
    assert_eq!(
        typed(&mut d, &mut app),
        ["heliumx", "hex", "helpful"],
        "three and more"
    );
    let indexed = app.completion.indexed;
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("b.py").display()),
    );
    typed(&mut d, &mut app);
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("a.rs").display()),
    );
    typed(&mut d, &mut app);
    // Each word's start indexes the two buffers edited since; the one
    // untouched is read as it was, whichever shortest asks.
    assert_eq!(app.completion.indexed - indexed, 4);
    std::fs::remove_dir_all(&dir).ok();
}
