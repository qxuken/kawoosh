//! The SDL3 shell (docs/design/mvp.md, milestones 2-5).
//!
//! Event-driven: the loop blocks on `wait_event`, drains whatever queued, and
//! redraws once if anything marked the frame dirty. Each dirty frame builds
//! the element tree ([`App::frame`]), solves it with `kawoosh_ui::layout`,
//! and executes the command list into a CPU framebuffer uploaded as one
//! streaming texture. Pty readers run on io threads and wake the loop with a
//! custom SDL event; bytes flow through one channel into the terminal
//! models. The painter is deliberately the thinnest replaceable layer.

use std::io::Read;
use std::os::unix::net::UnixStream;

use anyhow::{Context as _, Result};
use crossbeam_channel::{Receiver, Sender};
use kawoosh::remote;
use kawoosh_core::BufferId;
use kawoosh::app::{App, BG, DIM, EDITOR_VIEW, FG, TERMINAL_VIEW, View};
use kawoosh::editor::Mode;
use kawoosh::keys::{self, Key, KeyPress, Mods};
use kawoosh::paint::Frame;
use kawoosh::text::TextEngine;
use kawoosh_term::{TermSize, Terminal};
use kawoosh_ui::{Command, Rect};
use sdl3::event::{Event, EventSender, WindowEvent};
use sdl3::keyboard::{Keycode, Mod};
use sdl3::pixels::PixelFormat;

const FONT_SIZE_PT: f32 = 14.0;
const PAD_X: f32 = 8.0;
const SEL_BG: [u8; 4] = [0x2E, 0x43, 0x6E, 0xFF];

const WELCOME: &[u8] = b"kawoosh\n\nOpen a file:  kawoosh <path>\nTerminal:     ctrl-t   (ctrl-y: scrollback to buffer)\nCycle views:  ctrl-o   close view: ctrl-w   save: ctrl-s\nQuit:         ctrl-q\n\nInside a kawoosh terminal, $EDITOR opens files here (kawoosh edit --wait).\n";

/// Wake-up marker pushed by pty reader threads; payload rides the channel.
struct PtyWake;

fn spawn_pty_reader(
    id: usize,
    mut reader: Box<dyn Read + Send>,
    tx: Sender<(usize, Vec<u8>)>,
    wake: EventSender,
) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => {
                    let _ = tx.send((id, Vec::new()));
                    let _ = wake.push_custom_event(PtyWake);
                    break;
                }
                Ok(n) => {
                    let _ = tx.send((id, buf[..n].to_vec()));
                    let _ = wake.push_custom_event(PtyWake);
                }
            }
        }
    });
}

/// Prefix width of `text` up to byte offset `off`, clamped to a boundary.
fn prefix_x(engine: &mut TextEngine, text: &str, off: usize) -> f32 {
    let off = off.min(text.len());
    let prefix = text.get(..off).unwrap_or(text);
    engine.measure_text(prefix).0
}

