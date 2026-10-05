//! The bytes pane (`kawoosh/lua/hex.lua` over `kawoosh.fs.bytes` and
//! `kawoosh.fs.find`): a file's bytes in rows of offset, hex and text,
//! the cursor a cell walked by the keys and put by a click, an offset
//! gone to, bytes found, a selection copied — and a file that is not
//! text opened there rather than as a buffer.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::{InputEvent, KeyMods, Vec2};

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
}

fn lua(app: &mut Kawoosh, src: &str) -> String {
    app.run_lua_source("t", src);
    app.ed.message.clone()
}

/// `cursor top columns anchor order` of the pane with the keyboard.
fn shown(app: &mut Kawoosh) -> String {
    lua(
        app,
        r#"local s = kawoosh.hex.state()
           kawoosh.echo(s and string.format("%d %d %d %s %s", s.cursor, s.top, s.columns,
             tostring(s.anchor), s.order) or "nil")"#,
    )
}

fn cursor(app: &mut Kawoosh) -> u64 {
    shown(app)
        .split(' ')
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(u64::MAX)
}

/// The grid's rows as drawn, the columns' names first.
fn rows(app: &mut Kawoosh) -> Vec<String> {
    lua(
        app,
        r#"kawoosh.echo(table.concat(kawoosh.hex.state().lines, "\n"))"#,
    )
    .lines()
    .map(str::to_string)
    .collect()
}

fn text(app: &mut Kawoosh) -> String {
    lua(app, "kawoosh.echo(kawoosh.buf.text())")
}

