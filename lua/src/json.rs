//! `kawoosh.json` (docs/design/completion.md Decision 8): Lua data ⇄
//! JSON, a door for any plugin — the user's snippets are one reader of
//! it. The text is read by `kawoosh_systems::json` (comments and
//! trailing commas passed over, bounded); this is the Lua half.
//!
//! How the two meet, both ways:
//!
//! - an object is a table with string keys; an array a list, carrying
//!   [`ARRAY`]'s metatable (`kawoosh.json.array`) so it stays an array
//!   when written back however many items it holds — `[]` too;
//! - `null` inside an array is `kawoosh.json.null`, so the items keep
//!   their places; an object's `null` member is a key left out, read as
//!   nil (what a field read wants); a text that is just `null` is
//!   `kawoosh.json.null`, so a nil back means an error;
//! - written: a table with the array metatable, or with keys `1..n` and
//!   none else, is an array; one with string keys an object (its keys
//!   sorted, so the text is the same each time); an empty table with
//!   no metatable `{}`. A table with both, a list with a hole, a key
//!   that is neither, a number JSON cannot hold (`nan`, `inf`), a
//!   string that is not UTF-8, a function, nesting past
//!   [`MAX_DEPTH`] (a table holding itself) — each an error naming
//!   where (`$.a[2]`).
//!
//! Errors come back as `nil, why`, never raised.

use mlua::{LightUserData, Lua, Table, Value as LV};
use serde_json::Value as J;

/// How deep a written value nests at most: as deep as a read one may.
pub const MAX_DEPTH: usize = 128;

/// The registry's name for the arrays' metatable.
const ARRAY: &str = "kawoosh.json.array";

/// `kawoosh.json.null`: a light userdata holding no address.
pub fn null() -> LV {
    LV::LightUserData(LightUserData(std::ptr::null_mut()))
}

fn is_null(v: &LV) -> bool {
    matches!(v, LV::LightUserData(p) if p.0.is_null())
}

/// The arrays' metatable, made once per state.
pub fn array_mt(lua: &Lua) -> mlua::Result<Table> {
    if let Ok(t) = lua.named_registry_value::<Table>(ARRAY) {
        return Ok(t);
    }
    let t = lua.create_table()?;
    t.set("__name", ARRAY)?;
    lua.set_named_registry_value(ARRAY, &t)?;
    Ok(t)
}

/// A JSON value as Lua data, by the rules above.
pub fn to_lua(lua: &Lua, v: &J) -> mlua::Result<LV> {
    let mt = array_mt(lua)?;
    value(lua, &mt, v)
}

fn value(lua: &Lua, mt: &Table, v: &J) -> mlua::Result<LV> {
    Ok(match v {
        J::Null => null(),
        J::Bool(b) => LV::Boolean(*b),
        J::Number(n) => match n.as_i64() {
            Some(i) => LV::Integer(i),
            None => LV::Number(n.as_f64().unwrap_or(f64::NAN)),
        },
        J::String(s) => LV::String(lua.create_string(s)?),
        J::Array(l) => {
            let t = lua.create_table_with_capacity(l.len(), 0)?;
            for (i, v) in l.iter().enumerate() {
                t.raw_set(i + 1, value(lua, mt, v)?)?;
            }
            t.set_metatable(Some(mt.clone()))?;
            LV::Table(t)
        }
        J::Object(m) => {
            let t = lua.create_table_with_capacity(0, m.len())?;
            for (k, v) in m {
                if !v.is_null() {
                    t.raw_set(k.as_str(), value(lua, mt, v)?)?;
                }
            }
            LV::Table(t)
        }
    })
}

/// Where in the value being written an error is: `$.a[2]`.
#[derive(Default)]
struct At(Vec<Step>);

enum Step {
    Key(String),
    Index(usize),
}

impl At {
    fn err(&self, why: impl std::fmt::Display) -> String {
        let mut s = String::from("$");
        for st in &self.0 {
            match st {
                Step::Key(k)
                    if k.chars().all(|c| c.is_alphanumeric() || c == '_') && !k.is_empty() =>
                {
                    s.push('.');
                    s.push_str(k);
                }
                Step::Key(k) => s.push_str(&format!("[{k:?}]")),
                Step::Index(i) => s.push_str(&format!("[{i}]")),
            }
        }
        format!("{s}: {why}")
    }
}

