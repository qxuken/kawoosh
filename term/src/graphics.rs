//! Kitty's graphics protocol (docs/design/kitty-graphics.md, roadmap
//! step 56): the commands an APC `ESC _ G … ESC \` carries, the images
//! they transmit, the placements that show them, and the replies. The
//! [`crate::Terminal`] scans the APCs out of the pty's bytes and hands
//! each here with what it knows — the cursor, the screen, the cell's
//! size in pixels — and asks back what is on screen to draw
//! ([`crate::Terminal::images`]). Nothing here knows of kui.
//!
//! A transmission is decoded off the frame's thread: its size is read
//! at once (the raw formats say it, a PNG's header does), so a
//! placement takes its cells and the cursor moves in order with the
//! text around it; the pixels come back from a worker and the image is
//! drawn from then on.

use std::collections::HashMap;
use std::io::Read as _;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

/// The most bytes one APC keeps: a chunk is at most 4096, but a direct
/// transmission need not be chunked.
pub(crate) const APC_MAX: usize = 64 << 20;
/// The most RGBA a terminal keeps, the oldest images unplaced first.
const QUOTA: usize = 256 << 20;
/// kitty's own limit on a side.
const MAX_SIDE: u32 = 10_000;

/// A command's keys, as the spec names them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Keys {
    /// `a`: t transmit, T transmit and put, p put, d delete, q query.
    action: u8,
    /// `f`: 24 RGB, 32 RGBA (the default), 100 PNG.
    format: u32,
    /// `t`: d direct (the default), f file, t temporary file, s shm.
    medium: u8,
    /// `o`: z for zlib.
    compression: u8,
    /// `i`, `I`, `p`: the image's id, its number, the placement's id.
    id: u32,
    number: u32,
    placement: u32,
    /// `m`: 1 while more chunks follow.
    more: u32,
    /// `s`, `v`: a raw image's width and height.
    width: u32,
    height: u32,
    /// `S`, `O`: a file's size and offset to read.
    file_size: u32,
    file_offset: u32,
    /// `x`, `y`, `w`, `h`: the source rect (and a delete's cell).
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    /// `c`, `r`: the cells to fill.
    cols: u32,
    rows: u32,
    /// `X`, `Y`: the offset inside the first cell, in pixels.
    off_x: u32,
    off_y: u32,
    /// `z`: under the text when negative.
    z: i32,
    /// `C`: 1 leaves the cursor where it is.
    stay: u32,
    /// `q`: 1 errors only, 2 no reply.
    quiet: u32,
    /// `d`: what a delete deletes.
    delete: u8,
    /// `U`, `P`, `Q`: unicode placeholders and relative placements,
    /// answered "not supported".
    virtual_: u32,
    parent: u32,
    /// Sent with neither an id nor a number: stored under an id of the
    /// terminal's, and never answered.
    silent: bool,
}

impl Keys {
    fn parse(s: &str) -> Result<Keys, String> {
        let mut k = Keys {
            action: b't',
            format: 32,
            medium: b'd',
            ..Keys::default()
        };
        for pair in s.split(',').filter(|p| !p.is_empty()) {
            let (key, val) = pair
                .split_once('=')
                .ok_or_else(|| format!("EINVAL:malformed key {pair}"))?;
            let ch = || -> Result<u8, String> {
                match val.as_bytes() {
                    [c] => Ok(*c),
                    _ => Err(format!("EINVAL:{key} takes one character")),
                }
            };
            let int = || -> Result<u32, String> {
                val.parse()
                    .map_err(|_| format!("EINVAL:{key} is not a number"))
            };
            match key {
                "a" => k.action = ch()?,
                "f" => k.format = int()?,
                "t" => k.medium = ch()?,
                "o" => k.compression = ch()?,
                "i" => k.id = int()?,
                "I" => k.number = int()?,
                "p" => k.placement = int()?,
                "m" => k.more = int()?,
                "s" => k.width = int()?,
                "v" => k.height = int()?,
                "S" => k.file_size = int()?,
                "O" => k.file_offset = int()?,
                "x" => k.x = int()?,
                "y" => k.y = int()?,
                "w" => k.w = int()?,
                "h" => k.h = int()?,
                "c" => k.cols = int()?,
                "r" => k.rows = int()?,
                "X" => k.off_x = int()?,
                "Y" => k.off_y = int()?,
                "z" => {
                    k.z = val
                        .parse()
                        .map_err(|_| format!("EINVAL:{key} is not a number"))?
                }
                "C" => k.stay = int()?,
                "q" => k.quiet = int()?,
                "d" => k.delete = ch()?,
                "U" => k.virtual_ = int()?,
                "P" | "Q" => k.parent = int()?,
                // What this terminal does not read (`H`, `V`, the
                // animation's keys) is left alone, as kitty leaves an
                // unknown key.
                _ => {}
            }
        }
        Ok(k)
    }
}

