//! Clay-inspired immediate-mode layout (docs/design/mvp.md, Decision 2).
//!
//! The element tree is plain data rebuilt on dirty frames; `layout` solves it
//! against a viewport and returns a flat command list. Two leaf kinds: `Text`
//! (measured through the [`Measure`] trait the shell provides) and `Custom`
//! (the editor and terminal views — they receive their solved rect and draw
//! themselves). No SDL, no cosmic-text, no floating elements: panes and
//! strips only, which is what keeps this crate small.
//!
//! Sizing model, same two-pass shape as clay:
//! 1. **Fit** (bottom-up): every element's intrinsic size.
//! 2. **Place** (top-down): distribute the parent's content box — `Fixed`
//!    and `Percent` first, `Fit` from pass 1, leftover split across `Grow`
//!    weights — then position children along the axis and recurse.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

pub type Color = [u8; 4];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Size {
    /// Intrinsic: a text's measured size, a box's content size.
    #[default]
    Fit,
    /// A weighted share of the parent's leftover space; fills the cross axis.
    Grow(f32),
    Fixed(f32),
    /// Fraction (0..=1) of the parent's content box on that axis.
    Percent(f32),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Dir {
    #[default]
    Row,
    Col,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Edges {
    pub l: f32,
    pub r: f32,
    pub t: f32,
    pub b: f32,
}

impl Edges {
    pub fn all(v: f32) -> Self {
        Self { l: v, r: v, t: v, b: v }
    }

    pub fn xy(x: f32, y: f32) -> Self {
        Self { l: x, r: x, t: y, b: y }
    }
}

#[derive(Clone, Debug, Default)]
pub struct BoxStyle {
    pub dir: Dir,
    pub width: Size,
    pub height: Size,
    pub padding: Edges,
    pub gap: f32,
    pub bg: Option<Color>,
    /// Clip children to this box's rect (scrolling containers, panes).
    pub clip: bool,
}

#[derive(Clone, Debug)]
pub enum Element {
    Box {
        style: BoxStyle,
        children: Vec<Element>,
    },
    Text {
        text: String,
        color: Color,
    },
    Custom {
        id: u64,
        width: Size,
        height: Size,
    },
}

impl Element {
    pub fn row(children: Vec<Element>) -> Self {
        Self::Box {
            style: BoxStyle { dir: Dir::Row, ..Default::default() },
            children,
        }
    }

    pub fn col(children: Vec<Element>) -> Self {
        Self::Box {
            style: BoxStyle { dir: Dir::Col, ..Default::default() },
            children,
        }
    }

    pub fn text(text: impl Into<String>, color: Color) -> Self {
        Self::Text { text: text.into(), color }
    }

    pub fn custom(id: u64, width: Size, height: Size) -> Self {
        Self::Custom { id, width, height }
    }

    /// An empty grow box: pushes siblings apart.
    pub fn spacer() -> Self {
        Self::Box {
            style: BoxStyle { width: Size::Grow(1.0), height: Size::Grow(1.0), ..Default::default() },
            children: Vec::new(),
        }
    }

    // -- builder-style style tweaks (only meaningful on Box) ---------------

    pub fn width(mut self, size: Size) -> Self {
        if let Self::Box { style, .. } = &mut self {
            style.width = size;
        }
        self
    }

    pub fn height(mut self, size: Size) -> Self {
        if let Self::Box { style, .. } = &mut self {
            style.height = size;
        }
        self
    }

    pub fn padding(mut self, edges: Edges) -> Self {
        if let Self::Box { style, .. } = &mut self {
            style.padding = edges;
        }
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        if let Self::Box { style, .. } = &mut self {
            style.gap = gap;
        }
        self
    }

    pub fn bg(mut self, color: Color) -> Self {
        if let Self::Box { style, .. } = &mut self {
            style.bg = Some(color);
        }
        self
    }

    pub fn clip(mut self) -> Self {
        if let Self::Box { style, .. } = &mut self {
            style.clip = true;
        }
        self
    }
}

/// Text measurement, provided by the shell's text engine.
pub trait Measure {
    /// Width and height of one run of text, unwrapped.
    fn text_size(&mut self, text: &str) -> (f32, f32);
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command<'a> {
    Rect { rect: Rect, color: Color },
    Text { x: f32, y: f32, text: &'a str, color: Color },
    Custom { id: u64, rect: Rect },
    PushClip(Rect),
    PopClip,
}

/// Solve `root` against `viewport` and emit paint-ordered commands.
pub fn layout<'a>(
    root: &'a Element,
    viewport: Rect,
    measure: &mut dyn Measure,
) -> Vec<Command<'a>> {
    let mut out = Vec::new();
    place(root, viewport, measure, &mut out);
    out
}

/// Intrinsic (fit) size of an element, bottom-up.
fn fit_size(el: &Element, measure: &mut dyn Measure) -> (f32, f32) {
    match el {
        Element::Text { text, .. } => measure.text_size(text),
        Element::Custom { width, height, .. } => {
            let solve = |s: &Size| match s {
                Size::Fixed(v) => *v,
                _ => 0.0,
            };
            (solve(width), solve(height))
        }
        Element::Box { style, children } => {
            let mut main = 0.0f32;
            let mut cross = 0.0f32;
            for child in children {
                let (cw, ch) = fit_size(child, measure);
                let (cmain, ccross) = axis(style.dir, cw, ch);
                main += cmain;
                cross = cross.max(ccross);
            }
            if children.len() > 1 {
                main += style.gap * (children.len() - 1) as f32;
            }

            let (mut w, mut h) = unaxis(style.dir, main, cross);
            w += style.padding.l + style.padding.r;
            h += style.padding.t + style.padding.b;

            if let Size::Fixed(v) = style.width {
                w = v;
            }
            if let Size::Fixed(v) = style.height {
                h = v;
            }
            (w, h)
        }
    }
}

fn el_sizes(el: &Element) -> (Size, Size) {
    match el {
        Element::Box { style, .. } => (style.width, style.height),
        Element::Text { .. } => (Size::Fit, Size::Fit),
        Element::Custom { width, height, .. } => (*width, *height),
    }
}

/// Project (w, h) onto (main, cross) for a direction.
fn axis(dir: Dir, w: f32, h: f32) -> (f32, f32) {
    match dir {
        Dir::Row => (w, h),
        Dir::Col => (h, w),
    }
}

fn unaxis(dir: Dir, main: f32, cross: f32) -> (f32, f32) {
    match dir {
        Dir::Row => (main, cross),
        Dir::Col => (cross, main),
    }
}

fn place<'a>(
    el: &'a Element,
    rect: Rect,
    measure: &mut dyn Measure,
    out: &mut Vec<Command<'a>>,
) {
    match el {
        Element::Text { text, color } => {
            out.push(Command::Text { x: rect.x, y: rect.y, text, color: *color });
        }
        Element::Custom { id, .. } => {
            out.push(Command::Custom { id: *id, rect });
        }
        Element::Box { style, children } => {
            if let Some(color) = style.bg {
                out.push(Command::Rect { rect, color });
            }
            if style.clip {
                out.push(Command::PushClip(rect));
            }

            let content = Rect {
                x: rect.x + style.padding.l,
                y: rect.y + style.padding.t,
                w: (rect.w - style.padding.l - style.padding.r).max(0.0),
                h: (rect.h - style.padding.t - style.padding.b).max(0.0),
            };
            let (content_main, content_cross) = axis(style.dir, content.w, content.h);

            // Resolve main-axis sizes: everything but Grow first.
            let mut sizes = vec![0.0f32; children.len()];
            let mut grow_total = 0.0f32;
            let mut used = if children.len() > 1 {
                style.gap * (children.len() - 1) as f32
            } else {
                0.0
            };

            for (i, child) in children.iter().enumerate() {
                let (sw, sh) = el_sizes(child);
                let (smain, _) = match style.dir {
                    Dir::Row => (sw, sh),
                    Dir::Col => (sh, sw),
                };
                match smain {
                    Size::Fixed(v) => {
                        sizes[i] = v;
                        used += v;
                    }
                    Size::Percent(p) => {
                        sizes[i] = p * content_main;
                        used += sizes[i];
                    }
                    Size::Fit => {
                        let (fw, fh) = fit_size(child, measure);
                        let (fmain, _) = axis(style.dir, fw, fh);
                        sizes[i] = fmain;
                        used += fmain;
                    }
                    Size::Grow(weight) => grow_total += weight,
                }
            }

            let leftover = (content_main - used).max(0.0);
            for (i, child) in children.iter().enumerate() {
                let (sw, sh) = el_sizes(child);
                let (smain, _) = match style.dir {
                    Dir::Row => (sw, sh),
                    Dir::Col => (sh, sw),
                };
                if let Size::Grow(weight) = smain {
                    sizes[i] = if grow_total > 0.0 {
                        leftover * weight / grow_total
                    } else {
                        0.0
                    };
                }
            }

            // Cross-axis size + position, then recurse.
            let mut cursor = 0.0f32;
            for (i, child) in children.iter().enumerate() {
                let (sw, sh) = el_sizes(child);
                let (_, scross) = match style.dir {
                    Dir::Row => (sw, sh),
                    Dir::Col => (sh, sw),
                };
                let cross_size = match scross {
                    Size::Fixed(v) => v,
                    Size::Percent(p) => p * content_cross,
                    Size::Grow(_) => content_cross,
                    Size::Fit => {
                        let (fw, fh) = fit_size(child, measure);
                        let (_, fcross) = axis(style.dir, fw, fh);
                        fcross.min(content_cross)
                    }
                };

                let (cw, ch) = unaxis(style.dir, sizes[i], cross_size);
                let (cx, cy) = match style.dir {
                    Dir::Row => (content.x + cursor, content.y),
                    Dir::Col => (content.x, content.y + cursor),
                };

                place(child, Rect::new(cx, cy, cw, ch), measure, out);
                cursor += sizes[i] + style.gap;
            }

            if style.clip {
                out.push(Command::PopClip);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 8px per char, 16px tall: predictable arithmetic for golden tests.
    struct Mono;

    impl Measure for Mono {
        fn text_size(&mut self, text: &str) -> (f32, f32) {
            (text.chars().count() as f32 * 8.0, 16.0)
        }
    }

    fn rects(commands: &[Command]) -> Vec<(u64, Rect)> {
        commands
            .iter()
            .filter_map(|c| match c {
                Command::Custom { id, rect } => Some((*id, *rect)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn shell_frame_fills_viewport() {
        // Title bar, growing editor, statusline: the milestone-3 frame.
        let root = Element::col(vec![
            Element::row(vec![Element::text("title", [255; 4])])
                .height(Size::Fixed(24.0))
                .bg([42, 42, 42, 255]),
            Element::custom(1, Size::Grow(1.0), Size::Grow(1.0)),
            Element::row(vec![Element::text("status", [255; 4])])
                .height(Size::Fixed(24.0)),
        ]);

        let commands = layout(&root, Rect::new(0.0, 0.0, 800.0, 600.0), &mut Mono);
        assert_eq!(rects(&commands), vec![(1, Rect::new(0.0, 24.0, 800.0, 552.0))]);
    }

    #[test]
    fn grow_split_respects_weights() {
        let root = Element::row(vec![
            Element::custom(1, Size::Grow(1.0), Size::Grow(1.0)),
            Element::custom(2, Size::Grow(3.0), Size::Grow(1.0)),
        ]);

        let commands = layout(&root, Rect::new(0.0, 0.0, 400.0, 100.0), &mut Mono);
        assert_eq!(
            rects(&commands),
            vec![
                (1, Rect::new(0.0, 0.0, 100.0, 100.0)),
                (2, Rect::new(100.0, 0.0, 300.0, 100.0)),
            ]
        );
    }

    #[test]
    fn padding_and_gap_arithmetic() {
        let root = Element::row(vec![
            Element::custom(1, Size::Fixed(50.0), Size::Grow(1.0)),
            Element::custom(2, Size::Grow(1.0), Size::Grow(1.0)),
        ])
        .padding(Edges::all(10.0))
        .gap(5.0);

        let commands = layout(&root, Rect::new(0.0, 0.0, 200.0, 100.0), &mut Mono);
        assert_eq!(
            rects(&commands),
            vec![
                (1, Rect::new(10.0, 10.0, 50.0, 80.0)),
                // 200 - 20 padding - 5 gap - 50 fixed = 125
                (2, Rect::new(65.0, 10.0, 125.0, 80.0)),
            ]
        );
    }

    #[test]
    fn percent_of_content_box() {
        let root = Element::row(vec![
            Element::custom(1, Size::Percent(0.25), Size::Grow(1.0)),
            Element::custom(2, Size::Grow(1.0), Size::Grow(1.0)),
        ]);

        let commands = layout(&root, Rect::new(0.0, 0.0, 400.0, 100.0), &mut Mono);
        assert_eq!(
            rects(&commands),
            vec![
                (1, Rect::new(0.0, 0.0, 100.0, 100.0)),
                (2, Rect::new(100.0, 0.0, 300.0, 100.0)),
            ]
        );
    }

    #[test]
    fn text_fit_sizes_its_box() {
        let root = Element::row(vec![
            Element::row(vec![Element::text("abcd", [255; 4])]).bg([1, 1, 1, 255]),
            Element::custom(1, Size::Grow(1.0), Size::Grow(1.0)),
        ]);

        let commands = layout(&root, Rect::new(0.0, 0.0, 100.0, 16.0), &mut Mono);
        // "abcd" = 32px wide, so the custom leaf starts at 32 and gets the rest.
        assert_eq!(rects(&commands), vec![(1, Rect::new(32.0, 0.0, 68.0, 16.0))]);
        assert!(commands.contains(&Command::Rect {
            rect: Rect::new(0.0, 0.0, 32.0, 16.0),
            color: [1, 1, 1, 255],
        }));
    }

    #[test]
    fn spacer_pushes_apart() {
        let root = Element::row(vec![
            Element::text("left", [255; 4]),
            Element::spacer(),
            Element::text("right", [255; 4]),
        ]);

        let commands = layout(&root, Rect::new(0.0, 0.0, 200.0, 16.0), &mut Mono);
        let texts: Vec<(f32, &str)> = commands
            .iter()
            .filter_map(|c| match c {
                Command::Text { x, text, .. } => Some((*x, *text)),
                _ => None,
            })
            .collect();
        // "left" = 32px, "right" = 40px → right begins at 200 - 40 = 160.
        assert_eq!(texts, vec![(0.0, "left"), (160.0, "right")]);
    }

    #[test]
    fn clip_wraps_children() {
        let root = Element::col(vec![Element::text("x", [255; 4])]).clip();
        let commands = layout(&root, Rect::new(0.0, 0.0, 50.0, 50.0), &mut Mono);
        assert!(matches!(commands.first(), Some(Command::PushClip(_))));
        assert!(matches!(commands.last(), Some(Command::PopClip)));
    }
}