/// Lua data as a JSON value, by the rules above; why not, naming where.
pub fn from_lua(lua: &Lua, v: &LV) -> Result<J, String> {
    let mt = array_mt(lua).map_err(|e| e.to_string())?;
    let mut at = At::default();
    write(&mt, v, &mut at)
}

fn write(mt: &Table, v: &LV, at: &mut At) -> Result<J, String> {
    if at.0.len() > MAX_DEPTH {
        return Err(at.err(format!(
            "nested deeper than {MAX_DEPTH} (a table holding itself?)"
        )));
    }
    Ok(match v {
        LV::Nil => J::Null,
        v if is_null(v) => J::Null,
        LV::Boolean(b) => J::Bool(*b),
        LV::Integer(i) => J::from(*i),
        LV::Number(n) => match serde_json::Number::from_f64(*n) {
            Some(n) => J::Number(n),
            None => return Err(at.err(format!("{n} is no JSON number"))),
        },
        LV::String(s) => match s.to_str() {
            Ok(s) => J::String(s.to_string()),
            Err(_) => return Err(at.err("a string that is not UTF-8")),
        },
        LV::Table(t) => table(mt, t, at)?,
        other => return Err(at.err(format!("a {} is not JSON", other.type_name()))),
    })
}

fn table(mt: &Table, t: &Table, at: &mut At) -> Result<J, String> {
    let marked = t
        .metatable()
        .is_some_and(|m| m.to_pointer() == mt.to_pointer());
    let mut ints: Vec<i64> = Vec::new();
    let mut named: Vec<(String, LV)> = Vec::new();
    for pair in t.clone().pairs::<LV, LV>() {
        let (k, v) = pair.map_err(|e| at.err(e))?;
        match k {
            LV::Integer(i) => ints.push(i),
            LV::Number(n) if n.fract() == 0.0 && n.abs() < 9.0e15 => ints.push(n as i64),
            LV::String(s) => match s.to_str() {
                Ok(k) => named.push((k.to_string(), v)),
                Err(_) => return Err(at.err("a key that is not UTF-8")),
            },
            other => {
                return Err(at.err(format!(
                    "a {} key: an object's keys are strings",
                    other.type_name()
                )));
            }
        }
    }
    if !ints.is_empty() && !named.is_empty() {
        return Err(at.err(format!(
            "a table with both list items and named keys ({:?})",
            named[0].0
        )));
    }
    if marked || !ints.is_empty() {
        if !named.is_empty() {
            return Err(at.err(format!("an array with a named key ({:?})", named[0].0)));
        }
        ints.sort_unstable();
        if let Some((i, k)) = ints.iter().enumerate().find(|(i, k)| **k != *i as i64 + 1) {
            return Err(at.err(format!(
                "a list with a hole at {} (kawoosh.json.null holds a place)",
                if *k < 1 { *k } else { i as i64 + 1 }
            )));
        }
        let mut out = Vec::with_capacity(ints.len());
        for i in 1..=ints.len() {
            at.0.push(Step::Index(i));
            let v = t.raw_get::<LV>(i).map_err(|e| at.err(e))?;
            out.push(write(mt, &v, at)?);
            at.0.pop();
        }
        return Ok(J::Array(out));
    }
    named.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let mut out = serde_json::Map::new();
    for (k, v) in named {
        at.0.push(Step::Key(k));
        let j = write(mt, &v, at)?;
        let Some(Step::Key(k)) = at.0.pop() else {
            unreachable!()
        };
        out.insert(k, j);
    }
    Ok(J::Object(out))
}