fn fixture(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!("kawoosh-hex-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    // 4096 bytes: each its own offset's low byte, `EZNO` at the start
    // and `needle` at 1000.
    let mut bytes: Vec<u8> = (0..4096u32).map(|i| i as u8).collect();
    bytes[..5].copy_from_slice(b"EZNO\0");
    bytes[1000..1006].copy_from_slice(b"needle");
    bytes[2000..2004].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
    std::fs::write(root.join("blob.bin"), &bytes).unwrap();
    std::fs::write(root.join("plain.txt"), "only text\n").unwrap();
    let root = kawoosh_systems::fs::canonicalize(&root).unwrap();
    (root.clone(), root.join("blob.bin"))
}

fn open(name: &str) -> (Drive, Kawoosh, std::path::PathBuf) {
    let (root, blob) = fixture(name);
    let mut app = Kawoosh::new("t", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    let mut d = Drive::new(1600.0, 700.0);
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("hex {}", blob.display()));
    d.frame(&mut app);
    d.frame(&mut app);
    d.frame(&mut app);
    (d, app, root)
}

#[test]
fn the_bytes_pane_shows_rows_and_the_keys_walk_cells() {
    let (mut d, mut app, root) = open("walk");
    assert_eq!(shown(&mut app), "0 0 16 nil little");
    let grid = rows(&mut app);
    assert!(
        grid[0].contains("00 01 02 03 04 05 06 07  08 09 0A 0B 0C 0D 0E 0F"),
        "the columns' names: {:?}",
        grid.first()
    );
    assert_eq!(
        grid[1].trim_end(),
        "00000000  45 5A 4E 4F 00 05 06 07  08 09 0A 0B 0C 0D 0E 0F  EZNO............"
    );
    assert!(grid[3].starts_with("00000020  20 21 22"), "{}", grid[3]);
    assert!(
        grid[3].trim_end().ends_with(" !\"#$%&'()*+,-./"),
        "{}",
        grid[3]
    );
    // A byte, a row, a group of four, the row's ends, counts.
    d.press(&mut app, "l");
    assert_eq!(cursor(&mut app), 1);
    d.press(&mut app, "j");
    assert_eq!(cursor(&mut app), 17);
    d.press(&mut app, "w");
    assert_eq!(cursor(&mut app), 20);
    d.press(&mut app, "b");
    assert_eq!(cursor(&mut app), 16);
    d.press(&mut app, "lb");
    assert_eq!(cursor(&mut app), 16);
    d.press(&mut app, "$");
    assert_eq!(cursor(&mut app), 31);
    d.press(&mut app, "l");
    assert_eq!(cursor(&mut app), 32, "past a row's end is the next row");
    d.press(&mut app, "3j2l");
    assert_eq!(cursor(&mut app), 82);
    d.press(&mut app, "0");
    assert_eq!(cursor(&mut app), 80);
    d.press(&mut app, "k");
    assert_eq!(cursor(&mut app), 64);
    d.press(&mut app, "ggk");
    assert_eq!(cursor(&mut app), 0, "not past the start");
    d.press(&mut app, "hh");
    assert_eq!(cursor(&mut app), 0);
    // The end, shown: the last row is on show and the cursor on its
    // last byte.
    d.press(&mut app, "G");
    d.frame(&mut app);
    assert_eq!(cursor(&mut app), 4095);
    assert!(
        rows(&mut app)
            .iter()
            .any(|r| r.starts_with("00000FF0  F0 F1")),
        "the last row is drawn"
    );
    d.press(&mut app, "lj");
    assert_eq!(cursor(&mut app), 4095, "not past the end");
    // The count's byte, and an offset asked for.
    d.press(&mut app, "100go");
    assert_eq!(cursor(&mut app), 100);
    ex(&mut d, &mut app, "hex goto 0x3E8");
    assert_eq!(cursor(&mut app), 1000);
    ex(&mut d, &mut app, "hex goto +16");
    assert_eq!(cursor(&mut app), 1016);
    ex(&mut d, &mut app, "hex goto -0x10");
    assert_eq!(cursor(&mut app), 1000);
    ex(&mut d, &mut app, "hex goto 50%");
    assert_eq!(cursor(&mut app), 2048);
    ex(&mut d, &mut app, "hex goto nowhere");
    assert!(
        app.ed.message.contains("not an offset"),
        "{}",
        app.ed.message
    );
    d.frame(&mut app);
    assert!(
        rows(&mut app).iter().any(|r| r.starts_with("00000800  ")),
        "where it went is on show: {:?}",
        rows(&mut app)
    );
    // `go` alone asks.
    d.press(&mut app, "go");
    d.keys(&mut app, "8");
    d.key(&mut app, "enter", KeyMods::default());
    assert_eq!(cursor(&mut app), 8);
    assert_eq!(d.warnings(), Vec::<String>::new());
    d.press(&mut app, "q");
    assert_eq!(shown(&mut app), "nil");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn bytes_are_found_selected_and_copied() {
    let (mut d, mut app, root) = open("find");
    ex(&mut d, &mut app, "hex find needle");
    assert_eq!(cursor(&mut app), 1000);
    // Hex digits, spaced or not; round the end to the start.
    ex(&mut d, &mut app, "hex find 0xDEADBEEF");
    assert_eq!(cursor(&mut app), 2000);
    ex(&mut d, &mut app, "hex find 0x de ad be ef");
    assert_eq!(cursor(&mut app), 2000, "what starts under the cursor");
    d.press(&mut app, "n");
    assert_eq!(cursor(&mut app), 2000, "the only one, round the file");
    ex(&mut d, &mut app, "hex find EZNO");
    assert_eq!(cursor(&mut app), 0, "round the end");
    ex(&mut d, &mut app, "hex find \u{1}\u{2}");
    // Each offset's low byte: 01 02 is at 257, 513, …
    assert_eq!(cursor(&mut app), 257);
    d.press(&mut app, "n");
    assert_eq!(cursor(&mut app), 513);
    d.press(&mut app, "NN");
    assert_eq!(cursor(&mut app), 3841, "back round the start");
    ex(&mut d, &mut app, "hex find no such bytes");
    assert!(app.ed.message.contains("not found"), "{}", app.ed.message);
    assert_eq!(cursor(&mut app), 3841);
    // A selection the moves stretch, copied as hex and as text.
    ex(&mut d, &mut app, "hex goto 1000");
    d.press(&mut app, "v5l");
    assert_eq!(cursor(&mut app), 1005);
    assert!(
        shown(&mut app).ends_with(" 16 1000 little"),
        "{}",
        app.ed.message
    );
    d.press(&mut app, "y");
    assert_eq!(app.clipboard_last(), Some("6E 65 65 64 6C 65"));
    assert!(
        shown(&mut app).contains(" nil "),
        "copied, the selection is dropped"
    );
    d.press(&mut app, "v5hY");
    assert_eq!(app.clipboard_last(), Some("needle"));
    // The byte order the foot reads in.
    d.press(&mut app, "e");
    assert!(shown(&mut app).ends_with("big"));
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn a_click_puts_the_cursor_and_a_file_that_is_not_text_opens_here() {
    let (root, blob) = fixture("open");
    let mut app = Kawoosh::new("t", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    let mut d = Drive::new(1600.0, 700.0);
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    // Text opens as text.
    ex(
        &mut d,
        &mut app,
        &format!("e {}", root.join("plain.txt").display()),
    );
    d.frame(&mut app);
    assert_eq!(shown(&mut app), "nil");
    assert_eq!(text(&mut app), "only text\n");
    // What is not, here.
    ex(&mut d, &mut app, &format!("e {}", blob.display()));
    d.frame(&mut app);
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(shown(&mut app), "0 0 16 nil little");
    // A click on a byte, in the hex half and in the text half.
    let grid = d
        .core
        .nodes()
        .into_iter()
        .find(|n| n.kind == kui_native::NodeKind::Cells)
        .expect("the grid is one cells node");
    let lines = rows(&mut app);
    let (cw, ch) = (grid.rect.w / 76.0, grid.rect.h / lines.len() as f32);
    // Row 2 of the file (the grid's third), byte 5: column 10 + 5 * 3.
    d.click(&mut app, grid.rect.x + 25.5 * cw, grid.rect.y + 3.5 * ch);
    assert_eq!(cursor(&mut app), 0x25);
    // Row 0, byte 7 of the text half: column 60 + 7.
    d.click(&mut app, grid.rect.x + 67.5 * cw, grid.rect.y + 1.5 * ch);
    assert_eq!(cursor(&mut app), 7);
    // The names' row and the offsets are no byte.
    d.click(&mut app, grid.rect.x + 25.5 * cw, grid.rect.y + 0.5 * ch);
    d.click(&mut app, grid.rect.x + 2.5 * cw, grid.rect.y + 3.5 * ch);
    assert_eq!(cursor(&mut app), 7);
    // The wheel moves the rows by whole ones, the cursor left alone;
    // a key brings the rows back to it.
    d.input(
        &mut app,
        InputEvent::CursorMoved(Vec2::new(grid.rect.x + 100.0, grid.rect.y + 100.0)),
    );
    d.input(
        &mut app,
        InputEvent::ScrollGesture {
            delta: Vec2::new(0.0, -ch * 10.0),
            begins: true,
        },
    );
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(shown(&mut app), "7 10 16 nil little", "ten rows on");
    assert!(rows(&mut app)[1].starts_with("000000A0  "));
    d.press(&mut app, "l");
    d.frame(&mut app);
    assert_eq!(shown(&mut app), "8 0 16 nil little");
    // A drag selects from the byte pressed to the one under the
    // pointer: row 0's byte 2 to row 1's byte 3.
    let at = |col: f32, row: f32| Vec2::new(grid.rect.x + col * cw, grid.rect.y + row * ch);
    d.input(&mut app, InputEvent::CursorMoved(at(16.5, 1.5)));
    d.input(&mut app, InputEvent::mouse_down(1));
    d.input(&mut app, InputEvent::CursorMoved(at(19.5, 2.5)));
    d.input(&mut app, InputEvent::mouse_up());
    d.frame(&mut app);
    assert_eq!(shown(&mut app), "19 0 16 2 little");
    d.press(&mut app, "<Esc>");
    assert_eq!(shown(&mut app), "19 0 16 nil little");
    assert_eq!(d.warnings(), Vec::<String>::new());
    // `t`: as text after all.
    d.press(&mut app, "t");
    d.frame(&mut app);
    assert!(text(&mut app).starts_with("EZNO"), "opened as a buffer");
    std::fs::remove_dir_all(&root).ok();
}

/// `changed mode side` of the pane with the keyboard.
fn editing(app: &mut Kawoosh) -> String {
    lua(
        app,
        r#"local s = kawoosh.hex.state()
           kawoosh.echo(string.format("%d %s %s", s.changed, tostring(s.mode), s.side))"#,
    )
}

/// Bytes are written over in either half, undone, found among, and
/// reach the disk only when written — in place, the file as long as it
/// was — and not over a file that changed meanwhile.
#[test]
fn bytes_are_written_over_undone_and_saved_in_place() {
    let (mut d, mut app, root) = open("edit");
    let blob = root.join("blob.bin");
    let before = std::fs::read(&blob).unwrap();
    // `r` and two digits: one byte, the cursor where it was.
    d.press(&mut app, "lrf");
    assert_eq!(
        editing(&mut app),
        "1 one hex",
        "the first digit shows at once"
    );
    d.press(&mut app, "F");
    assert_eq!(editing(&mut app), "1 nil hex");
    assert_eq!(cursor(&mut app), 1);
    d.frame(&mut app);
    assert!(
        rows(&mut app)[1].starts_with("00000000  45 FF 4E 4F"),
        "{}",
        rows(&mut app)[1]
    );
    assert_eq!(std::fs::read(&blob).unwrap(), before, "not on the disk yet");
    // One undo for the two digits, and a redo.
    d.press(&mut app, "u");
    assert_eq!(editing(&mut app), "0 nil hex");
    d.press(&mut app, "<C-r>");
    assert_eq!(editing(&mut app), "1 nil hex");
    // `R` in the text half: characters, the cursor going on; a
    // backspace takes the last back; a letter that is a key of the
    // pane's is typed, not run.
    ex(&mut d, &mut app, "hex goto 16");
    d.press(&mut app, "<Tab>Rhjq<BS>");
    assert_eq!(editing(&mut app), "3 replace text");
    assert_eq!(cursor(&mut app), 18);
    d.press(&mut app, "<Esc>");
    assert_eq!(editing(&mut app), "3 nil text");
    d.frame(&mut app);
    assert!(
        rows(&mut app)[2].starts_with("00000010  68 6A 12 13"),
        "{}",
        rows(&mut app)[2]
    );
    // `R` in the hex half: a byte each two digits, a key that is no
    // digit nothing; the byte typed as it was is no change.
    d.press(&mut app, "<Tab>");
    ex(&mut d, &mut app, "hex goto 0x20");
    d.press(&mut app, "R00zz2122<Esc>");
    assert_eq!(cursor(&mut app), 0x23);
    assert_eq!(editing(&mut app), "4 nil hex", "0x21 as 21 is no change");
    let changes = |app: &mut Kawoosh| {
        lua(
            app,
            r#"local out = {}
               for _, c in ipairs(kawoosh.hex.changes(kawoosh.hex.state().path)) do
                 out[#out + 1] = string.format("%X:%02X>%02X", c.at, c.disk, c.new)
               end
               kawoosh.echo(table.concat(out, " "))"#,
        )
    };
    assert_eq!(changes(&mut app), "1:5A>FF 10:10>68 11:11>6A 20:20>00");
    // The changes are walked, and found among: `hj` is nowhere on the
    // disk, and `EZNO` is no longer in the file as it is to be.
    d.press(&mut app, "gg]c");
    assert_eq!(cursor(&mut app), 1);
    d.press(&mut app, "]c]c]c]c");
    assert_eq!(cursor(&mut app), 1, "round the end");
    d.press(&mut app, "[c");
    assert_eq!(cursor(&mut app), 0x20);
    ex(&mut d, &mut app, "hex find hj");
    assert_eq!(cursor(&mut app), 0x10);
    ex(&mut d, &mut app, "hex find EZNO");
    assert!(app.ed.message.contains("not found"), "{}", app.ed.message);
    // A copy is of the bytes as they are to be.
    ex(&mut d, &mut app, "hex goto 0");
    d.press(&mut app, "vly");
    assert_eq!(app.clipboard_last(), Some("45 FF"));
    // Written: those bytes alone, the file as long as it was.
    assert_eq!(std::fs::read(&blob).unwrap(), before, "not on the disk yet");
    d.press(&mut app, "<C-s>");
    assert!(
        app.ed.message.contains("4 bytes written"),
        "{}",
        app.ed.message
    );
    let mut want = before.clone();
    (want[1], want[0x10], want[0x11], want[0x20]) = (0xFF, b'h', b'j', 0);
    assert_eq!(std::fs::read(&blob).unwrap(), want);
    assert_eq!(editing(&mut app), "0 nil hex");
    // An undo past the write is a change again.
    d.press(&mut app, "u");
    assert_eq!(changes(&mut app), "20:00>20");
    // The file changed on disk since: the write is refused, `!` writes.
    want[4000] = 0xAB;
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&blob, &want).unwrap();
    let later =
        std::fs::metadata(&blob).unwrap().modified().unwrap() + std::time::Duration::from_secs(5);
    std::fs::File::options()
        .write(true)
        .open(&blob)
        .unwrap()
        .set_modified(later)
        .unwrap();
    ex(&mut d, &mut app, "hex write");
    assert!(
        app.ed.message.contains("changed on disk"),
        "{}",
        app.ed.message
    );
    assert_eq!(std::fs::read(&blob).unwrap(), want);
    ex(&mut d, &mut app, "hex write!");
    want[0x20] = 0x20;
    assert_eq!(std::fs::read(&blob).unwrap(), want, "{}", app.ed.message);
    // Every change dropped at once, and back by one undo.
    d.press(&mut app, "rAr0");
    ex(&mut d, &mut app, "hex goto 9");
    d.press(&mut app, "r00");
    assert_eq!(editing(&mut app), "2 nil hex");
    ex(&mut d, &mut app, "hex revert");
    assert_eq!(editing(&mut app), "0 nil hex");
    d.press(&mut app, "u");
    assert_eq!(editing(&mut app), "2 nil hex");
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&root).ok();
}

/// What is not written is kept: by the pane's close, and by the store
/// past the editor.
#[test]
fn changes_not_written_are_kept_past_the_pane_and_the_editor() {
    let (root, blob) = fixture("kept");
    let db = root.join("state.db");
    let launch = |db: &std::path::Path| {
        let mut app = Kawoosh::new("t", "");
        app.jobs_inline = true;
        let ext = app.attach_lua().unwrap();
        let mut d = Drive::new(1600.0, 700.0);
        d.extension("lua", ext).unwrap();
        app.open_store(Some(db));
        d.frame(&mut app);
        (d, app)
    };
    let show = |d: &mut Drive, app: &mut Kawoosh| {
        ex(d, app, &format!("hex {}", blob.display()));
        for _ in 0..3 {
            d.frame(app);
        }
    };
    let (mut d, mut app) = launch(&db);
    show(&mut d, &mut app);
    d.press(&mut app, "llrAB");
    assert_eq!(editing(&mut app), "1 nil hex");
    d.press(&mut app, "q");
    assert!(app.ed.message.contains("kept"), "{}", app.ed.message);
    show(&mut d, &mut app);
    assert_eq!(editing(&mut app), "1 nil hex", "the pane closed and opened");
    drop((d, app));
    let (mut d, mut app) = launch(&db);
    show(&mut d, &mut app);
    assert_eq!(editing(&mut app), "1 nil hex", "another run of the editor");
    assert!(
        rows(&mut app)[1].starts_with("00000000  45 5A AB 4F"),
        "{}",
        rows(&mut app)[1]
    );
    d.press(&mut app, "<C-s>");
    assert_eq!(std::fs::read(&blob).unwrap()[2], 0xAB, "{}", app.ed.message);
    drop((d, app));
    let (mut d, mut app) = launch(&db);
    show(&mut d, &mut app);
    assert_eq!(editing(&mut app), "0 nil hex", "written, nothing is kept");
    std::fs::remove_dir_all(&root).ok();
}

/// The rows on show are counted from the room the grid was laid out
/// in, not guessed from the pane's height: as many as fit, and no
/// fewer.
#[test]
fn the_rows_are_counted_from_the_grid_s_own_room() {
    let (mut d, mut app, root) = open("room");
    d.frame(&mut app);
    assert_eq!(
        lua(
            &mut app,
            "kawoosh.echo(tostring(kawoosh.hex.state().measured))"
        ),
        "true"
    );
    let _ = std::fs::remove_dir_all(&root);
}
