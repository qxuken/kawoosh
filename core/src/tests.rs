// Damage is a slice of ranges; a one-element expectation is not a typo.
#![allow(clippy::single_range_in_vec_init)]

use super::*;

fn rgb(value: u32) -> Option<Rgba> {
    Some(Rgba(value))
}

fn fg(value: u32) -> HighlightStyle {
    HighlightStyle {
        fg: rgb(value),
        ..Default::default()
    }
}

fn bg(value: u32) -> HighlightStyle {
    HighlightStyle {
        bg: rgb(value),
        ..Default::default()
    }
}

fn chunk_shape(core: &Core, buffer: BufferId) -> Vec<(Range<usize>, HighlightStyle)> {
    let len = core.buffer(buffer).unwrap().len();
    core.chunks(buffer, 0..len)
        .map(|chunk| (chunk.range, chunk.style))
        .collect()
}

/// Stand in for a provider that has caught up: claim the whole buffer, which
/// clears the layer's outstanding damage.
fn settle(core: &mut Core, buffer: BufferId, layer: LayerId) {
    let version = core.buffer(buffer).unwrap().version();
    let len = core.buffer(buffer).unwrap().len();
    core.apply(buffer, Update::new(layer, version, 0..len))
        .unwrap();
}

fn text_of(core: &Core, buffer: BufferId) -> Vec<u8> {
    let buf = core.buffer(buffer).unwrap();
    let mut out = Vec::new();
    buf.read_into(0..buf.len(), &mut out);
    out
}

// -- chunking -------------------------------------------------------------

#[test]
fn chunks_follow_metadata_boundaries_and_gaps() {
    let mut core = Core::default();
    let layer = core.create_layer(LayerSpec::new("syntax"));
    let highlight = core.create_highlight(Highlight::styled(fg(0xff0000)));
    let buffer = core.create_buffer();

    core.set_text(buffer, b"hello world");
    core.set_highlight(buffer, layer, 0..5, Some(highlight));

    let chunks: Vec<_> = core.chunks(buffer, 0..11).collect();
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].range, 0..5);
    assert_eq!(chunks[0].highlights, vec![(layer, highlight)]);
    assert_eq!(chunks[1].range, 5..11);
    assert!(chunks[1].highlights.is_empty());
}

#[test]
fn chunks_zip_overlapping_layers_at_every_boundary() {
    let mut core = Core::default();
    let first = core.create_layer(LayerSpec::new("a"));
    let second = core.create_layer(LayerSpec::new("b"));
    let red = core.create_highlight(Highlight::styled(fg(0xff0000)));
    let blue = core.create_highlight(Highlight::styled(bg(0x0000ff)));
    let buffer = core.create_buffer();

    core.set_text(buffer, b"abcdefgh");
    core.set_highlight(buffer, first, 0..5, Some(red));
    core.set_highlight(buffer, second, 2..8, Some(blue));

    let shape: Vec<_> = core
        .chunks(buffer, 0..8)
        .map(|chunk| (chunk.range, chunk.highlights))
        .collect();

    assert_eq!(
        shape,
        vec![
            (0..2, vec![(first, red)]),
            (2..5, vec![(first, red), (second, blue)]),
            (5..8, vec![(second, blue)]),
        ]
    );
}

// -- style composition ----------------------------------------------------

#[test]
fn overlapping_styles_compose_field_wise_in_z_order() {
    let mut core = Core::default();
    let syntax = core.create_layer(LayerSpec::new("syntax").with_z(0));
    let selection = core.create_layer(LayerSpec::new("selection").with_z(10));

    let keyword = core.create_highlight(Highlight::styled(fg(0x111111)));
    let selected = core.create_highlight(Highlight::styled(bg(0x222222)));

    let buffer = core.create_buffer();
    core.set_text(buffer, b"abcdef");
    core.set_highlight(buffer, syntax, 0..6, Some(keyword));
    core.set_highlight(buffer, selection, 2..4, Some(selected));

    let chunks = chunk_shape(&core, buffer);

    // The selection contributes only `bg`, so the keyword's `fg` survives
    // underneath it — the whole point of optional style fields.
    assert_eq!(chunks[1].0, 2..4);
    assert_eq!(chunks[1].1.fg, rgb(0x111111));
    assert_eq!(chunks[1].1.bg, rgb(0x222222));
}

