//! A picture file read into pixels: a still (PNG, JPEG, WebP, BMP), the
//! frames of one that moves (GIF, APNG, animated WebP) each with how
//! long it stays, or an SVG drawn at a width asked for — and drawn
//! again at another, from the tree kept beside its pixels, so a
//! drawing is as sharp as the size it is shown at.
//!
//! What a pane draws is RGBA8, not premultiplied, a frame the whole
//! canvas. Three caps keep a file from taking the machine: its bytes on
//! disk (the caller's), a side of [`MAX_SIDE`] pixels — a still past it
//! is scaled down, its own size still said — and [`MAX_FRAMES_BYTES`]
//! of frames, the ones past it left out and said so.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use image::AnimationDecoder;

/// The longest side kept, in pixels: a texture a GPU takes anywhere.
pub const MAX_SIDE: u32 = 8192;
/// The longest side a drawing is drawn at: 64 MB of pixels at most.
pub const MAX_VECTOR_SIDE: u32 = 4096;
/// The frames kept of a picture that moves, in bytes of pixels.
pub const MAX_FRAMES_BYTES: usize = 256 << 20;

/// One frame: the whole canvas, and how long it stays, in ms.
pub struct Frame {
    pub rgba: Vec<u8>,
    pub delay_ms: u32,
}

/// A drawing as parsed, to draw at any width ([`Vector::draw`]).
#[derive(Clone)]
pub struct Vector(Arc<resvg::usvg::Tree>);

/// A picture read.
pub struct Picture {
    /// The pixels' size: every frame's.
    pub width: u32,
    pub height: u32,
    /// The picture's own size — a still's before it was scaled down to
    /// [`MAX_SIDE`], a drawing's as its file says.
    pub natural: (u32, u32),
    /// One frame for a still; never none.
    pub frames: Vec<Frame>,
    /// Frames were left out past [`MAX_FRAMES_BYTES`].
    pub more: bool,
    /// `png`, `jpeg`, `gif`, `webp`, `bmp`, `svg`.
    pub format: &'static str,
    /// A drawing's tree; `None` for pixels.
    pub vector: Option<Vector>,
}

/// Its facts, never its pixels.
impl std::fmt::Debug for Picture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Picture({} {}x{}, {} frames)",
            self.format,
            self.width,
            self.height,
            self.frames.len()
        )
    }
}

impl Picture {
    fn still(img: image::DynamicImage, format: &'static str) -> Picture {
        let natural = (img.width(), img.height());
        let img = if natural.0.max(natural.1) > MAX_SIDE {
            img.thumbnail(MAX_SIDE, MAX_SIDE)
        } else {
            img
        };
        let rgba = img.to_rgba8();
        let (width, height) = rgba.dimensions();
        Picture {
            width,
            height,
            natural,
            frames: vec![Frame {
                rgba: rgba.into_raw(),
                delay_ms: 0,
            }],
            more: false,
            format,
            vector: None,
        }
    }
}

/// The picture at `path`, refused past `max` bytes on disk. A drawing
/// is drawn `width` pixels wide, or twice its own size without one.
pub fn decode(path: &Path, max: u64, width: Option<u32>) -> Result<Picture, String> {
    // A host's too, through the file system layer.
    let size = crate::fs::stat(path).map_err(|e| e.to_string())?.size;
    if size > max {
        return Err(format!("{} MB, past the cap", size >> 20));
    }
    let bytes = crate::fs::read_bytes(path).map_err(|e| e.to_string())?;
    decode_bytes(&bytes, path.parent(), width)
}

/// [`decode`] of bytes in hand — a `data:` URI's; `dir` is where a
/// drawing's own links start.
pub fn decode_bytes(
    bytes: &[u8],
    dir: Option<&Path>,
    width: Option<u32>,
) -> Result<Picture, String> {
    use image::ImageFormat as F;
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let cursor = || std::io::Cursor::new(bytes);
    let err = |e: image::ImageError| e.to_string();
    match reader.format() {
        Some(F::Gif) => {
            let d = image::codecs::gif::GifDecoder::new(cursor()).map_err(err)?;
            moving(d.into_frames(), "gif")
        }
        Some(F::Png) => {
            let d = image::codecs::png::PngDecoder::new(cursor()).map_err(err)?;
            if d.is_apng().map_err(err)? {
                moving(d.apng().map_err(err)?.into_frames(), "png")
            } else {
                Ok(Picture::still(reader.decode().map_err(err)?, "png"))
            }
        }
        Some(F::WebP) => {
            let d = image::codecs::webp::WebPDecoder::new(cursor()).map_err(err)?;
            if d.has_animation() {
                moving(d.into_frames(), "webp")
            } else {
                Ok(Picture::still(reader.decode().map_err(err)?, "webp"))
            }
        }
        Some(f) => {
            let name = match f {
                F::Jpeg => "jpeg",
                F::Bmp => "bmp",
                _ => "image",
            };
            Ok(Picture::still(reader.decode().map_err(err)?, name))
        }
        None if is_svg(bytes) => vector(bytes, dir, width),
        None => Err("not a picture kawoosh reads".into()),
    }
}