/// What the terminal is, for a command: where the cursor is, what the
/// screen shows, how big a cell is.
pub(crate) struct Here {
    /// The cursor's session line and column.
    pub line: u64,
    pub col: u16,
    /// The session line of the screen's top row, and its size.
    pub top: u64,
    pub rows: u16,
    pub cols: u16,
    /// A cell's size in pixels, as the pty was told it.
    pub cell: (u32, u32),
    /// Whether files named by a command are this machine's.
    pub local: bool,
    pub alt: bool,
}

/// What a command asks of the terminal besides a reply.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Effect {
    /// Move the cursor right by this many columns and down by this many
    /// rows, kitty's way (its column past the edge wraps to the next
    /// line).
    pub cursor: Option<(u16, u16)>,
}

/// A placement's size, as its keys asked for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fill {
    /// The source rect's own pixels.
    Natural,
    /// `c` and `r`: stretched to the cells.
    Cells(u32, u32),
    /// `c` alone: that many columns, the height by the aspect.
    Cols(u32),
    /// `r` alone.
    Rows(u32),
}

#[derive(Clone, Debug)]
pub(crate) struct Placement {
    image: u32,
    id: u32,
    /// The session line and column of its top-left cell.
    line: u64,
    col: u16,
    /// The cells it covers, from the cell size when it was put.
    cols: u16,
    rows: u16,
    src: (u32, u32, u32, u32),
    fill: Fill,
    off: (u32, u32),
    z: i32,
    alt: bool,
}

enum Pixels {
    Ready(Arc<Vec<u8>>),
    /// Decoding on a worker; the reply to write when it is back.
    Pending(Receiver<Result<Vec<u8>, String>>, Option<(Keys, bool)>),
    Failed,
}

pub(crate) struct Image {
    id: u32,
    number: u32,
    width: u32,
    height: u32,
    pixels: Pixels,
    generation: u64,
    /// Transmission order, for the quota.
    order: u64,
}

impl Image {
    fn bytes(&self) -> usize {
        self.width as usize * self.height as usize * 4
    }
}

/// A placement on screen, to draw ([`crate::Terminal::images`]).
#[derive(Clone, Debug)]
pub struct Placed {
    /// The image's id, and a number that moves whenever its pixels do.
    pub image: u32,
    pub generation: u64,
    /// The image's pixels, RGBA, `width × height`.
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
    /// The part of the image shown: x, y, width, height in its pixels.
    pub src: (u32, u32, u32, u32),
    /// The screen cell of its top-left: a row above the screen is
    /// negative, the image's top scrolled off.
    pub row: i32,
    pub col: u16,
    /// Its offset inside that cell and its drawn size, in the pixels
    /// the pty was told a cell is.
    pub offset: (u32, u32),
    pub size: (u32, u32),
    /// Under the text when negative.
    pub z: i32,
}

/// A chunked transmission being gathered.
struct Loading {
    keys: Keys,
    data: Vec<u8>,
}