#[test]
fn higher_z_wins_a_contested_field() {
    let mut core = Core::default();
    let low = core.create_layer(LayerSpec::new("low").with_z(0));
    let high = core.create_layer(LayerSpec::new("high").with_z(5));

    let a = core.create_highlight(Highlight::styled(fg(0xaaaaaa)));
    let b = core.create_highlight(Highlight::styled(fg(0xbbbbbb)));

    let buffer = core.create_buffer();
    core.set_text(buffer, b"abcd");
    core.set_highlight(buffer, low, 0..4, Some(a));
    core.set_highlight(buffer, high, 0..4, Some(b));

    assert_eq!(chunk_shape(&core, buffer)[0].1.fg, rgb(0xbbbbbb));
}

#[test]
fn retheming_a_definition_updates_every_use() {
    let mut core = Core::default();
    let layer = core.create_layer(LayerSpec::new("syntax"));
    let keyword = core.create_highlight(Highlight::styled(fg(0x111111)));

    let buffer = core.create_buffer();
    core.set_text(buffer, b"abcdef");
    core.set_highlight(buffer, layer, 0..3, Some(keyword));
    core.set_highlight(buffer, layer, 4..6, Some(keyword));

    core.update_highlight(keyword, |def| def.style.fg = rgb(0x999999));

    let chunks = chunk_shape(&core, buffer);
    assert_eq!(chunks[0].1.fg, rgb(0x999999));
    assert_eq!(chunks[2].1.fg, rgb(0x999999));
}

#[test]
fn inheritance_is_flattened_at_definition_time() {
    let mut core = Core::default();
    let base = core.create_highlight(
        Highlight::styled(HighlightStyle {
            fg: rgb(0x111111),
            bold: Some(true),
            ..Default::default()
        })
        .with_flags(HighlightFlags::NO_SELECT),
    );
    let derived = core.create_highlight(Highlight::styled(bg(0x222222)).with_parent(base));

    let style = core.resolved_style(derived).unwrap();
    assert_eq!(style.fg, rgb(0x111111));
    assert_eq!(style.bg, rgb(0x222222));
    assert_eq!(style.bold, Some(true));
    assert!(
        core.resolved_flags(derived)
            .unwrap()
            .contains(HighlightFlags::NO_SELECT)
    );
}

#[test]
fn a_parent_cycle_does_not_hang() {
    let mut core = Core::default();
    let a = core.create_highlight(Highlight::styled(fg(1)));
    let b = core.create_highlight(Highlight::styled(bg(2)).with_parent(a));
    core.update_highlight(a, |def| def.parent = Some(b));

    assert!(core.resolved_style(a).is_some());
    assert!(core.resolved_style(b).is_some());
}

// -- edit policies --------------------------------------------------------

#[test]
fn stretch_absorbs_inserted_text_without_fragmenting() {
    let mut core = Core::default();
    let layer =
        core.create_layer(LayerSpec::new("selection").with_policy(EditPolicy::Stretch(Bias::Left)));
    let mark = core.create_highlight(Highlight::styled(bg(1)));

    let buffer = core.create_buffer();
    core.set_text(buffer, b"abcdef");
    core.set_highlight(buffer, layer, 0..3, Some(mark));

    for _ in 0..50 {
        core.insert(buffer, 3, b"x");
    }

    let buf = core.buffer(buffer).unwrap();
    assert_eq!(buf.len(), 56);
    assert_eq!(buf.highlights_at(40), vec![(layer, mark)]);
    // One run for the mark, one for the tail: typing did not shred the tree.
    assert_eq!(core.chunks(buffer, 0..56).count(), 2);
}

#[test]
fn shift_moves_runs_but_inserted_text_lands_in_a_gap() {
    let mut core = Core::default();
    let layer = core.create_layer(LayerSpec::new("marks").with_policy(EditPolicy::Shift));
    let mark = core.create_highlight(Highlight::styled(bg(1)));

    let buffer = core.create_buffer();
    core.set_text(buffer, b"abcdef");
    core.set_highlight(buffer, layer, 4..6, Some(mark));

    core.insert(buffer, 0, b"123");

    let buf = core.buffer(buffer).unwrap();
    assert!(buf.highlights_at(0).is_empty());
    assert_eq!(buf.highlights_at(7), vec![(layer, mark)]);
}

