//! The user's snippets (docs/design/completion.md Decision 8): what
//! `kawoosh.snippets.add` and the loader (`lua/snippets.lua`) put here,
//! by language, and what completion's `snippets` source offers of it.
//!
//! A snippet is VS Code's JSON shape as Lua — `["name"] = { prefix =
//! string | list, body = string | list, description = string | list,
//! scope = "a,b" }` — its body the LSP grammar round 3 expands
//! (`kawoosh_systems::snippet`). One candidate per prefix: the prefix
//! its label, its plain text (each field its default) what it puts and
//! is ranked by, the body expanded where it is accepted.
//!
//! Each snippet has an origin: `""` for `kawoosh.snippets.add` (a name
//! added again replaces the one there), a file's for the loader (a file
//! read again replaces all it put). Languages are kept as written,
//! lowercased; a buffer's language reads its own, its aliases' and the
//! VS Code ids that mean it (`shellscript` for `bash`), and `all`'s.
//!
//! A language's files are read the first time a buffer of it asks
//! ([`Store::want`]): completion marks it, and the frame tells the
//! loader (`kawoosh._snippets_want`).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use mlua::{Lua, Table, Value as LV};

use crate::completion::Item;

/// Every language's snippets, `all` for each.
pub const ALL: &str = "all";

/// VS Code's language ids that are kawoosh's under another name:
/// friendly-snippets' and a user's files name these.
const VSCODE: &[(&str, &str)] = &[
    ("shellscript", "bash"),
    ("typescriptreact", "tsx"),
    ("javascriptreact", "javascript"),
    ("plaintext", "text"),
];

/// One snippet as it was given.
#[derive(Debug)]
struct Def {
    origin: Rc<str>,
    name: String,
    prefixes: Vec<String>,
    body: Rc<str>,
    /// The body's plain text: what it puts before it is expanded.
    plain: String,
    description: Option<String>,
}

/// A snippet read from a Lua table, and the languages its `scope`
/// names in place of the one it was added under.
struct Read {
    def: Def,
    scope: Option<Vec<String>>,
}

#[derive(Default)]
pub struct Store {
    langs: HashMap<String, Vec<Def>>,
    /// Moves with every change: completion offers again when it does.
    pub generation: u64,
    /// The candidates of a buffer's language (its own and `all`'s), as
    /// built at `generation`.
    cache: HashMap<String, (u64, Rc<[Item]>)>,
    /// The languages asked for, and those not told to the loader yet.
    wanted: HashSet<String>,
    pending: Vec<String>,
}

pub type Shared = Rc<RefCell<Store>>;

impl Store {
    /// `read` put under `lang` from `origin`; with `replace`, everything
    /// `origin` put before taken out first.
    fn put(&mut self, origin: &str, lang: &str, read: Vec<Read>, replace: bool) {
        if replace {
            for defs in self.langs.values_mut() {
                defs.retain(|d| &*d.origin != origin);
            }
        }
        let lang = lang.to_lowercase();
        for Read { def, scope } in read {
            for l in scope.unwrap_or_else(|| vec![lang.clone()]) {
                let defs = self.langs.entry(l).or_default();
                if origin.is_empty() {
                    defs.retain(|d| !(d.origin.is_empty() && d.name == def.name));
                }
                defs.push(clone_def(&def));
            }
        }
        self.langs.retain(|_, d| !d.is_empty());
        self.generation += 1;
        self.cache.clear();
    }

    /// Everything forgotten for a new Lua runtime, whose `init.lua`
    /// and loader put their own: no snippet, no language asked for —
    /// the generation moved on, so a live session offers again.
    pub fn reset(&mut self) {
        *self = Store {
            generation: self.generation + 1,
            ..Store::default()
        };
    }

    /// Whether `lang` is asked for the first time: the loader is told
    /// at the next frame.
    pub fn want(&mut self, lang: &str) {
        for l in [lang, ALL] {
            if self.wanted.insert(l.to_string()) {
                self.pending.push(l.to_string());
            }
        }
    }

