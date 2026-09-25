//! `kawoosh-gen`: the big files the perf work opens, made rather than
//! kept — a ten-gigabyte CSV is nothing to commit. Deterministic, so a
//! number from one run means the same thing on another machine.
//!
//!     cargo run --release -p kawoosh --bin kawoosh-gen -- csv 10G
//!     cargo run --release -p kawoosh --bin kawoosh-gen -- js 1723
//!
//! `csv SIZE [PATH]` writes rows of an id, a timestamp, a name, an email,
//! two coordinates, an amount and a short note, until SIZE (`500M`,
//! `10G`, or bytes). `js LINES [PATH]` writes LINES minified-looking
//! lines of about 50 KB each — the shape of a bundled `htmx.min.js` —
//! so the parser, the layers and the rows see what a bundle costs. The
//! default PATH is `target/gen/<kind>-<arg>.<ext>` under the workspace.

use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;

/// xorshift64*, seeded: enough randomness for shapes, and repeatable.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn pick<'a>(&mut self, from: &[&'a str]) -> &'a str {
        from[self.below(from.len() as u64) as usize]
    }
}

const FIRST: &[&str] = &[
    "Ada", "Grace", "Linus", "Ken", "Dennis", "Barbara", "Alan", "Margaret", "Edsger", "Niklaus",
    "Frances", "Bjarne", "Yukihiro", "Anders", "Guido", "Rich", "Simon", "Leslie", "Radia", "Tim",
];
const LAST: &[&str] = &[
    "Lovelace",
    "Hopper",
    "Torvalds",
    "Thompson",
    "Ritchie",
    "Liskov",
    "Turing",
    "Hamilton",
    "Dijkstra",
    "Wirth",
    "Allen",
    "Stroustrup",
    "Matsumoto",
    "Hejlsberg",
    "Rossum",
    "Hickey",
    "Peyton-Jones",
    "Lamport",
    "Perlman",
    "Berners-Lee",
];
const DOMAIN: &[&str] = &[
    "example.com",
    "mail.test",
    "corp.internal",
    "uni.edu",
    "post.io",
];
const NOTE: &[&str] = &[
    "renewed",
    "first order",
    "refund pending",
    "vip",
    "",
    "callback requested",
    "duplicate?",
    "moved",
    "paid in full",
    "on hold",
];

/// One CSV row, appended to `out`.
fn csv_row(rng: &mut Rng, id: u64, out: &mut String) {
    let ts = 1_600_000_000 + rng.below(200_000_000);
    let first = rng.pick(FIRST);
    let last = rng.pick(LAST);
    let domain = rng.pick(DOMAIN);
    let lat = rng.below(180_000_000) as f64 / 1e6 - 90.0;
    let lon = rng.below(360_000_000) as f64 / 1e6 - 180.0;
    let amount = rng.below(1_000_000) as f64 / 100.0;
    let note = rng.pick(NOTE);
    let _ = writeln!(
        out,
        "{id},{ts},{first} {last},{}.{}@{domain},{lat:.6},{lon:.6},{amount:.2},\"{note}\"",
        first.to_ascii_lowercase(),
        last.to_ascii_lowercase()
    );
}

fn gen_csv(size: u64, path: &PathBuf) -> std::io::Result<()> {
    let mut w = BufWriter::with_capacity(1 << 20, File::create(path)?);
    w.write_all(b"id,ts,name,email,lat,lon,amount,note\n")?;
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut written = 0u64;
    let mut id = 1u64;
    let mut block = String::with_capacity(1 << 20);
    let started = web_time::Instant::now();
    while written < size {
        block.clear();
        while block.len() < (1 << 20) - 256 {
            csv_row(&mut rng, id, &mut block);
            id += 1;
        }
        w.write_all(block.as_bytes())?;
        written += block.len() as u64;
    }
    w.flush()?;
    eprintln!(
        "{}: {} rows, {} bytes in {:.1} s",
        path.display(),
        id - 1,
        written,
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

/// One minified-looking line of about 50 KB: functions, objects, string
/// literals, regexes and numbers, the way a bundler leaves them.
fn js_line(rng: &mut Rng, n: usize, out: &mut String) {
    let _ = write!(out, "var m{n}=(()=>{{\"use strict\";const e={{");
    let mut k = 0;
    while out.len() % 60_000 < 50_000 {
        k += 1;
        match rng.below(6) {
            0 => {
                let _ = write!(
                    out,
                    "f{k}(t,n){{if(!t)return{{}};return n?t.a+{}:t.b}},",
                    rng.below(100)
                );
            }
            1 => {
                let _ = write!(out, "s{k}:\"{}\",", rng.pick(NOTE));
            }
            2 => {
                let _ = write!(out, "r{k}:/(?:\"([^\"]+)\"|'([^']+)'|([^\\s,:]+))/,");
            }
            3 => {
                let _ = write!(out, "n{k}:{}.{},", rng.below(1000), rng.below(100));
            }
            4 => {
                let _ = write!(
                    out,
                    "g{k}(t){{return t.split(\",\").map(e=>e.trim()).filter(e=>e.length>{})}},",
                    rng.below(5)
                );
            }
            _ => {
                let _ = write!(
                    out,
                    "o{k}:{{a:{},b:[{},{}],c:null}},",
                    rng.below(9),
                    rng.below(9),
                    rng.below(9)
                );
            }
        }
    }
    let _ = writeln!(out, "}};return e}})();");
}

fn gen_js(lines: usize, path: &PathBuf) -> std::io::Result<()> {
    let mut w = BufWriter::with_capacity(1 << 20, File::create(path)?);
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    let mut line = String::with_capacity(64 << 10);
    let mut written = 0usize;
    for n in 0..lines {
        line.clear();
        js_line(&mut rng, n, &mut line);
        w.write_all(line.as_bytes())?;
        written += line.len();
    }
    w.flush()?;
    eprintln!("{}: {lines} lines, {written} bytes", path.display());
    Ok(())
}

fn size_of(s: &str) -> Option<u64> {
    let (num, mul) = match s.chars().last()? {
        'G' | 'g' => (&s[..s.len() - 1], 1u64 << 30),
        'M' | 'm' => (&s[..s.len() - 1], 1 << 20),
        'K' | 'k' => (&s[..s.len() - 1], 1 << 10),
        _ => (s, 1),
    };
    Some((num.parse::<f64>().ok()? * mul as f64) as u64)
}

fn default_path(kind: &str, arg: &str, ext: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/gen");
    std::fs::create_dir_all(&root).ok();
    root.join(format!("{kind}-{arg}.{ext}"))
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [kind, arg, rest @ ..] if kind == "csv" => {
            let Some(size) = size_of(arg) else {
                eprintln!("csv: a size like 500M or 10G, not {arg:?}");
                std::process::exit(2);
            };
            let path = rest
                .first()
                .map(PathBuf::from)
                .unwrap_or_else(|| default_path("csv", arg, "csv"));
            gen_csv(size, &path)
        }
        [kind, arg, rest @ ..] if kind == "js" => {
            let Ok(lines) = arg.parse::<usize>() else {
                eprintln!("js: a line count, not {arg:?}");
                std::process::exit(2);
            };
            let path = rest
                .first()
                .map(PathBuf::from)
                .unwrap_or_else(|| default_path("js", arg, "js"));
            gen_js(lines, &path)
        }
        _ => {
            eprintln!("usage: kawoosh-gen csv SIZE [PATH] | js LINES [PATH]");
            std::process::exit(2);
        }
    }
}