/// Draw the active editor view into its solved rect.
fn draw_editor(app: &App, engine: &mut TextEngine, frame: &mut Frame, rect: Rect, pad_x: f32) {
    let Some(ed) = app.active_editor() else {
        return;
    };
    let Some(buf) = app.core.buffer(ed.buffer) else {
        return;
    };

    frame.set_clip(Some(rect));

    let rows = (rect.h / engine.line_height).ceil() as usize;
    let cell_w = engine.measure_text("M").0;

    let mut scratch = Vec::new();
    for row in 0..rows {
        let line = ed.scroll + row;
        let Some(range) = buf.line_range(line) else {
            break;
        };
        scratch.clear();
        buf.read_into(range.clone(), &mut scratch);

        let text = String::from_utf8_lossy(&scratch).into_owned();
        let x0 = rect.x + pad_x;
        let y = rect.y + row as f32 * engine.line_height;
        let base = [FG[0], FG[1], FG[2]];

        // Selection backgrounds under the text.
        for sel in &ed.selections {
            let sr = sel.range();
            if sel.is_caret() || sr.end <= range.start || sr.start > range.end {
                continue;
            }
            let s = sr.start.max(range.start) - range.start;
            let e = sr.end.min(range.end) - range.start;
            let sx = x0 + prefix_x(engine, &text, s);
            let mut ex = x0 + prefix_x(engine, &text, e);
            if sr.end > range.end {
                // The selection swallows this line's newline.
                ex += cell_w * 0.5;
            }
            frame.fill_rect(
                Rect::new(sx, y, (ex - sx).max(1.0), engine.line_height),
                SEL_BG,
            );
        }

        // Text, split at core's chunk boundaries so each span draws in its
        // composed style — this is where the ts system's runs become pixels.
        if range.is_empty() {
            // Nothing to draw on an empty line.
        } else {
            let mut cx = x0;
            let mut drawn_to = 0usize;
            for chunk in app.core.chunks(ed.buffer, range.clone()) {
                let s = chunk.range.start - range.start;
                let e = chunk.range.end - range.start;
                let Some(segment) = text.get(s..e) else {
                    break;
                };
                drawn_to = e;
                if segment.is_empty() {
                    continue;
                }
                let color = chunk
                    .style
                    .fg
                    .map(|rgba| {
                        let v = rgba.0;
                        [(v >> 16) as u8, (v >> 8) as u8, v as u8]
                    })
                    .unwrap_or(base);
                engine.draw_line(frame, segment, cx as i32, y as i32, color);
                cx += engine.measure_text(segment).0;
            }
            // Fallback for anything the chunk walk could not slice.
            if drawn_to < text.len()
                && let Some(rest) = text.get(drawn_to..)
            {
                engine.draw_line(frame, rest, cx as i32, y as i32, base);
            }
        }

        // Carets on this line (line end included).
        for sel in &ed.selections {
            let head = sel.head.min(buf.len());
            if head < range.start || head > range.end || buf.line_of_offset(head) != line {
                continue;
            }
            let col = head - range.start;
            let cx = x0 + prefix_x(engine, &text, col);

            if ed.mode == Mode::Insert {
                frame.fill_rect(Rect::new(cx, y, 2.0, engine.line_height), FG);
            } else {
                let ch = text
                    .get(col..)
                    .and_then(|rest| rest.chars().next())
                    .filter(|c| !c.is_control());
                let w = match ch {
                    Some(c) => engine.measure_text(&c.to_string()).0.max(2.0),
                    None => cell_w,
                };
                frame.fill_rect(Rect::new(cx, y, w, engine.line_height), FG);
                if let Some(c) = ch {
                    engine.draw_line(
                        frame,
                        &c.to_string(),
                        cx as i32,
                        y as i32,
                        [BG[0], BG[1], BG[2]],
                    );
                }
            }
        }
    }

    frame.set_clip(None);
}