    /// The languages asked for since the last call.
    pub fn take_pending(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending)
    }

    /// The candidates for a buffer of `lang`, whose snippets are under
    /// `names` (`names_for`) and `all`; shared, built once a change.
    pub fn items(&mut self, lang: &str, names: &[String]) -> Rc<[Item]> {
        if let Some((g, items)) = self.cache.get(lang)
            && *g == self.generation
        {
            return items.clone();
        }
        let mut out = Vec::new();
        let mut seen: HashSet<&str> = HashSet::new();
        for name in names.iter().map(String::as_str).chain([ALL]) {
            if !seen.insert(name) {
                continue;
            }
            for d in self.langs.get(name).into_iter().flatten() {
                for p in &d.prefixes {
                    out.push(Item {
                        label: p.clone(),
                        insert: Some(d.plain.clone()),
                        kind: "snippet",
                        detail: Some(d.description.clone().unwrap_or_else(|| d.name.clone())),
                        documentation: Some(d.plain.clone()),
                        lines: Box::new([]),
                        resolve: None,
                        filled: RefCell::new(None),
                        snippet: Some(d.body.clone()),
                        as_is: false,
                    });
                }
            }
        }
        let items: Rc<[Item]> = out.into();
        self.cache
            .insert(lang.to_string(), (self.generation, items.clone()));
        items
    }

    /// The names of `lang`'s snippets (as written, lowercased), in the
    /// order they were put: a test's look.
    pub fn names(&self, lang: &str) -> Vec<String> {
        self.langs
            .get(&lang.to_lowercase())
            .map(|d| d.iter().map(|d| d.name.clone()).collect())
            .unwrap_or_default()
    }
}

fn clone_def(d: &Def) -> Def {
    Def {
        origin: d.origin.clone(),
        name: d.name.clone(),
        prefixes: d.prefixes.clone(),
        body: d.body.clone(),
        plain: d.plain.clone(),
        description: d.description.clone(),
    }
}

/// The names a buffer of `lang` finds its snippets under: its own, its
/// aliases, the VS Code ids that mean it — lowercased, its own first.
pub fn names_for(languages: &kawoosh_languages::Registry, lang: &str) -> Vec<String> {
    let mut out = vec![lang.to_lowercase()];
    if let Some(d) = languages.get(lang) {
        out.extend(d.aliases.iter().map(|a| a.to_lowercase()));
    }
    out.extend(
        VSCODE
            .iter()
            .filter(|(_, k)| *k == lang)
            .map(|(v, _)| v.to_string()),
    );
    let mut seen = HashSet::new();
    out.retain(|n| seen.insert(n.clone()));
    out
}

/// What a Lua value is, as a message says it: a number whether or not
/// it is whole.
fn kind(v: &LV) -> &'static str {
    match v {
        LV::Integer(_) | LV::Number(_) => "number",
        v => v.type_name(),
    }
}

/// A string, or a list of strings joined by `sep` — VS Code's `body`
/// and `description`; why not, else.
fn text_or_lines(v: LV, sep: &str) -> Result<Option<String>, String> {
    match v {
        LV::Nil => Ok(None),
        LV::String(s) => Ok(Some(s.to_string_lossy())),
        LV::Table(t) => {
            let mut parts = Vec::new();
            for v in t.sequence_values::<LV>() {
                match v.map_err(|e| e.to_string())? {
                    LV::String(s) => parts.push(s.to_string_lossy()),
                    other => return Err(format!("a {} among the lines", kind(&other))),
                }
            }
            Ok(Some(parts.join(sep)))
        }
        other => Err(format!(
            "a {}, not a string or a list of lines",
            kind(&other)
        )),
    }
}

/// `t`'s snippets, each as VS Code's shape has it, and why each one
/// that is not was passed over.
fn read_table(t: &Table, origin: &Rc<str>) -> (Vec<Read>, Vec<String>) {
    let mut out = Vec::new();
    let mut problems = Vec::new();
    for pair in t.pairs::<LV, LV>() {
        let Ok((k, v)) = pair else {
            continue;
        };
        let name = match &k {
            LV::String(s) => s.to_string_lossy(),
            LV::Integer(i) => i.to_string(),
            other => {
                problems.push(format!(
                    "a {} key: a snippet's name is a string",
                    kind(other)
                ));
                continue;
            }
        };
        match read_one(&name, v, origin) {
            Ok(r) => out.push(r),
            Err(why) => problems.push(format!("{name}: {why}")),
        }
    }
    // Pairs come in no order: by name, so the same file offers the same.
    out.sort_by(|a, b| a.def.name.cmp(&b.def.name));
    problems.sort();
    (out, problems)
}

