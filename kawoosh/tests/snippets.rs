//! Completion's round 3 (docs/design/completion.md Decision 7): LSP
//! snippets as selections — the field being filled selected, a field
//! that repeats several carets, `$0` the last caret, a choice's options
//! the popup's; `<C-l>` / `<C-k>` while one is live, and every way one
//! ends.

mod drive;

use drive::{Drive, fake_lsp};
use kawoosh::Kawoosh;
use kawoosh_systems::lsp::ServerDef;

fn tmp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-snippets-{tag}-{}", std::process::id()));
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

/// `name` in a directory of its own holding `text`, open, its jobs
/// answered on the spot.
fn open(tag: &str, name: &str, text: &str) -> (std::path::PathBuf, Drive, Kawoosh) {
    let dir = tmp(tag);
    std::fs::write(dir.join(name), text).unwrap();
    let mut app = Kawoosh::from_file(&dir.join(name));
    app.jobs_inline = true;
    let mut d = Drive::new(900.0, 500.0);
    app.wait_for_open();
    d.settle(&mut app);
    (dir, d, app)
}

fn text(app: &Kawoosh) -> String {
    app.ed.buffer_of(app.focused_view().unwrap()).text()
}

/// The text of every selection, primary first.
fn selected(app: &Kawoosh) -> Vec<String> {
    let v = app.focused_view().unwrap();
    let sels = &app.ed.views[v].sels;
    let buf = app.ed.buffer_of(v);
    let mut out = vec![buf.slice(sels.primary().range())];
    for (i, s) in sels.iter().enumerate() {
        if i != sels.primary {
            out.push(buf.slice(s.range()));
        }
    }
    out
}

fn carets(app: &Kawoosh) -> Vec<usize> {
    let v = app.focused_view().unwrap();
    app.ed.views[v].sels.iter().map(|s| s.head).collect()
}

fn live(app: &Kawoosh) -> bool {
    app.snippet.is_some()
}

fn holds(app: &Kawoosh, fact: &str) -> bool {
    app.ed.holds(app.focused_view(), fact)
}