/// Draw the active terminal view: resize the model to the rect's grid, then
/// paint row runs grouped by style.
fn draw_terminal(app: &mut App, engine: &mut TextEngine, frame: &mut Frame, rect: Rect, pad_x: f32) {
    let cell_w = engine.measure_text("M").0;
    let line_h = engine.line_height;
    let grid_rows = ((rect.h / line_h).floor() as u16).max(1);
    let grid_cols = (((rect.w - pad_x * 2.0) / cell_w).floor() as u16).max(1);

    let Some(view) = app.active_terminal_mut() else {
        return;
    };
    view.terminal.resize(TermSize { rows: grid_rows, cols: grid_cols });

    // Materialize the grid; alacritty iterates cells in order.
    let mut cells: Vec<(usize, usize, kawoosh_term::CellView)> = Vec::new();
    view.terminal.for_each_cell(|row, col, cell| cells.push((row, col, cell)));
    let cursor = view.terminal.cursor();

    frame.set_clip(Some(rect));
    let x0 = rect.x + pad_x;

    // Backgrounds first, then glyph runs grouped by row + color.
    for &(row, col, ref cell) in &cells {
        if let Some(bg) = cell.bg {
            frame.fill_rect(
                Rect::new(x0 + col as f32 * cell_w, rect.y + row as f32 * line_h, cell_w, line_h),
                bg,
            );
        }
    }

    let mut run = String::new();
    let mut run_start: Option<(usize, usize, [u8; 4])> = None;
    let mut flush = |engine: &mut TextEngine, frame: &mut Frame, run: &mut String,
                     start: &mut Option<(usize, usize, [u8; 4])>| {
        if let Some((row, col, fg)) = start.take() {
            if !run.trim().is_empty() {
                engine.draw_line(
                    frame,
                    run,
                    (x0 + col as f32 * cell_w) as i32,
                    (rect.y + row as f32 * line_h) as i32,
                    [fg[0], fg[1], fg[2]],
                );
            }
            run.clear();
        }
    };

    let mut expected: Option<(usize, usize, [u8; 4])> = None;
    for &(row, col, ref cell) in &cells {
        let continues = matches!(
            (&run_start, expected),
            (Some((srow, _, sfg)), Some((erow, ecol, efg)))
                if *srow == erow && row == erow && col == ecol && *sfg == efg && cell.fg == efg
        );
        if !continues {
            flush(engine, frame, &mut run, &mut run_start);
            run_start = Some((row, col, cell.fg));
        }
        run.push(cell.c);
        expected = Some((row, col + 1, cell.fg));
    }
    flush(engine, frame, &mut run, &mut run_start);

    // Block cursor.
    if let Some((crow, ccol)) = cursor {
        let cx = x0 + ccol as f32 * cell_w;
        let cy = rect.y + crow as f32 * line_h;
        frame.fill_rect(Rect::new(cx, cy, cell_w, line_h), DIM);
        if let Some((_, _, cell)) = cells
            .iter()
            .find(|(r, c, _)| *r == crow && *c == ccol)
        {
            let s = cell.c.to_string();
            engine.draw_line(frame, &s, cx as i32, cy as i32, [BG[0], BG[1], BG[2]]);
        }
    }

    frame.set_clip(None);
}

/// Map an SDL key event to the binding-layer key (Decision 4b hybrid).
fn map_key(keycode: Option<Keycode>, scancode: u16, keymod: Mod) -> Option<KeyPress> {
    let mods = Mods {
        ctrl: keymod.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD),
        shift: keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD),
        alt: keymod.intersects(Mod::LALTMOD | Mod::RALTMOD),
    };

    let key = match keycode {
        Some(Keycode::Escape) => Key::Esc,
        Some(Keycode::Return) => Key::Enter,
        Some(Keycode::Backspace) => Key::Backspace,
        Some(Keycode::Tab) => Key::Tab,
        Some(Keycode::Up) => Key::Up,
        Some(Keycode::Down) => Key::Down,
        Some(Keycode::Left) => Key::Left,
        Some(Keycode::Right) => Key::Right,
        Some(Keycode::PageUp) => Key::PageUp,
        Some(Keycode::PageDown) => Key::PageDown,
        Some(Keycode::Home) => Key::Home,
        Some(Keycode::End) => Key::End,
        Some(Keycode::Delete) => Key::Delete,
        other => {
            let layout = other
                .map(|k| k.to_ll())
                .and_then(|raw| u32::try_from(raw.0).ok())
                .and_then(char::from_u32);
            Key::Char(keys::resolve_char(layout, scancode)?)
        }
    };

    Some(KeyPress { key, mods })
}

fn write_ppm(path: &str, rgba: &[u8], width: u32, height: u32) -> Result<()> {
    let mut out = format!("P6\n{width} {height}\n255\n").into_bytes();
    out.extend(rgba.chunks_exact(4).flat_map(|px| [px[0], px[1], px[2]]));
    std::fs::write(path, out).with_context(|| format!("writing frame dump {path}"))
}

fn open_terminal(
    app: &mut App,
    tx: &Sender<(usize, Vec<u8>)>,
    wake: &sdl3::EventSubsystem,
    socket: &str,
) -> Result<()> {
    // The $EDITOR handoff (Decision 3b): tools inside this pty open files
    // as editor views in this instance instead of nesting an editor.
    let editor = remote::editor_value();
    let envs = [
        ("KAWOOSH_SOCKET", socket.to_string()),
        ("EDITOR", editor.clone()),
        ("VISUAL", editor),
    ];
    let (terminal, reader) = Terminal::spawn(None, TermSize { rows: 24, cols: 80 }, &envs)?;
    let id = app.open_terminal(terminal);
    spawn_pty_reader(id, reader, tx.clone(), wake.event_sender());
    Ok(())
}