#[test]
fn invalidate_keeps_stale_runs_but_records_damage() {
    let mut core = Core::default();
    let layer = core.create_layer(LayerSpec::derived("syntax"));
    let keyword = core.create_highlight(Highlight::styled(fg(1)));

    let buffer = core.create_buffer();
    core.set_text(buffer, b"let x = 1");
    settle(&mut core, buffer, layer);
    core.set_highlight(buffer, layer, 0..3, Some(keyword));

    core.insert(buffer, 9, b"0");

    let buf = core.buffer(buffer).unwrap();
    // The old colouring is still there — no flicker while the provider catches up.
    assert_eq!(buf.highlights_at(1), vec![(layer, keyword)]);
    assert_eq!(buf.damage(layer), &[9..10]);
}

#[test]
fn drop_clears_any_run_the_edit_touched() {
    let mut core = Core::default();
    let layer = core.create_layer(LayerSpec::new("search").with_policy(EditPolicy::Drop));
    let hit = core.create_highlight(Highlight::styled(bg(1)));

    let buffer = core.create_buffer();
    core.set_text(buffer, b"foo bar foo");
    core.set_highlight(buffer, layer, 0..3, Some(hit));
    core.set_highlight(buffer, layer, 8..11, Some(hit));

    // Delete one byte of the first hit; the whole hit must go, not a fragment.
    core.erase(buffer, 1..2);

    let buf = core.buffer(buffer).unwrap();
    assert!(buf.highlights_at(0).is_empty());
    assert!(buf.highlights_at(1).is_empty());
    // The untouched hit survives, shifted.
    assert_eq!(buf.highlights_at(7), vec![(layer, hit)]);
}

#[test]
fn damage_coalesces_adjacent_spans() {
    let mut core = Core::default();
    let layer = core.create_layer(LayerSpec::derived("syntax"));
    let buffer = core.create_buffer();
    core.set_text(buffer, b"abcdefghij");
    settle(&mut core, buffer, layer);

    core.insert(buffer, 2, b"x");
    core.insert(buffer, 3, b"y");
    core.insert(buffer, 8, b"z");

    // The two adjacent insertions fuse; the distant one stays separate.
    assert_eq!(core.buffer(buffer).unwrap().damage(layer), &[2..4, 8..9]);
}

#[test]
fn set_text_damages_derived_layers_but_not_authoritative_ones() {
    let mut core = Core::default();
    let derived = core.create_layer(LayerSpec::derived("syntax"));
    let authoritative = core.create_layer(LayerSpec::new("marks"));

    let buffer = core.create_buffer();
    core.set_text(buffer, b"abcdefghij");

    let buf = core.buffer(buffer).unwrap();
    assert_eq!(buf.damage(derived), &[0..10]);
    assert!(buf.damage(authoritative).is_empty());
}

// -- providers ------------------------------------------------------------

#[test]
fn a_current_update_applies_verbatim() {
    let mut core = Core::default();
    let layer = core.create_layer(LayerSpec::derived("syntax"));
    let keyword = core.create_highlight(Highlight::styled(fg(1)));

    let buffer = core.create_buffer();
    core.set_text(buffer, b"let x = 1");

    let version = core.buffer(buffer).unwrap().version();
    let mut update = Update::new(layer, version, 0..9);
    update.push(0..3, keyword);

    let applied = core.apply(buffer, update).unwrap();
    assert!(!applied.transformed);
    assert_eq!(applied.dropped, 0);
    assert_eq!(
        core.buffer(buffer).unwrap().highlights_at(1),
        vec![(layer, keyword)]
    );
}