/// Typing replaces the field selected; `<C-l>` goes on, `<C-k>` back;
/// at `$0` the snippet is done and the keys are nobody's. The body's
/// lines are indented as the line it goes into, its tabs the buffer's
/// indent unit.
#[test]
fn typing_fills_a_field_and_the_keys_walk_them() {
    let (dir, mut d, mut app) = open("walk", "main.rs", "fn main() {\n    \n}\n");
    d.press(&mut app, "jA");
    assert!(app.snippet_expand("for ${1:x} in ${2:iter} {\n\t$0\n}"));
    d.settle(&mut app);
    assert_eq!(
        text(&app),
        "fn main() {\n    for x in iter {\n        \n    }\n}\n"
    );
    assert!(live(&app));
    assert!(holds(&app, "snippet"), "the keys hold while one is live");
    assert_eq!(selected(&app), ["x"]);
    d.keys(&mut app, "item");
    assert_eq!(selected(&app), [""], "typed over: a caret after it");
    assert!(
        text(&app).contains("for item in iter {"),
        "{:?}",
        text(&app)
    );
    d.press(&mut app, "<C-l>");
    assert_eq!(selected(&app), ["iter"]);
    d.keys(&mut app, "list");
    d.press(&mut app, "<C-k>");
    assert_eq!(selected(&app), ["item"], "back: the field selected again");
    d.press(&mut app, "<C-l>");
    assert_eq!(selected(&app), ["list"]);
    d.press(&mut app, "<C-l>");
    assert!(!live(&app), "at $0 it is done");
    assert!(!holds(&app, "snippet"));
    let at = carets(&app);
    let t = text(&app);
    assert_eq!(
        t,
        "fn main() {\n    for item in list {\n        \n    }\n}\n"
    );
    assert_eq!(
        at,
        [t.find("\n        \n").unwrap() + 9],
        "the last caret at $0"
    );
    // Done: `<C-l>` is nothing now, `<C-k>` too.
    d.press(&mut app, "<C-l>");
    d.press(&mut app, "<C-k>");
    assert_eq!(text(&app), t);
    assert_eq!(carets(&app), at);
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// A field that repeats is a caret in each place, edited together —
/// multi-byte text and Backspace too.
#[test]
fn a_repeated_field_is_several_carets() {
    let (dir, mut d, mut app) = open("mirror", "main.rs", "\n");
    d.press(&mut app, "i");
    app.snippet_expand("pair(${1:a}, ${1:a})$0");
    d.settle(&mut app);
    assert_eq!(selected(&app), ["a", "a"]);
    d.keys(&mut app, "ключ");
    assert_eq!(text(&app), "pair(ключ, ключ)\n");
    d.press(&mut app, "<BS>");
    assert_eq!(text(&app), "pair(клю, клю)\n");
    d.keys(&mut app, "ч🦀");
    assert_eq!(text(&app), "pair(ключ🦀, ключ🦀)\n");
    let l = app.snippet.as_ref().unwrap();
    let buf = app.ed.buffer_of(l.view);
    let fields: Vec<String> = l.fields[0]
        .ranges
        .iter()
        .map(|r| buf.slice(r.clone()))
        .collect();
    assert_eq!(
        fields,
        ["ключ🦀", "ключ🦀"],
        "the ranges followed every byte"
    );
    d.press(&mut app, "<C-l>");
    assert!(!live(&app));
    assert_eq!(carets(&app), ["pair(ключ🦀, ключ🦀)".len()]);
    std::fs::remove_dir_all(&dir).ok();
}

/// A placeholder in a placeholder: the outer selected whole, the inner
/// next; typing over the outer takes the inner with it.
#[test]
fn nested_placeholders() {
    let (dir, mut d, mut app) = open("nested", "main.rs", "\n");
    d.press(&mut app, "i");
    app.snippet_expand("call(${1:first ${2:second}}) $0");
    d.settle(&mut app);
    assert_eq!(selected(&app), ["first second"]);
    d.press(&mut app, "<C-l>");
    assert_eq!(selected(&app), ["second"]);
    d.keys(&mut app, "2nd");
    d.press(&mut app, "<C-k>");
    assert_eq!(
        selected(&app),
        ["first 2nd"],
        "the outer holds what the inner took"
    );
    // Typed over, the inner field is gone: `<C-l>` goes to `$0`.
    d.keys(&mut app, "x");
    assert_eq!(text(&app), "call(x) \n");
    d.press(&mut app, "<C-l>");
    assert!(!live(&app));
    assert_eq!(carets(&app), [8]);
    std::fs::remove_dir_all(&dir).ok();
}

/// Typing goes over a snippet's field alone: a selection an undo put
/// back is typed at its caret — by a key, `<Del>`, `<CR>` or a macro —
/// and so is a field's once its snippet ended.
#[test]
fn only_a_fields_selection_is_typed_over() {
    let (dir, mut d, mut app) = open("undone", "main.rs", "hello world\n");
    // Each: what is done before, what is typed in insert mode after the
    // undo, and the text a typing at the caret leaves (nothing erased).
    let base = "hello world\n";
    let erased_nothing = |app: &Kawoosh, typed: &str, what: &str| {
        let t = text(app);
        let mut left = t.clone();
        for c in typed.chars() {
            if let Some(i) = left.find(c) {
                left.remove(i);
            }
        }
        assert_eq!(left, base, "{what}: {t:?}");
    };
    for (before, what) in [
        ("vlld", "a visual delete"),
        ("viwcfoo<Esc>", "a visual change"),
    ] {
        d.press(&mut app, &format!("<Esc>gg0{before}u"));
        assert_eq!(text(&app), base, "{what}: undone");
        d.press(&mut app, "iX");
        erased_nothing(&app, "X", what);
        d.press(&mut app, "<Esc>u");
        // `<BS>`, `<Del>`, `<CR>` right after `i`: one character, one
        // character, a line break — never the range.
        d.press(&mut app, &format!("<Esc>gg0{before}ui<Del>"));
        assert_eq!(text(&app).len(), base.len() - 1, "{what}: <Del>");
        d.press(&mut app, &format!("<Esc>uu<Esc>gg0{before}ui<CR>"));
        erased_nothing(&app, "\n", &format!("{what}: <CR>"));
        d.press(&mut app, "<Esc>u");
        assert_eq!(text(&app), base, "{what}: back");
    }
    // A macro's insert replayed over a selection an undo put back (`.`
    // after the undo repeats the undone change itself).
    d.press(&mut app, "<Esc>gg0qaiZ<Esc>qu");
    assert_eq!(text(&app), base);
    d.press(&mut app, "gg0vlldu@a");
    erased_nothing(&app, "Z", "a macro");
    d.press(&mut app, "u");
    // A field selected, the snippet ended by `<Esc>`: `i` types at the
    // caret, the field's text kept.
    d.press(&mut app, "<Esc>Go");
    app.snippet_expand("f(${1:arg})");
    d.settle(&mut app);
    assert_eq!(selected(&app), ["arg"]);
    d.press(&mut app, "<Esc>");
    d.settle(&mut app);
    d.press(&mut app, "iW");
    let t = text(&app);
    assert!(t.replacen('W', "", 1).ends_with("f(arg)"), "{t:?}");
    std::fs::remove_dir_all(&dir).ok();
}

/// A field with only a transform of it in the body is no field: the
/// first that has a place is selected, and none panics.
#[test]
fn a_transform_only_field_is_passed() {
    let (dir, mut d, mut app) = open("transform-only", "main.rs", "\n");
    d.press(&mut app, "i");
    app.snippet_expand("${1/(.*)/${1:/upcase}/} ${2:a}");
    d.settle(&mut app);
    assert_eq!(text(&app), " a\n");
    assert_eq!(selected(&app), ["a"]);
    d.press(&mut app, "<C-l>");
    assert!(!live(&app));
    app.snippet_expand("x${1/x/y/}");
    d.settle(&mut app);
    assert!(!live(&app));
    d.keys(&mut app, "z");
    assert_eq!(text(&app), " axz\n");
    std::fs::remove_dir_all(&dir).ok();
}

/// A transform of a field writes its places as the field is left.
#[test]
fn a_transform_is_written_as_its_field_is_left() {
    let (dir, mut d, mut app) = open("transform", "main.rs", "\n");
    d.press(&mut app, "i");
    app.snippet_expand("let ${1:name} = \"${1/(.*)/${1:/upcase}/}\"; ${2:rest}");
    d.settle(&mut app);
    assert_eq!(text(&app), "let name = \"NAME\"; rest\n");
    d.keys(&mut app, "кот_x");
    assert_eq!(
        text(&app),
        "let кот_x = \"NAME\"; rest\n",
        "not as it is typed"
    );
    d.press(&mut app, "<C-l>");
    assert_eq!(text(&app), "let кот_x = \"КОТ_X\"; rest\n");
    assert_eq!(selected(&app), ["rest"], "the next field's place carried");
    std::fs::remove_dir_all(&dir).ok();
}

/// Accepted at several carets, a snippet is a copy at each; a field is
/// its places in every copy, and one walk moves them all.
#[test]
fn several_carets_each_get_a_copy() {
    let (dir, mut d, mut app) = open("carets", "main.rs", "one\ntwo\n");
    d.press(&mut app, "gg<C-j>A");
    assert_eq!(carets(&app).len(), 2);
    app.snippet_expand(" f(${1:a}, ${2:b})");
    d.settle(&mut app);
    assert_eq!(text(&app), "one f(a, b)\ntwo f(a, b)\n");
    assert_eq!(selected(&app), ["a", "a"]);
    d.keys(&mut app, "x");
    d.press(&mut app, "<C-l>");
    assert_eq!(selected(&app), ["b", "b"]);
    d.keys(&mut app, "yy");
    d.press(&mut app, "<C-l>");
    assert!(!live(&app));
    assert_eq!(text(&app), "one f(x, yy)\ntwo f(x, yy)\n");
    assert_eq!(carets(&app), [12, 25], "the last carets, one a copy");
    std::fs::remove_dir_all(&dir).ok();
}

/// A field one copy has and another not — a variable's default holding
/// it, the variable known at one caret — is paired by its number, never
/// by its place in the first copy.
#[test]
fn copies_pair_their_fields_by_number() {
    let (dir, mut d, mut app) = open("pair-by-number", "main.rs", "abc\n\n");
    d.press(&mut app, "gg<C-j>A");
    assert_eq!(carets(&app).len(), 2);
    app.snippet_expand("[${TM_CURRENT_WORD:${1:x}}|${2:y}]");
    d.settle(&mut app);
    assert_eq!(text(&app), "abc[abc|y]\n[x|y]\n");
    assert_eq!(selected(&app), ["x"], "field 1: the second copy's alone");
    d.press(&mut app, "<C-l>");
    assert_eq!(selected(&app), ["y", "y"], "field 2: both copies'");
    d.keys(&mut app, "z");
    assert_eq!(text(&app), "abc[abc|z]\n[x|z]\n");
    std::fs::remove_dir_all(&dir).ok();
}

/// A choice field offers its options in the popup at once, whatever
/// the look says; `<C-n>` walks them, `<Tab>` takes one into every
/// place of the field; typing narrows them.
#[test]
fn a_choice_is_the_popups() {
    let (dir, mut d, mut app) = open("choice", "main.rs", "\n");
    ex(&mut d, &mut app, "setlocal completion.popup=never");
    d.press(&mut app, "i");
    app.snippet_expand("${1|pub,pub(crate),priv|} fn ${2:name}() // ${1}");
    d.settle(&mut app);
    assert_eq!(selected(&app), ["pub", "pub"]);
    assert!(app.completion_popup_on(), "the options shown");
    let rows = d.completion_rows();
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert!(holds(&app, "completion"));
    d.press(&mut app, "<C-n>");
    assert_eq!(app.completion.selected_text(), Some("pub(crate)"));
    d.press(&mut app, "<Tab>");
    assert_eq!(text(&app), "pub(crate) fn name() // pub(crate)\n");
    assert!(live(&app), "still on the field");
    assert!(!app.completion_popup_on());
    d.press(&mut app, "<C-l>");
    assert_eq!(selected(&app), ["name"]);
    // Back onto the choice: offered again; typing narrows.
    d.press(&mut app, "<C-k>");
    d.settle(&mut app);
    assert!(app.completion_popup_on());
    d.keys(&mut app, "pri");
    d.settle(&mut app);
    assert_eq!(
        app.completion.candidates(),
        ["priv"],
        "narrowed by what is typed"
    );
    d.press(&mut app, "<Tab>");
    assert_eq!(text(&app), "priv fn name() // priv\n");
    std::fs::remove_dir_all(&dir).ok();
}

/// `<Tab>` is never a snippet's: with one live and nothing to accept it
/// is a tab, typed into the field.
#[test]
fn tab_is_left_alone() {
    let (dir, mut d, mut app) = open("tab", "main.rs", "\n");
    d.press(&mut app, "i");
    app.snippet_expand("f(${1:a}, ${2:b})");
    d.settle(&mut app);
    d.press(&mut app, "<Tab>");
    assert_eq!(
        text(&app),
        "f(  , b)\n",
        "a tab to the next stop, over the field"
    );
    assert_eq!(app.snippet.as_ref().map(|l| l.at), Some(0), "no jump");
    std::fs::remove_dir_all(&dir).ok();
}

/// Each way a snippet ends, and after each `<C-l>` is nothing — the
/// LuaSnip complaint: never back into a snippet long left.
#[test]
fn every_end_and_no_jump_back() {
    let (dir, mut d, mut app) = open("ends", "main.rs", "one\n\nthree\n");
    let start = |d: &mut Drive, app: &mut Kawoosh| {
        d.press(app, "<Esc>ggjS");
        app.snippet_expand("a(${1:x}, ${2:y})");
        d.settle(app);
        assert!(live(app));
    };
    // `<C-l>` in insert mode from where each left it: nothing moves,
    // nothing is typed.
    let no_jump = |d: &mut Drive, app: &mut Kawoosh, what: &str| {
        assert!(!live(app), "{what}: ended");
        assert!(!holds(app, "snippet"), "{what}: the keys gone");
        if app.ed.mode(app.focused_view().unwrap()) != kawoosh_editor::Mode::Insert {
            d.press(app, "i");
        }
        let (t, at) = (text(app), carets(app));
        d.press(app, "<C-l>");
        d.press(app, "<C-k>");
        assert_eq!(
            (text(app), carets(app)),
            (t, at),
            "{what}: <C-l> is nothing"
        );
    };

    // Leaving insert mode; and back in, inside its old text, nothing.
    start(&mut d, &mut app);
    d.press(&mut app, "<Esc>");
    d.settle(&mut app);
    no_jump(&mut d, &mut app, "<Esc>");
    d.press(&mut app, "0a");
    no_jump(&mut d, &mut app, "insert again inside it");

    // The caret leaving its text by a move; one inside it does not.
    start(&mut d, &mut app);
    d.press(&mut app, "<Left>");
    d.settle(&mut app);
    assert!(live(&app), "a move inside it");
    d.press(&mut app, "<Up>");
    d.settle(&mut app);
    no_jump(&mut d, &mut app, "a caret moved out");

    // An edit outside it — a server's, say — at the next frame.
    start(&mut d, &mut app);
    let buffer = app.ed.views[app.focused_view().unwrap()].buffer;
    app.ed.apply_edits(buffer, &[(0..0, "zz".into())]);
    d.settle(&mut app);
    no_jump(&mut d, &mut app, "an edit outside");

    // An undo puts the text back, the snippet with it gone; a redo
    // brings back no snippet.
    start(&mut d, &mut app);
    d.keys(&mut app, "q");
    let v = app.focused_view().unwrap();
    app.ed.undo(v);
    d.settle(&mut app);
    no_jump(&mut d, &mut app, "undo");
    app.ed.redo(v);
    d.settle(&mut app);
    assert!(!live(&app), "redo");

    // Another accepted in its field: the first gone, the second live.
    start(&mut d, &mut app);
    app.snippet_expand("b(${1:p})");
    d.settle(&mut app);
    assert_eq!(selected(&app), ["p"]);
    d.press(&mut app, "<C-l>");
    assert!(!live(&app), "the second done, the first not come back");
    let line = text(&app).lines().nth(1).unwrap().to_string();
    assert_eq!(line, "a(b(p), y)");

    // Another pane taking the keys.
    ex(&mut d, &mut app, "vsplit");
    start(&mut d, &mut app);
    let here = app.layout.focused();
    let (go, back) = (["<C-S-l>", "<C-S-h>"], ["<C-S-h>", "<C-S-l>"]);
    d.press(&mut app, go[0]);
    let way = if app.layout.focused() == here {
        d.press(&mut app, go[1]);
        1
    } else {
        0
    };
    assert_ne!(app.layout.focused(), here, "the keys in the other pane");
    d.settle(&mut app);
    assert!(!live(&app), "another pane");
    d.press(&mut app, back[way]);
    d.settle(&mut app);
    assert_eq!(app.layout.focused(), here);
    no_jump(&mut d, &mut app, "back in the pane");

    // Its view showing another buffer.
    ex(&mut d, &mut app, "only");
    std::fs::write(dir.join("other.rs"), "other\n").unwrap();
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("other.rs").display()),
    );
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("main.rs").display()),
    );
    start(&mut d, &mut app);
    let v = app.focused_view().unwrap();
    let other = app.ed.buffer_at(&dir.join("other.rs")).unwrap();
    // As `:bn`, a listing's `<CR>`, a tab's click show one.
    app.show_buffer(v, other);
    d.settle(&mut app);
    assert!(!live(&app), "a buffer switch");
    std::fs::remove_dir_all(&dir).ok();
}