#[derive(Default)]
pub(crate) struct Graphics {
    pub(crate) images: HashMap<u32, Image>,
    pub(crate) placements: Vec<Placement>,
    loading: Option<Loading>,
    /// Ids given to images sent by number alone, from the top down.
    next_id: u32,
    next_generation: u64,
    order: u64,
    /// Replies to write to the pty.
    pub replies: Vec<u8>,
}

impl Graphics {
    /// One APC's payload after its `G`.
    pub(crate) fn command(&mut self, payload: &[u8], here: &Here) -> Effect {
        let (keys, data) = match payload.iter().position(|b| *b == b';') {
            Some(i) => (&payload[..i], &payload[i + 1..]),
            None => (payload, &[][..]),
        };
        let keys = match std::str::from_utf8(keys)
            .map_err(|_| "EINVAL:keys are not text".to_string())
            .and_then(Keys::parse)
        {
            Ok(k) => k,
            Err(e) => {
                self.reply(&Keys::default(), Err(e));
                return Effect::default();
            }
        };
        // A chunk after the first carries `m` (and perhaps `q`) only:
        // the first's keys stand for the whole.
        if let Some(mut l) = self.loading.take() {
            l.data.extend_from_slice(data);
            if keys.more == 1 {
                self.loading = Some(l);
                return Effect::default();
            }
            return self.transmitted(l.keys, &l.data, here);
        }
        match keys.action {
            b't' | b'T' | b'q' if keys.more == 1 => {
                self.loading = Some(Loading {
                    keys,
                    data: data.to_vec(),
                });
                Effect::default()
            }
            b't' | b'T' | b'q' => self.transmitted(keys, data, here),
            b'p' => {
                let r = self.put(&keys, here);
                self.reply_put(&keys, r)
            }
            b'd' => {
                self.delete(&keys, here);
                Effect::default()
            }
            b'f' | b'a' | b'c' => {
                self.reply(&keys, Err("ENOTSUP:animation is not supported".into()));
                Effect::default()
            }
            _ => {
                self.reply(&keys, Err("EINVAL:unknown action".into()));
                Effect::default()
            }
        }
    }

    /// A whole transmission (`a=t`, `T`, `q`): its bytes read, its size
    /// known, its pixels on their way; then the placement `T` asks for.
    fn transmitted(&mut self, mut keys: Keys, data: &[u8], here: &Here) -> Effect {
        if keys.virtual_ != 0 {
            self.reply(
                &keys,
                Err("ENOTSUP:unicode placeholders are not supported".into()),
            );
            return Effect::default();
        }
        let bytes = match read_medium(&keys, data, here.local) {
            Ok(b) => b,
            Err(e) => {
                self.reply(&keys, Err(e));
                return Effect::default();
            }
        };
        let (width, height) = match size_of(&keys, &bytes) {
            Ok(s) => s,
            Err(e) => {
                self.reply(&keys, Err(e));
                return Effect::default();
            }
        };
        if keys.action == b'q' {
            // A query stores nothing: the answer is whether it would.
            let r = decode(keys.format, keys.compression, width, height, &bytes).map(|_| ());
            self.reply(&keys, r);
            return Effect::default();
        }
        if keys.id == 0 {
            // A number alone is answered with the id given it; neither,
            // kitty stores it all the same and answers nothing.
            keys.silent = keys.number == 0;
            keys.id = self.fresh_id();
        }
        // The same id again replaces the image, and its placements go.
        self.forget_image(keys.id);
        let (tx, rx) = channel();
        let (format, compression) = (keys.format, keys.compression);
        std::thread::spawn(move || {
            let _ = tx.send(decode(format, compression, width, height, &bytes));
        });
        self.order += 1;
        self.next_generation += 1;
        let put = keys.action == b'T';
        self.images.insert(
            keys.id,
            Image {
                id: keys.id,
                number: keys.number,
                width,
                height,
                pixels: Pixels::Pending(rx, Some((keys.clone(), put))),
                generation: self.next_generation,
                order: self.order,
            },
        );
        self.enforce_quota();
        if put {
            // Placed now, so the cursor moves in order with the text;
            // drawn when the pixels are back. The reply waits for them.
            match self.put(&keys, here) {
                Ok(cells) => {
                    return Effect {
                        cursor: (keys.stay != 1).then_some(cells),
                    };
                }
                Err(e) => {
                    if let Some(img) = self.images.get_mut(&keys.id)
                        && let Pixels::Pending(_, reply) = &mut img.pixels
                    {
                        *reply = None;
                    }
                    self.reply(&keys, Err(e));
                }
            }
        }
        Effect::default()
    }

