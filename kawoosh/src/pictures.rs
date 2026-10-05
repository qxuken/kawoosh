//! The pictures the panes draw, by path: a markdown buffer's images, a
//! preview's, the image pane's (`kawoosh/lua/image.lua`) — each file
//! read once on a thread of its own (`kawoosh_systems::picture`),
//! registered with kui the next frame, and one picture however many
//! panes show it.
//!
//! A picture that moves keeps its frames here and one image in kui,
//! the frame shown written over it: it steps while a view that drew it
//! last frame asked it to (`kawoosh.image(path, { play = true })`),
//! the window woken when the frame's time is up, and holds still
//! otherwise. A drawing keeps its tree and is drawn again at the width
//! a view asks for; a file is read again when a view says it changed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use kawoosh_systems::io::IoMsg;
use kawoosh_systems::picture::{self, Frame, Picture, Vector};

use crate::app::Kawoosh;

/// An image a pane shows: being read, ready (kui's id and the
/// picture's own size, what it is laid out by), or why not.
pub enum Image {
    Loading,
    Ready {
        id: kui_native::ImageId,
        w: u32,
        h: u32,
    },
    Failed(String),
}

/// What is kept of a picture beside its image in kui.
pub struct Kept {
    /// The pixels kui has.
    pixels: (u32, u32),
    /// Every frame of a picture that moves; none of a still.
    frames: Vec<Frame>,
    more: bool,
    format: &'static str,
    vector: Option<Vector>,
    /// The frame shown, since when, and whether kui has it yet.
    at: usize,
    since: Instant,
    stale: bool,
    /// A view that drew it asked it to go on, since the last step.
    asked: bool,
    /// When the window is woken for the next frame.
    due: Option<Instant>,
    /// The width a drawing is being drawn at now.
    drawing: Option<u32>,
    /// The file is being read again.
    reading: bool,
}

/// The pictures asked for, by path.
#[derive(Default)]
pub struct Images {
    pub by_path: HashMap<PathBuf, Image>,
    pub kept: HashMap<PathBuf, Kept>,
    /// Pictures read since the last frame: `true` for a drawing drawn
    /// again.
    pub pending: Vec<(PathBuf, Picture, bool)>,
    /// Images of files that stopped being pictures, for kui to drop.
    dead: Vec<kui_native::ImageId>,
}

impl Kawoosh {
    /// The image at `dest` (relative to the buffer's directory), asked
    /// for once and read on the io thread; what is known of it now.
    pub(crate) fn markdown_image(&mut self, dir: Option<&Path>, dest: &str) -> Option<&Image> {
        // `data:image/png;base64,…`: the pixels in the text, decoded
        // here, once — kept under a name of their own.
        if let Some(data) = dest.strip_prefix("data:") {
            let key = PathBuf::from(format!("data:{:016x}", crate::markdown::hash_of(data)));
            if !self.md_images.by_path.contains_key(&key) {
                let r = data
                    .split_once(";base64,")
                    .ok_or_else(|| "not base64".to_string())
                    .and_then(|(_, b64)| {
                        crate::markdown::base64(b64).ok_or_else(|| "bad base64".into())
                    })
                    .and_then(|bytes| picture::decode_bytes(&bytes, None, None));
                self.md_images.by_path.insert(key.clone(), Image::Loading);
                self.image_decoded(key.clone(), r, false);
            }
            return self.md_images.by_path.get(&key);
        }
        if dest.contains("://") {
            return None;
        }
        let path = match dir {
            Some(d) => kawoosh_systems::fs::expand(Path::new(dest), d),
            None => PathBuf::from(dest),
        };
        let max = self.picture_cap("markdown.image_max_mb", 16);
        self.picture(&path, max)
    }

    /// A cap on a picture's file, a setting's megabytes in bytes.
    fn picture_cap(&self, setting: &str, default: u64) -> u64 {
        self.ed
            .settings
            .int(setting)
            .map_or(default, |n| n.max(0) as u64)
            << 20
    }

    /// The picture at `path`, asked for once, refused past `max` bytes.
    fn picture(&mut self, path: &Path, max: u64) -> Option<&Image> {
        if !self.md_images.by_path.contains_key(path) {
            self.md_images
                .by_path
                .insert(path.to_path_buf(), Image::Loading);
            self.read_picture(path.to_path_buf(), max, None);
        }
        self.md_images.by_path.get(path)
    }