#[test]
fn a_stale_update_is_carried_forward_across_later_edits() {
    let mut core = Core::default();
    let layer = core.create_layer(LayerSpec::derived("syntax"));
    let keyword = core.create_highlight(Highlight::styled(fg(1)));

    let buffer = core.create_buffer();
    core.set_text(buffer, b"let x = 1");

    // The provider snapshots, then the user types before it answers.
    let snapshot = core.buffer(buffer).unwrap().snapshot();
    core.insert(buffer, 0, b"// ");

    let mut update = Update::new(layer, snapshot.version(), 0..9);
    update.push(0..3, keyword);

    let applied = core.apply(buffer, update).unwrap();
    assert!(applied.transformed);
    assert_eq!(applied.dropped, 0);

    let buf = core.buffer(buffer).unwrap();
    // `let` moved from 0..3 to 3..6 and the highlight moved with it.
    assert!(buf.highlights_at(1).is_empty());
    assert_eq!(buf.highlights_at(4), vec![(layer, keyword)]);
}

#[test]
fn results_an_edit_landed_inside_are_dropped_not_misplaced() {
    let mut core = Core::default();
    let layer = core.create_layer(LayerSpec::derived("syntax"));
    let keyword = core.create_highlight(Highlight::styled(fg(1)));

    let buffer = core.create_buffer();
    core.set_text(buffer, b"let x = 1");

    let snapshot = core.buffer(buffer).unwrap().snapshot();
    // Type inside the token the provider is about to report on.
    core.insert(buffer, 1, b"e");

    let mut update = Update::new(layer, snapshot.version(), 0..9);
    update.push(0..3, keyword);

    let applied = core.apply(buffer, update).unwrap();
    assert_eq!(applied.dropped, 1);
    assert!(core.buffer(buffer).unwrap().highlights_at(1).is_empty());
}

#[test]
fn applying_an_update_clears_that_span_of_damage() {
    let mut core = Core::default();
    let layer = core.create_layer(LayerSpec::derived("syntax"));
    let buffer = core.create_buffer();
    core.set_text(buffer, b"abcdefghij");
    settle(&mut core, buffer, layer);

    core.insert(buffer, 2, b"x");
    assert_eq!(core.buffer(buffer).unwrap().damage(layer), &[2..3]);

    settle(&mut core, buffer, layer);
    assert!(core.buffer(buffer).unwrap().damage(layer).is_empty());
}

#[test]
fn an_update_replaces_its_whole_span() {
    let mut core = Core::default();
    let layer = core.create_layer(LayerSpec::derived("syntax"));
    let a = core.create_highlight(Highlight::styled(fg(1)));
    let b = core.create_highlight(Highlight::styled(fg(2)));

    let buffer = core.create_buffer();
    core.set_text(buffer, b"abcdefgh");
    core.set_highlight(buffer, layer, 0..8, Some(a));

    let version = core.buffer(buffer).unwrap().version();
    let mut update = Update::new(layer, version, 2..6);
    update.push(2..4, b);

    core.apply(buffer, update).unwrap();

    let buf = core.buffer(buffer).unwrap();
    assert_eq!(buf.highlights_at(0), vec![(layer, a)]);
    assert_eq!(buf.highlights_at(2), vec![(layer, b)]);
    // 4..6 was inside the span but uncovered by a run: an explicit gap now.
    assert!(buf.highlights_at(4).is_empty());
    assert_eq!(buf.highlights_at(6), vec![(layer, a)]);
}

#[test]
fn one_provider_cannot_write_another_layer() {
    let mut core = Core::default();
    let mine = core.create_layer(LayerSpec::derived("mine"));
    let theirs = core.create_layer(LayerSpec::derived("theirs"));
    let highlight = core.create_highlight(Highlight::styled(fg(1)));

    let buffer = core.create_buffer();
    core.set_text(buffer, b"abcdefgh");

    let version = core.buffer(buffer).unwrap().version();
    let mut update = Update::new(mine, version, 0..8);
    update.push(0..4, highlight);
    core.apply(buffer, update).unwrap();

    let buf = core.buffer(buffer).unwrap();
    assert_eq!(buf.highlights_at(0), vec![(mine, highlight)]);
    assert!(
        buf.highlights_at(0)
            .iter()
            .all(|(layer, _)| *layer != theirs)
    );
}

// -- constraints ----------------------------------------------------------