/// Typing on past a snippet's end is typing after it: the snippet ends
/// there, and `<C-l>` never reaches back into it. Where the field being
/// filled is at the end, typing there is the field's.
#[test]
fn typing_past_the_end_ends_it() {
    let (dir, mut d, mut app) = open("past-end", "main.rs", "\n");
    d.press(&mut app, "i");
    app.snippet_expand("foo(${1:a}, ${2:b})");
    d.settle(&mut app);
    d.keys(&mut app, "x");
    d.press(&mut app, "<End>");
    d.keys(&mut app, ";");
    d.press(&mut app, "<CR>");
    d.keys(&mut app, "let z = 1;");
    d.settle(&mut app);
    assert!(!live(&app), "typed past its end");
    let v = app.focused_view().unwrap();
    assert!(app.snippet_marks(v).is_empty(), "`b` no longer marked");
    let (t, at) = (text(&app), carets(&app));
    d.press(&mut app, "<C-l>");
    assert_eq!((text(&app), carets(&app)), (t, at), "no jump back to `b`");
    assert_eq!(text(&app), "foo(x, b);\nlet z = 1;\n");
    // A field at the end: typing there is the field's.
    d.press(&mut app, "<Esc>o");
    app.snippet_expand("let ${1:x} = ${2:value}");
    d.settle(&mut app);
    d.press(&mut app, "<C-l>");
    d.keys(&mut app, "v");
    d.keys(&mut app, "al");
    assert!(live(&app), "the last field at the end grows");
    let l = app.snippet.as_ref().unwrap();
    let buf = app.ed.buffer_of(l.view);
    assert_eq!(buf.slice(l.fields[1].ranges[0].clone()), "val");
    std::fs::remove_dir_all(&dir).ok();
}

