# `core`: buffers, metadata, and providers

Status: implemented, 2026-08-21. This documents why `core` is shaped the way it
is, so the reasoning survives longer than the memory of the afternoon it was
written in. *Since 2026-09-15* (the rebuild on kui) the crate is `doc`
(`kawoosh-doc`), and this note is its spec for the framing, the journal and
versions; what changed is the storage of runs — a layer is a sorted list
shifted eagerly on every edit, not a `RunTree` treap — so "One run tree"
and "Possible consolidation" below describe the first build.

`core` is UI-independent on purpose. The frontend question (GPUI or otherwise)
is deliberately not settled here, and nothing below depends on the answer.

## The framing: four kinds of metadata, not one

The obvious first design gives every annotation one type and one shape:
`(layer, range, highlight)`. It does not survive contact with the actual list of
things an editor wants to attach to text, because those things have genuinely
different shapes:

| Kind | Shape | Read by | Volume | On edit |
|---|---|---|---|---|
| **Styling** — syntax, search hits, diagnostics | range | renderer | thousands, churny | recompute |
| **Constraints** — read-only, atomic, non-selectable | range | the *edit / movement* path | dozens, stable | must survive exactly |
| **Gutter marks** — icons, breakpoints, fold arrows | point or line | renderer, out of flow | dozens | shift |
| **Virtual content** — inlay hints, fold placeholders | anchor + synthetic text | the *layout* path | hundreds | shift or invalidate |

Styling and constraints are genuinely range-shaped, and both live here — but in
separate stores, not as sibling layers of one store, because their readers and
their cost profiles have nothing in common.

Gutter marks are a sparse side table keyed by anchor; a run tree is the wrong
structure for a handful of points, and they are not implemented here.

Virtual content is not metadata at all. It is a coordinate transform, and it
belongs in a display map *above* `core`.

## The five decisions

### 1. Versions and an edit journal (`version.rs`)

Every mutation bumps a `Version` and appends an `Edit` to a bounded journal.
Because a buffer has exactly one writer, carrying a coordinate from an old
version to the current one is plain index shifting — not OT, not CRDT, about
forty lines.

This is the keystone. Without it there is no way for a highlighter that parsed
version 7 to hand its answer back at version 12, which means every provider must
be synchronous, which means tree-sitter and LSP cannot be providers.

Two transform modes, and the distinction matters:

- `transform_range` — for a provider's **results**. Fails with
  `Stale::Overwritten` if an edit landed inside the range, because a token whose
  text changed is not a token you should still be colouring.
- `clamp_range` (`Buffer::transform_span`) — for a provider's **scope**. An edit
  inside the span shrinks the region the provider is authoritative over; it does
  not poison the whole submission.
- `carry_range` (2026-09-17) — what the layers actually do with a run, and what
  `Buffer::apply` does with a late result: an edit inside the range stretches or
  shrinks it, one over an edge cuts it, and only an edit that swallows it empties
  it. The strict form turned out to be the wrong default for colours: a
  highlighter's incremental answer covers the edit and what tree-sitter says
  changed, not the token around the edit, so dropping the token's run left a
  string plain from the first typed char until the next whole parse. The stale
  colour over the typed text is the `Invalidate` policy below — keep drawing,
  let the provider correct — and `transform_range` stays for a caller that wants
  the failure (the row cache).

Boundary behaviour is bias-driven and worth stating explicitly, because it is
the part that is easy to get subtly wrong: a range **excludes** text inserted at
either of its edges (start moves right, end stays left), while a scope
**includes** it (start stays left, end moves right).

### 2. Layer ownership, and replacement as the primitive (`provider.rs`)

A layer has one producer. It submits an `Update { layer, version, span, runs }`
— a whole span at once, which is how highlighters actually work: retokenize a
damaged region, hand back a run list. `Core::apply` transforms it forward,
discards individual runs that an edit landed inside, fills anything uncovered
with explicit gaps, and writes the span atomically.

Consequences worth the price:

- Stale-result handling lives in one place instead of in every provider.
- Producer A structurally cannot corrupt producer B's layer.
- The boundary is **data**, not a trait object, so the same `Update` can come
  from an in-process Rust provider or be deserialized from an out-of-process
  one. This is why there is no `ChunkIterator`-style plugin trait: a pipeline
  stage taking `&mut dyn Any` buys nothing over a generic parameter in-process,
  and does not survive a real FFI boundary anyway.

`Core::set_highlight` remains for authoritative layers driven directly by user
action (select a range, lock a region), where there is no version to check.

### 3. Durability: authoritative vs derived (`layer.rs`)

`Durability::Authoritative` is user intent that cannot be recomputed — marks,
read-only regions, selections. `Durability::Derived` is anything a provider can
regenerate — syntax, diagnostics, search hits.

Only authoritative layers enter a `Checkpoint`. Derived layers are invalidated
on restore and repopulated. This keeps undo state small and removes the question
of what undoing a syntax highlight is supposed to mean.

`restore` also resets the journal, so any result computed against a pre-restore
version is rejected rather than misapplied.

### 4. Edit policy per layer (`layer.rs`)

`InsertAffinity { Before, After, Gap }` was a per-call decision, but the right
answer is a property of what the layer *means*, and there are more than three
cases:

| Policy | Behaviour | For |
|---|---|---|
| `Shift` | Moves with the text, never grows; inserted text lands in a gap | breakpoints, gutter icons, cursors |
| `Stretch(Bias)` | Inserted text joins the neighbouring run | selections, read-only regions |
| `Invalidate` | Shifts, and records damage; stale runs keep rendering | syntax highlighting |
| `Drop` | Any run the edit intersects is cleared outright | search hits, LSP squiggles |

