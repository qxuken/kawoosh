//! Kitty's images in a terminal pane (docs/design/kitty-graphics.md,
//! roadmap step 56): what `term` says is on screen, uploaded to kui
//! once per image and source rect (a part of an image is a cropped
//! copy) and drawn as image nodes floated over the grid — under its
//! text for a negative `z`, over it otherwise.

use std::collections::HashMap;

use kawoosh_term::Placed;
use kui_native::{ImageId, Ui};

use crate::app::Kawoosh;
use crate::terminals::TermId;

/// An upload's key: the terminal, the image, the source rect.
type Key = (TermId, u32, (u32, u32, u32, u32));

/// An image as kui has it, by terminal, image and source rect.
struct Uploaded {
    id: ImageId,
    generation: u64,
    /// The frame it was last drawn in: one not drawn is freed.
    frame: u64,
}

#[derive(Default)]
pub struct TermImages {
    map: HashMap<Key, Uploaded>,
    frame: u64,
}

/// One placement to draw, in the grid's logical pixels from its
/// top-left.
pub(crate) struct Shown {
    pub id: ImageId,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Under the text (`z < 0`).
    pub under: bool,
}

impl Kawoosh {
    /// Each frame, before the panes: every terminal told its cell in
    /// the window's pixels, the decodes that finished taken in (and a
    /// frame asked for while any is out), and the uploads no pane drew
    /// last frame freed.
    pub(crate) fn sync_term_graphics(&mut self, ui: &mut Ui<'_>) {
        let (cw, ch) = self.cell_px(ui);
        for t in self.terms.map.values_mut() {
            t.set_cell_pixels(cw, ch);
            if t.poll_graphics() || t.graphics_busy() {
                crate::frames::request(ui, "terminal graphics");
            }
        }
        let images = &mut self.term_images;
        let last = images.frame;
        images.frame += 1;
        let stale: Vec<_> = images
            .map
            .iter()
            .filter(|(_, u)| u.frame < last)
            .map(|(k, _)| *k)
            .collect();
        for k in stale {
            if let Some(u) = images.map.remove(&k) {
                ui.core().remove_image(u.id);
            }
        }
    }

    /// A cell's size in the window's pixels.
    fn cell_px(&self, ui: &mut Ui<'_>) -> (u16, u16) {
        let scale = ui.core().scale();
        let (cw, ch) = self.cell;
        (
            (cw * scale).round().max(1.0) as u16,
            (ch * scale).round().max(1.0) as u16,
        )
    }

    /// The placements on terminal `id`'s screen, uploaded where they were
    /// not, as boxes in the grid's logical pixels.
    pub(crate) fn term_image_boxes(&mut self, ui: &mut Ui<'_>, id: TermId) -> Vec<Shown> {
        let Some(t) = self.terms.map.get(&id) else {
            return Vec::new();
        };
        let placed = t.images();
        if placed.is_empty() {
            return Vec::new();
        }
        let (pw, ph) = self.cell_px(ui);
        let (cw, ch) = self.cell;
        // A pixel as the program counted it, in logical ones: a cell is
        // a cell whatever the rounding.
        let (sx, sy) = (cw / pw as f32, ch / ph as f32);
        let frame = self.term_images.frame;
        let mut out = Vec::with_capacity(placed.len());
        for p in &placed {
            let Some(image) = self.upload(ui, id, p, frame) else {
                continue;
            };
            out.push(Shown {
                id: image,
                x: p.col as f32 * cw + p.offset.0 as f32 * sx,
                y: p.row as f32 * ch + p.offset.1 as f32 * sy,
                w: p.size.0 as f32 * sx,
                h: p.size.1 as f32 * sy,
                under: p.z < 0,
            });
        }
        out
    }

    /// Placement `p`'s pixels as a kui image: the one uploaded before
    /// while its generation holds, else uploaded now — cropped to its
    /// source rect when that is not the whole image.
    fn upload(&mut self, ui: &mut Ui<'_>, term: TermId, p: &Placed, frame: u64) -> Option<ImageId> {
        let key = (term, p.image, p.src);
        if let Some(u) = self.term_images.map.get_mut(&key)
            && u.generation == p.generation
        {
            u.frame = frame;
            return Some(u.id);
        }
        let (x, y, w, h) = p.src;
        if w == 0 || h == 0 {
            return None;
        }
        let rgba = if (x, y, w, h) == (0, 0, p.width, p.height) {
            p.rgba.as_ref().clone()
        } else {
            let row = p.width as usize * 4;
            let mut out = Vec::with_capacity(w as usize * h as usize * 4);
            for r in y..y + h {
                let from = r as usize * row + x as usize * 4;
                out.extend_from_slice(p.rgba.get(from..from + w as usize * 4)?);
            }
            out
        };
        // New pixels: the old texture goes, whatever its size was.
        if let Some(old) = self.term_images.map.remove(&key) {
            ui.core().remove_image(old.id);
        }
        let id = ui.core().resources.add_image(w, h, rgba);
        self.term_images.map.insert(
            key,
            Uploaded {
                id,
                generation: p.generation,
                frame,
            },
        );
        Some(id)
    }
}
