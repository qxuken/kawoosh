# Where a pane opens: under the buffer, or a column of its own

Status: decided 2026-09-30 (roadmap step 74) and built the same day, as
designed, with the one departure under "Built" at the end. Asked as
"everything that affects the buffer opens as vertical split and
everything that doesn't is not — tools by default in their own column,
but can be overridden; the terminal its own column". Companion to
[scrolling-tab.md](scrolling-tab.md) (a column is a stack), keys.md's
"Panes, tabs, the dock", [plugin-panes.md](plugin-panes.md)'s
`view_open`, [lists.md](lists.md), and `kawoosh/src/layout.rs`.

One reading is fixed up front, since the word cuts both ways: the
*vertical split* of the ask is `SplitDir::V` — the pane split top from
bottom, so the new one stands *under* the old inside its column, vim's
`:split` — not vim's `:vsplit`. Read the other way the rule would put
the terminal beside the buffer *and* say the terminal is not beside it,
so this is the reading the examples allow.

## What there is

A tab is a strip of columns and a column is a stack
(scrolling-tab.md Decision 1); a tree tab is the same two axes without
the ribbon. So every pane that opens has exactly two places to go from
the focused one, and the dock as a third: *under it* (`SplitDir::V`,
the column's stack, a tree's split below), *a column of its own*
(`SplitDir::H`, the next column on the ribbon, a tree's split beside),
or *the dock*. Which one a pane takes today is decided caller by
caller, and the callers do not agree:

| opens under the focused pane (`V`) | a column of its own (`H`) | the dock |
|---|---|---|
| `:terminal`, `:!`, a tool without `dock` (`fill_or_split`) | `:fonts`, `:themes`, `:settings`, `:du`, `:theme lab` (`view_open`'s default) | a tool with `dock = true` |
| the lists — `*references*`, `*diagnostics*`, `gr`, `<C-e>` (`show_multi`, `beside = true`) | `:help`, `:tutor`, a file opened with no editor pane | an ssh master (domains.md) |
| `*compile*`, `*hover*`, `*diagnostic*`, `*messages*`, `*lua*`, `*maps*`, `*keymap.json*`, `*lsp*`, `*domains*`, `*theme check*`, `*shell integration*`, `*diff NAME*` (`show_in_pane`) | `:undo`, `:memory` (`PANEL_SHARE`, 0.35) | |
| the picker, the search bar (`below = true`) | a plugin's view unless it says `below` | |

The left column is one function's default (`show_in_pane` was written
for `*references*` and grew), the middle another's, and neither was
chosen for what the pane *is*. So a `*compile*` run stacks under the
file and halves it, a second `:terminal` halves the terminal, and the
undo tree of the file stands two columns away from it after a
`<C-w>v`. The strip made this visible: on a ribbon, a column is the
unit the eye and `<C-w>l` move by, so what a column holds is the
question, and today it holds whatever happened to be focused when
something opened.

## The thesis

A column is a *subject and what is about it*. The buffer is the
subject; the panes made from it and acting back on it — a list whose
`<CR>` lands in it, its undo tree, the hover at its caret — belong
under it, in its column, where they scroll, move (`<C-w>H`), and close
with it. Everything else is a subject of its own: a terminal, a tool,
a build, a settings page, the fonts. Each takes a column, and the
ribbon reads as a row of subjects. This is niri's and PaperWM's
reason for having stacks inside columns at all.

The rule is a default, not a cage: every door that opens a pane says
where by data, and the launcher's `<C-w>s t` / `<C-w>v t` put a pane
anywhere by hand, as they do now.

## Decisions

### 1. Two places, named, and one door

```rust
pub enum Place { Under, Column, Dock }
impl Layout { pub fn open(&mut self, content: Content, place: Place) -> PaneId }
```

`Layout::open` is the one way a new pane enters the layout;
`Layout::split(dir, content)` stays as its spelling for the two
explicit keys (`<C-w>s` `<C-w>v`, `:new` `:vnew`, `:split` `:vsplit`)
and for `Place`'s own use, and every other caller moves to `open`.
`Under` is `SplitDir::V` on the focused pane; `Column` is `SplitDir::H`
— a column after the focused one at `layout.column_width` in a strip,
a split beside in a tree; `Dock` is what a docked tool does today (the
dock's strip or tree, opened and focused). From inside the dock, `Under`
and `Column` are the dock's, as a split from a dock pane is now. When
the launcher has the keyboard, the content fills it whatever the place
(`fill_or_split`'s rule, kept).

*Beat:* a `SplitDir` per caller, which is what there is — the table
above is its result; and a `Place` with a width or a share in it, which
the callers that want one (`PANEL_SHARE`, the picker's `share`) set
after, as they do.

### 2. The place is a property of what the pane is

The kind decides; the table is the whole rule, and it is what
Decision 1's callers pass:

| under the focused pane | a column of its own |
|---|---|
| a list of places — `*references*`, the diagnostics, `gr`, `<C-e>`, the search's results: `<CR>` lands in the pane above it | a terminal — `:terminal`, `:!`, a tool without another `place` |
| `*hover*`, `*diagnostic*` — the caret's | `*compile*` — a build is the project's, its `<CR>` lands in an editor pane of its own choosing (`compile.rs`'s `other`) |
| `:undo` — "the undo history of whichever buffer has the keyboard" (`Content::Undo`'s own words) | `:memory` — the register's past is the session's |
| the picker, the search bar — a strip of the pane itself | the named text panes — `*messages*`, `*lua*`, `*maps*`, `*keymap.json*`, `*lsp*`, `*domains*`, `*theme check*`, `*shell integration*`, `*diff NAME*` |
| | `:fonts`, `:themes`, `:settings`, `:du`, `:theme lab`, `:help`, `:tutor` |
| | a file opened when no editor pane is on screen |

The test for the left is *made from the buffer and acting back on
it*: the rows move its caret or the pane shows its state. A workspace
list (`<C-e>` whole, the project search) is still of the pane it was
asked from — its rows land there — so it stays under it. `*compile*`
is the one that could go either way and goes right: its rows jump, but
a build is started from a command, not a caret, is watched while the
file is typed in (`glance_in_pane`), and is a tool in `<leader>t`'s
picker; stacked under the file it halved the file for the run's whole
length. `*hover*` goes left even though it is a read-only text like
`*messages*`, because it is the caret's.

What flips against today: the terminal, `:!` and the undocked tools
(under → column); `*compile*` and the named text panes (under →
column); `:undo` (column → under, at the stack's half). The
lists, the picker, the search bar, the Lua panes and the docked tools
stay where they are. `show_in_pane` takes a `Place` and its callers
say which; `show_multi`'s `beside` means `Under` as it does.

*Beat:* a setting per kind (`layout.place.compile = "under"`) — a
table nobody edits, in front of a rule nobody remembers; the overrides
that are worth having are the two below, on the doors that already
take data.

### 3. Tools and terminals say where, by data

`kawoosh.tool(name, { cmd, cwd, place, restore })` and the `tools`
table take `place = "column" | "under" | "dock"`, `column` when absent.
`dock = true` stays as the spelling of `place = "dock"` — it is in every
`settings.lua` that has a dev server — and is what `kawoosh.tools()`
keeps reporting beside `place`. `:tool NAME` opens where the tool says,
finds it there (`tool_pane`), and toggles the dock away for a docked
one as now.

`terminal.place = "column" | "under"` (`column`) is where a bare
`:terminal` and `:!` go, for whoever wants the old shape back. The
launcher is the per-pane override — `<C-w>s` then `t` is a terminal
under, `<C-w>v` then `t` one beside, whatever the setting — so
`:terminal` takes no flag of its own.

A Lua view keeps `view_open`'s `below = true` as its override, the
default a column as it is: the shipped views are all subjects
(`:fonts`, `:settings`) or say `below` already (the picker, the
search bar). One exception is named rather than argued around:
`:dir preview` opens beside the listing with the keyboard staying in
it, and stays beside — a preview wants the listing's height, and a
stack would halve it. Read by Decision 2's test it is *of* the listing
and would go under; it is the case the test gets wrong, so it is named
here, on `view_open`'s default, rather than made a rule of its own.

*Beat:* `place` on `view_open` replacing `below` — a rename of a
working option for symmetry, which every plugin pays for; and a
window-wide `layout.place` that flips the whole rule, which is
`:layout tree` and the launcher's job already.

### 4. What the docs and the keys say

keys.md's `<leader>t` row, `:terminal`'s doc string ("a terminal in a
split below"), tools.lua's header, lists.md's "a split beside" — which
means under, and will say so — and plugin-panes.md's `view_open` row
are corrected when built; a "Where a pane opens" paragraph goes under
keys.md's "Panes, tabs, the dock", the table of Decision 2 in two
lines. `terminal.place` lands in the settings pane's Terminal section
by its prefix, with a doc.

## When built

One round: `Place` and `Layout::open`; the callers of `layout.split`
and `fill_or_split` outside the explicit keys moved onto it with the
kind's place; `show_in_pane_as` taking a `Place`, `glance_in_pane` and
the named panes passing `Column`, `*hover*` `*diagnostic*` `Under`;
`ToolDef.place` through boot.lua's `kawoosh.tool` and tools.lua's
table, `dock = true` mapped; `terminal.place` read where `:terminal`
and `:!` spawn; the docs of Decision 4. Tests: a `:terminal` from an
editor pane is a new column (and under it with `terminal.place =
"under"`, and under it from `<C-w>s t`); `*compile*` a column with the
keyboard staying; `*references*` under the file it was asked from;
`:undo` under the buffer at its share; a tool's `place` each way and
`dock = true` still the dock; `add_headless_terminal` unchanged for the
tests that feed it.

## Built

2026-09-30, as designed. `Place { Under, Column, Dock }` and
`Layout::open` in `layout.rs`; `fill_or_split` became `fill_or_open`,
which takes a `Place` and never fills the launcher with the dock's;
`show_in_pane_as` takes a `Place`, `*hover*` and `*diagnostic*` under,
the rest a column; `ToolDef.place` from `kawoosh.tool`'s `place` or
`dock = true`, reported both ways by `kawoosh.tools()`;
`terminal.place` with a doc. Two departures: `:undo history` under
the buffer takes the stack's half, not `PANEL_SHARE` — at a third of
the height the panel's header, rows and hunk did not fit, and a click
on a row past the fold did nothing; and `Place::parse` also reads
`below` and `beside`, since `view_open` and `multi open` say them
already, so a `tools` table written with either word still lands.
`kawoosh/tests/placement.rs` is the round's test. Step 73 (version
control) merged to main meanwhile: its `*hunk*` is the caret's and
goes under the buffer; its review multibuffer is a list (`beside`,
under); the blame is a gutter, not a pane.
