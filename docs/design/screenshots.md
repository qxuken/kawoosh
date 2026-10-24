# Screenshots: the window keeps pictures of itself

Status: built 2026-10-24 on kui `=0.1.0-alpha.66` (kui ADR 0054, the
frame drawn again for its pixels; ADR 0055, what an app asks a driver to
keep), `kawoosh/src/screenshots.rs`. Each decision keeps the alternative
it beat.

## The thesis

Looking at a running kawoosh meant asking the OS: `screencapture -R` of
the window's frame, which needs the window in front, or `screencapture
-l` with an id found by a small Swift tool walking
`CGWindowListCopyWindowInfo` for the process's largest window — and a
guard that it is not the user's own Kawoosh.app. kui alpha.66 draws a
finished frame again on a device of its own and writes it, so the
window can take the picture: in front or behind, the same pixels it
drew, no window to find.

## Decisions

### 1. Three commands, each the kui verb it asks for

| command | kui | what |
|---|---|---|
| `:screenshot [FILE]` | `Ui::capture_to` | the next frame, into FILE |
| `:screenshot copy` | `Ui::capture_to_clipboard` | the next frame, on the clipboard |
| `:screencast [FILE]` | `Ui::record_to` | every frame from the next, as an animation |
| `:screencast stop` | `Ui::stop_recording` | the recording ended and written |

The format is FILE's extension, kui's one rule (`CaptureFormat`):
`.png`, `.jpg`, `.webp`, `.gif` for a still; `.png` / `.apng` (an
animated PNG) or `.gif` for a screencast. kawoosh asks kui's rule
itself before it asks for a frame or makes a directory, so a name kui
would refuse leaves nothing behind, and the corner says kui's words.

`:screencast` rather than `:record`: `:terminal record` is a terminal's
session as text, and `q` records a macro; a bare `:record` would be the
third thing the word means. Alternative: `:screenshot record`, which
reads as a still that records.

### 2. Where a picture goes unless a file is named

A dated file in `screenshots` under the data directory
(`~/.local/share/kawoosh/screenshots/2026-10-24-141502.png`), beside
`recordings`, which is where `:terminal record` puts its `.cast` files
and by the same namer (`term_cast::dated_file`). A screencast is
`.apng`, so the folder says which files move. A FILE naming a directory,
or with no extension at all, takes the dated name inside it
(`:screenshot ~/Desktop`, `:screenshot shots`), and FILE's directory is
made when it is not there. Not a trailing `/`: the engine normalises a
path argument, and the slash is gone before the command sees it.

Alternatives: the working directory, which puts pictures into a project
a `git status` then shows; the Desktop, macOS's habit, which is not the
habit on Linux or Windows; a cache directory, which the system may
empty — and these are files the user asked to keep.

### 3. The ask is queued; the answer is a message on the root

A command has no `Ui`, so it queues the ask (`Screenshots::out`) and the
view hands it to kui at the next frame, as `clip_out` hands the
clipboard its text. kui serves it after that frame is drawn and answers
with a `capture` event in the message the ask named:
`Msg::Capture { of: Shot }` (`Still`, `Copy`, `Cast`), matched in
`on_ui_event` like every other event. The answer says in the corner
message where the file is, its size in physical pixels and, for a
screencast, its frames and seconds; a failure says kui's reason. Never
a toast: the user asked, and the corner is where an answer to a command
goes.

`:screenshot` clears the corner message, since the frame it takes is
the next one: the picture shows the window, not the command's word.
`:screencast` says where it is recording, and that word is in its
first frames — the one place the user is told a recording runs.

Stills and copies are answered in the order asked, so their callers
wait in a queue; a screencast is one at a time (`:screencast` while one
runs says so), its answer coming when `:screencast stop`'s frame has
been written — on a thread of kui's, a GIF taking a while.

### 4. Over the socket, the reply waits for the file

`kawoosh ex ':screenshot /tmp/shot.png'` answers when the file is
written, with the whole path (not `~`-shortened), and `kawoosh ex
':screencast stop'` when the recording is: `on_request` lends the
line's reply to whatever ask the line makes (`Screenshots::asker`), and
a line that asks nothing answers at once as before. An agent driving a
test kawoosh takes a shot and reads it in two steps, with no window id,
no frontmost window and no chance of photographing the user's own
Kawoosh.app.

A window that never draws again never answers: kui serves an ask after
a frame. A covered or minimised window is still served (the frame is
drawn again off screen); a window that has closed is not.

### 5. Menu rows, not keys

Window ▸ Save Screenshot and Copy Screenshot in the Mac's bar; no key
is bound. Taking a screenshot is rare, and every chord near the obvious
ones means something already.

The frames a screencast's answer names are the frames drawn while it
ran, as ADR 0055 decided; the encoder folds a frame that changed
nothing into the one before and splits a long hold, so the file can
hold fewer or more (20 drawn, 3 in the APNG, seen 2026-10-24). kui's
Rust docs on `Captured` and `record_to` say "what the file holds"
against the ADR: kui F189, filed. The message says kui's number.

## Not done

- A status-line block while a screencast runs: its word in the corner
  is the sign today.
- A Lua door (`kawoosh.screenshot`): `kawoosh.run("screenshot …")`
  reaches it, without the answer.
- A region or a pane alone: kui captures the window's frame whole.