    fn fresh_id(&mut self) -> u32 {
        if self.next_id == 0 {
            self.next_id = u32::MAX;
        }
        while self.images.contains_key(&self.next_id) {
            self.next_id -= 1;
        }
        let id = self.next_id;
        self.next_id -= 1;
        id
    }

    /// The image a command names: by id, or the newest with its number.
    fn named(&self, keys: &Keys) -> Option<u32> {
        if keys.id != 0 {
            return self.images.contains_key(&keys.id).then_some(keys.id);
        }
        (keys.number != 0)
            .then(|| {
                self.images
                    .values()
                    .filter(|i| i.number == keys.number)
                    .max_by_key(|i| i.order)
                    .map(|i| i.id)
            })
            .flatten()
    }

    /// `a=p` (and `T`'s second half): a placement at the cursor, and the
    /// cells the cursor moves by.
    fn put(&mut self, keys: &Keys, here: &Here) -> Result<(u16, u16), String> {
        if keys.parent != 0 {
            return Err("ENOTSUP:relative placements are not supported".into());
        }
        let Some(id) = self.named(keys) else {
            return Err("ENOENT:no such image".into());
        };
        let img = &self.images[&id];
        if matches!(img.pixels, Pixels::Failed) {
            return Err("ENOENT:the image did not load".into());
        }
        let (iw, ih) = (img.width, img.height);
        let sx = keys.x.min(iw);
        let sy = keys.y.min(ih);
        let sw = if keys.w == 0 {
            iw - sx
        } else {
            keys.w.min(iw - sx)
        };
        let sh = if keys.h == 0 {
            ih - sy
        } else {
            keys.h.min(ih - sy)
        };
        let fill = match (keys.cols, keys.rows) {
            (0, 0) => Fill::Natural,
            (c, 0) => Fill::Cols(c),
            (0, r) => Fill::Rows(r),
            (c, r) => Fill::Cells(c, r),
        };
        let (cw, ch) = (here.cell.0.max(1), here.cell.1.max(1));
        let (pw, ph) = drawn(fill, (sw, sh), (cw, ch));
        let cols = (keys.off_x + pw).div_ceil(cw).max(1).min(u16::MAX as u32) as u16;
        let rows = (keys.off_y + ph).div_ceil(ch).max(1).min(u16::MAX as u32) as u16;
        let pid = keys.placement;
        if pid != 0 {
            self.placements.retain(|p| !(p.image == id && p.id == pid));
        }
        self.placements.push(Placement {
            image: id,
            id: pid,
            line: here.line,
            col: here.col,
            cols,
            rows,
            src: (sx, sy, sw, sh),
            fill,
            off: (keys.off_x.min(cw - 1), keys.off_y.min(ch - 1)),
            z: keys.z,
            alt: here.alt,
        });
        Ok((cols, rows))
    }

    fn reply_put(&mut self, keys: &Keys, r: Result<(u16, u16), String>) -> Effect {
        match r {
            Ok(cells) => {
                self.reply(keys, Ok(()));
                Effect {
                    cursor: (keys.stay != 1).then_some(cells),
                }
            }
            Err(e) => {
                self.reply(keys, Err(e));
                Effect::default()
            }
        }
    }

