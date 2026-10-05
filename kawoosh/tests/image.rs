//! The picture pane (`kawoosh/lua/image.lua` over `kawoosh.image` and
//! `pictures.rs`): a still fitted, zoomed and moved over; a picture
//! that moves played, held and stepped; a drawing opened as text and
//! drawn beside its source at the size it is shown; a file that
//! changed read again.

mod drive;

use std::path::{Path, PathBuf};

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::KeyMods;

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
}

fn lua(app: &mut Kawoosh, src: &str) -> String {
    app.run_lua_source("t", src);
    app.ed.message.clone()
}

/// A field of the pane's state, as Lua writes it; `nil` without one.
fn state(app: &mut Kawoosh, field: &str) -> String {
    lua(
        app,
        &format!(
            r#"local s = kawoosh.picture.state()
               kawoosh.echo(tostring(s and s.{field}))"#
        ),
    )
}

fn num(app: &mut Kawoosh, field: &str) -> f64 {
    let s = state(app, field);
    s.parse().unwrap_or_else(|_| panic!("{field} = {s}"))
}

fn frames(d: &mut Drive, app: &mut Kawoosh, n: usize) {
    for _ in 0..n {
        d.frame(app);
    }
}

fn png(path: &Path, w: u32, h: u32) {
    image::RgbaImage::from_pixel(w, h, image::Rgba([200, 40, 40, 255]))
        .save_with_format(path, image::ImageFormat::Png)
        .unwrap();
}

/// Three frames, each 20 ms.
fn gif(path: &Path) {
    let mut out = Vec::new();
    {
        let mut enc = image::codecs::gif::GifEncoder::new(&mut out);
        enc.set_repeat(image::codecs::gif::Repeat::Infinite)
            .unwrap();
        for px in [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]] {
            enc.encode_frame(image::Frame::from_parts(
                image::RgbaImage::from_pixel(8, 8, image::Rgba(px)),
                0,
                0,
                image::Delay::from_numer_denom_ms(20, 1),
            ))
            .unwrap();
        }
    }
    std::fs::write(path, out).unwrap();
}

fn svg(path: &Path, w: u32, h: u32) {
    std::fs::write(
        path,
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}">
  <rect width="{w}" height="{h}" fill="#3366ff"/>
</svg>
"##
        ),
    )
    .unwrap();
}

