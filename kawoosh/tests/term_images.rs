//! Kitty's images in a terminal pane (roadmap step 56,
//! docs/design/kitty-graphics.md): a placement drawn at its cell at the
//! size its keys ask for, over the text — or under it for a negative
//! `z`, the grid then a float of its own at the same place — and gone
//! with its line or a clear.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::{NodeKind, Rect};

fn apc(keys: &str, data: &[u8]) -> Vec<u8> {
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode(data);
    format!("\x1b_G{keys};{b64}\x1b\\").into_bytes()
}

fn nodes(d: &Drive, kind: NodeKind) -> Vec<(usize, Rect)> {
    d.core
        .nodes()
        .into_iter()
        .enumerate()
        .filter(|(_, n)| n.kind == kind)
        .map(|(i, n)| (i, n.rect))
        .collect()
}

fn settle(d: &mut Drive, app: &mut Kawoosh, t: u64) {
    app.terms.map.get_mut(&t).unwrap().settle_graphics();
    d.frame(app);
    d.frame(app);
}

#[test]
fn an_image_is_drawn_at_its_cell_over_or_under_the_text() {
    let mut app = Kawoosh::new("t", "");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    let t = app.add_headless_terminal();
    d.frame(&mut app);
    let (cw, ch) = app.grid_cell_metrics();
    // Two cells by one, at the third column of the second row.
    app.feed_terminal(t, b"one\r\nab");
    app.feed_terminal(t, &apc("a=T,f=24,s=4,v=2,i=1,c=2,r=1,q=2", &[9; 4 * 2 * 3]));
    settle(&mut d, &mut app, t);
    let (_, cells) = nodes(&d, NodeKind::Cells)[0];
    let images = nodes(&d, NodeKind::Image);
    assert_eq!(images.len(), 1);
    let (i_img, r) = images[0];
    assert!(
        (r.x - (cells.x + 2.0 * cw)).abs() < 0.01,
        "{r:?} in {cells:?}"
    );
    assert!((r.y - (cells.y + ch)).abs() < 0.01);
    assert!((r.w - 2.0 * cw).abs() < 0.01 && (r.h - ch).abs() < 0.01);
    let (i_cells, _) = nodes(&d, NodeKind::Cells)[0];
    assert!(i_img > i_cells, "over the text: opened after the grid");
    // Under the text: before the grid, which stays where it was.
    app.feed_terminal(t, &apc("a=p,i=1,z=-1,p=7,q=2", &[]));
    d.frame(&mut app);
    d.frame(&mut app);
    let (i_cells, moved) = nodes(&d, NodeKind::Cells)[0];
    assert_eq!(moved, cells, "the grid floated at its own place");
    let images = nodes(&d, NodeKind::Image);
    assert_eq!(images.len(), 2);
    assert!(images[0].0 < i_cells && images[1].0 > i_cells);
    // A clear takes them; so does scrolling their lines away.
    app.feed_terminal(t, b"\x1b[2J");
    d.frame(&mut app);
    assert!(nodes(&d, NodeKind::Image).is_empty());
    app.feed_terminal(t, &apc("a=p,i=1,q=2", &[]));
    d.frame(&mut app);
    assert_eq!(nodes(&d, NodeKind::Image).len(), 1);
    app.feed_terminal(t, "\r\n".repeat(60).as_bytes());
    d.frame(&mut app);
    assert!(nodes(&d, NodeKind::Image).is_empty(), "scrolled off");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// The pty is told the cell's size in pixels, which is what a program
/// sizes an image by: `CSI 14 t` answers with the grid's.
#[test]
fn the_pixel_size_follows_the_cell() {
    let mut app = Kawoosh::new("t", "");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    let t = app.add_headless_terminal();
    d.frame(&mut app);
    d.frame(&mut app);
    let (cw, ch) = app.cell_metrics();
    let term = app.terms.map.get_mut(&t).unwrap();
    let size = term.size();
    term.take_sent();
    term.feed(b"\x1b[14t");
    let want = format!(
        "\x1b[4;{};{}t",
        size.rows as u32 * ch.round() as u32,
        size.cols as u32 * cw.round() as u32
    );
    assert_eq!(String::from_utf8(term.take_sent()).unwrap(), want);
}