    /// `a=d`: the placements its `d` names gone; an uppercase `d` frees
    /// the images left with none.
    fn delete(&mut self, keys: &Keys, here: &Here) {
        let what = if keys.delete == 0 { b'a' } else { keys.delete };
        let free = what.is_ascii_uppercase();
        let top = here.top;
        let screen = |p: &Placement| -> (i64, i64, i64, i64) {
            let row = p.line as i64 - top as i64;
            (
                row,
                row + p.rows as i64,
                p.col as i64,
                p.col as i64 + p.cols as i64,
            )
        };
        let at = |p: &Placement, row: i64, col: i64| {
            let (r0, r1, c0, c1) = screen(p);
            (r0..r1).contains(&row) && (c0..c1).contains(&col)
        };
        let cur_row = here.line as i64 - top as i64;
        let named = self.named(keys);
        let before: Vec<u32> = self.placements.iter().map(|p| p.image).collect();
        match what.to_ascii_lowercase() {
            b'a' => {
                let rows = here.rows as i64;
                self.placements.retain(|p| {
                    let (r0, r1, ..) = screen(p);
                    p.alt != here.alt || r1 <= 0 || r0 >= rows
                });
            }
            b'i' | b'n' => {
                let pid = keys.placement;
                self.placements
                    .retain(|p| Some(p.image) != named || (pid != 0 && p.id != pid));
                if free && let Some(id) = named {
                    // By id, the image goes whether or not it was placed.
                    self.forget_image(id);
                }
            }
            b'c' => self.placements.retain(|p| !at(p, cur_row, here.col as i64)),
            b'p' => {
                let (row, col) = (keys.y as i64 - 1, keys.x as i64 - 1);
                self.placements.retain(|p| !at(p, row, col));
            }
            b'q' => {
                let (row, col) = (keys.y as i64 - 1, keys.x as i64 - 1);
                self.placements
                    .retain(|p| !(at(p, row, col) && p.z == keys.z));
            }
            b'r' => {
                let range = keys.x..=keys.y;
                self.placements.retain(|p| !range.contains(&p.image));
                if free {
                    let gone: Vec<u32> = self
                        .images
                        .keys()
                        .copied()
                        .filter(|id| range.contains(id))
                        .collect();
                    for id in gone {
                        self.forget_image(id);
                    }
                }
            }
            b'x' => {
                let col = keys.x as i64 - 1;
                self.placements.retain(|p| {
                    let (.., c0, c1) = screen(p);
                    !(c0..c1).contains(&col)
                });
            }
            b'y' => {
                let row = keys.y as i64 - 1;
                self.placements.retain(|p| {
                    let (r0, r1, ..) = screen(p);
                    !(r0..r1).contains(&row)
                });
            }
            b'z' => self.placements.retain(|p| p.z != keys.z),
            _ => {}
        }
        if free {
            for id in before {
                if !self.placements.iter().any(|p| p.image == id) {
                    self.images.remove(&id);
                }
            }
        }
    }

    /// The image and its placements gone.
    fn forget_image(&mut self, id: u32) {
        self.images.remove(&id);
        self.placements.retain(|p| p.image != id);
    }

    /// Over the quota, the oldest images go, the unplaced first.
    fn enforce_quota(&mut self) {
        let mut total: usize = self.images.values().map(Image::bytes).sum();
        if total <= QUOTA {
            return;
        }
        let mut by_age: Vec<(bool, u64, u32)> = self
            .images
            .values()
            .map(|i| {
                let placed = self.placements.iter().any(|p| p.image == i.id);
                (placed, i.order, i.id)
            })
            .collect();
        by_age.sort();
        for (_, _, id) in by_age {
            if total <= QUOTA {
                break;
            }
            total -= self.images[&id].bytes();
            self.forget_image(id);
        }
    }

