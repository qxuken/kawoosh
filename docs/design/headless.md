# Headless: kawoosh with no window

Status: built 2026-10-24, on kui's runner with no window (kui C58, ADR
0057, `Launcher::open_headless`, on kui's main by path until a release
carries it): `kawoosh/src/headless.rs`, `presses.rs`, `main.rs`'s
`headless`. Items 1, 2 and 4 of *Cost to build* are built; 3 (pictures
from `kawoosh test`) and the gallery are not. Each decision keeps the
alternative it beat; *As built* at the end says what the build changed.

## The thesis

Up to alpha.66 kawoosh had two ways to run: a window, which draws real
pixels but needs a screen and takes the user's keyboard focus, and the
harness (`harness.rs`, `kawoosh test`), which needs neither but has no
pixels, a frame clock that only moves when the test says so, and no
socket. kui can now draw a headless core's finished frame on a device of
its own (`kui_native::capture`, `Captures::serve`), with no window and
no display server. That makes a third way possible: **the real editor,
running on the real clock and listening on its socket, drawing frames no
one sees, and writing them to files when asked.**

## What is there today

| | window | `kawoosh test` / `Drive` | proposed `--headless` |
|---|---|---|---|
| who runs frames | kui's runner (winit) | the test, by hand | a loop of kawoosh's or kui's |
| clock | real | fake (`advance`) | real |
| jobs (Lua, io, LSP) | threads, woken | inline (`jobs_inline`) | threads, woken |
| socket (`kawoosh ex`) | yes (`App::setup`) | no | yes, under another name |
| keys from outside | the OS | `Drive::key` | `Drive::key`, over the socket |
| pictures | `:screenshot`, served by the runner | none: no one calls `Captures::serve` | served after each frame |
| config, fonts | the user's, all fonts | none, symbols only | the user's or none, all fonts |
| GPU | yes | no | only for pictures |

What is already pinned: `kawoosh/tests/screenshots.rs`'s GPU test serves
a `:screenshot` asked from a headless `Drive` through a `Captures` it
holds itself, and the PNG is written at the drive's size times scale. So
**a headless kawoosh core can already take pictures of itself.** What
is missing is the loop around it, not the drawing.

## The runner's jobs a headless loop takes on

`main.rs` hands the app to `kui_native::app(...).run(app)`, and the
runner does six things kawoosh depends on:

1. **`App::setup(waker)`**. kawoosh's every system wakes through the
   `Waker` it is handed: `WakeHandle::set`, `set_at`, the native
   extensions' waker. The socket's `listen` is in `setup` too.
   `kui_native::Waker` is a winit `EventLoopProxy`, and **nothing outside
   kui can make one**. This is the decisive fact for Decision 1.
2. **Frame pacing**: a frame on a wake, at a `wake_at` time, at the
   view's `request_frame_at`, on input. `Drive` has `wake` and `due`; it
   does not sleep until either one comes.
3. **`Captures::serve` after each drawn frame**, with `write_on_thread`
   for recordings.
4. **Menu actions**: the clipboard text that `clip_out` hands over
   (`Effect::SetClipboard`), a copied screenshot's picture, the Mac's
   menu bar.
5. **`on_event_with(core)`**: events with the window's core lent.
   `Drive` delivers these as the runner does.
6. **System facts**: appearance (dark or light), scale, the window's
   size, focus, occlusion.

## Use cases, most valuable first

### 1. Agent and smoke runs that cannot touch the user's desktop

This is the reason to build it. The way a test build is checked today
(memory: driving by socket, the keystroke guard) is a release window
started with isolated `XDG_*` and `KAWOOSH_STATE`, driven with `kawoosh
ex`, and photographed with `:screenshot`. Keys that have no command go
through a `CGEvent.postToPid` tool, and twice they landed in the user's
own Kawoosh.app instead (2026-09-29). The window also comes up over
whatever the user is doing.

Headless, the driver owns the core, so keys are `Drive::key`: a
`Request::Keys { notation }` over the socket (`kawoosh keys '<C-w>v'`),
parsed by the harness's `press`, reaches exactly this process's core and
nothing else. No window appears, nothing takes focus, and a key cannot
land in the wrong app. Add `:screenshot` answered once the file is
written, and an agent's loop of drive, look and check runs on any
machine, the user's laptop included, while they keep working.

### 2. Pictures from `kawoosh test`

The harness frames by hand and never serves captures, so `:screenshot`
from a Lua test is asked and never answered. Two small changes:

- `Harness` holds a `Captures` and serves it after `frame` (no GPU: the
  ask fails "no GPU" and the test goes on).
- A failing script leaves a picture of its last frame beside its
  message (`FAIL x.lua … last frame: /tmp/…/x.png`). A failure you can
  look at beats `line_rows()` printed out.

Then the Lua door `screenshots.md` lists as not done,
`kawoosh.screenshot(path)`, becomes useful in tests first.

### 3. Pictures for the docs, the help and the README, rebuilt by a script

