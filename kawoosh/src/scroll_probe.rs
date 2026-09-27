//! The editor's scrolling probe (`KAWOOSH_PROBE_SCROLL=OUT.csv`,
//! `scripts/probe-scroll.nu`): with a file open in a window, three held
//! keys, one press a frame from the file's top — `j` (a line a frame
//! once the caret reaches the bottom), `<C-d>` (half a screen) and
//! `<C-f>` (a screen) — each frame's work as kui timed it written to
//! OUT, summed up per key on stderr and in OUT's `.txt`, and the window
//! closed. Run once per `font.family` to compare faces.

use std::fmt::Write as _;
use std::path::PathBuf;

use kui_native::Ui;

use crate::app::Kawoosh;

/// The keys held, by the command each runs, and for how many frames.
const PHASES: &[(&str, &str, u32)] = &[
    ("j", "move down", 1500),
    ("<C-d>", "page half down", 400),
    ("<C-f>", "page down", 300),
];

/// Frames to let the file load and highlight before the first press,
/// and between keys.
const SETTLE: u32 = 90;

pub struct ScrollProbe {
    out: PathBuf,
    frame: u32,
    phase: usize,
    pressed: u32,
    seen: u64,
    rows: Vec<Row>,
}

struct Row {
    phase: usize,
    input: f32,
    view: f32,
    layout: f32,
    render: f32,
    work: f32,
}

impl ScrollProbe {
    pub fn from_env() -> Option<Self> {
        let out = std::env::var_os("KAWOOSH_PROBE_SCROLL")?;
        Some(Self {
            out: PathBuf::from(out),
            frame: 0,
            phase: 0,
            pressed: 0,
            seen: 0,
            rows: Vec::new(),
        })
    }

    fn summary(&self, family: &str, lines: usize) -> String {
        let mut s = format!("scroll probe: {family}, {lines} lines\n");
        for (i, (key, _, _)) in PHASES.iter().enumerate() {
            let rows: Vec<&Row> = self.rows.iter().filter(|r| r.phase == i).collect();
            if rows.is_empty() {
                let _ = writeln!(s, "{key}: none");
                continue;
            }
            let pick = |f: fn(&Row) -> f32| {
                let mut w: Vec<f32> = rows.iter().map(|r| f(r)).collect();
                w.sort_by(f32::total_cmp);
                w
            };
            let mean = |w: &[f32]| w.iter().sum::<f32>() / w.len() as f32;
            let at = |w: &[f32], q: f32| w[((w.len() - 1) as f32 * q) as usize];
            let w = pick(|r| r.work);
            let over = |ms: f32| w.iter().filter(|x| **x > ms).count();
            let _ = writeln!(
                s,
                "{key}: {} frames, work mean {:.2} ms, p50 {:.2}, p95 {:.2}, p99 {:.2}, max {:.2}, >8 ms {}, >16 ms {} \
                 (means: input {:.2}, view {:.2}, layout {:.2}, render {:.2})",
                w.len(),
                mean(&w),
                at(&w, 0.5),
                at(&w, 0.95),
                at(&w, 0.99),
                w[w.len() - 1],
                over(8.0),
                over(16.0),
                mean(&pick(|r| r.input)),
                mean(&pick(|r| r.view)),
                mean(&pick(|r| r.layout)),
                mean(&pick(|r| r.render)),
            );
        }
        s
    }
}

impl Kawoosh {
    /// A frame of the probe, if one runs: see [`ScrollProbe`].
    pub(crate) fn probe_scroll(&mut self, ui: &mut Ui<'_>) {
        let Some(mut p) = self.scroll_probe.take() else {
            return;
        };
        p.frame += 1;
        ui.request_frame();
        // The frame just drawn is the last press's (the first, `gg`'s, left out).
        let total = ui.core().stats.total;
        if p.pressed > 1 && total > p.seen {
            p.seen = total;
            if let Some(f) = ui.core().stats.iter().last() {
                p.rows.push(Row {
                    phase: p.phase,
                    input: f.input_ms,
                    view: f.view_ms,
                    layout: f.layout_ms,
                    render: f.render_ms,
                    work: f.work(),
                });
            }
        }
        let loaded = self
            .focused_view()
            .is_some_and(|v| self.ed.buffer_of(v).loading.is_none());
        if !loaded || p.frame < SETTLE {
            self.scroll_probe = Some(p);
            return;
        }
        if p.phase == PHASES.len() {
            let v = self.focused_view().unwrap();
            let lines = self.ed.buffer_of(v).line_count();
            let family = self
                .ed
                .settings
                .str("font.family")
                .unwrap_or("")
                .to_string();
            let family = if family.is_empty() {
                "(bundled)".into()
            } else {
                family
            };
            let family = format!(
                "{family} (face {:?}, the bundled {:?}, window {}x{})",
                self.face.id,
                self.bundled_font,
                ui.viewport().w,
                ui.viewport().h
            );
            let mut csv = String::from("key,work_ms,input_ms,view_ms,layout_ms,render_ms\n");
            for r in &p.rows {
                let _ = writeln!(
                    csv,
                    "{},{:.3},{:.3},{:.3},{:.3},{:.3}",
                    PHASES[r.phase].0, r.work, r.input, r.view, r.layout, r.render
                );
            }
            let summary = p.summary(&family, lines);
            let _ = std::fs::write(&p.out, csv);
            let _ = std::fs::write(p.out.with_extension("txt"), &summary);
            eprint!("{summary}");
            self.quit = true;
            return;
        }
        let (_, command, frames) = PHASES[p.phase];
        if p.pressed == 0 {
            // Each key from the file's top, the screen settled first.
            self.run_line("goto file start");
            p.pressed = 1;
            p.seen = total;
            p.rows.retain(|r| r.phase != p.phase);
        } else if p.pressed <= frames {
            self.run_line(command);
            p.pressed += 1;
        } else {
            p.phase += 1;
            p.pressed = 0;
            p.frame = SETTLE / 2;
        }
        self.scroll_probe = Some(p);
    }
}
