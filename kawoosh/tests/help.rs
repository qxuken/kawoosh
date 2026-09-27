//! The help (roadmap step 53, `kawoosh/src/help.rs`): `:help` and a
//! topic's page, read-only and rendered; a command and a key found in
//! the generated pages; `gx` from page to page; `:tutor`; and every link
//! in every page reaching a page and a heading that exist.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::help::PAGES;
use kui_native::KeyMods;

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
}

fn app_with_lua(d: &mut Drive) -> Kawoosh {
    let mut app = Kawoosh::new("t", "text");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    app
}

fn settle(d: &mut Drive, app: &mut Kawoosh) {
    for _ in 0..40 {
        app.wait_for_open();
        d.frame(app);
        if app
            .focused_view()
            .is_some_and(|v| app.ed.buffer_of(v).loading.is_none())
        {
            break;
        }
    }
}

/// The page on show and the caret's line in it.
fn shown(app: &Kawoosh) -> (String, String) {
    let v = app.focused_view().unwrap();
    let b = app.ed.buffer_of(v);
    let line = b.line_of(app.ed.views[v].sels.primary().head);
    (b.name.clone(), b.slice(b.line_range(line)))
}

#[test]
fn help_opens_a_topic_read_only_and_links_follow() {
    let mut d = Drive::new(1000.0, 700.0);
    let mut app = app_with_lua(&mut d);
    ex(&mut d, &mut app, "help");
    settle(&mut d, &mut app);
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.buffer_of(v).name, "index.md");
    assert!(app.ed.buffer_of(v).read_only, "a page is read-only");
    // A page by name, a command and a key in the generated pages.
    ex(&mut d, &mut app, "help panes");
    settle(&mut d, &mut app);
    assert_eq!(shown(&app).0, "panes.md");
    ex(&mut d, &mut app, "help open link");
    settle(&mut d, &mut app);
    let (page, line) = shown(&app);
    assert_eq!(page, "commands.md");
    assert!(line.starts_with("- `:open link`"), "{line}");
    ex(&mut d, &mut app, "help gx");
    settle(&mut d, &mut app);
    let (page, line) = shown(&app);
    assert_eq!(page, "keys.md");
    assert!(line.starts_with("- `gx` — `:open link`"), "{line}");
    // What a plugin added is there: the fonts pane's command.
    ex(&mut d, &mut app, "help fonts");
    settle(&mut d, &mut app);
    assert_eq!(shown(&app).0, "commands.md");
    // `gx` on a link in the index follows it.
    ex(&mut d, &mut app, "help");
    settle(&mut d, &mut app);
    d.keys(&mut app, "/");
    d.keys(&mut app, "files.md");
    d.key(&mut app, "enter", KeyMods::default());
    d.press(&mut app, "gx");
    settle(&mut d, &mut app);
    assert_eq!(shown(&app).0, "files.md", "{}", app.ed.message);
    ex(&mut d, &mut app, "help no such thing at all");
    assert_eq!(app.ed.message, "no help for no such thing at all");
    // The tutorial is a scratch to edit.
    ex(&mut d, &mut app, "tutor");
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.buffer_of(v).name, "tutor");
    assert!(!app.ed.buffer_of(v).read_only);
    assert!(app.ed.buffer_of(v).line_count() > 50);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// Every link in every page reaches a page there is — one of the
/// shipped ones or a generated one — and, when it names a heading, a
/// heading that page has; the tutorial's too.
#[test]
fn every_link_in_the_pages_reaches_a_page_and_a_heading() {
    let pages: Vec<(&str, &str)> = PAGES
        .iter()
        .copied()
        .chain([("tutor", kawoosh::help::TUTOR)])
        .collect();
    let anchors = |text: &str| -> Vec<String> {
        text.lines()
            .filter(|l| l.starts_with('#'))
            .map(|l| kawoosh::markdown::slug(l.trim_start_matches('#').trim()))
            .collect()
    };
    let mut bad = Vec::new();
    for (name, text) in &pages {
        let mut rest = *text;
        while let Some(i) = rest.find("](") {
            rest = &rest[i + 2..];
            let Some(end) = rest.find(')') else { break };
            let dest = &rest[..end];
            rest = &rest[end..];
            if dest.contains("://") || dest.starts_with("mailto:") {
                continue;
            }
            let (file, anchor) = dest.split_once('#').unwrap_or((dest, ""));
            let target = if file.is_empty() {
                Some(*text)
            } else {
                let stem = file.trim_end_matches(".md");
                if stem == "commands" || stem == "keys" {
                    None
                } else {
                    match pages.iter().find(|(n, _)| *n == stem) {
                        Some((_, t)) => Some(*t),
                        None => {
                            bad.push(format!("{name}.md: no page {dest}"));
                            continue;
                        }
                    }
                }
            };
            if !anchor.is_empty()
                && let Some(t) = target
                && !anchors(t).iter().any(|a| a == anchor)
            {
                bad.push(format!("{name}.md: no heading {dest}"));
            }
        }
    }
    assert_eq!(bad, Vec::<String>::new());
}
