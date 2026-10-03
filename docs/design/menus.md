# Menus: a right-click's, and the Mac's bar

Status: decided and built 2026-10-03 (roadmap step 79). Asked
2026-10-02: "Let's implement proper context menu for editors and maybe
panes. Also for macos let's add system menu. kui should support both
already". It does: a context menu is kui's ADR 0017 (rows as data,
`on_context_menu`, `Core::open_menu`, a chosen row one `menu` event)
and the bar its ADR 0018 (`MenuBar`, `declare_menu_bar`, `NSApp`'s
main menu on macOS) with ADR 0030's Window menu. Nothing in kui moved.
Each decision keeps the alternative it beat.

## The thesis

A menu is another way to the commands, never a second implementation
of one. Every row kawoosh puts in a menu runs a command line on its
pane, as the `:` prompt would, or one of the six edits a menu owes
(Cut, Copy, Paste, Select All, Undo, Redo) through the commands the
keys run. The keymap stays the keyboard's only source: a row shows
the keys its command is bound to, and the bar binds a chord only
where no binding takes it.

## Decisions

### 1. Who answers a right-click

kui asks the topmost node under the pointer that declares a menu, and
a press on a child that declares none reaches the nearest that does
(kui backlog T1). So:

- **A pane's title bar** declares `{kind = "panemenu", pane, title =
  true}`: the pane's menu (Decision 3) whatever the pane holds.
- **A terminal's grid** declares it while no program reads the mouse,
  on the grid itself, so kui's Select All selects the grid. While one
  does, every button is the program's (roadmap step 55) and the
  right-click is its, as before.
- **An editor's rows** do not ask: the `editor` node *claims* the
  secondary button (`on_button`, `Buttons::SECONDARY`), the mechanism
  the terminal's middle button uses. A claimed press comes with the
  `line` and `byte` it landed on, as a left press does, which
  `contextmenu` does not carry — and the caret has to go there first
  (Decision 2). The press maps back through `Kawoosh::offset_at`,
  factored out of the drag's code so the two cannot disagree about a
  long line's slice or a rendered row's fold.

- **Nothing else.** A Lua view (a picker, the settings, help), the
  undo and memory panes and a header over an editor declare no menu of
  kawoosh's: a right-click there is the view's own (`on_context_menu`
  from Lua), or kui's stock menu over a kui field or selectable text a
  plugin draws, or nothing, as before.

*Beat:* the pane's menu on every pane's root, for a right-click
anywhere. kui hands a press to the nearest enclosing declaration, and
a declaration is a claim: kui's own menu over a plugin's field (Cut,
Copy, Paste) would have become the pane's.

*Beat:* `on_context_menu` on the editor too, and the press located
again from `x` `y` with `Core::text_hit` over the row keys the frame
drew. A second way to the same answer, kept only for rows the frame
remembered.

### 2. The editor's menu

The caret goes to the press unless it lands inside a selection, which
a menu is usually for (VS Code's and Zed's rule); outside one, visual
mode ends there. Then, every row present and only its enabling moving
(kui's rule for menus):

| Row | Runs | Lit when |
|---|---|---|
| Go to Definition · References | `lsp definition` · `lsp references` | a server answers for the buffer and does it (`Kawoosh::lsp_answers`, `caps_of`) |
| Rename Symbol… · Code Actions… | `lsp rename` · `lsp action` | the same |
| Format | `format` | `Editor::can` says so (a formatter, the server's or not) |
| Cut · Copy | visual `d` · `y` over the selection | a selection |
| Paste · Select All | `paste clipboard` · `select all` | always |
| Split Right · Down, Close Other Panes, Close Pane | `vsplit` · `split` · `only` · `close` | always (Close Other with another pane) |

A selection made outside visual mode (a drag's, before its mode caught
up) is taken into visual mode first, so Copy is exactly `y`: the
register, and the clipboard through `Effect::SetClipboard`.

*Beat:* kui's standard rows (`MenuRole::Copy`…). kui performs those on
its own editors and selection scopes, and an editor pane is neither:
its selection is the engine's. A terminal's grid *is* a kui selection
scope, so its menu has kui's Copy and Select All and kawoosh's Paste
(⌘V's: the clipboard asked for, pasted into the pty).

### 3. The pane's menu

An editor's Copy Path (`path copy`, dimmed for a buffer with no file),
then Decision 2's last four rows. Every row's `id` names the pane, which
is focused before the row runs: the menu of a pane that does not hold
the keys acts on that pane, and a pane closed while its menu was open
is a row that does nothing.

### 4. The keymap is the keyboard

Two consequences of AppKit consuming a menu's key equivalent before
the window sees it (kui ADR 0018 Decision 7):

- **A row's keys are a hint.** The shortest normal-mode binding of its
  command (`gd`, `grr`, `grn`, `gra`, `grf`), as kui draws an
  accelerator it cannot parse: written as is, bound by nobody. A
  binding with a modifier (`<D-s>`) is not shown, so nothing a hint
  says can become a platform's shortcut.
- **The bar binds three chords, and only while free.** ⌘Q (Quit,
  `quit all`: refused, as `:qa` is, while a buffer is unsaved and the
  history does not keep it — where winit's Quit closed regardless), ⌘, (Settings,
  `settings`, the pane) and ⌘M (Minimize). A user who maps one gets
  it back, since the bar reads the keymap — every mode, every scope,
  cached by the keymap's version — and leaves a taken chord off its
  row. A chord bound only to its row's own command, global and
  unconditioned, is not taken: AppKit running the row runs what the
  key would. ⌘, is such a one — the settings pane binds it in every
  mode (settings.md) — so Settings… opens the pane, not the file. The Edit menu's rows bind nothing, so ⌘C,
  ⌘V, ⌘A, ⌘Z reach the keymap and kui's runner exactly as they do
  without a bar.

*Beat:* the Edit rows with ⌘C and the rest. kui's standard bar (ADR
0030) has them and *replays* the chord into the window, but a
declared bar replaces it and a declared row cannot replay: bound to
⌘C it would run the row instead of the key, and ⌘C means visual `y`
in an editor, the grid's selection in a terminal, and a field's copy
in a Lua view — three paths a menu row would have to restate. The
rows still work from the menu; they show no chord.

### 5. The bar only where the platform owns one

`sync_menu_bar` declares it only while `Core::native_menu_bar()` — on
macOS. On Windows and Linux kui would draw it as a strip in the window,
a second title bar in an app whose chrome is its own and whose keys
are the way; the context menus are there.

The menus: **kawoosh** (Keys, Commands…, Settings… ⌘,, Project
Settings, Quit ⌘Q), **File** (New Tab, New Terminal, New Scratch, Open
File…, Open Recent…, Save, Save All, Close Pane, Close Tab), **Edit**
(Undo, Redo, Cut, Copy, Paste, Select All, Find…, Find in Files…),
**View** (Bigger, Smaller, Actual Size, light and dark, the dock,
Split Right and Down), **Go** (Definition, References, Symbol…, Back,
Forward, Next and Previous Problem), **Window** (Minimize ⌘M, the tabs,
the next pane — named `Window` so it is the platform's: Fill, Center,
tiling and full screen join it), **Help**. Rebuilt every frame from
the focused pane (an editor's rows lit, a terminal's Copy kui's) and
diffed by kui, so an unchanged bar rebuilds no `NSMenu`.

**What a declared bar gives up.** Without one, kui's runner keeps
winit's application menu (About, Hide ⌘H, Hide Others, Show All,
Services, Quit) and its own Edit menu. A declared bar is exactly what
it declares (ADR 0030 Decision 2), and kui has no row for Hide or
Services (application-menu roles, declined in ADR 0018 until an app
asks). kawoosh asks: that is a kui round, and until it lands ⌘H does
nothing in kawoosh.

## Built

`kawoosh/src/menus.rs`: the three menus, the bar, `on_edit_button`,
`on_context_menu`, `on_menu` and `menu_edit`. `panes.rs` declares the
title bar's and the grid's menus and the editor's claim;
`app.rs` routes `editbutton`, `contextmenu` and `menu`, and declares
the bar each frame. Tests, `kawoosh/tests/menus.rs`:
`a_right_click_in_the_text_places_the_caret_and_opens_the_editors_menu`,
`the_edit_rows_run_what_the_keys_run`,
`a_panes_title_bar_has_the_panes_menu`,
`a_terminals_grid_has_kuis_copy_and_the_panes_rows`,
`a_lua_views_body_is_left_to_the_view`,
`the_menu_bar_is_declared_where_the_platform_owns_one` (the platform's
half by hand: `set_native_menu_bar`, `activate_menu_bar_item`).

Built and tested on Linux, where kui draws the context menus; seen on
a Mac 2026-10-03, where the same rows go to `NSMenu`: the bar (read
through Accessibility, rows run from it), the editor's menu at the
press with Go to Definition taken, a title bar's, a terminal's. Two
fixes from it. The language server's rows read `Editor::can`, and only
`lsp definition` and `lsp hover` have a `when` — and its `lsp` fact is
"a server is up somewhere" — so References, Rename and Code Actions
were lit in a file no server serves; they ask the shell now (Decision
2's table). Gating the commands instead was tried and beaten: a gate's
reason ("only in a buffer with a language server") hid the server's
own ("did not start: Could not find a valid TypeScript installation"),
and `lsp action N` runs from the actions' picker, where the focused
pane is not the buffer. And Settings… ran `settings user` while ⌘,,
bound since the settings pane (roadmap step 72), opened the pane — so
the bar left ⌘, off a row that did something else (Decision 4).

Not drawn on a Mac: the keys beside a row (Decision 4's hint). kui's
`macos_menu.rs` keeps an accelerator only when it parses with a
modifier, so `gd` reaches `NSMenu` as nothing, though its comment says
modifier-less ones are drawn. A kui round: a hint drawn in the title
(an attributed title with a right tab stop), bound by nobody.