#[test]
fn readonly_ranges_refuse_edits() {
    let mut core = Core::default();
    let layer = core.create_layer(LayerSpec::new("locks").constraining());
    let locked =
        core.create_highlight(Highlight::styled(bg(1)).with_flags(HighlightFlags::READONLY));

    let buffer = core.create_buffer();
    core.set_text(buffer, b"prompt> user input");
    core.set_highlight(buffer, layer, 0..8, Some(locked));

    let buf = core.buffer(buffer).unwrap();
    assert!(buf.constraints_at(3).contains(HighlightFlags::READONLY));
    assert!(buf.constraints_at(10).is_empty());

    assert!(matches!(
        core.try_replace(buffer, 2..4, b"x"),
        Err(Refused::ReadOnly(_))
    ));
    assert!(matches!(
        core.try_replace(buffer, 3..3, b"x"),
        Err(Refused::ReadOnly(_))
    ));

    // Editing past the locked prefix is fine.
    assert!(core.try_replace(buffer, 8..12, b"other").is_ok());
    assert_eq!(text_of(&core, buffer), b"prompt> other input");
}

#[test]
fn atomic_ranges_refuse_partial_edits_but_allow_whole_ones() {
    let mut core = Core::default();
    let layer = core.create_layer(LayerSpec::new("atoms").constraining());
    let atom = core.create_highlight(Highlight::default().with_flags(HighlightFlags::ATOMIC));

    let buffer = core.create_buffer();
    core.set_text(buffer, b"a{{token}}b");
    core.set_highlight(buffer, layer, 1..10, Some(atom));

    assert!(matches!(
        core.try_replace(buffer, 3..5, b""),
        Err(Refused::SplitsAtomic)
    ));
    assert!(core.try_replace(buffer, 1..10, b"").is_ok());
    assert_eq!(text_of(&core, buffer), b"ab");
}

#[test]
fn snap_moves_a_caret_out_of_an_atomic_run() {
    let mut core = Core::default();
    let layer = core.create_layer(LayerSpec::new("atoms").constraining());
    let atom = core.create_highlight(Highlight::default().with_flags(HighlightFlags::ATOMIC));

    let buffer = core.create_buffer();
    core.set_text(buffer, b"a{{token}}b");
    core.set_highlight(buffer, layer, 1..10, Some(atom));

    let buf = core.buffer(buffer).unwrap();
    assert_eq!(buf.snap(5, Bias::Left), 1);
    assert_eq!(buf.snap(5, Bias::Right), 10);
    assert_eq!(buf.snap(0, Bias::Left), 0);
}

#[test]
fn styling_layers_do_not_leak_constraints() {
    let mut core = Core::default();
    // Not marked `constraining`, so its flags must not reach the edit path.
    let layer = core.create_layer(LayerSpec::derived("syntax"));
    let sneaky =
        core.create_highlight(Highlight::styled(fg(1)).with_flags(HighlightFlags::READONLY));

    let buffer = core.create_buffer();
    core.set_text(buffer, b"abcdef");
    core.set_highlight(buffer, layer, 0..6, Some(sneaky));

    assert!(core.buffer(buffer).unwrap().constraints_at(2).is_empty());
    assert!(core.try_replace(buffer, 2..3, b"x").is_ok());
}

#[test]
fn constraints_track_edits() {
    let mut core = Core::default();
    let layer = core.create_layer(
        LayerSpec::new("locks")
            .with_policy(EditPolicy::Stretch(Bias::Left))
            .constraining(),
    );
    let locked = core.create_highlight(Highlight::default().with_flags(HighlightFlags::READONLY));

    let buffer = core.create_buffer();
    core.set_text(buffer, b"xxxxfree");
    core.set_highlight(buffer, layer, 0..4, Some(locked));

    core.insert(buffer, 8, b"!");

    let buf = core.buffer(buffer).unwrap();
    assert!(buf.constraints_at(2).contains(HighlightFlags::READONLY));
    assert!(buf.constraints_at(8).is_empty());
}

// -- checkpoints ----------------------------------------------------------