kui does this already: its examples' `--shot` and `--record` draw the
example headless and write the book's pictures (`examples/devtools`).
For kawoosh the same thing is one script in, a folder of PNGs and APNGs
out: the tutor's steps, `:help` pages that show a pane, the launcher,
every theme on one sample file (11 themes × dark and light, beside
`:theme lab`), the statusline's modules. They are rebuilt when the look
changes, never shot by hand.

### 4. A gallery to review, not golden images to assert

Pixel-exact asserts across machines are fragile: grayscale against
subpixel AA (ADR 0054: a headless core draws grayscale), GPU drivers,
fonts. kawoosh's harness already asserts on what was drawn as text
(`line_rows`, `gutter_texts`, `corner_texts`), and that stays the test.
A **gallery** is the useful form: the same N states drawn on each
`scripts/verify.nu` run (the overflow sweep's narrow widths, the
diagram round's shapes, the completion popup, the memory pane's still
lines hovered) and written to a folder, so a person or an agent looks
at the diff against the last run. A hard pixel assert only where a bug
had no text-level reading: the alpha.67 `hover_bg` wash was a colour,
not a text.

### 5. Turning bug reports into pictures

`:terminal record` already keeps a terminal session as a `.cast` file
for a bug report, and `:terminal replay` plays it back. Headless,
`kawoosh --headless --size 1100x760` with a script that replays the cast
under `:screencast` turns it into an APNG of what the pane drew, on the
user's fonts and theme, with no screen recorder and no window to frame.

### 6. A display-less box: Linux over ssh, a container

winit needs an X or Wayland display to open its event loop at all. A
headless loop that never starts winit runs where there is none: a Linux
box over ssh, a container, a CI runner. Layout, Lua, LSP
and terminals all work with no GPU; pictures need an adapter, which on a
GPU-less Linux box is Mesa's lavapipe, as kui's CI does
(`KUI_REQUIRE_GPU=llvmpipe`).

### What it is not for

- **The frame probes** (`probe-scroll.nu`, `probe-fonts.nu`, `:frames`,
  the frame ledger). They measure what a window costs, presenting and
  pacing included. Headless measures the view and layout only, which is
  a reading of its own, not a replacement.
- **The OS's side**: IME, drag and drop from Finder, the Mac's menu bar,
  occlusion, live resize. These stay window-only.
- **A remote or browser front end**: the wasm build is not an intended
  target (memory: kawoosh in a browser), and frames as PNGs over a wire
  would be a worse version of it.

## Decisions (proposed)

### 1. The loop is kui's: a runner with no window

`Launcher::run_headless(app)` (or `kui_native::headless::run`): the same
`App` trait, `setup` handed a `Waker` that is a channel rather than a
winit proxy, frames paced by wakes, `wake_at` and `request_frame_at` on
the real clock, `Captures` served after each frame (recordings on
their thread), menu actions drained (the clipboard written through
arboard as the runner writes it; whether that holds with no window on
each platform is to be checked), the viewport and
scale given rather than read from a monitor, and input coming in through
a handle the app is given (`HeadlessInput`), since no OS sends any.
Every binding gets it: Node's `Ctx` and C's headless drives are the
same loop written by hand today.

This is a kui F-round on a `claude/` branch, merged when the user says
(memory: kui rounds from kawoosh).

Alternative: **kawoosh's own loop** in `headless.rs`, over a `Drive` on
the real clock. Feasible, but `App::setup` takes a `kui_native::Waker`
that kawoosh cannot make, so `setup`'s body (the wakes, the socket's
`listen`, the editor shim) moves into a `Kawoosh::attach(wake, at)` both
paths call, and kawoosh writes the pacing and the capture serving that
kui's runner already has. That is a second runner to keep in step with
the first, in the one app that needs it. Kept as the fallback if kui's
round waits. Its smallest form, a `Waker::from_fn(wake, wake_at)`
constructor in kui with the loop in kawoosh, is the middle way.

### 2. `kawoosh --headless`

```
kawoosh --headless [--size 1100x760] [--scale 2] [--no-config] [PATH]
```

- Starts as the window does, with fonts, config and plugins, `PATH`
  opened the same way, but **never restores the session and never saves
  one**. A headless run must not leave the user's next window on its
  files.
- `--no-config`: none of kawoosh's config read — the user's `init.lua`
  and `settings.lua`, the project's `.kawoosh/settings.lua` and
  `.kawoosh/init.lua` — so what is drawn is what kawoosh ships and
  nothing of the user's is read. A file's `.editorconfig` still applies:
  it is the file's, not kawoosh's. This is what an agent's smoke run and
  the docs pictures use. Without it the pictures are of the user's own
  setup, which is what a bug report wants.
- Its first line on stdout is `KAWOOSH_SOCKET=/path`, so a driver
  started by a script knows where to talk without guessing a pid.
- It ends on `:qa`, on SIGTERM or SIGINT, or when stdin closes (a
  parent that died leaves no orphan). `teardown` runs in every case.

