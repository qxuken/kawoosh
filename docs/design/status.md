# The title bar and the tab strip, Lua's too

Status: decided 2026-09-28 (roadmap step 64), the calls taken here.
Asked in the todo: "configurable tabbar and title? maybe someone likes
to watch diagnostics numbers. or i like to see a clock on windows with
hidden taskbar." *Built 2026-09-28.*

## What there is

The title bar (chrome.rs, step 13) draws the working directory on the
left and, on the right, blocks kawoosh fills itself: the language
servers and their documents (a click is `:lsp info`), `compiling…`
while a compile runs. The tab strip gives each tab an even share of its
row, and a tab dragged along it goes to the place under the pointer
while it is held (2026-10-01; `]T` `[T` by hand — the even share is
what makes the place a division of the row's content). Tabs whose
order changed glide to their places, by the drag or the keys alike;
nothing else in the row glides, so the gliding is declared from the
reorder until it has run and no longer (`chrome::TabsGlide` says why:
kui's `slide` eases a place in the window whatever moved it — the
wheel, a tab made, a resize). A plugin can rewrite each tab's label
(`kawoosh.tab_title(fn)`, step 50) and nothing else — the strip's own
room and the title bar's are kawoosh's alone. wezterm's answer is
`update-status`, a callback that sets a left and a right status on its
tab bar, on a timer.

## Decisions

### 1. `kawoosh.status(name, fn, opts)`: a segment

A named segment a plugin or `init.lua` adds, drawn on the right of the
title bar (`place = "title"`, the default) or at the right end of the
tab strip (`place = "tabs"`), in `order` (then by name), before
kawoosh's own blocks. `fn(ctx)` answers what it shows now: `nil` for
nothing (it is hidden), a string, or a part `{ text =, color = }`, or a
list of parts — `color` one of the theme's words (`fg`, `dim`,
`accent`, `ok`, `warning`, `danger`). `opts.run` is a command line a
click on it runs (`"diagnostics"`), as the servers block runs `lsp
info`. `kawoosh.status(name, nil)` takes it away. A segment whose `fn`
fails is taken away and says why once, as `tab_title`'s hook does.

The same name again replaces the segment, so a config reloaded is not
two clocks.

### 2. Asked when the chrome is drawn, and on a timer when it says so

`fn` is called each frame the window draws — a call of a few
microseconds, and a frame is drawn on every event, a pty's output
included — so a segment that reads state (diagnostics, the mode, a
branch) is never behind it. One that changes with time alone says
`every = SECONDS`: the window wakes then to draw it, lined up on the
wall clock (a minute's clock turns over on the minute, not a minute
after it was first drawn), and not otherwise — an idle window with no
such segment still draws nothing.

### 3. Two shipped, off by default

`kawoosh/lua/status.lua`, bundled, is what the API is for, and its
first two users:

- **A clock**: `status.clock`, an `os.date` format — `"%H:%M"` shows
  the time, `""` (the default) nothing; `every` the minute, or the
  second when the format has `%S`. On the title bar's right, the last.
- **The diagnostics' counts**: `status.diagnostics = true` shows the
  workspace's errors and warnings (`● 3 ▲ 5`, the error count in
  `danger`, the warnings in `warning`), nothing when there are none; a
  click opens the diagnostics list. Counted by `kawoosh.lsp.counts()`,
  new: the four severities' counts from what is published, without
  building the list `kawoosh.lsp.diagnostics()` builds.

*Amended 2026-09-29 (roadmap step 70):* a third place, `"statusline"`
— the line under the panes as modules, [statusline.md](statusline.md).

### 4. Not the window's title

The OS window title (what the taskbar and Mission Control say) stays
kawoosh's: kui had no call to set it after the window opens, and what
was asked is what is on screen. kui has one since (`Ui::window_title`),
and kawoosh sets `TITLE — kawoosh` with it each frame. A `place = "window"` is the obvious
next word if it is asked for.

### 5. The strip shown as `tabs.show` says

*Decided and built 2026-10-21.* Asked: "Option to hide tabs when only
one exists (default), hide all (add picker), show always".

