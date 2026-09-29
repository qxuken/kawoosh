# Panes, tabs and workspaces

How the window is laid out: panes and splits, tabs, the scrolling strip
of columns, the dock, a working directory per tab, and the launcher a
new pane opens on.

## Panes and splits

A pane shows a buffer, a terminal or a tool's view. Closing a pane only
hides what it showed: buffers and terminals live on and come back with
`:b`, the buffer picker or the launcher. Everything about panes is under
`<C-w>`.

| keys | what |
|---|---|
| `<C-w>v` `<C-w>s` | split beside, below (`:vsplit`, `:split`) — the new pane opens on [the launcher](#the-launcher) |
| `<C-w>q` `<C-w>c` | close the pane (`:close`) |
| `<C-w>o` | close every other pane (`:only`) |
| `<C-w>h` `<C-w>j` `<C-w>k` `<C-w>l` | focus the pane left, below, above, right (also `<C-w>` + arrows) |
| `<C-S-h>` `<C-S-j>` `<C-S-k>` `<C-S-l>` | the same, straight — from every mode and every pane, terminals included |
| `<C-w>w` `<C-w>x` | the next pane, swap places with it |
| `<C-w>H` `<C-w>J` `<C-w>K` `<C-w>L` | carry the pane a place left, down, up, right |
| `<A-S-h>` `<A-S-l>` | the pane narrower, wider (also `<C-w><` `<C-w>>`) |
| `<A-S-j>` `<A-S-k>` | the pane shorter, taller (also `<C-w>-` `<C-w>+`) |
| `⌘1` … `⌘9`, `<C-S-1>` … `<C-S-9>` | the Nth pane, or the Nth column of a strip (`:pane goto N`) |
| `<C-w>:` | the command line, from a pane without one (a view of a plugin's); from a terminal, `<C-\>:` |

Sizes take a count: `3<A-S-l>` is three steps wider. The dividers between
panes drag with the mouse.

`:split PATH` and `:vsplit PATH` open PATH straight away; `:new` and
`:vnew` a fresh scratch buffer below, beside.

### Dragging panes

Every pane has a title bar, and dragging it moves the pane. Let go over
the middle of another pane and the two swap places; let go near one of
its edges and the pane moves to that side of it. A pane dragged from the
tab into the dock, or out of it, moves there. The spot it would land on
is highlighted while you drag.

## Tabs

A tab is a layout of its own, shown in the strip at the top of the
window. A click on a tab goes to it; its `×` closes it.

| keys | what |
|---|---|
| `<C-w>t` | a new tab (`:tabnew`, or `:tabnew PATH`) |
| `gt` `gT`, `]t` `[t` | the next, previous tab |
| `<C-Tab>` `<C-S-Tab>` | the same, from every mode and pane, terminals included |
| `]T` `[T` | move the tab a place right, left (`:tabmove +N`, `-N`, `N`) |
| `<C-w>C` | close the tab and its panes (`:tabclose`) |
| `<C-w>m` | flip the tab between a strip and a tree (`:layout`) |
| `<leader>t` | the tools — lazygit, top, a shell, your compile and run commands — in a picker |

A tab's label is its number and the name of what its focused pane shows,
with `●` when something in it is unsaved. When your tabs are in more
than one directory, each label starts with its directory's name
(`kui · lib.rs`), so you can tell the projects apart. `tabs.directory`
changes that: `auto` (the default), `always`, or `never`.

To write the labels yourself, give `kawoosh.tab_title` a function in
your `init.lua`. It gets each tab's parts — `index`, `active`, `title`
(the label kawoosh would draw), `dir`, `cwd`, `kind`, `name`, `path`,
`modified`, `bell`, `panes` — and returns the label, or `nil` for the
default:

```lua
kawoosh.tab_title(function(tab)
  return tab.index .. " " .. tab.dir .. (tab.modified and " +" or "")
end)
```

## The scrolling strip

A tab is either a **tree** of splits, as in vim, or a **strip**: columns
laid left to right on a ribbon wider than the window, which scrolls
sideways to follow the focused column. A new column pushes the ribbon
instead of squeezing its neighbours, and keeps its width, so five panes
side by side stay readable. New tabs are strips by default; set
`layout.default = "tree"` for splits.

The pane keys mean the same thing on the strip's axis:

| keys | in a strip |
|---|---|
| `<C-w>v` | a new column after the focused one |
| `<C-w>s` | a split below, inside the column |
| `<C-w>h` `<C-w>l` | the column before, after — revealed if it is off screen |
| `<C-w>j` `<C-w>k` | the pane below, above inside the column |
| `<C-w>H` `<C-w>L` | move the column a place left, right |
| `<C-w>J` `<C-w>K` | move the pane down, up inside its column |
| `<A-S-h>` `<A-S-l>` | step the column's width down, up: a third, a half, two-thirds, full |
| `<C-w>e` | take the pane out of its column into a column of its own |
| `<C-w>i` | pull the next column's top pane into this column, under the focused one |
| `zs` `ze` `zz` | put the focused column at the left edge, the right edge, the middle (`:strip left`, `right`, `center`) |
| `⌘1` … `⌘9` | the Nth column |

`:layout scroll` and `:layout tree` turn the tab into one kind or the
other, keeping its panes; a bare `:layout` flips it. The status line
shows the columns as `▯▮▯`, the focused one filled. Dragging the gap
between two columns sets a width of your own. A sideways trackpad swipe
moves the ribbon — over a list, a terminal or the gaps, anywhere but text
wider than its pane, which scrolls itself — and a drag of the scrollbar
does too. A swipe keeps to what it started on: one that began on the
ribbon goes on moving it as a terminal passes under the pointer, and
one that starts where a list is already at its end moves what holds
the list instead.

| setting | what |
|---|---|
| `layout.column_width` | a new column's width: `third`, `half` (the default), `two-thirds`, `full`, or a fraction |
| `layout.scroll.center` | `always` keeps the focused column centred; `never` (the default) scrolls only as far as needed |
| `layout.gap` | the gap between columns, in pixels |

## The dock

The dock is a pane area along the bottom that stays put whichever tab is
in front — the place for a dev server, a watcher, a long build.

| keys | what |
|---|---|
| `<C-w>d` | show or hide the dock (`:dock`); the first time, it opens a terminal |
| `<C-w>D` | move the focused pane into the dock, or out of it into the tab |

`<C-w>j` `<C-w>k` move the keyboard between the dock and the tab;
`<C-w>J` from the tab's bottom carries a pane down into the dock, and
`<C-w>K` from the dock's top carries it back. A split made from a dock
pane stays in the dock. The dock is a tree of
splits; `layout.dock = "scroll"` makes it a strip of columns instead.

The title bar counts the dock's tasks (`2 docked`), brighter while the
dock is hidden and they run out of sight; a click on it shows or hides
the dock.

## Workspaces

Each tab has its own working directory. The focused tab's is the one
`:e`, the file picker, grep, a new terminal and a compile start from, and
the one the title bar shows. `:cd PATH` moves only the focused tab; a new
tab starts where the tab it was made from is. `:pwd` says where you are.

A **workspace** is the project a directory belongs to: the outermost
directory above it with a `.kawoosh` folder, else the root of its git
(or jj, hg) repository. Pinned files, recent files and the memory are
kept per workspace, and a project's `.kawoosh/settings.lua` applies while
a tab in it is in front. With `buffers.scope = "tab"` (the default) the
buffer lists show the focused tab's buffers; `<C-a>` in the buffer
picker shows every tab's.

| keys | what |
|---|---|
| `<leader>ww` | workspaces you worked in before: a pick moves the tab there and opens the file you had last |
| `<leader>sd` `<C-S-z>` | directory jumps (zoxide's, when installed): `<CR>` makes one the working directory, `<C-t>` opens a new tab on it |
| `~` | in a directory listing, make the listed directory the working one |
| `<leader>ws` `<leader>wr` | save, restore the session (it is also saved on quit and restored by a bare `kawoosh`) |

A dock pane belongs to the project that was in front when it opened, and
its title starts with that project's name while another is in front.
When the last tab of a project closes, or is `:cd`'d away, its idle dock
terminals close with it; if something is still running you are asked
whether to end it.

## The launcher

A pane made bare — `<C-w>v`, `<C-w>s`, `<C-w>t`, `:vsplit`, `:split`,
`:tabnew` without a path — opens on the launcher: a list of what the new
pane could be. Its sections are *here* (the buffer you split from, a
scratch, a terminal, the directory), the open buffers, your tools and
what plugins add, your pinned and recent files, the workspaces you worked
in before, and — once you type a query — every file under the working
directory.

It opens in normal mode. While the query is empty a key launches: `<CR>`
the first row (the same buffer, so `<C-w>v<CR>` is vim's split), `s` a
scratch, `t` a terminal, `d` the directory, `1` … `9` a pinned file,
and each tool its own letter, shown at the end of its row. `i`, `a` or
`/` start a query; `j` `k` move the cursor; `<Esc>` settles on a
scratch; `q` or `<C-c>` closes the pane again. `:` opens the command
line, and `:e PATH` from there fills the new pane.

| setting | what |
|---|---|
| `layout.new_pane` | what a bare split is: `launcher` (the default), `same`, `scratch`, `terminal`, `dir` |
| `layout.new_tab` | the same for a bare tab |
| `launcher.start` | `normal` (the default) or `insert`, to start typing at once |
| `launcher.layout` | which modules it shows, in what order and shape (below) |
| `launcher.width` | how wide it is drawn, a [size](settings.md#sizes): `720`, `"100%"`, `"clamp(400px, 80%, 1000px)"`; never wider than the pane |

### Its layout

The launcher is modules placed by `launcher.layout`, read again when
`settings.lua` is saved — so an open launcher shows an edit at once:

```lua
launcher = {
  layout = {
    "prompt",                                              -- the query
    { row = { { module = "here", style = "tiles" }, "plugins" } }, -- side by side
    { module = "pins", title = "pinned" },
    { module = "recent", limit = 5 },                      -- 5 rows until you type
    "...",                                                 -- whatever plugins add
    "files",
  },
}
```

The modules kawoosh has: `prompt`, `here`, `buffers`, `plugins`,
`pins`, `recent`, `files` (only with a query) and `workspaces`. A
place can override a module's `title` (`false` for none), `limit`,
`show` (`always`, `blank` — only while the query is empty — or
`query`), `style` (`list`, or `tiles` that wrap) and `keys` (whether
its rows take letters). `{ column = { … } }` stacks modules inside a
row, and a `width` sets a column's — a [size](settings.md#sizes), of
the row it is in (`"40%"`, `"clamp(200, 30%, 400)"`); columns side by side
share what the row has, a percentage giving back what the gap between
them needs, as in CSS (two `"50%"` columns fit). What comes after `prompt` scrolls
and what comes before it stays; with `prompt` last, the field sits at
the bottom. A file is listed once, in the first module that has it, so
`pins` before `recent` takes the pins out of recent. A name that is no
module shows as a line saying so.

`init.lua` or a plugin adds a module with `kawoosh.launcher.module`
— rows (`items`, `load`, a picker `source`) or a block drawn as it is
(`draw`) — and a row to one with `kawoosh.launcher.entry`; see
[Lua](lua.md).
