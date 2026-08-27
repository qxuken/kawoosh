//! CPU text engine: cosmic-text shaping + swash rasterization composited into
//! an RGBA framebuffer the shell uploads as one streaming texture per frame.
//!
//! Deliberately thin (docs/design/mvp.md, Decision 1): everything here is a
//! pure function of text + metrics, and nothing knows SDL exists.

use cosmic_text::{
    Attrs, Buffer as ShapeBuffer, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent,
    Wrap,
};

const EMBEDDED_FONT: &[u8] =
    include_bytes!("../../assets/fonts/IosevkaNavcon/IosevkaNavcon-Regular.ttf");

/// An RGBA8 framebuffer view the engine composites into.
pub struct Frame<'a> {
    pub data: &'a mut [u8],
    pub width: usize,
    pub height: usize,
}

pub struct TextEngine {
    font_system: FontSystem,
    cache: SwashCache,
    /// One reused single-line shaping buffer; width unbounded, wrap off.
    line: ShapeBuffer,
    family: Option<String>,
    pub font_size: f32,
    pub line_height: f32,
}

impl TextEngine {
    pub fn new(font_size: f32) -> Self {
        let mut font_system = FontSystem::new();
        font_system.db_mut().load_font_data(EMBEDDED_FONT.to_vec());

        // The embedded face, not whatever Iosevka variant the system has.
        let family = font_system
            .db()
            .faces()
            .filter_map(|face| face.families.first().map(|(name, _)| name.clone()))
            .find(|name| name.contains("Navcon"));

        let line_height = (font_size * 1.4).ceil();
        let mut line = ShapeBuffer::new(&mut font_system, Metrics::new(font_size, line_height));
        line.set_size(None, None);
        line.set_wrap(Wrap::None);

        Self {
            font_system,
            cache: SwashCache::new(),
            line,
            family,
            font_size,
            line_height,
        }
    }

    /// Composite one line of text with its top-left corner at `(x, y)`.
    pub fn draw_line(&mut self, frame: &mut Frame, text: &str, x: i32, y: i32, color: [u8; 3]) {
        let family = match &self.family {
            Some(name) => Family::Name(name),
            None => Family::Monospace,
        };
        let attrs = Attrs::new().family(family);

        self.line.set_text(text, &attrs, Shaping::Advanced, None);
        self.line.shape_until_scroll(&mut self.font_system, false);

        for run in self.line.layout_runs() {
            for glyph in run.glyphs {
                let physical = glyph.physical((x as f32, y as f32 + run.line_y), 1.0);

                let Some(image) = self
                    .cache
                    .get_image(&mut self.font_system, physical.cache_key)
                else {
                    continue;
                };

                let gx = physical.x + image.placement.left;
                let gy = physical.y - image.placement.top;

                blit(frame, &image.content, &image.data, image.placement.width,
                    image.placement.height, gx, gy, color);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn blit(
    frame: &mut Frame,
    content: &SwashContent,
    data: &[u8],
    width: u32,
    height: u32,
    x0: i32,
    y0: i32,
    color: [u8; 3],
) {
    let (width, height) = (width as i32, height as i32);

    for row in 0..height {
        let y = y0 + row;
        if y < 0 || y >= frame.height as i32 {
            continue;
        }

        for col in 0..width {
            let x = x0 + col;
            if x < 0 || x >= frame.width as i32 {
                continue;
            }

            let src = (row * width + col) as usize;
            let (rgb, alpha) = match content {
                SwashContent::Mask => (color, data[src]),
                SwashContent::Color => {
                    let px = &data[src * 4..src * 4 + 4];
                    ([px[0], px[1], px[2]], px[3])
                }
                // Only produced when subpixel rendering is requested; treat the
                // green channel as coverage if it ever appears.
                SwashContent::SubpixelMask => (color, data[src * 4 + 1]),
            };

            if alpha == 0 {
                continue;
            }

            let dst = (y as usize * frame.width + x as usize) * 4;
            let pixel = &mut frame.data[dst..dst + 4];
            let a = alpha as u32;
            for channel in 0..3 {
                let blended = (rgb[channel] as u32 * a + pixel[channel] as u32 * (255 - a)) / 255;
                pixel[channel] = blended as u8;
            }
            pixel[3] = 0xFF;
        }
    }
}