fn fixture(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("kawoosh-image-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    png(&root.join("a-wide.png"), 400, 200);
    png(&root.join("b-large.png"), 3000, 2400);
    gif(&root.join("c-moves.gif"));
    svg(&root.join("d-drawn.svg"), 20, 10);
    std::fs::write(root.join("e-not.png"), "no picture, whatever its name\n").unwrap();
    std::fs::write(root.join("plain.txt"), "only text\n").unwrap();
    kawoosh_systems::fs::canonicalize(&root).unwrap()
}

fn start(name: &str) -> (Drive, Kawoosh, PathBuf) {
    let root = fixture(name);
    let mut app = Kawoosh::new("t", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    let mut d = Drive::new(1600.0, 700.0);
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    (d, app, root)
}

fn open(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    ex(d, app, line);
    frames(d, app, 5);
}

#[test]
fn a_picture_opens_in_its_pane_fitted_zoomed_and_moved_over() {
    let (mut d, mut app, root) = start("still");
    // By its name, as any file is opened.
    open(
        &mut d,
        &mut app,
        &format!("e {}", root.join("a-wide.png").display()),
    );
    assert_eq!(state(&mut app, "format"), "png");
    assert_eq!(state(&mut app, "width"), "400");
    assert_eq!(state(&mut app, "height"), "200");
    assert_eq!(state(&mut app, "frames"), "1");
    assert_eq!(state(&mut app, "fitted"), "true");
    // Room for it: its own size, in the middle of the room.
    assert_eq!(num(&mut app, "zoom"), 1.0, "never larger than it is");
    let (x, room) = (num(&mut app, "x"), num(&mut app, "room_w"));
    assert!((x - (room - 400.0) / 2.0).abs() < 1.0, "{x} of {room}");
    // And drawn where it is said to be, in both axes.
    let (y, room_h) = (num(&mut app, "y"), num(&mut app, "room_h"));
    assert!((y - (room_h - 200.0) / 2.0).abs() < 1.0, "{y} of {room_h}");
    let dx = num(&mut app, "drawn_x");
    assert!(
        (dx - x).abs() <= 1.0,
        "drawn at {dx}, said {x}, room {room}"
    );
    assert!((num(&mut app, "drawn_y") - y).abs() <= 1.0);

    d.press(&mut app, "+");
    d.frame(&mut app);
    assert_eq!(num(&mut app, "zoom"), 1.25);
    assert_eq!(state(&mut app, "fitted"), "false");
    d.press(&mut app, "3-");
    d.frame(&mut app);
    assert!((num(&mut app, "zoom") - 1.25 / 1.25f64.powi(3)).abs() < 1e-9);
    d.press(&mut app, "0");
    d.frame(&mut app);
    assert_eq!(num(&mut app, "zoom"), 1.0);
    ex(&mut d, &mut app, "image zoom 800");
    frames(&mut d, &mut app, 2);
    assert_eq!(num(&mut app, "zoom"), 8.0);
    // Wider than the room now: its middle at the room's, and the keys
    // move over it, no further than its edges.
    let (x, w, room) = (
        num(&mut app, "x"),
        num(&mut app, "w"),
        num(&mut app, "room_w"),
    );
    assert_eq!(w, 3200.0);
    assert!((x - (room - w) / 2.0).abs() < 1.0, "{x}");
    d.press(&mut app, "l");
    d.frame(&mut app);
    assert!((num(&mut app, "x") - (x - room / 8.0)).abs() < 1.0);
    d.press(&mut app, "99l");
    frames(&mut d, &mut app, 2);
    assert!(
        (num(&mut app, "x") - (room - w)).abs() < 1.0,
        "its right edge"
    );
    assert!(
        (num(&mut app, "drawn_x") - (room - w)).abs() <= 1.0,
        "drawn there"
    );
    d.press(&mut app, "99h");
    d.frame(&mut app);
    assert_eq!(num(&mut app, "x"), 0.0, "its left edge");
    ex(&mut d, &mut app, "image zoom 100000");
    frames(&mut d, &mut app, 2);
    assert_eq!(num(&mut app, "zoom"), 64.0, "no further");
    d.press(&mut app, "f");
    d.frame(&mut app);
    assert_eq!(state(&mut app, "fitted"), "true");
    assert_eq!(num(&mut app, "zoom"), 1.0);

    // One larger than the room is fitted whole.
    open(
        &mut d,
        &mut app,
        &format!("image {}", root.join("b-large.png").display()),
    );
    assert_eq!(state(&mut app, "width"), "3000");
    let (z, w, h) = (
        num(&mut app, "zoom"),
        num(&mut app, "w"),
        num(&mut app, "h"),
    );
    assert!(z < 1.0, "{z}");
    assert!(w <= num(&mut app, "room_w") + 0.5 && h <= num(&mut app, "room_h") + 0.5);
    assert_eq!(lua(&mut app, "kawoosh.echo(#kawoosh.picture.panes())"), "1");

    // The ground, and the pane closed.
    assert_eq!(state(&mut app, "backdrop"), "none");
    d.press(&mut app, "b");
    assert_eq!(state(&mut app, "backdrop"), "light");
    d.press(&mut app, "bb");
    assert_eq!(state(&mut app, "backdrop"), "none");
    d.press(&mut app, "q");
    frames(&mut d, &mut app, 2);
    assert_eq!(state(&mut app, "path"), "nil");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_picture_that_moves_plays_is_held_and_stepped() {
    let (mut d, mut app, root) = start("moves");
    open(
        &mut d,
        &mut app,
        &format!("e {}", root.join("c-moves.gif").display()),
    );
    assert_eq!(state(&mut app, "format"), "gif");
    assert_eq!(state(&mut app, "frames"), "3");
    // It plays: each frame its 20 ms, round the end.
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..12 {
        std::thread::sleep(std::time::Duration::from_millis(25));
        frames(&mut d, &mut app, 2);
        seen.insert(state(&mut app, "frame"));
    }
    assert_eq!(seen.len(), 3, "every frame shown: {seen:?}");
    // Held, it stays.
    d.press(&mut app, "p");
    frames(&mut d, &mut app, 2);
    assert_eq!(state(&mut app, "paused"), "true");
    let at = state(&mut app, "frame");
    std::thread::sleep(std::time::Duration::from_millis(60));
    frames(&mut d, &mut app, 3);
    assert_eq!(state(&mut app, "frame"), at);
    // Stepped, a frame on and back, round the ends.
    let at: i64 = at.parse().unwrap();
    d.press(&mut app, ".");
    frames(&mut d, &mut app, 3);
    assert_eq!(state(&mut app, "frame"), (at % 3 + 1).to_string());
    d.press(&mut app, "2,");
    frames(&mut d, &mut app, 3);
    assert_eq!(state(&mut app, "frame"), ((at + 1) % 3 + 1).to_string());
    // Let go, it moves again.
    d.press(&mut app, "p");
    let held = state(&mut app, "frame");
    let mut moved = false;
    for _ in 0..8 {
        std::thread::sleep(std::time::Duration::from_millis(25));
        frames(&mut d, &mut app, 2);
        moved |= state(&mut app, "frame") != held;
    }
    assert!(moved);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_drawing_is_text_first_and_drawn_beside_it_at_the_size_shown() {
    let (mut d, mut app, root) = start("drawn");
    let path = root.join("d-drawn.svg");
    open(&mut d, &mut app, &format!("e {}", path.display()));
    assert_eq!(state(&mut app, "path"), "nil", "a drawing opens as text");
    assert!(lua(&mut app, "kawoosh.echo(kawoosh.buf.text())").contains("<svg"));
    open(&mut d, &mut app, "image");
    assert_eq!(state(&mut app, "format"), "svg");
    assert_eq!(state(&mut app, "vector"), "true");
    assert_eq!(state(&mut app, "width"), "20");
    // Fitted to the room, larger than it says it is, and drawn at the
    // pixels that takes.
    let (z, w) = (num(&mut app, "zoom"), num(&mut app, "w"));
    assert!(z > 1.0, "{z}");
    assert_eq!(num(&mut app, "pixel_width"), (w * 2.0).ceil());
    d.press(&mut app, "0");
    frames(&mut d, &mut app, 4);
    assert_eq!(num(&mut app, "zoom"), 1.0);
    assert_eq!(num(&mut app, "pixel_width"), 40.0, "drawn again, smaller");
    assert_eq!(num(&mut app, "pixel_height"), 20.0);
    // The file written again is read again, at the size it is shown.
    // (Another size on disk: a stamp is the size and the second.)
    svg(&path, 300, 30);
    frames(&mut d, &mut app, 4);
    assert_eq!(state(&mut app, "width"), "300");
    assert_eq!(num(&mut app, "pixel_width"), 600.0);
    assert_eq!(num(&mut app, "pixel_height"), 60.0);
    // Broken, it says so; mended, it is drawn.
    std::fs::write(&path, "<svg").unwrap();
    frames(&mut d, &mut app, 4);
    assert_eq!(state(&mut app, "width"), "nil");
    svg(&path, 12, 12);
    frames(&mut d, &mut app, 5);
    assert_eq!(state(&mut app, "width"), "12");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_folder_s_pictures_in_turn_and_a_file_as_what_it_is() {
    let (mut d, mut app, root) = start("turn");
    let name = |app: &mut Kawoosh| {
        let p = state(app, "path");
        p.rsplit(['/', '\\']).next().unwrap_or("").to_string()
    };
    open(
        &mut d,
        &mut app,
        &format!("e {}", root.join("a-wide.png").display()),
    );
    assert_eq!(name(&mut app), "a-wide.png");
    d.press(&mut app, "n");
    frames(&mut d, &mut app, 4);
    assert_eq!(name(&mut app), "b-large.png");
    d.press(&mut app, "2n");
    frames(&mut d, &mut app, 4);
    assert_eq!(name(&mut app), "d-drawn.svg");
    assert_eq!(state(&mut app, "format"), "svg");
    d.press(&mut app, "n");
    frames(&mut d, &mut app, 4);
    // Named a picture and none: said, not drawn.
    assert_eq!(name(&mut app), "e-not.png");
    assert_eq!(state(&mut app, "width"), "nil");
    d.press(&mut app, "n");
    frames(&mut d, &mut app, 4);
    assert_eq!(name(&mut app), "a-wide.png", "round the folder's end");
    d.press(&mut app, "N");
    frames(&mut d, &mut app, 4);
    assert_eq!(name(&mut app), "e-not.png");
    // As what it is under the picture: this one is text.
    d.press(&mut app, "t");
    frames(&mut d, &mut app, 4);
    assert!(lua(&mut app, "kawoosh.echo(kawoosh.buf.text())").contains("no picture"));
    // A picture's are bytes: the bytes pane's.
    open(
        &mut d,
        &mut app,
        &format!("image {}", root.join("a-wide.png").display()),
    );
    d.press(&mut app, "t");
    frames(&mut d, &mut app, 4);
    assert!(
        lua(
            &mut app,
            "local s = kawoosh.hex.state() kawoosh.echo(tostring(s and s.path))"
        )
        .ends_with("a-wide.png")
    );
    let _ = std::fs::remove_dir_all(&root);
}