/// `kawoosh.json` on `k`: `decode`, `encode`, `null`, `array`.
pub fn install(lua: &Lua, k: &Table) -> mlua::Result<()> {
    let json = lua.create_table()?;
    // `kawoosh.json.decode(text)`: the value, or `nil, why` — a text
    // that is not a string as much as one that is not JSON.
    json.set(
        "decode",
        lua.create_function(|lua, text: LV| {
            let fail = |why: String| Ok((LV::Nil, LV::String(lua.create_string(why)?)));
            let LV::String(s) = text else {
                return fail(format!("json.decode: a string, not a {}", text.type_name()));
            };
            let bytes = s.as_bytes();
            let Ok(text) = std::str::from_utf8(&bytes) else {
                return fail("json.decode: the text is not UTF-8".into());
            };
            match kawoosh_systems::json::parse(text) {
                Ok(v) => Ok((to_lua(lua, &v)?, LV::Nil)),
                Err(e) => fail(e),
            }
        })?,
    )?;
    // `kawoosh.json.encode(value[, { pretty = true }])`: the text, or
    // `nil, why`.
    json.set(
        "encode",
        lua.create_function(|lua, (v, opts): (LV, Option<Table>)| {
            let pretty = opts
                .and_then(|o| o.get::<Option<bool>>("pretty").ok().flatten())
                .unwrap_or(false);
            let out = from_lua(lua, &v).and_then(|j| {
                if pretty {
                    serde_json::to_string_pretty(&j)
                } else {
                    serde_json::to_string(&j)
                }
                .map_err(|e| e.to_string())
            });
            Ok(match out {
                Ok(s) => (LV::String(lua.create_string(s)?), LV::Nil),
                Err(e) => (LV::Nil, LV::String(lua.create_string(e)?)),
            })
        })?,
    )?;
    json.set("null", null())?;
    json.set("array", array_mt(lua)?)?;
    k.set("json", json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> String {
        let lua = Lua::new();
        let k = lua.create_table().unwrap();
        install(&lua, &k).unwrap();
        lua.globals().set("kawoosh", k).unwrap();
        lua.load(src).eval::<String>().unwrap()
    }

    #[test]
    fn a_value_goes_round() {
        let src = r#"
            local j = kawoosh.json
            local text = '{"a":[1,2.5,null,"x"],"b":{"c":true,"d":null},"e":[],"f":{}}'
            local v = assert(j.decode(text))
            assert(v.a[3] == j.null and #v.a == 4 and v.b.d == nil)
            assert(getmetatable(v.e) == j.array and next(v.f) == nil)
            return j.encode(v)
        "#;
        assert_eq!(
            run(src),
            r#"{"a":[1,2.5,null,"x"],"b":{"c":true},"e":[],"f":{}}"#
        );
    }

    #[test]
    fn what_is_not_json_is_nil_and_why() {
        let src = r#"
            local j = kawoosh.json
            local out = {}
            local function say(v, e) out[#out + 1] = tostring(v) .. " " .. tostring(e) end
            say(j.decode("{"))
            say(j.decode(nil))
            say(j.encode({ 1, x = 2 }))
            say(j.encode({ [1] = 1, [3] = 3 }))
            say(j.encode({ a = { b = function() end } }))
            say(j.encode(0/0))
            say(j.encode({ [true] = 1 }))
            local loop = {}; loop.self = loop
            say(j.encode(loop))
            return table.concat(out, "\n")
        "#;
        let out = run(src);
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines.iter().all(|l| l.starts_with("nil ")), "{out}");
        assert!(lines[2].contains("both list items and named keys"), "{out}");
        assert!(lines[3].contains("hole at 2"), "{out}");
        assert!(lines[4].contains("$.a.b: a function is not JSON"), "{out}");
        assert!(lines[5].contains("no JSON number"), "{out}");
        assert!(lines[6].contains("boolean key"), "{out}");
        assert!(lines[7].contains("nested deeper than 128"), "{out}");
    }

    #[test]
    fn null_alone_is_the_null_and_an_empty_table_an_object() {
        let src = r#"
            local j = kawoosh.json
            assert(j.decode("null") == j.null)
            assert(j.encode({}) == "{}")
            assert(j.encode(setmetatable({}, j.array)) == "[]")
            return j.encode({ j.null, "\u{e9}" }, { pretty = false })
        "#;
        assert_eq!(run(src), "[null,\"é\"]");
    }

    /// The worst a read at the value cap costs the frame: every value a
    /// table. Run by hand:
    /// `cargo test -p kawoosh-lua --lib timing_the_cap -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn timing_the_cap() {
        let n = kawoosh_systems::json::MAX_NODES - 1;
        let text = format!("[{}]", vec!["[]"; n].join(","));
        let v = kawoosh_systems::json::parse(&text).unwrap();
        let lua = Lua::new();
        let t = std::time::Instant::now();
        to_lua(&lua, &v).unwrap();
        println!("{n} tables made in {:?}", t.elapsed());
    }
}