    /// What a Lua view asked for by path (`kawoosh.image`): files up to
    /// `image.max_mb`.
    pub(crate) fn lua_picture(&mut self, path: &Path) {
        let max = self.picture_cap("image.max_mb", 64);
        self.picture(path, max);
        self.publish_picture(path);
    }

    fn read_picture(&mut self, path: PathBuf, max: u64, width: Option<u32>) {
        if self.jobs_inline {
            let r = picture::decode(&path, max, width);
            self.image_decoded(path, r, false);
        } else {
            self.pending_jobs += 1;
            self.io.run("image", move || IoMsg::Image {
                result: picture::decode(&path, max, width),
                path,
                drawn: false,
            });
        }
    }

    /// A picture read: registered with kui when the next frame has the
    /// core (`register_images`), or why it could not be. A drawing that
    /// could not be drawn again stays as it was.
    pub(crate) fn image_decoded(&mut self, path: PathBuf, r: Result<Picture, String>, drawn: bool) {
        match r {
            Ok(pic) => self.md_images.pending.push((path, pic, drawn)),
            Err(_) if drawn => {
                if let Some(k) = self.md_images.kept.get_mut(&path) {
                    k.drawing = None;
                }
            }
            Err(e) => {
                self.md_images.kept.remove(&path);
                if let Some(Image::Ready { id, .. }) = self
                    .md_images
                    .by_path
                    .insert(path.clone(), Image::Failed(e))
                {
                    self.md_images.dead.push(id);
                }
                self.publish_picture(&path);
            }
        }
    }

    /// Where a picture has got, said to the Lua views.
    fn publish_picture(&self, path: &Path) {
        let Some(rt) = &self.scripting.rt else { return };
        let snap = match self.md_images.by_path.get(path) {
            Some(Image::Ready { id, w, h }) => {
                let Some(k) = self.md_images.kept.get(path) else {
                    return;
                };
                kawoosh_lua::ImageSnap::Ready {
                    id: id.to_ffi() as i64,
                    width: *w,
                    height: *h,
                    pixels: k.pixels,
                    frames: k.frames.len().max(1),
                    frame: k.at,
                    duration_ms: k.frames.iter().map(|f| f.delay_ms as u64).sum(),
                    more: k.more,
                    format: k.format,
                    vector: k.vector.is_some(),
                }
            }
            Some(Image::Failed(e)) => kawoosh_lua::ImageSnap::Failed(e.clone()),
            _ => return,
        };
        rt.set_image(path.to_path_buf(), snap);
    }

    /// The pictures read since the last frame into kui — a file read
    /// again or a drawing drawn again over the image it had — and the
    /// ones that move stepped to the frame the clock says.
    pub(crate) fn register_images(&mut self, ui: &mut kui_native::Ui<'_>) {
        for id in std::mem::take(&mut self.md_images.dead) {
            ui.core().remove_image(id);
        }
        for (path, mut pic, drawn) in std::mem::take(&mut self.md_images.pending) {
            let (w, h) = (pic.width, pic.height);
            let moves = pic.frames.len() > 1;
            let first = if moves {
                pic.frames[0].rgba.clone()
            } else {
                std::mem::take(&mut pic.frames[0].rgba)
            };
            let had = match self.md_images.by_path.get(&path) {
                Some(Image::Ready { id, .. }) => Some(*id),
                _ => None,
            };
            let id = match had {
                Some(id)
                    if ui
                        .core()
                        .update_image_with(id, w, h, |to| to.copy_from_slice(&first)) =>
                {
                    id
                }
                _ => ui.core().resources.add_image(w, h, first),
            };
            if let (true, Some(k)) = (drawn, self.md_images.kept.get_mut(&path)) {
                k.pixels = (w, h);
                k.drawing = None;
            } else {
                self.md_images.kept.insert(
                    path.clone(),
                    Kept {
                        pixels: (w, h),
                        frames: if moves { pic.frames } else { Vec::new() },
                        more: pic.more,
                        format: pic.format,
                        vector: pic.vector,
                        at: 0,
                        since: Instant::now(),
                        stale: false,
                        asked: false,
                        due: None,
                        drawing: None,
                        reading: false,
                    },
                );
            }
            self.md_images.by_path.insert(
                path.clone(),
                Image::Ready {
                    id,
                    w: pic.natural.0,
                    h: pic.natural.1,
                },
            );
            self.publish_picture(&path);
        }
        let now = Instant::now();
        let mut stepped = Vec::new();
        for (path, k) in &mut self.md_images.kept {
            if k.frames.len() < 2 {
                continue;
            }
            if std::mem::take(&mut k.asked)
                && now >= k.since + Duration::from_millis(k.frames[k.at].delay_ms as u64)
            {
                k.at = (k.at + 1) % k.frames.len();
                k.since = now;
                k.stale = true;
            }
            if !std::mem::take(&mut k.stale) {
                continue;
            }
            let Some(Image::Ready { id, .. }) = self.md_images.by_path.get(path) else {
                continue;
            };
            let frame = &k.frames[k.at].rgba;
            ui.core()
                .update_image_with(*id, k.pixels.0, k.pixels.1, |to| to.copy_from_slice(frame));
            stepped.push(path.clone());
        }
        for path in stepped {
            self.publish_picture(&path);
        }
    }