    /// The decodes that finished: their pixels kept, their replies
    /// written. True when anything changed.
    pub(crate) fn poll(&mut self, wait: bool) -> bool {
        let mut changed = false;
        let mut replies = Vec::new();
        for img in self.images.values_mut() {
            let Pixels::Pending(rx, reply) = &mut img.pixels else {
                continue;
            };
            let got = if wait {
                rx.recv().ok()
            } else {
                rx.try_recv().ok()
            };
            let Some(got) = got else { continue };
            let reply = reply.take();
            changed = true;
            match got {
                Ok(rgba) => {
                    img.pixels = Pixels::Ready(Arc::new(rgba));
                    if let Some((keys, _)) = reply {
                        replies.push((keys, Ok(())));
                    }
                }
                Err(e) => {
                    img.pixels = Pixels::Failed;
                    if let Some((keys, _)) = reply {
                        replies.push((keys, Err(e)));
                    }
                }
            }
        }
        // A failed image shows nothing: its placements go with it.
        let failed: Vec<u32> = self
            .images
            .values()
            .filter(|i| matches!(i.pixels, Pixels::Failed))
            .map(|i| i.id)
            .collect();
        for id in failed {
            self.forget_image(id);
        }
        for (keys, r) in replies {
            self.reply(&keys, r);
        }
        changed
    }

    /// Whether a decode is still out.
    pub(crate) fn busy(&self) -> bool {
        self.images
            .values()
            .any(|i| matches!(i.pixels, Pixels::Pending(..)))
    }

    /// The reply to a command, unless its `q` or its lack of a name
    /// says none.
    fn reply(&mut self, keys: &Keys, r: Result<(), String>) {
        if keys.silent || (keys.id == 0 && keys.number == 0) {
            return;
        }
        match (&r, keys.quiet) {
            (_, 2) | (Ok(()), 1) => return,
            _ => {}
        }
        let mut s = String::from("\x1b_G");
        if keys.id != 0 {
            s.push_str(&format!("i={}", keys.id));
        }
        if keys.number != 0 {
            if keys.id != 0 {
                s.push(',');
            }
            s.push_str(&format!("I={}", keys.number));
        }
        if keys.placement != 0 {
            s.push_str(&format!(",p={}", keys.placement));
        }
        s.push(';');
        match r {
            Ok(()) => s.push_str("OK"),
            Err(e) => s.push_str(&e),
        }
        s.push_str("\x1b\\");
        self.replies.extend_from_slice(s.as_bytes());
    }

    /// The screen cleared (`ED 2`): its placements go; with its history
    /// (`ED 3`), those in history too.
    pub(crate) fn cleared(&mut self, here: &Here, history: bool) {
        let (top, rows) = (here.top, here.rows as u64);
        self.placements.retain(|p| {
            if p.alt != here.alt {
                return true;
            }
            let end = p.line + p.rows as u64;
            let on_screen = end > top && p.line < top + rows;
            let above = end <= top;
            !(on_screen || (history && above))
        });
    }

    /// The alternate screen left: its placements go.
    pub(crate) fn left_alt(&mut self) {
        self.placements.retain(|p| !p.alt);
    }

    /// History let lines before `oldest` go: placements wholly above it
    /// go with them.
    pub(crate) fn trimmed(&mut self, oldest: u64) {
        self.placements
            .retain(|p| p.alt || p.line + p.rows as u64 > oldest);
    }

    /// Everything gone (`RIS`).
    pub(crate) fn reset(&mut self) {
        *self = Graphics {
            next_id: self.next_id,
            next_generation: self.next_generation,
            order: self.order,
            ..Graphics::default()
        };
    }

    /// What is on the screen showing `top` onward, with pixels to draw.
    pub(crate) fn on_screen(&self, here: &Here) -> Vec<Placed> {
        let (cw, ch) = (here.cell.0.max(1), here.cell.1.max(1));
        let mut out = Vec::new();
        for p in &self.placements {
            if p.alt != here.alt {
                continue;
            }
            let row = p.line as i64 - here.top as i64;
            if row + p.rows as i64 <= 0 || row >= here.rows as i64 {
                continue;
            }
            let Some(img) = self.images.get(&p.image) else {
                continue;
            };
            let Pixels::Ready(rgba) = &img.pixels else {
                continue;
            };
            out.push(Placed {
                image: img.id,
                generation: img.generation,
                width: img.width,
                height: img.height,
                rgba: rgba.clone(),
                src: p.src,
                row: row as i32,
                col: p.col,
                offset: p.off,
                size: drawn(p.fill, (p.src.2, p.src.3), (cw, ch)),
                z: p.z,
            });
        }
        out.sort_by_key(|p| p.z);
        out
    }
}