/// Adopt open requests from the command socket as editor views.
fn drain_opens(
    app: &mut App,
    rx: &Receiver<remote::OpenRequest>,
    waiters: &mut Vec<(BufferId, UnixStream)>,
) -> bool {
    let mut dirty = false;
    while let Ok(mut req) = rx.try_recv() {
        let bytes = std::fs::read(&req.path).unwrap_or_default();
        let title = req
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| req.path.to_string_lossy().into_owned());
        let buffer = app.open_editor(title, &bytes, Some(req.path.clone()));
        remote::ack(&mut req.reply);
        if req.wait {
            waiters.push((buffer, req.reply));
        }
        dirty = true;
    }
    dirty
}

fn drain_pty(app: &mut App, rx: &Receiver<(usize, Vec<u8>)>) -> bool {
    let mut dirty = false;
    while let Ok((id, bytes)) = rx.try_recv() {
        let active_terminal = matches!(app.active_view(), View::Terminal(t) if t.id == id);
        if let Some(view) = app.terminal_by_id_mut(id) {
            if bytes.is_empty() {
                if !view.title.ends_with("[exited]") {
                    view.title = format!("{} [exited]", view.title);
                }
            } else {
                view.terminal.feed(&bytes);
            }
            dirty |= active_terminal;
        }
    }
    dirty
}