/// A malformed body is put as the text it is, never a panic, and
/// nothing is live where it names no field.
#[test]
fn malformed_snippets_are_their_text() {
    let (dir, mut d, mut app) = open("malformed", "main.rs", "x\n");
    for (body, want) in [
        ("broken(${1:unclosed", "broken(${1:unclosed"),
        ("${1|a,b", "${1|a,b"),
        (r"cost \$5 ${", "cost $5 ${"),
        ("$", "$"),
    ] {
        d.press(&mut app, "<Esc>\"_cc");
        assert!(app.snippet_expand(body));
        d.settle(&mut app);
        assert_eq!(text(&app), format!("{want}\n"), "{body}");
        assert!(!live(&app), "{body}: no field");
        assert_eq!(carets(&app), [want.len()], "{body}: the caret at its end");
    }
    // Too big to lay out — a default mirrored past a megabyte: its
    // text, at once.
    let big = format!("${{1:{}}}{}", "x".repeat(64 << 10), "$1".repeat(50_000));
    d.press(&mut app, "<Esc>\"_cc");
    let started = std::time::Instant::now();
    assert!(app.snippet_expand(&big));
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    d.settle(&mut app);
    assert!(!live(&app), "too big: no field");
    assert_eq!(text(&app), format!("{big}\n"));
    // A field after a broken one is still a field.
    d.press(&mut app, "<Esc>\"_cc");
    app.snippet_expand("${1:a ${2:b}");
    d.settle(&mut app);
    assert_eq!(text(&app), "${1:a b\n");
    assert_eq!(selected(&app), ["b"]);
    std::fs::remove_dir_all(&dir).ok();
}

