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
use kawoosh::paint::Frame;
use kawoosh::text::TextEngine;
use kawoosh_ui::{Command, Rect};
use sdl3::event::{Event, WindowEvent};
use sdl3::keyboard::Keycode;
use sdl3::pixels::PixelFormat;

const FONT_SIZE_PT: f32 = 14.0;
const PAD_X: f32 = 8.0;

const WELCOME: &[u8] = b"kawoosh\n\nOpen a file: kawoosh <path>\n";

/// Draw the editor view into its solved rect.
fn draw_editor(app: &App, engine: &mut TextEngine, frame: &mut Frame, rect: Rect, pad_x: f32) {
    frame.set_clip(Some(rect));

    let rows = (rect.h / engine.line_height).ceil() as usize;
    let buf = app.core.buffer(app.buffer).expect("app buffer exists");

    let mut scratch = Vec::new();
    for row in 0..rows {
        let Some(range) = buf.line_range(app.scroll + row) else {
            break;
        };
        scratch.clear();
        buf.read_into(range, &mut scratch);

        let text = String::from_utf8_lossy(&scratch);
        let x = (rect.x + pad_x) as i32;
        let y = (rect.y + row as f32 * engine.line_height) as i32;
        engine.draw_line(frame, &text, x, y, [FG[0], FG[1], FG[2]]);
    }

    frame.set_clip(None);
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
                    keycode: Some(key), ..
                } => match key {
                    Keycode::Escape | Keycode::Q => break 'run,
                    Keycode::Up | Keycode::K => dirty |= app.scroll_by(-1),
                    Keycode::Down | Keycode::J => dirty |= app.scroll_by(1),
                    Keycode::PageUp => dirty |= app.scroll_by(-page.max(1)),
                    Keycode::PageDown => dirty |= app.scroll_by(page.max(1)),
                    Keycode::Home => dirty |= app.scroll_to(0),
                    Keycode::End => dirty |= app.scroll_to(usize::MAX),
                    _ => {}
                },

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
    }

    Ok(())
}