/// A placement's drawn size in pixels, for a source rect `src` and a
/// cell of `cell`.
fn drawn(fill: Fill, src: (u32, u32), cell: (u32, u32)) -> (u32, u32) {
    let (sw, sh) = (src.0.max(1) as u64, src.1.max(1) as u64);
    let (cw, ch) = (cell.0 as u64, cell.1 as u64);
    let (w, h) = match fill {
        Fill::Natural => (sw, sh),
        Fill::Cells(c, r) => (c as u64 * cw, r as u64 * ch),
        Fill::Cols(c) => {
            let w = c as u64 * cw;
            (w, (w * sh).div_ceil(sw))
        }
        Fill::Rows(r) => {
            let h = r as u64 * ch;
            ((h * sw).div_ceil(sh), h)
        }
    };
    (w.min(u32::MAX as u64) as u32, h.min(u32::MAX as u64) as u32)
}

/// The bytes a transmission names: its payload's, or a file's.
fn read_medium(keys: &Keys, data: &[u8], local: bool) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(
            data.iter()
                .copied()
                .filter(|b| !b.is_ascii_whitespace())
                .collect::<Vec<_>>(),
        )
        .map_err(|_| "EINVAL:the payload is not base64".to_string())?;
    match keys.medium {
        b'd' => Ok(raw),
        b'f' | b't' => {
            if !local {
                return Err("EBADF:files are not read from another host".into());
            }
            let path = std::path::PathBuf::from(
                String::from_utf8(raw).map_err(|_| "EINVAL:the path is not text".to_string())?,
            );
            if keys.medium == b't' {
                // kitty's rule: a temporary file is under the temp
                // directory and names the protocol, and is deleted.
                let tmp = std::env::temp_dir();
                let named = path
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().contains("tty-graphics-protocol"));
                let canon = |p: &std::path::Path| std::fs::canonicalize(p).ok();
                let inside = matches!(
                    (canon(&path), canon(&tmp)),
                    (Some(p), Some(t)) if p.starts_with(&t)
                ) || path.starts_with("/tmp")
                    || path.starts_with("/dev/shm");
                if !named || !inside {
                    return Err("EPERM:not a temporary file".into());
                }
            }
            let mut f = std::fs::File::open(&path).map_err(|e| format!("EBADF:{e}"))?;
            if keys.file_offset != 0 {
                use std::io::Seek as _;
                f.seek(std::io::SeekFrom::Start(keys.file_offset as u64))
                    .map_err(|e| format!("EBADF:{e}"))?;
            }
            let mut out = Vec::new();
            let r = if keys.file_size != 0 {
                f.take(keys.file_size as u64).read_to_end(&mut out)
            } else {
                f.take(APC_MAX as u64 * 4).read_to_end(&mut out)
            };
            r.map_err(|e| format!("EBADF:{e}"))?;
            if keys.medium == b't' {
                let _ = std::fs::remove_file(&path);
            }
            Ok(out)
        }
        b's' => Err("ENOTSUP:shared memory is not supported".into()),
        _ => Err("EINVAL:unknown transmission medium".into()),
    }
}

/// A transmission's size in pixels, without decoding it: the keys'
/// for the raw formats, the header's for a PNG.
fn size_of(keys: &Keys, bytes: &[u8]) -> Result<(u32, u32), String> {
    let (w, h) = match keys.format {
        24 | 32 => {
            if keys.width == 0 || keys.height == 0 {
                return Err("EINVAL:a raw image needs s and v".into());
            }
            (keys.width, keys.height)
        }
        100 => {
            let head = if keys.compression == b'z' {
                let mut head = Vec::with_capacity(33);
                flate2::read::ZlibDecoder::new(bytes)
                    .take(33)
                    .read_to_end(&mut head)
                    .map_err(|e| format!("EINVAL:{e}"))?;
                head
            } else {
                bytes[..bytes.len().min(33)].to_vec()
            };
            png_size(&head).ok_or_else(|| "EINVAL:not a PNG".to_string())?
        }
        _ => return Err("EINVAL:unknown format".into()),
    };
    if w > MAX_SIDE || h > MAX_SIDE {
        return Err("EFBIG:larger than 10000 pixels a side".into());
    }
    Ok((w, h))
}