Alternative: a verb (`kawoosh headless`) beside `test`. But the window's
own arguments (a path, `--`) carry over unchanged, which is what a flag
on the window's command line says.

### 3. Its socket is not the one `kawoosh edit` finds

`running::socket()` falls back to "the Kawoosh started last", and
`kawoosh-edit` (every terminal's `$EDITOR`) and `--reuse` use it. A
headless kawoosh at `kawoosh-<pid>.sock` would quietly take the next
`git commit` from any terminal into a window no one can see. So it
listens at `kawoosh-headless-<pid>.sock`, which `running_sockets` does
not list, and only `$KAWOOSH_SOCKET` reaches it.

### 4. Keys over the socket, headless only

`Request::Keys { notation }` (`kawoosh keys '<C-w>v'`): the harness's
map-notation parser (`Harness::press`) moved where both use it, and
each press goes in as kui input, so a terminal pane, a Lua pane's field
or the picker gets it exactly as from a keyboard. It is answered after
the frame the last key produced, with the corner message, as `ex` is.
A window refuses it ("keys: only a headless kawoosh takes keys"):
injecting input into a window the user is typing in is the hazard this
note exists to remove.

Alongside it: `Request::Wait { idle }`, answered at the first frame with
no job out (the harness's `settle` on the real clock), so a driver's
`keys` then `wait` then `ex ':screenshot x.png'` shows what the keys
did, not a frame before a highlight landed.

### 5. Motion pinned for pictures

A screenshot in mid-glide is a different picture each run. `--headless`
draws with `MotionPref::Reduced` unless `--motion` is passed, so a
scroll lands in one frame and a toast stops fading. A screencast that
wants the motion passes `--motion full`.

## Cost to build, roughly

1. kui F-round: the headless runner, with a test driving its counter
   example over the input handle and capturing (kui-native). The bulk
   of the work.
2. kawoosh: `--headless`, `--no-config`, the socket name, `Request::Keys`
   and `Wait`, `press` moved out of the harness. One round, with a
   `kawoosh/tests` test that starts the binary headless, sends keys
   over the socket, and reads the buffer back with `ex`.
3. `kawoosh test` serving captures, and the failure picture. Small and
   independent of 1 and 2: it can land first.
4. Later: the docs-pictures script and the review gallery in
   `verify.nu`, once there is a reason to look at them.

## As built

- **Decision 1 went kui's way**: kui C58 is the runner, `Waker` grown a
  headless target, so `App::setup` is kawoosh's unchanged and the loop,
  the pacing, the captures and the clipboard are kui's.
- **The state db is always the run's own** (a scratch directory under the
  temp directory, removed at the end), not only with `--no-config`, unless
  `KAWOOSH_STATE` names one: a run leaves no moments, history or session
  in the user's db. The open question of whether `--no-config` should be the
  default was answered by that: without it a run reads the user's
  `init.lua` and `settings.lua` and writes nothing of theirs.
- **A project's `.kawoosh/init.lua` is never asked about.** The run's db
  has no trust records, so every project file is unknown, and the confirm
  would have taken the driver's first keys (it did, on the first try:
  `jjjwv$` went into *trust and run*). An untrusted file does not run,
  the log says so, and `kawoosh ex ':trust'` runs it.
- **`kawoosh keys` clears the corner message first**, as `ex` does, so
  its answer is what the keys said and not the last line's word.
- **`kawoosh wait` rings the frame itself**: it arrives in a view that
  has already asked for its frames, and with nothing else owed none would
  come. It answers `idle`, or the corner message.
- **`--no-config`, not `--clean`** (renamed 2026-10-24, the user's call:
  `--clean` did not say what it cleaned). The flag says what it does, and
  does all of it: built as `--clean`, it skipped the user's two files
  and still read the project's `.kawoosh/settings.lua`, so a picture taken
  in a repository with one was not the shipped look. Now the project
  layer is not read either, on start or on `:cd`. Alternatives:
  `--no-user-config` (true of `--clean` as first built, and the project
  half then left unnamed), `--defaults` (reads as a setting's value),
  neovim's `--clean` (the name the user found unclear).
- **`--dark` and `--light`** pin the system's base, beside `--motion`.
- **Lua types are not written** and the update watch is not started: a
  run is not a build the user runs.
- `--help` lists the flags and the two verbs; `:help terminal` has a
  section.

Tests: `kawoosh/tests/headless.rs` starts the binary headless in a home of
its own, types into a buffer over the socket, `:w`s it and reads the file
back, checks the socket is not one `running_sockets` lists, ends it with
`:qa` and with its stdin closed; `presses::tests`, `headless::tests` (the
options, a window refusing `keys` and `wait`). By hand: `<C-w>v` and a
visual selection shot with `:screenshot`, SIGTERM ending a run with its
socket and directory gone.

## Open questions

- Should a headless run start LSP servers and file watchers as the
  window does, or should `--no-config` leave them off for speed? Built as the
  window does.