fn read_one(name: &str, v: LV, origin: &Rc<str>) -> Result<Read, String> {
    let LV::Table(s) = v else {
        return Err(format!("a {}, not a table", kind(&v)));
    };
    let get = |k: &str| s.get::<LV>(k).map_err(|e| e.to_string());
    let prefixes: Vec<String> = match get("prefix")? {
        LV::String(p) => vec![p.to_string_lossy()],
        LV::Table(l) => l
            .sequence_values::<LV>()
            .filter_map(|v| match v {
                Ok(LV::String(p)) => Some(p.to_string_lossy()),
                _ => None,
            })
            .collect(),
        LV::Nil => Vec::new(),
        other => return Err(format!("prefix: a {}", kind(&other))),
    };
    let prefixes: Vec<String> = prefixes
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    if prefixes.is_empty() {
        return Err("no prefix".into());
    }
    let body = text_or_lines(get("body")?, "\n")
        .map_err(|e| format!("body: {e}"))?
        .ok_or("no body")?;
    if body.len() > kawoosh_systems::snippet::MAX_TEXT {
        return Err("its body is over 1 MiB".into());
    }
    let plain = kawoosh_systems::snippet::plain(&body).ok_or("its body expands past 1 MiB")?;
    let description = text_or_lines(get("description")?, "\n")
        .map_err(|e| format!("description: {e}"))?
        .filter(|d| !d.trim().is_empty());
    let scope = match get("scope")? {
        LV::String(s) => {
            let l: Vec<String> = s
                .to_string_lossy()
                .split(',')
                .map(|l| l.trim().to_lowercase())
                .filter(|l| !l.is_empty())
                .collect();
            (!l.is_empty()).then_some(l)
        }
        _ => None,
    };
    Ok(Read {
        def: Def {
            origin: origin.clone(),
            name: name.to_string(),
            prefixes,
            body: Rc::from(body),
            plain,
            description,
        },
        scope,
    })
}

