//! The themes pane's cards (docs/design/themes.md Decision 4): each one
//! outlined whole. kui's `border` insets nothing, so a card's sections
//! that paint a background — its title strip, the code's selected line,
//! the status strip — covered the ring where they ran, and the cursor's
//! accent outline showed only beside the code and the swatches
//! (reported 2026-10-02: "fix theme panel outline").

mod drive;

use std::collections::HashMap;

use drive::Drive;
use kawoosh::Kawoosh;

#[test]
fn every_card_is_outlined_whole_with_nothing_painted_over_its_ring() {
    let mut app = Kawoosh::new("t", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    let mut d = Drive::new(1300.0, 800.0);
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    d.press(&mut app, "<leader>oo");
    for _ in 0..3 {
        d.frame(&mut app);
    }
    let nodes = d.core.nodes();
    let by_key: HashMap<_, _> = nodes.iter().map(|n| (n.key, n)).collect();
    // Whether `n` is drawn inside `card`, by its parents.
    let under = |n: &kui_native::NodeInfo, card| {
        let mut at = n.parent;
        while let Some(k) = at {
            if k == card {
                return true;
            }
            at = by_key.get(&k).and_then(|p| p.parent);
        }
        false
    };
    let cards: Vec<_> = nodes
        .iter()
        .filter(|n| {
            n.label
                .as_deref()
                .is_some_and(|l| l.starts_with("theme card "))
        })
        .collect();
    assert!(cards.len() >= 3, "cards on screen: {}", cards.len());
    // One ring width for every card, so the content does not move as
    // the cursor walks onto a card and off it.
    let ring = cards[0].border_w;
    assert!(ring > 0.0, "a card is outlined");
    for card in &cards {
        let name = card.label.as_deref().unwrap();
        assert_eq!(card.border_w, ring, "{name}'s ring as wide as the others'");
        let r = card.rect;
        let (x0, y0, x1, y1) = (r.x + ring, r.y + ring, r.x + r.w - ring, r.y + r.h - ring);
        // The header, the sample, the status strip and the swatches.
        let sections: Vec<_> = nodes
            .iter()
            .filter(|n| n.parent == Some(card.key))
            .collect();
        assert_eq!(sections.len(), 4, "{name}'s sections");
        for n in nodes.iter().filter(|n| under(n, card.key)) {
            let painted = n.bg.a > 0.0;
            if !(painted || sections.iter().any(|s| s.key == n.key)) {
                continue;
            }
            let q = n.rect;
            assert!(
                q.x >= x0 - 0.01
                    && q.y >= y0 - 0.01
                    && q.x + q.w <= x1 + 0.01
                    && q.y + q.h <= y1 + 0.01,
                "{name}: a {} at {q:?} over the ring of {r:?}",
                n.label.as_deref().unwrap_or("section")
            );
            if !painted {
                continue;
            }
            // A background in a corner of the inside rounds it as the
            // ring's inner edge does, or its square corner covers the
            // ring's curve there.
            let inner = card.radius.map(|c| (c - ring).max(0.0));
            let corners = [
                (q.x <= x0 + 0.01 && q.y <= y0 + 0.01),
                (q.x + q.w >= x1 - 0.01 && q.y <= y0 + 0.01),
                (q.x + q.w >= x1 - 0.01 && q.y + q.h >= y1 - 0.01),
                (q.x <= x0 + 0.01 && q.y + q.h >= y1 - 0.01),
            ];
            for (i, at) in corners.iter().enumerate() {
                if *at && inner[i] > 0.0 {
                    assert!(
                        (n.radius[i] - inner[i]).abs() < 0.01,
                        "{name}: a background's corner {i} rounded {} in a ring rounded {} inside",
                        n.radius[i],
                        inner[i]
                    );
                }
            }
        }
    }
    assert_eq!(d.warnings(), Vec::<String>::new());
}