`tabs.show`, three words as `tabs.directory`'s and meaning the same
`auto`: `auto` (the default) draws the strip while there is more than
one tab, `always` with one too, as it always was, and `never` with any
number. Without it the panes reach up to the title bar: the row is the
panes' (`Kawoosh::tabs_h`, what the body's height, the toasts' place
and which-key's room take off, is nothing).

Hidden, nothing a strip carried is lost:

- **The tabs**: the keys go among them as before, and the tabs picker
  (`picker tabs`, `<leader>wt`, new) lists them — each row the label
  the strip would draw, the directory dim after it and matched too,
  the cursor starting on the one in front, the preview its file, a pick
  `:tab goto N`. It is a key whether the strip shows or not: among many
  tabs typing a name is quicker than reading the strip.
- **Which tab this is**: with more than one, the title bar says `tab 2
  of 3`, dim, among kawoosh's own blocks, and a click on it is the
  picker — the strip's place to click, kept.
- **The strip's segments** (`place = "tabs"`, Decision 1): the title
  bar's, after its own, since a plugin put them there to be seen.

The labels are made in one place (`Kawoosh::tab_facts`) for the strip
and for `kawoosh.tabs()`, which carries each tab's `title`, `kind`,
`name`, `path`, `modified` and `bell` now, so the picker is Lua and
reads them whether the strip is drawn or not. A `kawoosh.tab_title`
hook is not asked for the picker's rows: it is the strip's, called as
the strip draws ("keep it to strings"), and a picker of tabs written
for oneself reads `kawoosh.tabs()` as this one does.

*Beat:* **`auto` as "more than one" only while there is a second tab
in another workspace** — a rule to explain for a row that is either
needed or not; **the strip's segments hidden with it** — a clock that
goes when the last other tab closes is a surprise; **a count only,
not the picker on a click** — the click is free, and it is where the
strip was clicked; **`<leader>T`** for the picker — the workspace group
(`<leader>w`) is where the tabs' directories already live
(`<leader>ww` opens or goes to a tab on a workspace).

## Built

2026-09-28, as decided. `lua/lua/boot.lua`: `kawoosh.status` over
`kawoosh._status`; `lua/src/lib.rs`: `Runtime::status(place)` (each
segment asked, in order, its parts and its `run`; one that fails taken
away with an echo), `Runtime::status_every()` (a number, or a function
answering one), `kawoosh.lsp.counts()`. `kawoosh/src/chrome.rs`: the
title bar's blocks are parts in colours now, the segments first, then
kawoosh's own; the tab strip draws its segments in a row beside the
tabs when it has any; `sync_status_tick` asks one wake at a time for
the shortest `every`'s next beat on the wall clock (a soonest
`Alarm`, the loop asleep till then; until kui alpha.47 an io job
slept to it and sent `IoMsg::Tick`). `kawoosh/lua/status.lua`, bundled: the
clock (`status.clock`) and the counts (`status.diagnostics`). Tests:
`kawoosh/tests/status.rs` — a segment on each place, a click running
its command, hidden by `nil`, taken away by `nil` and by failing; the
clock drawn with a wake on the minute and none while it is off; the
counts drawn in their colours, a click opening the diagnostics.

Decision 5, 2026-10-21: `tabs.show` (editor/src/settings.rs);
`chrome.rs`: `tabs_shown`, `tabs_h`, `tab_facts` (the labels' parts,
`TabFacts`, out of `tab_strip`), the strip skipped while hidden (its
reveal and glide state forgotten), the title bar taking the strip's
segments and `tab N of M`; `kawoosh.tabs()` filled from `tab_facts`
(`TabSnap`, lua/src/lib.rs); `kawoosh/lua/picker.lua`: the `tabs`
source and `<leader>wt`. Tests: `kawoosh/tests/chrome.rs`
`the_strip_shows_as_tabs_show_says` (each word, the rows given back,
`tab 2 of 2` clicked into the picker, a pick going to the tab) and
`lua_reads_each_tabs_label`; `status.rs` a strip's segment in the title
bar while the strip is hidden; the tests about the strip ask for
`always`, `editor_pane.rs`'s windows a strip's row shorter.