/// Markup with an `<svg` near its head, or gzip's magic (an `.svgz`).
fn is_svg(bytes: &[u8]) -> bool {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        return true;
    }
    let head = &bytes[..bytes.len().min(4096)];
    memchr::memmem::find(head, b"<svg").is_some()
}

/// The frames of a picture that moves, until they are [`MAX_FRAMES_BYTES`].
fn moving(frames: image::Frames<'_>, format: &'static str) -> Result<Picture, String> {
    let mut out: Vec<Frame> = Vec::new();
    let (mut size, mut bytes, mut more) = ((0, 0), 0usize, false);
    for f in frames {
        let f = match f {
            Ok(f) => f,
            // A file cut short plays as far as it goes.
            Err(_) if !out.is_empty() => break,
            Err(e) => return Err(e.to_string()),
        };
        let (n, d) = f.delay().numer_denom_ms();
        let ms = n.checked_div(d).unwrap_or(0);
        let buf = f.into_buffer();
        if out.is_empty() {
            size = buf.dimensions();
            if size.0.max(size.1) > MAX_SIDE {
                return Ok(Picture::still(image::DynamicImage::ImageRgba8(buf), format));
            }
        } else if buf.dimensions() != size {
            break;
        }
        bytes += buf.len();
        if bytes > MAX_FRAMES_BYTES && !out.is_empty() {
            more = true;
            break;
        }
        out.push(Frame {
            rgba: buf.into_raw(),
            // What a browser does with a delay too short to mean it.
            delay_ms: if ms <= 10 { 100 } else { ms },
        });
    }
    if out.is_empty() {
        return Err("no frames".into());
    }
    if out.len() == 1 {
        out[0].delay_ms = 0;
    }
    Ok(Picture {
        width: size.0,
        height: size.1,
        natural: size,
        frames: out,
        more,
        format,
        vector: None,
    })
}

/// The machine's fonts, read once, the first time a drawing has text.
fn fonts() -> Arc<resvg::usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = resvg::usvg::fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone()
}

fn vector(bytes: &[u8], dir: Option<&Path>, width: Option<u32>) -> Result<Picture, String> {
    let mut opt = resvg::usvg::Options {
        resources_dir: dir.map(Path::to_path_buf),
        ..Default::default()
    };
    // Gzipped, the text cannot be looked for: the fonts then too.
    if bytes.starts_with(&[0x1f, 0x8b]) || memchr::memmem::find(bytes, b"<text").is_some() {
        opt.fontdb = fonts();
    }
    let tree = resvg::usvg::Tree::from_data(bytes, &opt).map_err(|e| e.to_string())?;
    let v = Vector(Arc::new(tree));
    let natural = v.size();
    let (width, height, rgba) = v.draw(width.unwrap_or(natural.0.saturating_mul(2)))?;
    Ok(Picture {
        width,
        height,
        natural,
        frames: vec![Frame { rgba, delay_ms: 0 }],
        more: false,
        format: "svg",
        vector: Some(v),
    })
}

impl Vector {
    /// The drawing's own size, in whole pixels, at least one.
    pub fn size(&self) -> (u32, u32) {
        let s = self.0.size();
        (
            (s.width().round() as u32).max(1),
            (s.height().round() as u32).max(1),
        )
    }

    /// The width [`Vector::draw`] draws at when asked for `width`: no
    /// side past [`MAX_VECTOR_SIDE`], none under a pixel.
    pub fn fit(&self, width: u32) -> u32 {
        let s = self.0.size();
        let cap = if s.height() > s.width() {
            (MAX_VECTOR_SIDE as f32 * s.width() / s.height()).floor() as u32
        } else {
            MAX_VECTOR_SIDE
        };
        width.clamp(1, cap.max(1))
    }