fn main() -> Result<()> {
    env_logger::init();

    // Client mode: `kawoosh edit [--wait] <path>` / `kawoosh open <path>`
    // talks to the running instance named by $KAWOOSH_SOCKET.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(args.first().map(String::as_str), Some("edit" | "open")) {
        let wait = args.iter().any(|a| a == "--wait");
        let path = args
            .iter()
            .skip(1)
            .find(|a| !a.starts_with("--"))
            .context("usage: kawoosh edit [--wait] <path>")?;
        return remote::client_open(std::path::Path::new(path), wait);
    }

    let path = args.first().cloned();
    let bytes = match &path {
        Some(path) => std::fs::read(path).with_context(|| format!("reading {path}"))?,
        None => WELCOME.to_vec(),
    };
    let title = path.as_deref().unwrap_or("welcome");
    let mut app = App::open(title, &bytes);
    if let (Some(path), Some(ed)) = (&path, app.active_editor_mut()) {
        ed.path = Some(std::path::PathBuf::from(path));
        ed.language = kawoosh_systems::ts::Language::detect(std::path::Path::new(path));
    }

    let sdl = sdl3::init().context("SDL_Init")?;
    let video = sdl.video().context("SDL video subsystem")?;
    let events = sdl.event().context("SDL event subsystem")?;
    events
        .register_custom_event::<PtyWake>()
        .ok()
        .context("registering pty wake event")?;

    let (pty_tx, pty_rx) = crossbeam_channel::unbounded::<(usize, Vec<u8>)>();

    // The command socket (Decision 3b): $EDITOR handoff and `kawoosh open`.
    let socket = remote::socket_path();
    let socket_str = socket.to_string_lossy().into_owned();
    let (open_tx, open_rx) = crossbeam_channel::unbounded::<remote::OpenRequest>();
    {
        let listener = remote::bind()?;
        let wake = events.event_sender();
        remote::listen(listener, move |req| {
            let _ = open_tx.send(req);
            let _ = wake.push_custom_event(PtyWake);
        });
    }
    let mut waiters: Vec<(BufferId, UnixStream)> = Vec::new();

    // The tree-sitter system (milestone 6): jobs out, updates in.
    let (ts_tx, ts_rx) = kawoosh_systems::ts::spawn({
        let wake = events.event_sender();
        move || {
            let _ = wake.push_custom_event(PtyWake);
        }
    });

    let window = video
        .window("kawoosh", 1200, 800)
        .position_centered()
        .resizable()
        .high_pixel_density()
        .build()
        .context("creating window")?;

    let mut canvas = window.into_canvas();
    let texture_creator = canvas.texture_creator();

    let (mut pw, mut ph) = canvas.output_size().context("canvas output size")?;
    let scale = pw as f32 / canvas.window().size().0 as f32;

    let mut engine = TextEngine::new(FONT_SIZE_PT * scale);
    let pad_x = PAD_X * scale;

    let pixel_format = PixelFormat::ABGR8888;
    let mut framebuffer = vec![0u8; pw as usize * ph as usize * 4];
    let mut texture = texture_creator
        .create_texture_streaming(pixel_format, pw, ph)
        .context("creating streaming texture")?;

    // Headless verification: feed a plain-char key script before first frame.
    if let Ok(script) = std::env::var("KAWOOSH_KEYS") {
        for c in script.chars() {
            app.handle_key(KeyPress::plain(c));
        }
        let rows = (ph as f32 / engine.line_height) as usize;
        app.ensure_visible(rows.saturating_sub(2).max(1));
    }

    // Headless verification: open a shell terminal and let it settle so the
    // first-frame dump shows real pty output.
    if std::env::var("KAWOOSH_TERM").is_ok() {
        open_terminal(&mut app, &pty_tx, &events, &socket_str)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(900);
        while std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
            drain_pty(&mut app, &pty_rx);
        }
    }

    // Bindings come from key events; insert-mode/terminal content from text
    // input, which runs only when a view wants it, so the keypress that
    // enters insert mode cannot also arrive as text.
    let text_input = video.text_input();
    let mut text_input_active = false;

    // Headless verification: let the first highlight land before the dump.
    if std::env::var("KAWOOSH_DUMP_FRAME").is_ok() {
        for job in app.syntax_jobs() {
            let _ = ts_tx.send(job);
        }
        while let Ok(result) = ts_rx.recv_timeout(std::time::Duration::from_secs(3)) {
            app.apply_syntax(result);
            if app.syntax_jobs().is_empty() {
                break;
            }
        }
    }

    let mut event_pump = sdl.event_pump().context("event pump")?;
    let mut dirty = true;
    // Trackpads report fractional wheel deltas; accumulate until a whole line.
    let mut wheel: f32 = 0.0;

    'run: loop {
        dirty |= drain_pty(&mut app, &pty_rx);
        dirty |= drain_opens(&mut app, &open_rx, &mut waiters);
        while let Ok(result) = ts_rx.try_recv() {
            dirty |= app.apply_syntax(result);
        }
        for job in app.syntax_jobs() {
            let _ = ts_tx.send(job);
        }

        if dirty {
            let root = app.frame(scale, engine.line_height);
            let viewport = Rect::new(0.0, 0.0, pw as f32, ph as f32);
            let commands = kawoosh_ui::layout(&root, viewport, &mut engine);

            let mut frame = Frame::new(&mut framebuffer, pw as usize, ph as usize);
            frame.fill(BG);

            let mut clips: Vec<Option<Rect>> = Vec::new();
            for command in &commands {
                match command {
                    Command::Rect { rect, color } => frame.fill_rect(*rect, *color),
                    Command::Text { x, y, text, color } => {
                        engine.draw_line(
                            &mut frame,
                            text,
                            *x as i32,
                            *y as i32,
                            [color[0], color[1], color[2]],
                        );
                    }
                    Command::Custom { id: EDITOR_VIEW, rect } => {
                        draw_editor(&app, &mut engine, &mut frame, *rect, pad_x);
                    }
                    Command::Custom { id: TERMINAL_VIEW, rect } => {
                        draw_terminal(&mut app, &mut engine, &mut frame, *rect, pad_x);
                    }
                    Command::Custom { .. } => {}
                    Command::PushClip(rect) => {
                        clips.push(Some(*rect));
                        frame.set_clip(Some(*rect));
                    }
                    Command::PopClip => {
                        clips.pop();
                        frame.set_clip(clips.last().copied().flatten());
                    }
                }
            }

            texture
                .update(None, &framebuffer, pw as usize * 4)
                .context("uploading frame")?;
            canvas.copy(&texture, None, None).ok();
            canvas.present();
            dirty = false;

            // Headless verification hook: dump the first frame and exit.
            if let Ok(dump) = std::env::var("KAWOOSH_DUMP_FRAME") {
                write_ppm(&dump, &framebuffer, pw, ph)?;
                return Ok(());
            }
        }

        let mut pending = Some(event_pump.wait_event());
        while let Some(event) = pending {
            let page = (ph as f32 / engine.line_height) as isize - 1;

            match event {
                Event::Quit { .. } => break 'run,

                Event::KeyDown {
                    keycode,
                    scancode,
                    keymod,
                    ..
                } => {
                    let scancode = scancode.map(|s| s as u16).unwrap_or(0);
                    if let Some(kp) = map_key(keycode, scancode, keymod) {
                        if kp.mods.ctrl {
                            // Global chords, valid in every view.
                            match kp.key {
                                Key::Char('q') => break 'run,
                                Key::Char('t') => {
                                    open_terminal(&mut app, &pty_tx, &events, &socket_str)?;
                                    dirty = true;
                                    pending = event_pump.poll_event();
                                    continue;
                                }
                                Key::Char('s') => {
                                    app.save_active();
                                    pending = event_pump.poll_event();
                                    continue;
                                }
                                Key::Char('w') => {
                                    if let Some(view) = app.close_active_view() {
                                        if let View::Editor(ed) = view {
                                            waiters.retain_mut(|(buffer, stream)| {
                                                if *buffer == ed.buffer {
                                                    remote::finish(stream);
                                                    false
                                                } else {
                                                    true
                                                }
                                            });
                                        }
                                        dirty = true;
                                    }
                                    pending = event_pump.poll_event();
                                    continue;
                                }
                                Key::Char('o') => {
                                    dirty |= app.cycle_view(1);
                                    pending = event_pump.poll_event();
                                    continue;
                                }
                                Key::Char('y') => {
                                    dirty |= app.scrollback_to_buffer();
                                    pending = event_pump.poll_event();
                                    continue;
                                }
                                _ => {}
                            }
                        }
                        let editor_active = app.active_editor().is_some();
                        match kp.key {
                            Key::PageUp if editor_active => {
                                dirty |= app.scroll_by(-page.max(1));
                            }
                            Key::PageDown if editor_active => {
                                dirty |= app.scroll_by(page.max(1));
                            }
                            _ => {
                                dirty |= app.handle_key(kp);
                                let rows = (ph as f32 / engine.line_height) as usize;
                                dirty |= app.ensure_visible(rows.saturating_sub(2).max(1));
                            }
                        }
                    }
                }

                Event::TextInput { ref text, .. } => {
                    dirty |= app.handle_text(text);
                    let rows = (ph as f32 / engine.line_height) as usize;
                    dirty |= app.ensure_visible(rows.saturating_sub(2).max(1));
                }

                Event::MouseWheel { y, .. } => {
                    if app.active_editor().is_some() {
                        wheel += -y * 3.0;
                        let lines = wheel.trunc() as isize;
                        if lines != 0 {
                            wheel -= lines as f32;
                            dirty |= app.scroll_by(lines);
                        }
                    }
                }

                Event::Window {
                    win_event: WindowEvent::PixelSizeChanged(..) | WindowEvent::Resized(..),
                    ..
                } => {
                    let (new_pw, new_ph) = canvas.output_size().context("canvas output size")?;
                    if (new_pw, new_ph) != (pw, ph) {
                        (pw, ph) = (new_pw, new_ph);
                        framebuffer = vec![0u8; pw as usize * ph as usize * 4];
                        texture = texture_creator
                            .create_texture_streaming(pixel_format, pw, ph)
                            .context("recreating streaming texture")?;
                    }
                    dirty = true;
                }

                _ => {}
            }

            pending = event_pump.poll_event();
        }

        dirty |= drain_pty(&mut app, &pty_rx);

        let want_text_input = app.wants_text_input();
        if want_text_input != text_input_active {
            if want_text_input {
                text_input.start(canvas.window());
            } else {
                text_input.stop(canvas.window());
            }
            text_input_active = want_text_input;
        }
    }

    // Unblock any --wait clients and remove the socket.
    for (_, mut stream) in waiters {
        remote::finish(&mut stream);
    }
    let _ = std::fs::remove_file(&socket);

    Ok(())
}