/// Fields not reached yet are drawn faintly underlined, and only those.
#[test]
fn fields_not_reached_are_marked() {
    let (dir, mut d, mut app) = open("marks", "main.rs", "\n");
    d.press(&mut app, "i");
    app.snippet_expand("f(${1:a}, ${2:bb}, ${3:ccc}, $4)");
    d.settle(&mut app);
    let v = app.focused_view().unwrap();
    let marks = |app: &Kawoosh| -> Vec<String> {
        let buf = app.ed.buffer_of(v);
        app.snippet_marks(v)
            .into_iter()
            .map(|r| buf.slice(r))
            .collect()
    };
    assert_eq!(
        marks(&app),
        ["bb", "ccc"],
        "the one being filled is selected"
    );
    d.press(&mut app, "<C-l>");
    assert_eq!(marks(&app), ["ccc"]);
    d.press(&mut app, "<C-k>");
    assert_eq!(marks(&app), ["ccc"], "visited stays visited");
    d.press(&mut app, "<Esc>");
    d.settle(&mut app);
    assert_eq!(marks(&app), Vec::<String>::new(), "gone with it");
    std::fs::remove_dir_all(&dir).ok();
}

/// A variable never carries masked text: the clipboard's newest when it
/// is a secret's or holds a masked word, the line when a mask is on it.
#[test]
fn variables_never_put_masked_text() {
    let dir = tmp("vars");
    std::fs::write(
        dir.join("keys.pem"),
        "-----BEGIN PRIVATE KEY-----\nMIIEvSecretKeyMaterial\n-----END PRIVATE KEY-----\n",
    )
    .unwrap();
    std::fs::write(dir.join("main.rs"), "x\n").unwrap();
    let mut app = Kawoosh::from_file(&dir.join("main.rs"));
    app.jobs_inline = true;
    let mut d = Drive::new(900.0, 500.0);
    app.wait_for_open();
    d.settle(&mut app);
    // A plain text yanked: put.
    d.press(&mut app, "\"_cchello<Esc>0yiw\"_cc");
    app.snippet_expand("[$CLIPBOARD] $TM_FILENAME");
    d.settle(&mut app);
    assert_eq!(text(&app), "[hello] main.rs\n");
    // A word from inside a mask, yanked in the key's file: nothing.
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("keys.pem").display()),
    );
    d.press(&mut app, "jyiw");
    assert_eq!(
        app.ed.memory.head().map(|m| m.text.as_str()),
        Some("MIIEvSecretKeyMaterial")
    );
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("main.rs").display()),
    );
    d.settle(&mut app);
    d.press(&mut app, "\"_cc");
    app.snippet_expand("[$CLIPBOARD]");
    d.settle(&mut app);
    assert_eq!(text(&app), "[]\n", "a masked word is no clipboard");
    // A secret's moment: nothing either.
    app.ed.memory.remember(kawoosh_editor::Moment {
        text: "hunter2".into(),
        linewise: false,
        took: kawoosh_editor::Took::Clipboard,
        origin: None,
        from: "clipboard".into(),
        at: std::time::Instant::now(),
        secret: true,
    });
    d.press(&mut app, "<Esc>\"_cc");
    app.snippet_expand("[$CLIPBOARD]");
    d.settle(&mut app);
    assert_eq!(text(&app), "[]\n", "a secret is no clipboard");
    // The line, where a mask is on it: nothing.
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("keys.pem").display()),
    );
    d.press(&mut app, "ggjA");
    app.snippet_expand("<$TM_CURRENT_LINE|$TM_CURRENT_WORD>");
    d.settle(&mut app);
    let t = text(&app);
    assert_eq!(t.lines().nth(1), Some("MIIEvSecretKeyMaterial<|>"), "{t}");
    assert_eq!(t.matches("MIIEv").count(), 1, "{t}");
    std::fs::remove_dir_all(&dir).ok();
}

