# The title bar and the tab strip, Lua's too

Status: decided 2026-09-28 (roadmap step 64), the calls taken here.
Asked in the todo: "configurable tabbar and title? maybe someone likes
to watch diagnostics numbers. or i like to see a clock on windows with
hidden taskbar."

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
kawoosh's: kui has no call to set it after the window opens, and what
was asked is what is on screen. A `place = "window"` is the obvious
next word if it is asked for.

## Built

2026-09-28, as decided. `lua/lua/boot.lua`: `kawoosh.status` over
`kawoosh._status`; `lua/src/lib.rs`: `Runtime::status(place)` (each
segment asked, in order, its parts and its `run`; one that fails taken
away with an echo), `Runtime::status_every()` (a number, or a function
answering one), `kawoosh.lsp.counts()`. `kawoosh/src/chrome.rs`: the
title bar's blocks are parts in colours now, the segments first, then
kawoosh's own; the tab strip draws its segments in a row beside the
tabs when it has any; `sync_status_tick` asks one wake at a time for
the shortest `every`'s next beat on the wall clock (`IoMsg::Tick`, a
thread sleeping till then). `kawoosh/lua/status.lua`, bundled: the
clock (`status.clock`) and the counts (`status.diagnostics`). Tests:
`kawoosh/tests/status.rs` — a segment on each place, a click running
its command, hidden by `nil`, taken away by `nil` and by failing; the
clock drawn with a wake on the minute and none while it is off; the
counts drawn in their colours, a click opening the diagnostics.
