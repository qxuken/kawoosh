//! A size a setting or a plugin writes against the room it is given,
//! CSS's way: `720` or `"720px"` (pixels), `"80%"` (of the room), and
//! `min(…)`, `max(…)`, `clamp(MIN, TARGET, MAX)` over those, nested —
//! `"clamp(400px, 80%, 1000px)"` is 80% of the room, never under 400
//! nor over 1000. Parsed once, resolved per frame against the room.

use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum Size {
    Px(f32),
    Percent(f32),
    Min(Vec<Size>),
    Max(Vec<Size>),
    /// `clamp(min, target, max)`: the target, held between the two.
    Clamp(Box<Size>, Box<Size>, Box<Size>),
}

impl Size {
    /// A size's spelling, or what is wrong with it.
    pub fn parse(s: &str) -> Result<Size, String> {
        let mut p = Parser {
            s: s.as_bytes(),
            at: 0,
        };
        let size = p.size()?;
        p.skip_ws();
        if p.at < p.s.len() {
            return Err(p.error("the end"));
        }
        Ok(size)
    }

    /// Pixels in `room` pixels, never below zero.
    pub fn resolve(&self, room: f32) -> f32 {
        let v = match self {
            Size::Px(px) => *px,
            Size::Percent(p) => room * p / 100.0,
            Size::Min(xs) => xs
                .iter()
                .map(|x| x.resolve(room))
                .fold(f32::INFINITY, f32::min),
            Size::Max(xs) => xs
                .iter()
                .map(|x| x.resolve(room))
                .fold(f32::NEG_INFINITY, f32::max),
            // CSS's order: the minimum wins over the maximum.
            Size::Clamp(lo, target, hi) => {
                let lo = lo.resolve(room);
                target.resolve(room).min(hi.resolve(room)).max(lo)
            }
        };
        v.max(0.0)
    }
}

impl fmt::Display for Size {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let list = |f: &mut fmt::Formatter<'_>, name: &str, xs: &[&Size]| {
            write!(f, "{name}(")?;
            for (i, x) in xs.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{x}")?;
            }
            write!(f, ")")
        };
        match self {
            Size::Px(px) => write!(f, "{px}px"),
            Size::Percent(p) => write!(f, "{p}%"),
            Size::Min(xs) => list(f, "min", &xs.iter().collect::<Vec<_>>()),
            Size::Max(xs) => list(f, "max", &xs.iter().collect::<Vec<_>>()),
            Size::Clamp(a, b, c) => list(f, "clamp", &[a, b, c]),
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn skip_ws(&mut self) {
        while self.s.get(self.at).is_some_and(|c| c.is_ascii_whitespace()) {
            self.at += 1;
        }
    }

    fn error(&self, wanted: &str) -> String {
        let rest = String::from_utf8_lossy(&self.s[self.at.min(self.s.len())..]);
        match rest.is_empty() {
            true => format!("a size: {wanted} expected at the end"),
            false => format!("a size: {wanted} expected at `{rest}`"),
        }
    }

    fn eat(&mut self, word: &str) -> bool {
        self.skip_ws();
        if self.s[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            true
        } else {
            false
        }
    }

    fn size(&mut self) -> Result<Size, String> {
        self.skip_ws();
        for name in ["clamp", "min", "max"] {
            if self.s[self.at..].starts_with(name.as_bytes()) {
                self.at += name.len();
                if !self.eat("(") {
                    return Err(self.error("`(`"));
                }
                let mut args = vec![self.size()?];
                while self.eat(",") {
                    args.push(self.size()?);
                }
                if !self.eat(")") {
                    return Err(self.error("`,` or `)`"));
                }
                return match name {
                    "clamp" => match <[Size; 3]>::try_from(args) {
                        Ok([a, b, c]) => Ok(Size::Clamp(Box::new(a), Box::new(b), Box::new(c))),
                        Err(_) => Err("a size: clamp takes three: clamp(MIN, TARGET, MAX)".into()),
                    },
                    "min" => Ok(Size::Min(args)),
                    _ => Ok(Size::Max(args)),
                };
            }
        }
        let start = self.at;
        while self
            .s
            .get(self.at)
            .is_some_and(|c| c.is_ascii_digit() || *c == b'.')
        {
            self.at += 1;
        }
        let n: f32 = std::str::from_utf8(&self.s[start..self.at])
            .ok()
            .and_then(|t| t.parse().ok())
            .ok_or_else(|| {
                self.at = start;
                self.error("a number, `N%`, `Npx`, `min(…)`, `max(…)` or `clamp(…)`")
            })?;
        if self.eat("%") {
            Ok(Size::Percent(n))
        } else {
            self.eat("px");
            Ok(Size::Px(n))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Size;

    fn px(s: &str, room: f32) -> f32 {
        Size::parse(s).unwrap().resolve(room)
    }

    #[test]
    fn the_spellings_resolve_against_the_room() {
        assert_eq!(px("720", 2000.0), 720.0);
        assert_eq!(px(" 720px ", 2000.0), 720.0);
        assert_eq!(px("80%", 1000.0), 800.0);
        assert_eq!(px("min(720px, 100%)", 500.0), 500.0);
        assert_eq!(px("max(50%, 300)", 400.0), 300.0);
        let c = "clamp(400px, 80%, 1000px)";
        assert_eq!(px(c, 300.0), 400.0, "the minimum");
        assert_eq!(px(c, 1000.0), 800.0, "the target");
        assert_eq!(px(c, 2000.0), 1000.0, "the maximum");
        assert_eq!(
            px("clamp(500, 10%, 200)", 1000.0),
            500.0,
            "the minimum over the maximum"
        );
        assert_eq!(px("min(clamp(1, 50%, 900), 30%)", 1000.0), 300.0, "nested");
    }

    #[test]
    fn a_bad_one_says_where() {
        assert_eq!(
            Size::parse("80 %x").unwrap_err(),
            "a size: the end expected at `x`"
        );
        assert!(Size::parse("clamp(1, 2)").unwrap_err().contains("three"));
        assert!(Size::parse("wide").unwrap_err().contains("at `wide`"));
        assert!(Size::parse("min(1, 2").unwrap_err().contains("at the end"));
        assert_eq!(
            Size::parse("clamp(400px, 80%, 1000px)")
                .unwrap()
                .to_string(),
            "clamp(400px, 80%, 1000px)"
        );
    }
}