/// `kawoosh.snippets`'s Rust half: `_add` and `_names` (under
/// `snippets.lua`'s `add` and `names`), and the loader's `_set`.
pub(crate) fn lua_door(lua: &Lua, store: Shared) -> mlua::Result<()> {
    let k: Table = lua.globals().get("kawoosh")?;
    let snippets: Table = match k.get::<LV>("snippets")? {
        LV::Table(t) => t,
        _ => {
            let t = lua.create_table()?;
            k.set("snippets", &t)?;
            t
        }
    };
    // The count put and, when some were passed over, why each was.
    fn answer(lua: &Lua, n: usize, problems: Vec<String>) -> mlua::Result<(usize, LV)> {
        if problems.is_empty() {
            return Ok((n, LV::Nil));
        }
        Ok((n, LV::Table(lua.create_sequence_from(problems)?)))
    }
    // `_add(lang, snippets)`, under `kawoosh.snippets.add`: VS Code's shape as Lua,
    // `lang` "all" for every language; a name added again replaces it.
    // Answers how many were put and, when some were not, why each.
    let s = store.clone();
    snippets.set(
        "_add",
        lua.create_function(move |lua, (lang, t): (String, Table)| {
            let origin: Rc<str> = Rc::from("");
            let (read, problems) = read_table(&t, &origin);
            let n = read.len();
            for p in &problems {
                log::warn!("snippets.add {lang}: {p}");
            }
            s.borrow_mut().put("", &lang, read, false);
            answer(lua, n, problems)
        })?,
    )?;
    // `kawoosh.snippets._set(origin, lang, snippets | nil)`: what
    // `origin` (a file the loader read) put, replaced by `snippets`.
    let s = store.clone();
    snippets.set(
        "_set",
        lua.create_function(
            move |lua, (origin, lang, t): (String, String, Option<Table>)| {
                let o: Rc<str> = Rc::from(origin.as_str());
                let (read, problems) = match &t {
                    Some(t) => read_table(t, &o),
                    None => (Vec::new(), Vec::new()),
                };
                let n = read.len();
                s.borrow_mut().put(&origin, &lang, read, true);
                answer(lua, n, problems)
            },
        )?,
    )?;
    // `_names(lang)`, under `kawoosh.snippets.names`: the names of the snippets put
    // under `lang` (as written, lowercased), in the order put.
    let s = store;
    snippets.set(
        "_names",
        lua.create_function(move |_, lang: String| Ok(s.borrow().names(&lang)))?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with(lua: &Lua, src: &str) -> Shared {
        let store: Shared = Default::default();
        lua.globals()
            .set("kawoosh", lua.create_table().unwrap())
            .unwrap();
        lua_door(lua, store.clone()).unwrap();
        lua.load(src).exec().unwrap();
        store
    }

    #[test]
    fn vs_codes_shape_is_read_and_the_bad_ones_said() {
        let lua = Lua::new();
        let store = store_with(
            &lua,
            r#"
            local n, why = kawoosh.snippets._add("Rust", {
              ["for loop"] = { prefix = { "for", "fl" }, body = { "for ${1:x} in ${2:it} {", "\t$0", "}" },
                               description = "a for loop" },
              ["no prefix"] = { body = "x" },
              ["no body"] = { prefix = "nb" },
              ["bad body"] = { prefix = "bb", body = { "a", 3 } },
              ["scoped"] = { prefix = "sc", body = "s", scope = "go, lua" },
            })
            assert(n == 2, n)
            assert(#why == 3 and why[1]:match("^bad body: body: a number"), why[1])
            "#,
        );
        let mut s = store.borrow_mut();
        assert_eq!(s.names("rust"), ["for loop"]);
        assert_eq!(s.names("go"), ["scoped"]);
        assert_eq!(s.names("lua"), ["scoped"]);
        let items = s.items("rust", &["rust".into()]);
        assert_eq!(items.len(), 2, "one per prefix");
        assert_eq!(items[0].label, "for");
        assert_eq!(items[0].text(), "for x in it {\n\t\n}");
        assert_eq!(items[0].detail.as_deref(), Some("a for loop"));
        assert_eq!(
            items[0].snippet.as_deref(),
            Some("for ${1:x} in ${2:it} {\n\t$0\n}")
        );
    }

    #[test]
    fn a_name_added_again_replaces_and_a_file_set_again_replaces_all_it_put() {
        let lua = Lua::new();
        let store = store_with(
            &lua,
            r#"
            kawoosh.snippets._add("all", { a = { prefix = "a", body = "1" } })
            kawoosh.snippets._add("all", { a = { prefix = "a", body = "2" } })
            kawoosh.snippets._set("f.json", "all", { b = { prefix = "b", body = "b" }, c = { prefix = "c", body = "c" } })
            kawoosh.snippets._set("f.json", "all", { c = { prefix = "c", body = "c" } })
            "#,
        );
        let mut s = store.borrow_mut();
        assert_eq!(s.names("all"), ["a", "c"]);
        let items = s.items("rust", &["rust".into()]);
        assert_eq!(items[0].text(), "2");
        drop(s);
        lua.load(r#"kawoosh.snippets._set("f.json", "all", nil)"#)
            .exec()
            .unwrap();
        assert_eq!(store.borrow().names("all"), ["a"]);
    }

    #[test]
    fn a_body_past_the_cap_is_passed_over() {
        let lua = Lua::new();
        let store = store_with(
            &lua,
            r#"
            local big = string.rep("x", 1100000)
            local n, why = kawoosh.snippets._add("rust", {
              big = { prefix = "big", body = big },
              doubling = { prefix = "dbl", body = "${1:" .. string.rep("y", 600000) .. "} $1" },
              broken = { prefix = "br", body = "${1:unclosed" },
            })
            assert(n == 1, n)
            assert(why[1]:match("^big: its body is over 1 MiB"), why[1])
            assert(why[2]:match("^doubling: its body expands past 1 MiB"), why[2])
            "#,
        );
        let mut s = store.borrow_mut();
        let items = s.items("rust", &["rust".into()]);
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].text(),
            "${1:unclosed",
            "a malformed body is its text"
        );
    }
}
