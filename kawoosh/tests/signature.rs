//! Completion's round 5 (docs/design/completion.md Decision 10's second
//! half): signature help — `completion.signature` manual, auto and off,
//! `<C-g>` toggling it, the lit parameter following `,`, overloads, the
//! ends, one panel, above the line with no popup, a late answer dropped,
//! masked text never drawn — and `completion.callable`, name against
//! args.

mod drive;

use drive::{Drive, fake_lsp};
use kawoosh::Kawoosh;
use kawoosh_systems::lsp::ServerDef;

fn tmp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-signature-{tag}-{}", std::process::id()));
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

fn server(extra: &[&str]) -> ServerDef {
    let mut s = fake_lsp("rust");
    s.args.push("--signature".into());
    s.args.extend(extra.iter().map(|a| a.to_string()));
    ServerDef {
        roots: vec!["Cargo.toml".into()],
        ..s
    }
}

/// A crate holding `src/main.rs` with `text`, open with the fake server
/// saying signatures, and the README beside it with a private key in
/// it when `secret` (its word masked). Completion asks only when asked,
/// unless a test says otherwise, so a signature is all that shows.
fn open(
    tag: &str,
    text: &str,
    extra: &[&str],
    secret: bool,
) -> (std::path::PathBuf, Drive, Kawoosh) {
    let dir = tmp(tag);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let file = dir.join("src/main.rs");
    std::fs::write(&file, text).unwrap();
    let mut app = Kawoosh::from_file(&file);
    app.jobs_inline = true;
    app.add_lsp_server(server(extra));
    let mut d = Drive::new(1000.0, 500.0);
    app.wait_for_open();
    d.settle(&mut app);
    if secret {
        let readme = dir.join("README.md");
        std::fs::write(
            &readme,
            "-----BEGIN PRIVATE KEY-----\nMIIEvSecretKeyMaterial\n-----END PRIVATE KEY-----\n",
        )
        .unwrap();
        ex(&mut d, &mut app, &format!("e {}", readme.display()));
        ex(&mut d, &mut app, &format!("e {}", file.display()));
    }
    ex(&mut d, &mut app, "setlocal completion.trigger=manual");
    (dir, d, app)
}

fn text(app: &Kawoosh) -> String {
    app.ed.buffer_of(app.focused_view().unwrap()).text()
}

fn selected(app: &Kawoosh) -> Vec<String> {
    let v = app.focused_view().unwrap();
    let sels = &app.ed.views[v].sels;
    let buf = app.ed.buffer_of(v);
    sels.iter().map(|s| buf.slice(s.range())).collect()
}

fn holds(app: &Kawoosh, fact: &str) -> bool {
    app.ed.holds(app.focused_view(), fact)
}

/// A file whose call goes on line 10, room over it for the panel.
const TALL: &str = "//\n//\n//\n//\n//\n//\n//\n//\nfn main() {\n    \n}\n";

