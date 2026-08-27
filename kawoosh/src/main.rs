//! The SDL3 shell (docs/design/mvp.md, milestones 2-3).
//!
//! Event-driven: the loop blocks on `wait_event`, drains whatever queued, and
//! redraws once if anything marked the frame dirty. Each dirty frame builds
//! the element tree ([`App::frame`]), solves it with `kawoosh_ui::layout`,
//! and executes the command list into a CPU framebuffer uploaded as one
//! streaming texture. The painter is deliberately the thinnest replaceable
//! layer.

use anyhow::{Context as _, Result};
use kawoosh::app::{App, BG, EDITOR_VIEW, FG};
use kawoosh::editor::Mode;
use kawoosh::keys::{self, Key, KeyPress, Mods};
use kawoosh::paint::Frame;
use kawoosh::text::TextEngine;
use kawoosh_ui::{Command, Rect};
use sdl3::event::{Event, WindowEvent};
use sdl3::keyboard::{Keycode, Mod};
use sdl3::pixels::PixelFormat;

const FONT_SIZE_PT: f32 = 14.0;
const PAD_X: f32 = 8.0;
const SEL_BG: [u8; 4] = [0x2E, 0x43, 0x6E, 0xFF];

const WELCOME: &[u8] = b"kawoosh\n\nOpen a file: kawoosh <path>\n";

/// Prefix width of `text` up to byte offset `off`, clamped to a boundary.
fn prefix_x(engine: &mut TextEngine, text: &str, off: usize) -> f32 {
    let off = off.min(text.len());
    let prefix = text.get(..off).unwrap_or(text);
    engine.measure_text(prefix).0
}

/// Draw the editor view into its solved rect.
fn draw_editor(app: &App, engine: &mut TextEngine, frame: &mut Frame, rect: Rect, pad_x: f32) {
    frame.set_clip(Some(rect));

    let rows = (rect.h / engine.line_height).ceil() as usize;
    let buf = app.core.buffer(app.buffer).expect("app buffer exists");
    let cell_w = engine.measure_text("M").0;

    let mut scratch = Vec::new();
    for row in 0..rows {
        let line = app.scroll + row;
        let Some(range) = buf.line_range(line) else {
            break;
        };
        scratch.clear();
        buf.read_into(range.clone(), &mut scratch);

        let text = String::from_utf8_lossy(&scratch).into_owned();
        let x0 = rect.x + pad_x;
        let y = rect.y + row as f32 * engine.line_height;

        // Selection backgrounds under the text.
        for sel in &app.selections {
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

        engine.draw_line(frame, &text, x0 as i32, y as i32, [FG[0], FG[1], FG[2]]);

        // Carets on this line (line end included).
        for sel in &app.selections {
            let head = sel.head.min(buf.len());
            if head < range.start || head > range.end || buf.line_of_offset(head) != line {
                continue;
            }
            let col = head - range.start;
            let cx = x0 + prefix_x(engine, &text, col);

            if app.mode == Mode::Insert {
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

fn main() -> Result<()> {
    env_logger::init();

    let path = std::env::args().nth(1);
    let bytes = match &path {
        Some(path) => std::fs::read(path).with_context(|| format!("reading {path}"))?,
        None => WELCOME.to_vec(),
    };
    let title = path.as_deref().unwrap_or("welcome");
    let mut app = App::open(title, &bytes);

    let sdl = sdl3::init().context("SDL_Init")?;
    let video = sdl.video().context("SDL video subsystem")?;

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

    // Bindings come from key events; insert-mode content from text input.
    // Text input runs only while in insert mode, so the keypress that enters
    // the mode cannot also arrive as text.
    let text_input = video.text_input();
    let mut text_input_active = false;

    let mut event_pump = sdl.event_pump().context("event pump")?;
    let mut dirty = true;
    // Trackpads report fractional wheel deltas; accumulate until a whole line.
    let mut wheel: f32 = 0.0;

    'run: loop {
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
                        if kp.key == Key::Char('q') && kp.mods.ctrl {
                            break 'run;
                        }
                        match kp.key {
                            Key::PageUp => dirty |= app.scroll_by(-page.max(1)),
                            Key::PageDown => dirty |= app.scroll_by(page.max(1)),
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
                    wheel += -y * 3.0;
                    let lines = wheel.trunc() as isize;
                    if lines != 0 {
                        wheel -= lines as f32;
                        dirty |= app.scroll_by(lines);
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

        let want_text_input = app.mode == Mode::Insert;
        if want_text_input != text_input_active {
            if want_text_input {
                text_input.start(canvas.window());
            } else {
                text_input.stop(canvas.window());
            }
            text_input_active = want_text_input;
        }
    }

    Ok(())
}