    /// The drawing `width` pixels wide ([`Vector::fit`]), its height by
    /// its shape: the size, and the pixels.
    pub fn draw(&self, width: u32) -> Result<(u32, u32, Vec<u8>), String> {
        let s = self.0.size();
        let w = self.fit(width);
        let scale = w as f32 / s.width();
        let h = ((s.height() * scale).round() as u32).clamp(1, MAX_VECTOR_SIDE);
        let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h).ok_or("no room for its pixels")?;
        resvg::render(
            &self.0,
            resvg::tiny_skia::Transform::from_scale(scale, h as f32 / s.height()),
            &mut pixmap.as_mut(),
        );
        let mut rgba = Vec::with_capacity(pixmap.data().len());
        for p in pixmap.pixels() {
            let c = p.demultiply();
            rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
        }
        Ok((w, h, rgba))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, px: [u8; 4]) -> Vec<u8> {
        let mut out = Vec::new();
        image::RgbaImage::from_pixel(w, h, image::Rgba(px))
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    /// Three frames, red, green, blue, 50 ms, 200 ms and no delay said.
    pub(crate) fn gif() -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = image::codecs::gif::GifEncoder::new(&mut out);
            enc.set_repeat(image::codecs::gif::Repeat::Infinite)
                .unwrap();
            for (px, ms) in [
                ([255, 0, 0, 255], 50),
                ([0, 255, 0, 255], 200),
                ([0, 0, 255, 255], 0),
            ] {
                let img = image::RgbaImage::from_pixel(4, 2, image::Rgba(px));
                enc.encode_frame(image::Frame::from_parts(
                    img,
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(ms, 1),
                ))
                .unwrap();
            }
        }
        out
    }

    /// A still is one frame of its own size; a picture that moves is
    /// its frames, each the canvas, a delay too short to mean it a
    /// tenth of a second; what is no picture says so.
    #[test]
    fn stills_and_frames_are_read() {
        let p = decode_bytes(&png(3, 2, [1, 2, 3, 4]), None, None).unwrap();
        assert_eq!(
            (p.width, p.height, p.natural, p.format),
            (3, 2, (3, 2), "png")
        );
        assert_eq!(p.frames.len(), 1);
        assert_eq!(&p.frames[0].rgba[..4], &[1, 2, 3, 4]);
        assert!(p.vector.is_none());

        let p = decode_bytes(&gif(), None, None).unwrap();
        assert_eq!((p.width, p.height, p.format, p.more), (4, 2, "gif", false));
        let delays: Vec<u32> = p.frames.iter().map(|f| f.delay_ms).collect();
        assert_eq!(delays, [50, 200, 100]);
        assert_eq!(&p.frames[0].rgba[..4], &[255, 0, 0, 255]);
        assert_eq!(&p.frames[1].rgba[..4], &[0, 255, 0, 255]);
        assert_eq!(p.frames[2].rgba.len(), 4 * 2 * 4);

        assert!(decode_bytes(b"plain text, no picture", None, None).is_err());
        assert!(decode_bytes(b"", None, None).is_err());
    }

    /// A drawing is drawn twice its size unasked, at the width asked
    /// for otherwise, its own size said either way — and again from
    /// its tree, no side past the cap.
    #[test]
    fn a_drawing_is_drawn_at_the_width_asked_for() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="5">
            <rect width="10" height="5" fill="#ff0000"/></svg>"##;
        let p = decode_bytes(svg, None, None).unwrap();
        assert_eq!(
            (p.width, p.height, p.natural, p.format),
            (20, 10, (10, 5), "svg")
        );
        assert_eq!(&p.frames[0].rgba[..4], &[255, 0, 0, 255]);
        let p = decode_bytes(svg, None, Some(100)).unwrap();
        assert_eq!((p.width, p.height, p.natural), (100, 50, (10, 5)));
        let v = p.vector.unwrap();
        let (w, h, rgba) = v.draw(7).unwrap();
        assert_eq!((w, h, rgba.len()), (7, 4, 7 * 4 * 4));
        assert_eq!(v.fit(1 << 20), MAX_VECTOR_SIDE);
        assert_eq!(v.fit(0), 1);
        // A tall one's height is what is capped.
        let tall = br##"<svg xmlns="http://www.w3.org/2000/svg" width="5" height="10"/>"##;
        let v = decode_bytes(tall, None, None).unwrap().vector.unwrap();
        assert_eq!(v.fit(1 << 20), MAX_VECTOR_SIDE / 2);
        assert!(decode_bytes(b"<svg", None, None).is_err());
    }
}