`Invalidate` is the one that could not be expressed before and is what syntax
highlighting actually wants: keep drawing the stale colours so typing does not
flicker, while flagging the span so the provider knows to recompute. `Drop`
widens to whole runs before clearing, so a partially-edited search hit
disappears rather than surviving as a truncated fragment.

Damage is exposed as `Buffer::damage(layer)`, coalesced and ordered, and cleared
by the span of whatever `Update` covers it.

### 5. Constraints are a separate flattened index (`buffer.rs`)

Answering "can I delete this range?" by walking every layer, collecting ids and
dereferencing each through the highlight table is a map lookup per layer per
keystroke, on the hottest path in the editor — and it silently depends on no
provider ever putting `READONLY` in its syntax layer.

Instead, `HighlightFlags::CONSTRAINTS` from layers marked `.constraining()` are
unioned into one `RunTree<HighlightFlags>`. `Buffer::can_edit` and
`Buffer::constraints_at` are then a single tree walk with no indirection.

`LayerSpec::constraining()` panics on a derived layer, which makes "constraints
come from authoritative state" true by construction rather than by convention. A
provider that genuinely needs to lock regions owns an authoritative layer and
writes it through `Core::apply` like anyone else.

`Buffer::snap` moves a caret out of the interior of an `ATOMIC` run.

## Supporting pieces

**One run tree, generic (`runs.rs`).** `RunTree<T>` is an implicit-key treap
with `Rc` nodes, so `clone` is O(1) and a clone is a real snapshot — the same
property `text_buffer::Buffer` provides. Metadata layers instantiate it at
`Option<HighlightId>`, the constraint index at `HighlightFlags`.

Every mutation **coalesces** adjacent equal runs, at both seams and internally.
This is not an optimization, it is a correctness-of-cost issue: without it,
typing a thousand characters into a stretched region leaves a thousand identical
runs behind. `EditPolicy::Stretch` goes further and grows the containing run in
place rather than splicing.

**Styles compose, they do not clobber (`highlight.rs`).** Every field of
`HighlightStyle` is `Option`. A selection sets only `bg`, a diagnostic only
`underline`, syntax only `fg` — with mandatory fields, an overlapping layer has
no way to say "leave this alone". Layers carry an explicit `z`, and `core`
resolves precedence rather than leaving each renderer to reinvent it and
disagree.

Highlight `parent` chains are flattened at definition time (and on
`rebuild_styles` after a theme edit), not walked per chunk per frame. Cycles are
broken by a depth cap.

**Chunks carry no text (`chunks.rs`).** `Chunk` is `{ range, style, flags,
highlights }`. Materializing bytes is the caller's choice: `Buffer::visit_range`
is zero-copy, `Buffer::read_into` appends into one reusable scratch buffer per
frame. Originally this was forced by `text_buffer` storing bytes behind
`Rc<RefCell<_>>`; since the Arc migration (below) slices borrow freely, but the
shape stays — the caller still knows best whether it wants borrows or one
scratch allocation, and it still avoids the per-chunk `Vec<u8>` allocation the
previous design paid on every frame.

A layer registered after a buffer contributes nothing rather than panicking; the
old `expect("metadata layer must cover every byte")` was reachable.

## Deliberately not here

- **Virtual content and the display map.** Inlay hints, folds, soft wrap and tab
  expansion form a stack of coordinate spaces above `core`. Nothing here assumes
  buffer offset equals screen position, which is what keeps that buildable
  later. Note that `get_line`-shaped APIs assume line == row and will need a
  chunk-oriented sibling once soft wrap exists.
- **Gutter marks.** A sparse anchor-keyed side table, not a run tree.
- **Anchors.** `Buffer::transform_offset` is the building block; a real `Anchor`
  type is `(offset, version, bias)` resolved lazily through it, and is maybe
  thirty lines whenever it is wanted.
- **UTF-8 validation.** Offsets are byte offsets and boundary correctness is the
  caller's job. This should be decided deliberately rather than left implicit.

## Threading: resolved 2026-08-28

`text_buffer::Buffer` originally used `Rc<RefCell<Shared>>`, which made
`Snapshot` not `Send` while tree-sitter and LSP normally run off the UI
thread. This was the one open decision, and it is now closed: providers run
off-thread on `Send + Sync` snapshots (MVP milestone 1, docs/design/mvp.md).

The migration went further than the anticipated type swap: the shared
append-only arena was removed entirely. Each `Piece` now owns an
`Arc<Vec<u8>>` of its text block — a block per insert, extended in place
through `Arc::get_mut` when the piece, its block, and its node are all
unshared (the coalescing fast path survives; a live snapshot fails the
uniqueness check and falls back to a fresh piece, exactly as before). This
buys three things over an arena behind `&mut self`: no interior mutability
anywhere, erased blocks actually free when their last piece drops (the old
arena only ever grew), and `visit_range` hands out borrowed slices with no
re-entrancy caveat. Compile-time `Send + Sync` assertions pin the property in
both crates.

## Possible consolidation

`text_buffer`'s piece tree and `core`'s `RunTree` are the same structure with
different payloads and different summaries (`length + newlines` versus `len`).
A generic `SumTree<T: Item, S: Summary>` would collapse them into one
implementation and give run trees line summaries for free. Worth doing when
either is next touched substantially; not urgent.