fn snippet_server() -> ServerDef {
    let mut s = fake_lsp("rust");
    s.args.push("--snippets".into());
    ServerDef {
        roots: vec!["Cargo.toml".into()],
        ..s
    }
}

/// Frames until `pred` holds, the server answering between them.
fn until(d: &mut Drive, app: &mut Kawoosh, what: &str, mut pred: impl FnMut(&Kawoosh) -> bool) {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        d.frame(app);
        if pred(app) {
            return;
        }
        assert!(
            std::time::Instant::now() < end,
            "waited for {what}: {:?}",
            app.completion.candidates()
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// A server told `snippetSupport: true` sends snippets; accepted, each
/// is expanded where it goes — matched and ghosted by its plain text,
/// its fields live, a choice's options the popup's, a server's
/// `textEdit` body the same, `asIs` lines left as they are.
#[test]
fn a_servers_snippet_end_to_end() {
    let dir = tmp("server");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let file = dir.join("src/main.rs");
    std::fs::write(&file, "fn main() {\n    \n}\n").unwrap();
    let mut app = Kawoosh::from_file(&file);
    app.jobs_inline = true;
    app.add_lsp_server(snippet_server());
    let mut d = Drive::new(900.0, 500.0);
    // The candidates picker is Lua's.
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app.wait_for_open();
    d.settle(&mut app);
    d.press(&mut app, "jA");
    d.keys(&mut app, "zfo");
    until(&mut d, &mut app, "the server's snippets", |a| {
        a.completion
            .candidates()
            .iter()
            .any(|c| c.starts_with("zfor "))
    });
    assert!(
        !app.completion
            .candidates()
            .iter()
            .any(|c| c == "nosnippets"),
        "snippetSupport was said"
    );
    d.settle(&mut app);
    // The plain text's first line is what a ghost shows: no `$`.
    assert_eq!(
        app.completion_ghost_text().as_deref(),
        Some("r x in iter {")
    );
    d.press(&mut app, "<Tab>");
    d.settle(&mut app);
    assert_eq!(
        text(&app),
        "fn main() {\n    zfor x in iter {\n        \n    }\n}\n"
    );
    assert_eq!(selected(&app), ["x"]);
    d.keys(&mut app, "i");
    d.press(&mut app, "<C-l><C-l>");
    assert!(!live(&app));

    // From the candidates picker: live as from the popup, the keys
    // back in the text.
    d.press(&mut app, "<Esc>o");
    d.keys(&mut app, "zpa");
    until(&mut d, &mut app, "zpair", |a| {
        a.completion
            .candidates()
            .iter()
            .any(|c| c.starts_with("zpair("))
    });
    d.press(&mut app, "<C-x>");
    d.frame(&mut app);
    d.key(&mut app, "enter", Default::default());
    d.settle(&mut app);
    assert!(live(&app), "taken from the picker: live");
    assert_eq!(selected(&app), ["a", "a"]);
    d.keys(&mut app, "b");
    assert!(text(&app).contains("zpair(b, b)"), "{}", text(&app));
    d.press(&mut app, "<Esc>dd");

    // A choice from the server, matched by its label though its body
    // is nothing like it: its options the popup's.
    d.press(&mut app, "<Esc>o");
    d.keys(&mut app, "zch");
    until(&mut d, &mut app, "zchoice", |a| {
        a.completion
            .candidates()
            .iter()
            .any(|c| c == "pub fn name()")
    });
    d.settle(&mut app);
    d.press(&mut app, "<Tab>");
    d.settle(&mut app);
    assert_eq!(selected(&app), ["pub"]);
    assert!(app.completion_popup_on());
    assert_eq!(d.completion_rows().len(), 3, "{:?}", d.completion_rows());
    d.press(&mut app, "<C-p><Tab><C-l>");
    assert!(
        text(&app).lines().any(|l| l.trim() == "priv fn name()"),
        "{}",
        text(&app)
    );
    assert_eq!(selected(&app), ["name"]);

    // A textEdit's body; a broken one; one put as it is.
    for (typed, want) in [
        ("zed", "    zedit(arg)"),
        ("zbr", "    zbroken(${1:unclosed"),
        ("zas", "    zasis {"),
    ] {
        d.press(&mut app, "<Esc>o");
        d.keys(&mut app, typed);
        until(&mut d, &mut app, typed, |a| {
            a.completion
                .candidates()
                .iter()
                .any(|c| c.starts_with(typed))
        });
        d.settle(&mut app);
        d.press(&mut app, "<Tab>");
        d.settle(&mut app);
        let t = text(&app);
        assert!(
            t.lines().any(|l| l.trim_start() == want.trim_start()),
            "{typed}: {t}"
        );
    }
    assert!(
        text(&app).contains("zasis {\n\tb\n}"),
        "asIs: its lines as sent: {}",
        text(&app)
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// Accepted at two carets whose words differ in length, each caret's
/// own word is replaced, nothing before it.
#[test]
fn each_caret_replaces_its_own_word() {
    let dir = tmp("own-word");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let file = dir.join("src/main.rs");
    std::fs::write(&file, "self.z\nzp\n").unwrap();
    let mut app = Kawoosh::from_file(&file);
    app.jobs_inline = true;
    app.add_lsp_server(snippet_server());
    let mut d = Drive::new(900.0, 500.0);
    app.wait_for_open();
    d.settle(&mut app);
    d.press(&mut app, "gg<C-j>A");
    let v = app.focused_view().unwrap();
    let primary = app.ed.views[v].sels.primary().head;
    assert_eq!(
        app.ed.buffer_of(v).line_of(primary),
        1,
        "the `zp` line's caret leads"
    );
    d.keys(&mut app, "a");
    until(&mut d, &mut app, "zpair", |a| {
        a.completion
            .candidates()
            .iter()
            .any(|c| c.starts_with("zpair("))
    });
    d.settle(&mut app);
    d.press(&mut app, "<Tab>");
    d.settle(&mut app);
    assert_eq!(text(&app), "self.zpair(a, a)\nzpair(a, a)\n");
    assert_eq!(
        selected(&app).len(),
        4,
        "a field in two places, at two carets"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A choice field's options are all its popup offers: typed in, it asks
/// no source — not the workspace's symbols, three letters in.
#[test]
fn a_choice_asks_no_source() {
    let dir = tmp("choice-asks");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let file = dir.join("src/main.rs");
    std::fs::write(&file, "\n").unwrap();
    let mut app = Kawoosh::from_file(&file);
    app.jobs_inline = true;
    app.add_lsp_server(snippet_server());
    let mut d = Drive::new(900.0, 500.0);
    app.wait_for_open();
    d.settle(&mut app);
    let sources = ["lsp", "workspace_symbols"]
        .iter()
        .map(|s| kawoosh_editor::Setting::Str(s.to_string()))
        .collect();
    app.ed.settings.set(
        kawoosh_editor::Layer::Session,
        "completion.sources",
        kawoosh_editor::Setting::List(sources),
    );
    // The server up and answering.
    d.press(&mut app, "o");
    d.keys(&mut app, "zfo");
    until(&mut d, &mut app, "the server", |a| {
        !a.completion.candidates().is_empty()
    });
    d.press(&mut app, "<Esc>\"_cc");
    app.snippet_expand("${1|widget_a,pub|} x");
    d.settle(&mut app);
    d.keys(&mut app, "wid");
    let end = std::time::Instant::now() + std::time::Duration::from_millis(600);
    while std::time::Instant::now() < end {
        d.frame(&mut app);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    d.settle(&mut app);
    assert_eq!(
        app.completion.candidates(),
        ["widget_a"],
        "the options alone, no `Widget` or `widget_fn`"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A keystroke inside a live snippet against one with none, on a
/// 20,000-line buffer of words: run with `-- --ignored --nocapture`.
#[test]
#[ignore]
fn timing_a_keystroke_in_a_snippet() {
    let dir = tmp("timing");
    let file = dir.join("words.txt");
    let text: String = (0..20_000)
        .map(|i| format!("alpha_{i} beta_{} gamma_{}\n", i % 97, i % 31))
        .collect();
    std::fs::write(&file, text).unwrap();
    // None; a snippet of one place a field; one whose field is in two
    // places, so each key edits two carets.
    for snippet in [
        None,
        Some("f(${1:x}, ${2:y})$0"),
        Some("f(${1:x}, ${2:y}, ${1})$0"),
    ] {
        let mut app = Kawoosh::from_file(&file);
        app.ed.settings.set(
            kawoosh_editor::Layer::Session,
            "completion.trigger",
            kawoosh_editor::Setting::Str("typing".into()),
        );
        let mut d = Drive::new(900.0, 500.0);
        d.frame(&mut app);
        d.press(&mut app, "Go");
        // The same text either way, the caret mid-line after `f(`, so
        // the look (the popup mid-line) is the same and the snippet's
        // own cost is the difference.
        match snippet {
            Some(body) => {
                app.snippet_expand(body);
            }
            None => {
                d.keys(&mut app, "f(x, y)");
                d.press(&mut app, "<Left><Left><Left><Left>");
            }
        }
        d.settle(&mut app);
        let mut keys = Vec::new();
        for round in 0..40 {
            for ch in [" ", "g", "a", "m", "m"] {
                let t = std::time::Instant::now();
                d.keys(&mut app, ch);
                let took = t.elapsed();
                d.settle(&mut app);
                if round > 0 && ch != "g" {
                    keys.push(took);
                }
            }
        }
        assert_eq!(app.snippet.is_some(), snippet.is_some());
        keys.sort();
        let (k, k90) = (keys[keys.len() / 2], keys[keys.len() * 9 / 10]);
        println!("snippet {snippet:?}: keystroke {k:?} (p90 {k90:?})");
    }
    std::fs::remove_dir_all(&dir).ok();
}
