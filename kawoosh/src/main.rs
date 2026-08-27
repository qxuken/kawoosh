//! The SDL3 shell (docs/design/mvp.md, milestone 2).
//!
//! Event-driven: the loop blocks on `wait_event`, drains whatever queued, and
//! redraws once if anything marked the frame dirty. Rendering is a CPU
//! framebuffer uploaded as one streaming texture — the painter is deliberately
//! the thinnest replaceable layer.

mod text;

use anyhow::{Context as _, Result};
use kawoosh_core::{BufferId, Core};
use sdl3::event::{Event, WindowEvent};
use sdl3::keyboard::Keycode;
use sdl3::pixels::PixelFormat;

use crate::text::{Frame, TextEngine};

const BG: [u8; 3] = [0x21, 0x21, 0x21];
const FG: [u8; 3] = [0xE6, 0xE6, 0xE6];
const FONT_SIZE_PT: f32 = 14.0;
const PAD_X: i32 = 8;

const WELCOME: &[u8] = b"kawoosh\n\nOpen a file: kawoosh <path>\n";

struct View {
    core: Core,
    buffer: BufferId,
    /// First visible line.
    scroll: usize,
}

impl View {
    fn line_count(&self) -> usize {
        self.core.buffer(self.buffer).map_or(1, |b| b.line_count())
    }

    fn scroll_by(&mut self, delta: isize) -> bool {
        let max = self.line_count().saturating_sub(1);
        let target = self
            .scroll
            .saturating_add_signed(delta)
            .min(max);
        let moved = target != self.scroll;
        self.scroll = target;
        moved
    }

    fn scroll_to(&mut self, line: usize) -> bool {
        let target = line.min(self.line_count().saturating_sub(1));
        let moved = target != self.scroll;
        self.scroll = target;
        moved
    }
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

    let mut core = Core::default();
    let buffer = core.create_buffer();
    core.set_text(buffer, &bytes);
    let mut view = View {
        core,
        buffer,
        scroll: 0,
    };

    let sdl = sdl3::init().context("SDL_Init")?;
    let video = sdl.video().context("SDL video subsystem")?;

    let title = path.as_deref().unwrap_or("kawoosh");
    let window = video
        .window(&format!("kawoosh — {title}"), 1200, 800)
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
    let pad_x = (PAD_X as f32 * scale) as i32;

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
            let rows = (ph as f32 / engine.line_height).ceil() as usize;

            for pixel in framebuffer.chunks_exact_mut(4) {
                pixel.copy_from_slice(&[BG[0], BG[1], BG[2], 0xFF]);
            }

            let mut frame = Frame {
                data: &mut framebuffer,
                width: pw as usize,
                height: ph as usize,
            };

            let buf = view.core.buffer(view.buffer).expect("view buffer exists");
            let mut scratch = Vec::new();
            for row in 0..rows {
                let line = view.scroll + row;
                let Some(range) = buf.line_range(line) else {
                    break;
                };
                scratch.clear();
                buf.read_into(range, &mut scratch);

                let text = String::from_utf8_lossy(&scratch);
                let y = row as f32 * engine.line_height;
                engine.draw_line(&mut frame, &text, pad_x, y as i32, FG);
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
                    Keycode::Up | Keycode::K => dirty |= view.scroll_by(-1),
                    Keycode::Down | Keycode::J => dirty |= view.scroll_by(1),
                    Keycode::PageUp => dirty |= view.scroll_by(-page.max(1)),
                    Keycode::PageDown => dirty |= view.scroll_by(page.max(1)),
                    Keycode::Home => dirty |= view.scroll_to(0),
                    Keycode::End => dirty |= view.scroll_to(usize::MAX),
                    _ => {}
                },

                Event::MouseWheel { y, .. } => {
                    wheel += -y * 3.0;
                    let lines = wheel.trunc() as isize;
                    if lines != 0 {
                        wheel -= lines as f32;
                        dirty |= view.scroll_by(lines);
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