#[test]
fn checkpoints_carry_text_and_authoritative_layers() {
    let mut core = Core::default();
    let marks = core.create_layer(LayerSpec::new("marks"));
    let highlight = core.create_highlight(Highlight::styled(bg(1)));

    let buffer = core.create_buffer();
    core.set_text(buffer, b"read");
    core.set_highlight(buffer, marks, 0..4, Some(highlight));

    let checkpoint = core.buffer(buffer).unwrap().checkpoint();

    core.insert(buffer, 4, b" only");
    assert_eq!(text_of(&core, buffer), b"read only");

    core.buffer_mut(buffer).unwrap().restore(&checkpoint);

    let buf = core.buffer(buffer).unwrap();
    assert_eq!(buf.len(), 4);
    assert_eq!(buf.highlights_at(3), vec![(marks, highlight)]);
}

#[test]
fn restore_invalidates_derived_layers_instead_of_carrying_them() {
    let mut core = Core::default();
    let syntax = core.create_layer(LayerSpec::derived("syntax"));
    let keyword = core.create_highlight(Highlight::styled(fg(1)));

    let buffer = core.create_buffer();
    core.set_text(buffer, b"let x = 1");
    core.set_highlight(buffer, syntax, 0..3, Some(keyword));

    let checkpoint = core.buffer(buffer).unwrap().checkpoint();
    core.insert(buffer, 9, b"0");
    core.buffer_mut(buffer).unwrap().restore(&checkpoint);

    let buf = core.buffer(buffer).unwrap();
    // Derived state is gone and flagged for recomputation, not resurrected.
    assert!(buf.highlights_at(1).is_empty());
    assert_eq!(buf.damage(syntax), &[0..9]);
}

#[test]
fn restore_rejects_updates_computed_before_it() {
    let mut core = Core::default();
    let layer = core.create_layer(LayerSpec::derived("syntax"));
    let keyword = core.create_highlight(Highlight::styled(fg(1)));

    let buffer = core.create_buffer();
    core.set_text(buffer, b"let x = 1");

    let checkpoint = core.buffer(buffer).unwrap().checkpoint();
    let snapshot = core.buffer(buffer).unwrap().snapshot();

    core.insert(buffer, 9, b"0");
    core.buffer_mut(buffer).unwrap().restore(&checkpoint);

    let mut update = Update::new(layer, snapshot.version(), 0..9);
    update.push(0..3, keyword);

    assert_eq!(core.apply(buffer, update), Err(Stale::HistoryPruned));
}

// -- text access ----------------------------------------------------------

#[test]
fn chunk_text_is_read_through_the_caller_s_scratch_buffer() {
    let mut core = Core::default();
    let layer = core.create_layer(LayerSpec::new("syntax"));
    let highlight = core.create_highlight(Highlight::styled(fg(1)));

    let buffer = core.create_buffer();
    core.set_text(buffer, b"hello world");
    core.set_highlight(buffer, layer, 0..5, Some(highlight));

    let buf = core.buffer(buffer).unwrap();
    let mut scratch = Vec::new();
    let mut rendered = Vec::new();

    for chunk in core.chunks(buffer, 0..11) {
        scratch.clear();
        buf.read_into(chunk.range.clone(), &mut scratch);
        rendered.push(String::from_utf8(scratch.clone()).unwrap());
    }

    assert_eq!(rendered, vec!["hello".to_string(), " world".to_string()]);
}

#[test]
fn visit_range_borrows_without_copying() {
    let mut core = Core::default();
    let buffer = core.create_buffer();
    core.set_text(buffer, b"hello world");
    core.insert(buffer, 5, b",");

    let buf = core.buffer(buffer).unwrap();
    let mut seen = Vec::new();
    buf.visit_range(0..buf.len(), |slice| seen.extend_from_slice(slice));

    assert_eq!(seen, b"hello, world");
}

#[test]
fn a_layer_registered_after_a_buffer_still_covers_it() {
    let mut core = Core::default();
    let buffer = core.create_buffer();
    core.set_text(buffer, b"abcdef");

    // Registered late: it must not panic the chunker, and it must report gaps.
    let layer = core.create_layer(LayerSpec::new("late"));

    assert_eq!(core.chunks(buffer, 0..6).count(), 1);
    assert!(core.buffer(buffer).unwrap().highlights_at(0).is_empty());

    let highlight = core.create_highlight(Highlight::styled(fg(1)));
    core.set_highlight(buffer, layer, 0..2, Some(highlight));
    assert_eq!(core.chunks(buffer, 0..6).count(), 2);
}
