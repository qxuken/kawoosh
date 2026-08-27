//! CPU framebuffer the painter composites into.

use kawoosh_ui::Rect;

/// RGBA8 framebuffer view with an optional clip rectangle (pixel coords).
pub struct Frame<'a> {
    pub data: &'a mut [u8],
    pub width: usize,
    pub height: usize,
    clip: Option<Rect>,
}

impl<'a> Frame<'a> {
    pub fn new(data: &'a mut [u8], width: usize, height: usize) -> Self {
        Self { data, width, height, clip: None }
    }

    pub fn set_clip(&mut self, clip: Option<Rect>) {
        self.clip = clip;
    }

    /// Whether a pixel is inside the frame and the current clip.
    pub fn admits(&self, x: i32, y: i32) -> bool {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return false;
        }
        match &self.clip {
            Some(clip) => clip.contains(x as f32, y as f32),
            None => true,
        }
    }

    pub fn fill(&mut self, color: [u8; 4]) {
        for pixel in self.data.chunks_exact_mut(4) {
            pixel.copy_from_slice(&color);
        }
    }

    pub fn fill_rect(&mut self, rect: Rect, color: [u8; 4]) {
        let x0 = rect.x.max(0.0) as i32;
        let y0 = rect.y.max(0.0) as i32;
        let x1 = ((rect.x + rect.w) as i32).min(self.width as i32);
        let y1 = ((rect.y + rect.h) as i32).min(self.height as i32);

        for y in y0..y1 {
            for x in x0..x1 {
                if !self.admits(x, y) {
                    continue;
                }
                let dst = (y as usize * self.width + x as usize) * 4;
                self.data[dst..dst + 4].copy_from_slice(&color);
            }
        }
    }

    /// Blend a single pixel: `rgb` over the existing pixel at coverage `alpha`.
    pub fn blend(&mut self, x: i32, y: i32, rgb: [u8; 3], alpha: u8) {
        if alpha == 0 || !self.admits(x, y) {
            return;
        }
        let dst = (y as usize * self.width + x as usize) * 4;
        let pixel = &mut self.data[dst..dst + 4];
        let a = alpha as u32;
        for channel in 0..3 {
            let blended = (rgb[channel] as u32 * a + pixel[channel] as u32 * (255 - a)) / 255;
            pixel[channel] = blended as u8;
        }
        pixel[3] = 0xFF;
    }
}