/// Frames until `pred` holds, the server answering between them: a
/// bound on waiting for a process, not a timing anything depends on.
fn until(d: &mut Drive, app: &mut Kawoosh, what: &str, mut pred: impl FnMut(&Kawoosh) -> bool) {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        d.frame(app);
        if pred(app) {
            return;
        }
        assert!(
            std::time::Instant::now() < end,
            "waited for {what}: {:?}, asking {:?}",
            app.signature_text_shown(),
            app.signature_asking()
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Frames until no ask is in flight: whatever the server said landed.
fn answered(d: &mut Drive, app: &mut Kawoosh) {
    until(d, app, "the server's answer", |a| {
        a.signature_asking().is_none()
    });
    d.settle(app);
}

/// Whether the server's candidate putting `text` is in: the buffer's
/// own word of the same text may be there before it.
fn from_server(app: &Kawoosh, text: &str) -> bool {
    app.completion
        .candidates_from()
        .iter()
        .any(|(t, src)| t == text && *src == "lsp")
}

fn head(app: &Kawoosh) -> Option<String> {
    app.signature_text_shown().map(|s| s.head)
}

fn lit(app: &Kawoosh) -> Option<String> {
    app.signature_text_shown().and_then(|s| s.lit)
}

/// `manual`: `(` asks nothing; `<C-g>` asks, shows the signature with
/// the parameter the caret is in lit and the documentation's first
/// paragraphs, follows `,`, and puts it away again; `<C-e>` closes it,
/// as do `)` leaving the call and leaving insert mode.
#[test]
fn ctrl_g_shows_the_signature_and_it_follows_the_arguments() {
    let (dir, mut d, mut app) = open("manual", "fn main() {\n    \n}\n", &[], false);
    d.press(&mut app, "jA");
    d.keys(&mut app, "pair(");
    d.settle(&mut app);
    assert_eq!(app.signature.asks, 0, "manual: `(` asks nothing");
    assert!(app.signature_text_shown().is_none());
    d.press(&mut app, "<C-g>");
    until(&mut d, &mut app, "the signature", |a| head(a).is_some());
    let kawoosh::signature::Shown {
        head: h,
        lit: l,
        of,
        body,
    } = app.signature_text_shown().unwrap();
    assert_eq!(h, "fn pair(a: i32, b: &str) -> bool");
    assert_eq!(l.as_deref(), Some("a: i32"));
    assert_eq!(of, None, "one signature: no count");
    assert_eq!(body, "the first\n\nPairs two things.", "first paragraphs");
    d.settle(&mut app);
    assert!(holds(&app, "signature"), "its keys hold while it is drawn");
    let drawn = d.texts_under("completion docs").join("");
    assert!(
        drawn.contains("a: i32") && drawn.contains("Pairs two things."),
        "{drawn:?}"
    );
    // `,` moves the lit parameter on.
    d.keys(&mut app, "1, ");
    until(&mut d, &mut app, "the second parameter", |a| {
        lit(a).as_deref() == Some("b: &str")
    });
    assert_eq!(
        app.signature_text_shown().unwrap().body,
        "the second\n\nPairs two things."
    );
    // Back over the comma: the first again.
    d.press(&mut app, "<BS><BS>");
    until(&mut d, &mut app, "the first again", |a| {
        lit(a).as_deref() == Some("a: i32")
    });
    // `<C-g>` again puts it away, and brings it back.
    d.press(&mut app, "<C-g>");
    d.settle(&mut app);
    assert!(app.signature.live.is_none());
    d.press(&mut app, "<C-g>");
    until(&mut d, &mut app, "the signature again", |a| {
        head(a).is_some()
    });
    d.settle(&mut app);
    d.press(&mut app, "<C-e>");
    assert!(app.signature.live.is_none(), "<C-e> closes it");
    // `)` leaves the call.
    d.press(&mut app, "<C-g>");
    until(&mut d, &mut app, "the signature again", |a| {
        head(a).is_some()
    });
    d.keys(&mut app, ")");
    assert!(
        app.signature.live.is_none(),
        "the caret left the parentheses"
    );
    // As does an arrow out of it, and leaving insert mode.
    d.press(&mut app, "<Left><C-g>");
    until(&mut d, &mut app, "the signature again", |a| {
        head(a).is_some()
    });
    d.press(&mut app, "<Right>");
    assert!(app.signature.live.is_none(), "moved past `)`");
    d.press(&mut app, "<Left><C-g>");
    until(&mut d, &mut app, "the signature again", |a| {
        head(a).is_some()
    });
    d.press(&mut app, "<Esc>");
    assert!(app.signature.live.is_none(), "normal mode");
    // Outside any call: said, nothing asked.
    let asks = app.signature.asks;
    d.press(&mut app, "o");
    d.press(&mut app, "<C-g>");
    assert_eq!(app.signature.asks, asks);
    assert_eq!(app.ed.message, "not inside a call");
    // A call the server knows no signature of: said, and gone.
    d.keys(&mut app, "nobody(");
    d.press(&mut app, "<C-g>");
    answered(&mut d, &mut app);
    assert!(app.signature.live.is_none());
    assert_eq!(app.ed.message, "no signature here");
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// `auto` opens on the server's trigger characters typed — `(` and `,`
/// — and `off` never shows one, `<C-g>` saying so.
#[test]
fn auto_opens_on_its_triggers_and_off_never() {
    let (dir, mut d, mut app) = open("auto", "fn main() {\n    \n}\n", &[], false);
    ex(&mut d, &mut app, "setlocal completion.signature=auto");
    d.press(&mut app, "jA");
    d.keys(&mut app, "pair(");
    until(&mut d, &mut app, "the signature on `(`", |a| {
        head(a).is_some()
    });
    assert_eq!(lit(&app).as_deref(), Some("a: i32"));
    d.keys(&mut app, "1");
    d.press(&mut app, "<C-e>");
    assert!(app.signature.live.is_none());
    d.keys(&mut app, ",");
    until(&mut d, &mut app, "the signature on `,`", |a| {
        lit(a).as_deref() == Some("b: &str")
    });
    d.keys(&mut app, " 2)");
    assert!(app.signature.live.is_none());
    // A letter is no trigger.
    let asks = app.signature.asks;
    d.keys(&mut app, " x");
    assert_eq!(app.signature.asks, asks);

    ex(&mut d, &mut app, "setlocal completion.signature=off");
    d.press(&mut app, "o");
    d.keys(&mut app, "pair(");
    d.settle(&mut app);
    assert_eq!(app.signature.asks, asks, "off: nothing asked");
    d.press(&mut app, "<C-g>");
    assert_eq!(app.signature.asks, asks);
    assert!(app.signature.live.is_none());
    assert!(app.ed.message.contains("off"), "{}", app.ed.message);
    std::fs::remove_dir_all(&dir).ok();
}

/// Overloads: which of how many before the head, `<C-n>` / `<C-p>` to
/// walk them while no completion is on show, the one chosen kept as the
/// arguments move; a parameter placed by its UTF-16 offsets.
#[test]
fn overloads_are_walked_and_kept() {
    let (dir, mut d, mut app) = open("overloads", "fn main() {\n    \n}\n", &[], false);
    d.press(&mut app, "jA");
    d.keys(&mut app, "over(");
    d.press(&mut app, "<C-g>");
    until(&mut d, &mut app, "the signature", |a| head(a).is_some());
    let kawoosh::signature::Shown {
        head: h,
        lit: l,
        of,
        ..
    } = app.signature_text_shown().unwrap();
    assert_eq!(h, "over(é: u8)");
    assert_eq!(l.as_deref(), Some("é: u8"), "UTF-16 offsets read");
    assert_eq!(of, Some((1, 2)));
    d.settle(&mut app);
    assert!(d.texts_under("completion docs").join("").contains("1/2"));
    d.press(&mut app, "<C-n>");
    let kawoosh::signature::Shown {
        head: h,
        lit: l,
        of,
        ..
    } = app.signature_text_shown().unwrap();
    assert_eq!(h, "over(x: u8, y: u8)");
    assert_eq!(l.as_deref(), Some("x: u8"));
    assert_eq!(of, Some((2, 2)));
    assert_eq!(
        text(&app).lines().nth(1),
        Some("    over("),
        "<C-n> typed nothing"
    );
    // Asked again as `,` is typed: the one chosen stays.
    d.keys(&mut app, "1,");
    until(&mut d, &mut app, "the second of the second", |a| {
        lit(a).as_deref() == Some("y: u8")
    });
    assert_eq!(app.signature_text_shown().unwrap().of, Some((2, 2)));
    d.press(&mut app, "<C-p>");
    assert_eq!(app.signature_text_shown().unwrap().of, Some((1, 2)));
    d.press(&mut app, "<C-p>");
    assert_eq!(
        app.signature_text_shown().unwrap().of,
        Some((2, 2)),
        "round"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The overload chosen is the call's: going into another call of
/// several overloads shows the one its server says, not the index
/// chosen outside, and sends no other call's answer back with the ask;
/// coming back out, the outer call's choice is there again.
#[test]
fn the_chosen_overload_is_the_calls_own() {
    let (dir, mut d, mut app) = open("overload-calls", "fn main() {\n    \n}\n", &[], false);
    d.press(&mut app, "jA");
    d.keys(&mut app, "over(");
    d.press(&mut app, "<C-g>");
    until(&mut d, &mut app, "over's", |a| {
        head(a).is_some_and(|h| h.starts_with("over("))
    });
    d.settle(&mut app);
    d.press(&mut app, "<C-n>");
    assert_eq!(app.signature_text_shown().unwrap().of, Some((2, 2)));
    d.keys(&mut app, "max(");
    until(&mut d, &mut app, "max's", |a| {
        head(a).is_some_and(|h| h.starts_with("max("))
    });
    let shown = app.signature_text_shown().unwrap();
    assert_eq!(shown.of, Some((1, 2)), "the server's, not over's choice");
    assert_eq!(shown.head, "max(a: i32, b: i32)");
    d.keys(&mut app, ")");
    until(&mut d, &mut app, "over's again", |a| {
        head(a).is_some_and(|h| h.starts_with("over("))
    });
    assert_eq!(
        app.signature_text_shown().unwrap().of,
        Some((2, 2)),
        "over's own choice back"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A context is built for an ask that goes out only: a key while one is
/// in flight copies nothing of the answer on show.
#[test]
fn a_key_while_one_is_asked_builds_nothing() {
    let (dir, mut d, mut app) = open(
        "contexts",
        "fn main() {\n    \n}\n",
        &["--signature-never"],
        false,
    );
    d.press(&mut app, "jA");
    d.keys(&mut app, "pair(");
    d.press(&mut app, "<C-g>");
    let (b, off, v) = app.signature_asking().unwrap();
    app.signature_answered(b, v, off, Ok(Some(pair_help(Some(0)))));
    d.settle(&mut app);
    assert!(head(&app).is_some());
    // The next key asks, building one; the keys after it, while that is
    // in flight, build none.
    let built = app.signature.contexts;
    d.keys(&mut app, "1");
    assert_eq!(app.signature.contexts, built + 1);
    d.keys(&mut app, "23, 4");
    d.settle(&mut app);
    assert_eq!(app.signature.contexts, built + 1, "nothing built in flight");
    std::fs::remove_dir_all(&dir).ok();
}

/// LSP 3.17: an answer that says no `activeParameter` lights the first
/// parameter, as one past the parameters does.
#[test]
fn an_omitted_active_parameter_lights_the_first() {
    let (dir, mut d, mut app) = open(
        "no-active",
        "fn main() {\n    \n}\n",
        &["--signature-never"],
        false,
    );
    d.press(&mut app, "jA");
    d.keys(&mut app, "pair(");
    d.press(&mut app, "<C-g>");
    let (b, off, v) = app.signature_asking().unwrap();
    app.signature_answered(b, v, off, Ok(Some(pair_help(None))));
    d.settle(&mut app);
    assert_eq!(lit(&app).as_deref(), Some("a: i32"));
    d.keys(&mut app, "1");
    let (b, off, v) = app.signature_asking().unwrap();
    app.signature_answered(b, v, off, Ok(Some(pair_help(Some(9)))));
    d.settle(&mut app);
    assert_eq!(lit(&app).as_deref(), Some("a: i32"), "past the parameters");
    std::fs::remove_dir_all(&dir).ok();
}

/// With several carets, a function taken by a match that does not
/// continue the word goes in at the primary caret alone, and is filled
/// there — its fields, or the caret after its brackets.
#[test]
fn a_fuzzy_call_at_several_carets_is_filled_where_it_went() {
    let (dir, mut d, mut app) = open("carets", "fn main() {\n    \n    \n}\n", &[], false);
    ex(&mut d, &mut app, "setlocal completion.callable=args");
    ex(&mut d, &mut app, "setlocal completion.trigger=typing");
    for (typed, want, line) in [
        ("pr", "pair", "    pair(a, b)"),
        ("ne", "none", "    none()"),
    ] {
        d.press(&mut app, "<Esc>");
        d.press(&mut app, "ggj\"_cc");
        d.press(&mut app, "<Esc>j\"_cc<Esc>k");
        d.press(&mut app, "<C-j>A");
        assert_eq!(app.ed.views[app.focused_view().unwrap()].sels.len(), 2);
        d.keys(&mut app, typed);
        until(&mut d, &mut app, want, |a| from_server(a, want));
        let mut n = 0;
        while app.completion.selected_text() != Some(want) {
            d.press(&mut app, "<C-n>");
            n += 1;
            assert!(n < 10, "{want}: {:?}", app.completion.candidates());
        }
        d.press(&mut app, "<Tab>");
        until(&mut d, &mut app, "filled", |a| {
            a.signature_asking().is_none() && (a.snippet.is_some() || want == "none")
        });
        d.settle(&mut app);
        let t = text(&app);
        let v = app.focused_view().unwrap();
        let primary = app.ed.views[v].sels.primary().head;
        let ln = app.ed.buffer_of(v).line_of(primary);
        assert_eq!(t.lines().nth(ln), Some(line), "{t}");
        if want == "pair" {
            assert_eq!(selected(&app)[0], "a", "{:?}", selected(&app));
            d.press(&mut app, "<Esc>");
            app.snippet = None;
        } else {
            assert!(
                t[..primary].ends_with("none()"),
                "the caret after: {t:?} at {primary}"
            );
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// A panel alone is placed by its own height: one too tall for the room
/// over the caret's row goes under it, and a short one after it, which
/// the room holds, over it — not placed by the one before.
#[test]
fn a_panel_alone_is_placed_by_its_own_height() {
    let (dir, mut d, mut app) = open("heights", "//\n//\nfn main() {\n    \n}\n", &[], false);
    d.press(&mut app, "3GA");
    d.keys(&mut app, "pair(");
    d.press(&mut app, "<C-g>");
    until(&mut d, &mut app, "pair's", |a| head(a).is_some());
    d.settle(&mut app);
    let tall = d.rect_labelled("completion docs").unwrap();
    let hang = d.rect_labelled("hang").unwrap();
    assert!(
        tall.y >= hang.y + hang.h - 0.5,
        "too tall for over it: {tall:?} {hang:?}"
    );
    d.press(&mut app, "<C-e>");
    d.settle(&mut app);
    d.keys(&mut app, "none(");
    d.press(&mut app, "<C-g>");
    // From the first frame it is drawn in: not placed by the one before.
    until(&mut d, &mut app, "none's", |a| {
        head(a).as_deref() == Some("fn none()")
    });
    let short = d.rect_labelled("completion docs").unwrap();
    let hang = d.rect_labelled("hang").unwrap();
    assert!(
        short.y + short.h <= hang.y + 0.5,
        "over it at once: {short:?} {hang:?}"
    );
    d.settle(&mut app);
    let short = d.rect_labelled("completion docs").unwrap();
    let hang = d.rect_labelled("hang").unwrap();
    assert!(
        short.y + short.h <= hang.y + 0.5,
        "over it: {short:?} {hang:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A call inside a call: the inner one's signature as its `(` is typed,
/// the outer one's again as the caret leaves it, gone as it leaves the
/// outer.
#[test]
fn an_inner_call_comes_back_to_the_outer() {
    let (dir, mut d, mut app) = open("nested", "fn main() {\n    \n}\n", &[], false);
    d.press(&mut app, "jA");
    d.keys(&mut app, "pair(");
    d.press(&mut app, "<C-g>");
    until(&mut d, &mut app, "pair's", |a| {
        head(a).is_some_and(|h| h.contains("pair"))
    });
    d.keys(&mut app, "none(");
    until(&mut d, &mut app, "none's", |a| {
        head(a).as_deref() == Some("fn none()")
    });
    assert_eq!(app.signature.live.as_ref().unwrap().calls.len(), 2);
    d.keys(&mut app, ")");
    until(&mut d, &mut app, "pair's again", |a| {
        head(a).is_some_and(|h| h.contains("pair"))
    });
    assert_eq!(lit(&app).as_deref(), Some("a: i32"));
    d.keys(&mut app, ", 2)");
    assert!(app.signature.live.is_none());
    std::fs::remove_dir_all(&dir).ok();
}

/// One panel (Decision 10): beside the popup, the signature while the
/// word is typed; the selected candidate's documentation while the
/// popup is walked; the signature again once typing goes on. `<C-e>`
/// closes the completion first, then the signature. With no popup the
/// panel sits above the caret's line.
#[test]
fn one_panel_the_docs_while_walking_the_signature_otherwise() {
    let (dir, mut d, mut app) = open("one", TALL, &[], false);
    ex(&mut d, &mut app, "setlocal completion.trigger=typing");
    ex(&mut d, &mut app, "setlocal completion.popup=always");
    ex(&mut d, &mut app, "setlocal completion.docs=0");
    d.press(&mut app, "10GA");
    d.keys(&mut app, "pair(");
    d.press(&mut app, "<C-g>");
    until(&mut d, &mut app, "the signature", |a| head(a).is_some());
    d.keys(&mut app, "pa");
    until(&mut d, &mut app, "the server's candidates", |a| {
        from_server(a, "pair")
    });
    d.settle(&mut app);
    assert!(app.completion_popup_on());
    let panels = |d: &Drive| {
        d.core
            .nodes()
            .iter()
            .filter(|n| n.label.as_deref() == Some("completion docs"))
            .count()
    };
    assert_eq!(panels(&d), 1);
    assert!(
        app.signature_text_shown().is_some(),
        "typing: the signature"
    );
    assert!(!holds(&app, "signature"), "the completion's keys over it");
    // Walked: the candidate's documentation, in the same one panel.
    d.press(&mut app, "<C-n>");
    until(&mut d, &mut app, "the candidate's documentation", |a| {
        a.completion_panel_text()
            .is_some_and(|(_, b)| b.starts_with("docs of"))
    });
    assert!(app.signature_text_shown().is_none());
    d.settle(&mut app);
    assert_eq!(panels(&d), 1, "never two");
    // Typing on: the signature again.
    d.press(&mut app, "<C-p>");
    d.keys(&mut app, "i");
    until(&mut d, &mut app, "the signature again", |a| {
        a.signature_text_shown().is_some()
    });
    d.settle(&mut app);
    assert_eq!(panels(&d), 1);
    // `<C-e>`: the completion, then the signature.
    d.press(&mut app, "<C-e>");
    d.settle(&mut app);
    assert!(!app.completion_popup_on());
    assert!(app.signature.live.is_some(), "the completion closed first");
    assert!(holds(&app, "signature"));
    // Alone: above the caret's line.
    let panel = d.rect_labelled("completion docs").unwrap();
    let hang = d.rect_labelled("hang").unwrap();
    assert!(
        panel.y + panel.h <= hang.y + 0.5,
        "above: {panel:?} {hang:?}"
    );
    d.press(&mut app, "<C-e>");
    assert!(app.signature.live.is_none());
    std::fs::remove_dir_all(&dir).ok();
}

/// With `completion.popup = "never"` and a ghost on show, the signature
/// is above the caret's line too: no popup to sit beside.
#[test]
fn above_the_line_with_no_popup() {
    let (dir, mut d, mut app) = open("above", TALL, &[], false);
    ex(&mut d, &mut app, "setlocal completion.trigger=typing");
    ex(&mut d, &mut app, "setlocal completion.popup=never");
    d.press(&mut app, "10GA");
    d.keys(&mut app, "pair(");
    d.press(&mut app, "<C-g>");
    until(&mut d, &mut app, "the signature", |a| head(a).is_some());
    d.keys(&mut app, "pa");
    until(&mut d, &mut app, "a ghost", |a| {
        a.completion_ghost_text().is_some()
    });
    d.settle(&mut app);
    assert!(!app.completion_popup_on());
    assert!(app.signature_text_shown().is_some());
    assert!(d.rect_labelled("completion popup").is_none());
    let panel = d.rect_labelled("completion docs").unwrap();
    let hang = d.rect_labelled("hang").unwrap();
    assert!(
        panel.y + panel.h <= hang.y + 0.5,
        "above: {panel:?} {hang:?}"
    );
    // On the pane's first line there is no room over it: under it.
    d.press(&mut app, "<C-e><C-e><Esc>ggO");
    d.keys(&mut app, "pair(");
    d.press(&mut app, "<C-g>");
    until(&mut d, &mut app, "the signature", |a| head(a).is_some());
    d.settle(&mut app);
    let panel = d.rect_labelled("completion docs").unwrap();
    let hang = d.rect_labelled("hang").unwrap();
    assert!(
        panel.y >= hang.y + hang.h - 0.5,
        "under: {panel:?} {hang:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

fn pair_help(active: Option<usize>) -> kawoosh_systems::lsp::SignatureHelp {
    let mut v = serde_json::json!({
        "signatures": [{ "label": "fn pair(a: i32, b: &str) -> bool",
                         "parameters": [{ "label": "a: i32" }, { "label": "b: &str" }] }],
        "activeSignature": 0
    });
    if let Some(a) = active {
        v["activeParameter"] = a.into();
    }
    kawoosh_systems::lsp::signature_help(Some(&v)).unwrap()
}

/// An answer for a place the caret left, or a version that moved, is
/// dropped as it lands — nothing changes under the key — and the place
/// the caret is at asked instead; one ask in flight. A function
/// accepted with `args` is not filled in when a key came first.
#[test]
fn a_late_answer_is_dropped() {
    let (dir, mut d, mut app) = open(
        "late",
        "fn main() {\n    \n}\n",
        &["--signature-never"],
        false,
    );
    d.press(&mut app, "jA");
    d.keys(&mut app, "pair(");
    d.press(&mut app, "<C-g>");
    let (b, off, v) = app.signature_asking().expect("asked");
    let asks = app.signature.asks;
    // A key while it is in flight asks nothing more.
    d.keys(&mut app, "1");
    assert_eq!(app.signature.asks, asks, "one in flight");
    // Its answer lands for the place left: dropped, the caret's asked.
    app.signature_answered(b, v, off, Ok(Some(pair_help(Some(0)))));
    d.settle(&mut app);
    assert!(head(&app).is_none(), "dropped as it landed");
    assert_eq!(app.signature.asks, asks + 1, "the caret's place asked");
    let (b2, off2, v2) = app.signature_asking().unwrap();
    assert_eq!(off2, off + 1);
    assert_ne!(v2, v);
    // An answer nobody waits on is nothing.
    app.signature_answered(b, v, off, Ok(Some(pair_help(Some(1)))));
    assert!(head(&app).is_none());
    assert_eq!(app.signature_asking(), Some((b2, off2, v2)));
    // The one asked, for where the caret is: shown.
    app.signature_answered(b2, v2, off2, Ok(Some(pair_help(Some(0)))));
    d.settle(&mut app);
    assert_eq!(lit(&app).as_deref(), Some("a: i32"));
    // A refusal keeps what is on show.
    d.keys(&mut app, ",");
    let (b3, off3, v3) = app.signature_asking().unwrap();
    app.signature_answered(b3, v3, off3, Err("content modified".into()));
    d.settle(&mut app);
    assert_eq!(lit(&app).as_deref(), Some("a: i32"));

    // `callable = "args"`: `pair()` put, its fill waiting on an answer
    // a key came before.
    d.press(&mut app, "<C-e><Esc>o");
    ex(&mut d, &mut app, "setlocal completion.callable=args");
    d.press(&mut app, "o");
    d.keys(&mut app, "pai");
    d.press(&mut app, "<C-Space>");
    until(&mut d, &mut app, "the server's candidates", |a| {
        from_server(a, "pair")
    });
    while app.completion.selected_text() != Some("pair") {
        d.press(&mut app, "<C-n>");
    }
    d.press(&mut app, "<Tab>");
    assert!(text(&app).contains("    pair()\n"), "{}", text(&app));
    let (b4, off4, v4) = app.signature_asking().unwrap();
    d.keys(&mut app, "x");
    app.signature_answered(b4, v4, off4, Ok(Some(pair_help(Some(0)))));
    d.settle(&mut app);
    assert!(
        text(&app).contains("    pair(x)\n"),
        "not filled under the key: {}",
        text(&app)
    );
    assert!(app.snippet.is_none());
    std::fs::remove_dir_all(&dir).ok();
}

/// Masked (secrets.md Decision 9): a signature's label and its
/// documentation drawn through the masks — a word inside an open
/// buffer's mask `•` — and its parameter still lit.
#[test]
fn a_signatures_masked_text_is_never_drawn() {
    let (dir, mut d, mut app) = open("masked", "fn main() {\n    \n}\n", &[], true);
    d.press(&mut app, "jA");
    d.keys(&mut app, "secret(");
    d.press(&mut app, "<C-g>");
    until(&mut d, &mut app, "the signature", |a| head(a).is_some());
    let kawoosh::signature::Shown {
        head: h,
        lit: l,
        body,
        ..
    } = app.signature_text_shown().unwrap();
    assert!(!h.contains("MIIEv") && h.contains('•'), "{h:?}");
    assert!(h.starts_with("fn secret(key: "), "{h:?}");
    let l = l.expect("the parameter still lit");
    assert!(l.starts_with("key: ") && !l.contains("MIIEv"), "{l:?}");
    assert!(!body.contains("MIIEv") && body.contains('•'), "{body:?}");
    d.settle(&mut app);
    let all: String = d
        .core
        .nodes()
        .iter()
        .filter_map(|n| n.text.clone())
        .collect();
    assert!(all.contains("fn secret(key: "), "drawn");
    assert!(!all.contains("MIIEvSecretKeyMaterial"), "no frame draws it");
    std::fs::remove_dir_all(&dir).ok();
}

/// `completion.callable`: `name` puts a function's name alone — a
/// server's snippet with brackets too, and its ghost shows no more —
/// and `args` its brackets and parameters as snippet fields, the
/// server's own snippet where it sends one, else built from the
/// signature it says; a function of none puts the caret after them.
#[test]
fn callable_name_against_args() {
    let (dir, mut d, mut app) = open("callable", "fn main() {\n    \n}\n", &[], false);
    ex(&mut d, &mut app, "setlocal completion.trigger=typing");
    let take = |d: &mut Drive, app: &mut Kawoosh, typed: &str, want: &str| {
        d.press(app, "<Esc>");
        d.press(app, "o");
        d.keys(app, typed);
        until(d, app, want, |a| from_server(a, want));
        let mut n = 0;
        while app.completion.selected_text() != Some(want) {
            d.press(app, "<C-n>");
            n += 1;
            assert!(
                n < 10,
                "{want} never selected: {:?}",
                app.completion.candidates()
            );
        }
        d.press(app, "<Tab>");
        d.settle(app);
    };
    // `name`, the default.
    d.press(&mut app, "j");
    take(&mut d, &mut app, "pair_s", "pair_snip");
    let line = |app: &Kawoosh| {
        let t = text(app);
        let v = app.focused_view().unwrap();
        let ln = app
            .ed
            .buffer_of(v)
            .line_of(app.ed.views[v].sels.primary().head);
        t.lines().nth(ln).unwrap_or("").to_string()
    };
    assert_eq!(
        line(&app),
        "    pair_snip",
        "the server's snippet's name alone"
    );
    assert!(app.snippet.is_none());
    d.press(&mut app, "<Esc>o");
    d.keys(&mut app, "pair_s");
    until(&mut d, &mut app, "a ghost", |a| {
        a.completion_ghost_text().is_some()
    });
    assert_eq!(
        app.completion_ghost_text().as_deref(),
        Some("nip"),
        "no brackets shown"
    );
    take(&mut d, &mut app, "pai", "pair");
    assert_eq!(line(&app), "    pair");

    // `args`.
    ex(&mut d, &mut app, "setlocal completion.callable=args");
    take(&mut d, &mut app, "pair_s", "pair_snip(a, b)");
    assert_eq!(line(&app), "    pair_snip(a, b)", "the server's snippet");
    assert_eq!(selected(&app), ["a"]);
    assert!(app.snippet.is_some());
    take(&mut d, &mut app, "pai", "pair");
    until(&mut d, &mut app, "the parameters filled", |a| {
        a.snippet.is_some()
    });
    assert_eq!(line(&app), "    pair(a, b)", "built from the signature");
    assert_eq!(selected(&app), ["a"]);
    d.keys(&mut app, "1");
    d.press(&mut app, "<C-l>");
    assert_eq!(selected(&app), ["b"]);
    d.keys(&mut app, "\"x\"");
    d.press(&mut app, "<C-l>");
    assert!(app.snippet.is_none());
    assert_eq!(line(&app), "    pair(1, \"x\")");
    let v = app.focused_view().unwrap();
    let caret = app.ed.views[v].sels.primary().head;
    assert_eq!(&text(&app)[caret - 1..caret], ")", "$0 after the brackets");
    // None: the brackets, the caret after them once the server said so.
    take(&mut d, &mut app, "non", "none");
    assert_eq!(line(&app), "    none()");
    until(&mut d, &mut app, "the caret after `)`", |a| {
        let v = a.focused_view().unwrap();
        let c = a.ed.views[v].sels.primary().head;
        text(a)[..c].ends_with("none()")
    });
    // A bracket already after the caret: the name alone.
    d.press(&mut app, "<Esc>o");
    d.keys(&mut app, "(x)");
    d.press(&mut app, "<Home><Right><Right><Right><Right>");
    d.keys(&mut app, "pai");
    until(&mut d, &mut app, "pair", |a| from_server(a, "pair"));
    while app.completion.selected_text() != Some("pair") {
        d.press(&mut app, "<C-n>");
    }
    d.press(&mut app, "<Tab>");
    d.settle(&mut app);
    assert_eq!(line(&app), "    pair(x)");
    std::fs::remove_dir_all(&dir).ok();
}

/// A keystroke with the signature on show against one with none, each
/// key inside the call (asked again at each, answered by the fake
/// server between them): run with `-- --ignored --nocapture`.
#[test]
#[ignore]
fn timing_a_keystroke_with_the_signature() {
    let filler: String = (0..20_000)
        .map(|i| format!("// alpha_{i} beta_{} gamma_{}\n", i % 97, i % 31))
        .collect();
    for shown in [false, true] {
        let (dir, mut d, mut app) = open(
            if shown { "timing-on" } else { "timing-off" },
            &format!("fn main() {{\n    \n}}\n{filler}"),
            &[],
            false,
        );
        d.press(&mut app, "jA");
        d.keys(&mut app, "pair(");
        if shown {
            d.press(&mut app, "<C-g>");
            until(&mut d, &mut app, "the signature", |a| head(a).is_some());
        }
        d.settle(&mut app);
        let mut keys = Vec::new();
        for round in 0..40 {
            for ch in ["x", "y", ",", " ", "<BS>"] {
                let t = std::time::Instant::now();
                if ch == "<BS>" {
                    d.press(&mut app, ch);
                } else {
                    d.keys(&mut app, ch);
                }
                let took = t.elapsed();
                d.settle(&mut app);
                if round > 0 {
                    keys.push(took);
                }
            }
        }
        assert_eq!(app.signature.live.is_some(), shown);
        keys.sort();
        let (k, k90) = (keys[keys.len() / 2], keys[keys.len() * 9 / 10]);
        println!("signature shown {shown}: keystroke {k:?} (p90 {k90:?})");
        std::fs::remove_dir_all(&dir).ok();
    }
}
