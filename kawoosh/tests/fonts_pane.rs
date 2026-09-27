//! The fonts pane's cards are drawn whole (docs/design/fonts.md): each
//! card on screen holds its family's name and its sample, in the family
//! itself. A card was its frame alone from 2026-09-27 (a handle check
//! left behind when the cards began to name their family) until this
//! test.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;

#[test]
fn every_card_on_screen_holds_its_name_and_its_sample() {
    let mut app = Kawoosh::new("t", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    let mut d = Drive::new(1300.0, 800.0);
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    d.press(&mut app, "<leader>of");
    d.press(&mut app, "m");
    for _ in 0..3 {
        d.frame(&mut app);
    }
    let cards: Vec<String> = d
        .core
        .nodes()
        .iter()
        .filter_map(|n| n.label.clone())
        .filter(|l| l.starts_with("card "))
        .collect();
    assert!(cards.len() >= 3, "cards on screen: {cards:?}");
    for card in &cards {
        let texts = d.texts_under(card).join("");
        let name = &card["card ".len()..];
        assert!(texts.contains(name), "{card}: {texts:?}");
        assert!(texts.contains("greet"), "{card}'s sample: {texts:?}");
    }
    assert_eq!(d.warnings(), Vec::<String>::new());
}
