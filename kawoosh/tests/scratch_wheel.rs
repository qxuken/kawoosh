mod drive;
use drive::Drive;
use kawoosh::Kawoosh;
#[test]
fn wheel_still_scrolls_lines() {
    let doc = (0..80).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
    let mut app = Kawoosh::new("t", &doc);
    let mut d = Drive::new(300.0, 200.0);
    d.frame(&mut app);
    d.wheel(&mut app, 150.0, 80.0, 0.0, -60.0);
    d.frame(&mut app);
    let top = app.ed.views[app.focused_view().unwrap()].top;
    println!("top after wheel: {top}");
}