    /// `play`: a view drew the picture and wants it moving — the window
    /// woken when the frame shown has had its time.
    pub(crate) fn picture_play(&mut self, path: &Path) {
        let Some(k) = self.md_images.kept.get_mut(path) else {
            return;
        };
        if k.frames.len() < 2 {
            return;
        }
        k.asked = true;
        let now = Instant::now();
        if k.due.is_some_and(|d| d > now) {
            return;
        }
        let due = (k.since + Duration::from_millis(k.frames[k.at].delay_ms as u64)).max(now);
        k.due = Some(due);
        // A test draws its frames itself.
        if !self.jobs_inline {
            self.io.run("picture frame", move || {
                std::thread::sleep(due.saturating_duration_since(Instant::now()));
                IoMsg::Wake
            });
        }
    }

    /// `frame = n`: the frame shown, from 0, held from now.
    pub(crate) fn picture_frame(&mut self, path: &Path, n: usize) {
        let Some(k) = self.md_images.kept.get_mut(path) else {
            return;
        };
        if n >= k.frames.len() || n == k.at {
            return;
        }
        k.at = n;
        k.since = Instant::now();
        k.stale = true;
        self.wake.named("picture frame").wake();
    }

    /// `width = px`: a drawing drawn again that wide, one at a time —
    /// the view asks again while what it has is not what it wants.
    pub(crate) fn picture_width(&mut self, path: &Path, width: u32) {
        let Some(k) = self.md_images.kept.get_mut(path) else {
            return;
        };
        let Some(v) = k.vector.clone() else { return };
        let width = v.fit(width);
        if width == k.pixels.0 || k.drawing.is_some() || k.reading {
            return;
        }
        k.drawing = Some(width);
        let natural = v.size();
        let draw = move || {
            v.draw(width).map(|(width, height, rgba)| Picture {
                width,
                height,
                natural,
                frames: vec![Frame { rgba, delay_ms: 0 }],
                more: false,
                format: "svg",
                vector: None,
            })
        };
        let path = path.to_path_buf();
        if self.jobs_inline {
            self.image_decoded(path, draw(), true);
        } else {
            self.pending_jobs += 1;
            self.io.run("image", move || IoMsg::Image {
                result: draw(),
                path,
                drawn: true,
            });
        }
    }

    /// `reload = true`: the file read again, a drawing at the width it
    /// is shown at; the picture there was stays until it lands.
    pub(crate) fn picture_reload(&mut self, path: &Path) {
        let max = self.picture_cap("image.max_mb", 64);
        let width = match self.md_images.kept.get_mut(path) {
            Some(k) if k.reading => return,
            Some(k) => {
                k.reading = true;
                k.vector.as_ref().map(|_| k.pixels.0)
            }
            None => match self.md_images.by_path.get(path) {
                Some(Image::Failed(_)) => None,
                _ => return,
            },
        };
        self.read_picture(path.to_path_buf(), max, width);
    }
}