/// A PNG's width and height, from its IHDR.
fn png_size(head: &[u8]) -> Option<(u32, u32)> {
    if head.len() < 24 || &head[..8] != b"\x89PNG\r\n\x1a\n" || &head[12..16] != b"IHDR" {
        return None;
    }
    let be = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    Some((be(&head[16..20]), be(&head[20..24])))
}

/// A transmission's pixels as RGBA.
fn decode(format: u32, compression: u8, w: u32, h: u32, bytes: &[u8]) -> Result<Vec<u8>, String> {
    let inflated;
    let bytes = if compression == b'z' {
        let mut out = Vec::new();
        flate2::read::ZlibDecoder::new(bytes)
            .take(QUOTA as u64)
            .read_to_end(&mut out)
            .map_err(|e| format!("EINVAL:{e}"))?;
        inflated = out;
        &inflated[..]
    } else if compression == 0 {
        bytes
    } else {
        return Err("EINVAL:unknown compression".into());
    };
    let n = w as usize * h as usize;
    match format {
        32 => {
            if bytes.len() < n * 4 {
                return Err("ENODATA:fewer bytes than the image's size".into());
            }
            Ok(bytes[..n * 4].to_vec())
        }
        24 => {
            if bytes.len() < n * 3 {
                return Err("ENODATA:fewer bytes than the image's size".into());
            }
            let mut out = Vec::with_capacity(n * 4);
            for [r, g, b] in bytes[..n * 3].as_chunks::<3>().0 {
                out.extend_from_slice(&[*r, *g, *b, 255]);
            }
            Ok(out)
        }
        100 => {
            let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
                .map_err(|e| format!("EINVAL:{e}"))?;
            Ok(img.to_rgba8().into_raw())
        }
        _ => Err("EINVAL:unknown format".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_parse_as_the_spec_spells_them() {
        let k = Keys::parse("a=T,f=100,i=7,c=4,r=2,z=-1,C=1,q=2").unwrap();
        assert_eq!(
            (
                k.action, k.format, k.id, k.cols, k.rows, k.z, k.stay, k.quiet
            ),
            (b'T', 100, 7, 4, 2, -1, 1, 2)
        );
        assert_eq!(Keys::parse("").unwrap().action, b't', "transmit by default");
        assert!(Keys::parse("i=x").is_err());
    }

    #[test]
    fn a_png_is_sized_by_its_header() {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(3, 2, image::Rgba([1, 2, 3, 4]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        assert_eq!(png_size(&png), Some((3, 2)));
        assert_eq!(
            decode(100, 0, 3, 2, &png).unwrap()[..4],
            [1, 2, 3, 4],
            "RGBA out"
        );
        assert_eq!(decode(24, 0, 1, 1, &[9, 8, 7]).unwrap(), vec![9, 8, 7, 255]);
        assert!(
            decode(32, 0, 2, 2, &[0; 8])
                .unwrap_err()
                .starts_with("ENODATA")
        );
    }

    #[test]
    fn a_drawn_size_follows_the_cells_asked_for() {
        let cell = (10, 20);
        assert_eq!(drawn(Fill::Natural, (30, 40), cell), (30, 40));
        assert_eq!(drawn(Fill::Cells(2, 3), (30, 40), cell), (20, 60));
        assert_eq!(drawn(Fill::Cols(3), (30, 40), cell), (30, 40));
        assert_eq!(drawn(Fill::Rows(1), (30, 40), cell), (15, 20));
    }
}
