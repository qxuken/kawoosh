//! A recorded byte stream fed through [`Terminal`] as the app feeds it:
//! the pty reader's 64 KiB chunks, a `screen()` after each, as a frame
//! would take one. Times the parser and the screen copy apart.
//!
//! `cargo run --release -p kawoosh-term --example feed_bench -- FILE [ROWS COLS]`
//! — record FILE with `script -q FILE sh -c 'stty rows 50 cols 180; PROGRAM' >/dev/null`.

use std::time::{Duration, Instant};

use kawoosh_term::{TermSize, Terminal};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("a recorded stream");
    let rows = args.next().map_or(50, |s| s.parse().unwrap());
    let cols = args.next().map_or(180, |s| s.parse().unwrap());
    let bytes = std::fs::read(&path).unwrap();
    let mut t = Terminal::headless(TermSize { rows, cols });
    let (mut feed, mut screen, mut screens) = (Duration::ZERO, Duration::ZERO, 0u32);
    let mut worst_feed = Duration::ZERO;
    for chunk in bytes.chunks(64 * 1024) {
        let t0 = Instant::now();
        t.feed(chunk);
        let d = t0.elapsed();
        feed += d;
        worst_feed = worst_feed.max(d);
        let t0 = Instant::now();
        std::hint::black_box(t.screen());
        screen += t0.elapsed();
        screens += 1;
    }
    let mb = bytes.len() as f64 / 1e6;
    println!(
        "{mb:.1} MB in {:.0} ms: {:.1} MB/s parsing, worst 64 KiB chunk {:.2} ms",
        feed.as_secs_f64() * 1e3,
        mb / feed.as_secs_f64(),
        worst_feed.as_secs_f64() * 1e3,
    );
    println!(
        "screen() {rows}x{cols}: {:.3} ms each ({screens} taken)",
        screen.as_secs_f64() * 1e3 / screens as f64
    );
}
