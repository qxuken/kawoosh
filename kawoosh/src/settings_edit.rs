//! A settings file's text, edited where a key is (docs/design/
//! settings.md Decision 6): what the settings pane writes. The file is
//! the user's, so a change is a splice — the value's expression
//! replaced, a field added beside its siblings, a field taken out with
//! its separator and its line — and every comment, blank line and
//! spelling the change does not touch stays as it was written.
//!
//! The table edited is the one the chunk returns: `return { … }`, or a
//! `local NAME = { … }` the chunk returns by name. Anything else is
//! code building the table, which an edit would have to run to
//! understand, so it is refused with the `return`'s line to go to.
//!
//! A key is a field `name = …` or `["name"] = …`; when one is there
//! twice, the last, since Lua keeps the last.

use kawoosh_editor::Setting;
use tree_sitter::{Node, Parser, Tree};

/// Why a file could not be edited, and the line (0-based) to show the
/// user, where there is one.
#[derive(Clone, Debug, PartialEq)]
pub struct Refused {
    pub why: String,
    pub line: Option<usize>,
}

impl Refused {
    fn new(why: impl Into<String>, line: Option<usize>) -> Self {
        Self {
            why: why.into(),
            line,
        }
    }
}

/// Where a key is in a file.
#[derive(Clone, Debug, PartialEq)]
pub struct Located {
    /// The key's own field's line, when the file sets it.
    pub exact: Option<usize>,
    /// The line of the deepest table on the key's path the file has —
    /// where a field for it would go, the returned table's at least.
    pub nearest: usize,
}

fn parse(src: &str) -> Result<Tree, Refused> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_lua::LANGUAGE.into())
        .map_err(|e| Refused::new(e.to_string(), None))?;
    let tree = parser
        .parse(src, None)
        .ok_or_else(|| Refused::new("the file did not parse", None))?;
    if tree.root_node().has_error() {
        let line = first_error(tree.root_node()).map(|n| n.start_position().row);
        return Err(Refused::new("it does not parse as Lua: fix it first", line));
    }
    Ok(tree)
}

fn first_error(n: Node<'_>) -> Option<Node<'_>> {
    if n.is_error() || n.is_missing() {
        return Some(n);
    }
    let mut c = n.walk();
    n.children(&mut c).find_map(first_error)
}

fn text<'a>(src: &'a str, n: Node<'_>) -> &'a str {
    &src[n.byte_range()]
}

/// The table the chunk returns.
fn returned<'t>(src: &str, tree: &'t Tree) -> Result<Node<'t>, Refused> {
    let root = tree.root_node();
    let mut c = root.walk();
    let children: Vec<Node<'t>> = root.named_children(&mut c).collect();
    let ret = children
        .iter()
        .rev()
        .find(|n| n.kind() == "return_statement")
        .copied()
        .ok_or_else(|| Refused::new("it returns nothing: a settings file returns a table", None))?;
    let line = Some(ret.start_position().row);
    let value = ret
        .named_child(0)
        .and_then(|list| list.named_child(0))
        .ok_or_else(|| Refused::new("it returns nothing: a settings file returns a table", line))?;
    match value.kind() {
        "table_constructor" => Ok(value),
        "identifier" => {
            let name = text(src, value);
            // The last `local NAME = { … }` before the return.
            children
                .iter()
                .filter(|n| n.kind() == "variable_declaration" && n.end_byte() <= ret.start_byte())
                .filter_map(|decl| local_table(src, *decl, name))
                .next_back()
                .ok_or_else(|| {
                    Refused::new(format!("it builds `{name}` in code: change it there"), line)
                })
        }
        _ => Err(Refused::new(
            "it builds its table in code: change it there",
            line,
        )),
    }
}

/// The table `local NAME = { … }` gives `name`, when `decl` is that.
fn local_table<'t>(src: &str, decl: Node<'t>, name: &str) -> Option<Node<'t>> {
    let assign = decl
        .named_child(0)
        .filter(|n| n.kind() == "assignment_statement")?;
    let mut c = assign.walk();
    let parts: Vec<Node<'t>> = assign.named_children(&mut c).collect();
    let vars = parts.iter().find(|n| n.kind() == "variable_list")?;
    let values = parts.iter().find(|n| n.kind() == "expression_list")?;
    let mut c = vars.walk();
    let i = vars
        .children_by_field_name("name", &mut c)
        .position(|v| text(src, v) == name)?;
    let mut c = values.walk();
    let value = values.children_by_field_name("value", &mut c).nth(i)?;
    (value.kind() == "table_constructor").then_some(value)
}

/// The key a field names: an identifier, or a string in brackets.
fn field_key<'a>(src: &'a str, field: Node<'_>) -> Option<&'a str> {
    let name = field.child_by_field_name("name")?;
    match name.kind() {
        "identifier" => Some(text(src, name)),
        "string" => {
            let content = name.child_by_field_name("content");
            Some(content.map(|c| text(src, c)).unwrap_or(""))
        }
        _ => None,
    }
}

fn fields<'t>(table: Node<'t>) -> Vec<Node<'t>> {
    let mut c = table.walk();
    table
        .named_children(&mut c)
        .filter(|n| n.kind() == "field")
        .collect()
}

/// The last field of `table` named `key`.
fn find<'t>(src: &str, table: Node<'t>, key: &str) -> Option<Node<'t>> {
    fields(table)
        .into_iter()
        .rev()
        .find(|f| field_key(src, *f) == Some(key))
}

/// The fields along `parts` from `table`, as far as the file has them,
/// each with the table it is in.
fn walk_path<'t>(src: &str, mut table: Node<'t>, parts: &[&str]) -> Vec<(Node<'t>, Node<'t>)> {
    let mut out = Vec::new();
    for (i, key) in parts.iter().enumerate() {
        let Some(f) = find(src, table, key) else {
            break;
        };
        out.push((table, f));
        let Some(v) = f.child_by_field_name("value") else {
            break;
        };
        if i + 1 < parts.len() {
            if v.kind() != "table_constructor" {
                break;
            }
            table = v;
        }
    }
    out
}

fn split(path: &str) -> Result<Vec<&str>, Refused> {
    let parts: Vec<&str> = path.split('.').collect();
    if parts.iter().any(|p| p.is_empty()) {
        return Err(Refused::new(
            format!("`{path}` is not a setting's path"),
            None,
        ));
    }
    Ok(parts)
}

/// Where `path` is in `src`.
pub fn locate(src: &str, path: &str) -> Result<Located, Refused> {
    let tree = parse(src)?;
    let root = returned(src, &tree)?;
    let parts = split(path)?;
    let found = walk_path(src, root, &parts);
    let exact = (found.len() == parts.len()).then(|| found[found.len() - 1].1.start_position().row);
    let nearest = match found.last() {
        Some((_, f)) => f.start_position().row,
        None => root.start_position().row,
    };
    Ok(Located { exact, nearest })
}

/// `src` with `path` set to `value`: the value replaced where the file
/// has the key, else a field added at the deepest table on the path it
/// has.
pub fn set(src: &str, path: &str, value: &Setting) -> Result<String, Refused> {
    let tree = parse(src)?;
    let root = returned(src, &tree)?;
    let parts = split(path)?;
    let found = walk_path(src, root, &parts);
    let unit = indent_unit(src);
    if found.len() == parts.len() {
        let field = found[found.len() - 1].1;
        let v = field
            .child_by_field_name("value")
            .expect("a named field has a value");
        let at = line_indent(src, field.start_byte());
        return Ok(splice(src, v.byte_range(), &spell_at(value, &at, &unit)));
    }
    // What the file has of the path ends at a table or at a value that
    // is not one: a table gets a field for the rest; a value is
    // replaced by a table holding the rest.
    let rest = &parts[found.len()..];
    if let Some((_, f)) = found.last() {
        let v = f.child_by_field_name("value").expect("a field has a value");
        if v.kind() != "table_constructor" {
            let at = line_indent(src, f.start_byte());
            return Ok(splice(
                src,
                v.byte_range(),
                &spell_at(&nest(rest, value), &at, &unit),
            ));
        }
        return Ok(insert(src, v, rest, value, &unit, false));
    }
    Ok(insert(src, root, rest, value, &unit, true))
}

/// `src` with `path` taken out, and a table it leaves empty with it;
/// `None` when the file does not set it.
pub fn remove(src: &str, path: &str) -> Result<Option<String>, Refused> {
    let tree = parse(src)?;
    let root = returned(src, &tree)?;
    let parts = split(path)?;
    let found = walk_path(src, root, &parts);
    if found.len() < parts.len() {
        return Ok(None);
    }
    // The field to take out: the key's, or the one holding the table
    // it would leave empty, as far up as that goes — never the
    // returned table itself.
    let mut at = found.len() - 1;
    while at > 0 {
        let (table, _) = found[at];
        if fields(table).len() == 1 && !has_comment(table) {
            at -= 1;
        } else {
            break;
        }
    }
    let (table, field) = found[at];
    Ok(Some(take_out(src, table, field)))
}

fn has_comment(table: Node<'_>) -> bool {
    let mut c = table.walk();
    table.named_children(&mut c).any(|n| n.kind() == "comment")
}

/// The separator right after `field` in its table, when there is one.
fn separator_after<'t>(field: Node<'t>) -> Option<Node<'t>> {
    field
        .next_sibling()
        .filter(|n| matches!(n.kind(), "," | ";"))
}

fn separator_before<'t>(field: Node<'t>) -> Option<Node<'t>> {
    field
        .prev_sibling()
        .filter(|n| matches!(n.kind(), "," | ";"))
}

fn take_out(src: &str, table: Node<'_>, field: Node<'_>) -> String {
    let sep = separator_after(field);
    let end = sep.map(|s| s.end_byte()).unwrap_or(field.end_byte());
    let line_start = src[..field.start_byte()]
        .rfind('\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    let alone_before = src[line_start..field.start_byte()].trim().is_empty();
    // What follows on the line: nothing, or a comment that is the
    // field's own and goes with it.
    let line_end = src[end..].find('\n').map(|i| end + i).unwrap_or(src.len());
    let after = src[end..line_end].trim();
    let alone_after = after.is_empty() || after.starts_with("--");
    let on_lines_of_its_own =
        alone_before && alone_after && table.start_position().row < field.start_position().row;
    if on_lines_of_its_own {
        // The whole lines, newline and all.
        let to = (line_end + 1).min(src.len());
        // A last field without a separator after one that has one:
        // that comma is now trailing, which Lua takes.
        return splice(src, line_start..to, "");
    }
    // Inline: the field and its separator and the spaces after it; the
    // last one takes the separator before it instead.
    if sep.is_some() {
        let mut to = end;
        while src[to..].starts_with(' ') {
            to += 1;
        }
        // `{ a = 1, }` keeps its closing spacing when `a` goes.
        return splice(src, field.start_byte()..to, "");
    }
    if let Some(before) = separator_before(field) {
        return splice(src, before.start_byte()..field.end_byte(), "");
    }
    // The only field of a one-line table: `{ a = 1 }` becomes `{}`.
    let mut from = field.start_byte();
    while from > table.start_byte() + 1 && src[..from].ends_with(' ') {
        from -= 1;
    }
    let mut to = field.end_byte();
    while to < table.end_byte() - 1 && src[to..].starts_with(' ') {
        to += 1;
    }
    splice(src, from..to, "")
}

/// A field for `rest` = `value` added to `table`, beside its siblings.
fn insert(
    src: &str,
    table: Node<'_>,
    rest: &[&str],
    value: &Setting,
    unit: &str,
    root: bool,
) -> String {
    let fs = fields(table);
    // A table across lines gets a line; one on a line stays on it — but
    // the returned table with nothing in it yet, which is the file.
    let multi = table.start_position().row < table.end_position().row || (root && fs.is_empty());
    let close = table.end_byte() - 1;
    if multi {
        let (indent, anchor) = match fs.last() {
            Some(last) => {
                let lead = line_indent(src, last.start_byte());
                let first_on_line = src[..last.start_byte()]
                    .rsplit('\n')
                    .next()
                    .is_some_and(|s| s.trim().is_empty());
                let indent = if first_on_line {
                    lead
                } else {
                    format!("{}{unit}", line_indent(src, table.start_byte()))
                };
                (indent, Some(*last))
            }
            None => (format!("{}{unit}", line_indent(src, close)), None),
        };
        let field = format!(
            "{} = {}",
            key_spelling(rest[0]),
            spell_at(&nest(&rest[1..], value), &indent, unit)
        );
        let mut out = src.to_string();
        let mut shift = 0;
        let mut after_line = match anchor {
            Some(last) => {
                let sep = separator_after(last);
                if sep.is_none() {
                    out.insert(last.end_byte(), ',');
                    shift = 1;
                }
                sep.map(|s| s.end_byte()).unwrap_or(last.end_byte())
            }
            None => table.start_byte() + 1,
        };
        // After what ends the last field's line — a comment there is
        // the field's — unless the table closes on it.
        let line_end = src[after_line..].find('\n').map(|i| after_line + i);
        match line_end {
            Some(e) if e < close => after_line = e,
            _ => {
                // The table closes on this line: the field goes on a
                // line of its own, and the `}` on the next, at the
                // table's own indentation.
                let closing = line_indent(src, table.start_byte());
                let mut from = close;
                while from > after_line && src[..from].ends_with(' ') {
                    from -= 1;
                }
                out.replace_range(
                    from + shift..close + shift,
                    &format!("\n{indent}{field},\n{closing}"),
                );
                return out;
            }
        }
        out.insert_str(after_line + shift, &format!("\n{indent}{field},"));
        return out;
    }
    let field = format!(
        "{} = {}",
        key_spelling(rest[0]),
        spell(&nest(&rest[1..], value))
    );
    match fs.last() {
        Some(last) => splice(src, last.end_byte()..last.end_byte(), &format!(", {field}")),
        None => splice(src, table.byte_range(), &format!("{{ {field} }}")),
    }
}

fn splice(src: &str, range: std::ops::Range<usize>, with: &str) -> String {
    let mut out = String::with_capacity(src.len() + with.len());
    out.push_str(&src[..range.start]);
    out.push_str(with);
    out.push_str(&src[range.end..]);
    out
}

/// The leading whitespace of the line `at` is on.
fn line_indent(src: &str, at: usize) -> String {
    let start = src[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
    src[start..]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

/// One level of the file's indentation: a tab when its lines are
/// indented with tabs, else the fewest spaces an indented line has, two
/// when none is.
fn indent_unit(src: &str) -> String {
    let mut spaces = usize::MAX;
    for line in src.lines() {
        if line.starts_with('\t') {
            return "\t".into();
        }
        let n = line.chars().take_while(|c| *c == ' ').count();
        if n > 0 && n < line.len() {
            spaces = spaces.min(n);
        }
    }
    " ".repeat(if spaces == usize::MAX { 2 } else { spaces })
}

/// `value` under the keys of `rest`: `{ a = { b = value } }`.
fn nest(rest: &[&str], value: &Setting) -> Setting {
    rest.iter().rev().fold(value.clone(), |v, k| {
        let mut t = std::collections::BTreeMap::new();
        t.insert(k.to_string(), v);
        Setting::Table(t)
    })
}

const KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

fn is_identifier(k: &str) -> bool {
    let mut cs = k.chars();
    matches!(cs.next(), Some(c) if c == '_' || c.is_ascii_alphabetic())
        && cs.all(|c| c == '_' || c.is_ascii_alphanumeric())
        && !KEYWORDS.contains(&k)
}

fn key_spelling(k: &str) -> String {
    if is_identifier(k) {
        k.to_string()
    } else {
        format!("[{}]", quote(k))
    }
}

/// A string in double quotes, escaped as Lua reads it.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => out.push_str(&format!("\\{:03}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `value` as Lua spells it, on one line.
pub fn spell(value: &Setting) -> String {
    match value {
        Setting::Bool(b) => b.to_string(),
        Setting::Int(i) => i.to_string(),
        Setting::Float(x) if x.is_finite() && x.fract() == 0.0 && x.abs() < 1e15 => {
            format!("{x:.1}")
        }
        Setting::Float(x) if x.is_nan() => "0/0".into(),
        Setting::Float(x) if x.is_infinite() => {
            if *x > 0.0 { "math.huge" } else { "-math.huge" }.into()
        }
        Setting::Float(x) => x.to_string(),
        Setting::Str(s) => quote(s),
        Setting::List(l) if l.is_empty() => "{}".into(),
        Setting::List(l) => {
            let parts: Vec<String> = l.iter().map(spell).collect();
            format!("{{ {} }}", parts.join(", "))
        }
        Setting::Table(t) if t.is_empty() => "{}".into(),
        Setting::Table(t) => {
            let parts: Vec<String> = t
                .iter()
                .map(|(k, v)| format!("{} = {}", key_spelling(k), spell(v)))
                .collect();
            format!("{{ {} }}", parts.join(", "))
        }
    }
}

/// The widest a value is written on one line before it is written a
/// field to a line.
const ONE_LINE: usize = 72;

/// `value` spelled at a line indented `indent`: on one line when it
/// fits, else a table or a list a part to a line, one `unit` in.
fn spell_at(value: &Setting, indent: &str, unit: &str) -> String {
    let flat = spell(value);
    if flat.len() + indent.len() <= ONE_LINE {
        return flat;
    }
    let inner = format!("{indent}{unit}");
    let parts: Vec<String> = match value {
        Setting::List(l) => l.iter().map(|v| spell_at(v, &inner, unit)).collect(),
        Setting::Table(t) => t
            .iter()
            .map(|(k, v)| format!("{} = {}", key_spelling(k), spell_at(v, &inner, unit)))
            .collect(),
        _ => return flat,
    };
    let mut out = String::from("{\n");
    for p in parts {
        out.push_str(&format!("{inner}{p},\n"));
    }
    out.push_str(indent);
    out.push('}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int(i: i64) -> Setting {
        Setting::Int(i)
    }
    fn s(v: &str) -> Setting {
        Setting::Str(v.into())
    }

    const FILE: &str = "\
-- mine
---@type kawoosh.Settings
return {
  tabstop = 4, -- wide
  font = {
    family = \"Iosevka\",
    size = 13,
  },
  theme = { dark = \"ayu\" },
}
";

    #[test]
    fn a_value_there_is_replaced_and_nothing_else_moves() {
        let out = set(FILE, "font.size", &int(14)).unwrap();
        assert_eq!(out, FILE.replace("size = 13", "size = 14"));
        let out = set(FILE, "tabstop", &int(2)).unwrap();
        assert_eq!(
            out,
            FILE.replace("tabstop = 4, -- wide", "tabstop = 2, -- wide")
        );
    }

    #[test]
    fn a_key_not_there_goes_beside_its_siblings() {
        let out = set(FILE, "font.line_height", &Setting::Float(1.4)).unwrap();
        assert_eq!(
            out,
            FILE.replace(
                "    size = 13,\n",
                "    size = 13,\n    line_height = 1.4,\n"
            )
        );
        // A top-level key after the last field, a comma kept.
        let out = set(FILE, "scrolloff", &int(8)).unwrap();
        assert_eq!(
            out,
            FILE.replace(
                "  theme = { dark = \"ayu\" },\n",
                "  theme = { dark = \"ayu\" },\n  scrolloff = 8,\n"
            )
        );
        // Into a one-line table, on its line.
        let out = set(FILE, "theme.light", &s("paper")).unwrap();
        assert!(
            out.contains("theme = { dark = \"ayu\", light = \"paper\" },"),
            "{out}"
        );
        // A path the file has none of, nested inline.
        let out = set(FILE, "layout.scroll.center", &s("always")).unwrap();
        assert!(
            out.contains("\n  layout = { scroll = { center = \"always\" } },\n}"),
            "{out}"
        );
    }

    #[test]
    fn a_trailing_comment_stays_with_its_field() {
        let src = "return {\n  a = 1 -- one\n}\n";
        let out = set(src, "b", &int(2)).unwrap();
        assert_eq!(out, "return {\n  a = 1, -- one\n  b = 2,\n}\n");
    }

    #[test]
    fn an_empty_file_table_takes_the_field_on_a_line() {
        let stub = "---@type kawoosh.Settings\nreturn {\n}\n";
        assert_eq!(
            set(stub, "font.size", &int(14)).unwrap(),
            "---@type kawoosh.Settings\nreturn {\n  font = { size = 14 },\n}\n"
        );
        assert_eq!(
            set("return {}", "tabstop", &int(2)).unwrap(),
            "return {\n  tabstop = 2,\n}"
        );
        // A file that is one line stays one.
        assert_eq!(
            set("return { a = 1 }", "b.c", &int(2)).unwrap(),
            "return { a = 1, b = { c = 2 } }"
        );
        // A table closing on its last field's line.
        assert_eq!(
            set("return {\n  a = 1 }", "b", &int(2)).unwrap(),
            "return {\n  a = 1,\n  b = 2,\n}"
        );
    }

    #[test]
    fn tabs_and_wide_indents_are_the_files() {
        let src = "return {\n\ta = 1,\n}\n";
        assert_eq!(
            set(src, "b.c", &int(2)).unwrap(),
            "return {\n\ta = 1,\n\tb = { c = 2 },\n}\n"
        );
        let src = "return {\n    a = {\n        x = 1,\n    },\n}\n";
        assert_eq!(
            set(src, "a.y", &int(2)).unwrap(),
            "return {\n    a = {\n        x = 1,\n        y = 2,\n    },\n}\n"
        );
    }

    #[test]
    fn keys_in_brackets_and_the_last_of_two() {
        let src = "return {\n  [\"font\"] = { size = 12 },\n  tabstop = 2,\n  tabstop = 3,\n}\n";
        assert!(
            set(src, "font.size", &int(14))
                .unwrap()
                .contains("[\"font\"] = { size = 14 }")
        );
        let out = set(src, "tabstop", &int(8)).unwrap();
        assert!(out.contains("tabstop = 2,\n  tabstop = 8,"), "{out}");
        // A key that is no identifier is spelled in brackets.
        let out = set(src, "format.clang-format.cmd", &s("cf")).unwrap();
        assert!(
            out.contains("format = { [\"clang-format\"] = { cmd = \"cf\" } },"),
            "{out}"
        );
    }

    #[test]
    fn a_value_that_is_not_a_table_on_the_path_is_replaced_by_one() {
        let src = "return {\n  font = \"x\",\n}\n";
        assert_eq!(
            set(src, "font.size", &int(14)).unwrap(),
            "return {\n  font = { size = 14 },\n}\n"
        );
    }

    #[test]
    fn a_computed_value_is_replaced() {
        let src = "local base = 12\nreturn {\n  font = { size = base + 1 },\n}\n";
        assert!(
            set(src, "font.size", &int(14))
                .unwrap()
                .contains("font = { size = 14 }")
        );
    }

    #[test]
    fn a_local_returned_by_name_is_edited() {
        let src = "local s = {\n  tabstop = 2,\n}\nreturn s\n";
        assert_eq!(
            set(src, "tabstop", &int(4)).unwrap(),
            "local s = {\n  tabstop = 4,\n}\nreturn s\n"
        );
    }

    #[test]
    fn code_that_builds_the_table_is_refused_with_its_line() {
        let src = "local s = make()\nreturn s\n";
        let e = set(src, "tabstop", &int(4)).unwrap_err();
        assert_eq!(e.line, Some(1));
        assert!(e.why.contains("builds"), "{}", e.why);
        let e = set("return make()\n", "a", &int(1)).unwrap_err();
        assert_eq!(e.line, Some(0));
        let e = set("return {\n  a = ,\n}\n", "a", &int(1)).unwrap_err();
        assert!(e.why.contains("does not parse"), "{}", e.why);
        assert_eq!(e.line, Some(1));
        assert!(set("local a = 1\n", "a", &int(1)).is_err());
    }

    #[test]
    fn remove_takes_the_line_and_an_emptied_table() {
        // A line of its own, with its comment.
        let out = remove(FILE, "tabstop").unwrap().unwrap();
        assert_eq!(out, FILE.replace("  tabstop = 4, -- wide\n", ""));
        // A table with others left keeps them.
        let out = remove(FILE, "font.size").unwrap().unwrap();
        assert_eq!(out, FILE.replace("    size = 13,\n", ""));
        // A table left empty goes, a line of its own.
        let out = remove(FILE, "theme.dark").unwrap().unwrap();
        assert_eq!(out, FILE.replace("  theme = { dark = \"ayu\" },\n", ""));
        // Not there: nothing.
        assert_eq!(remove(FILE, "scrolloff").unwrap(), None);
        assert_eq!(remove(FILE, "font.size.x").unwrap(), None);
    }

    #[test]
    fn remove_inline_takes_the_separator() {
        let src = "return {\n  t = { a = 1, b = 2, c = 3 },\n}\n";
        assert!(
            remove(src, "t.a")
                .unwrap()
                .unwrap()
                .contains("t = { b = 2, c = 3 },")
        );
        assert!(
            remove(src, "t.b")
                .unwrap()
                .unwrap()
                .contains("t = { a = 1, c = 3 },")
        );
        assert!(
            remove(src, "t.c")
                .unwrap()
                .unwrap()
                .contains("t = { a = 1, b = 2 },")
        );
        // A table holding a comment stays, empty.
        let src = "return {\n  t = {\n    -- keep\n    a = 1,\n  },\n}\n";
        assert_eq!(
            remove(src, "t.a").unwrap().unwrap(),
            "return {\n  t = {\n    -- keep\n  },\n}\n"
        );
    }

    #[test]
    fn the_last_field_of_the_file_goes_and_the_table_stays() {
        let src = "return {\n  a = 1,\n}\n";
        assert_eq!(remove(src, "a").unwrap().unwrap(), "return {\n}\n");
        assert_eq!(
            remove("return { a = 1 }", "a").unwrap().unwrap(),
            "return {}"
        );
    }

    #[test]
    fn values_are_spelled_as_lua_reads_them() {
        assert_eq!(spell(&Setting::Float(1.0)), "1.0");
        assert_eq!(spell(&Setting::Float(0.25)), "0.25");
        assert_eq!(spell(&s("a\"b\\c\n")), "\"a\\\"b\\\\c\\n\"");
        assert_eq!(
            spell(&Setting::List(vec![s("text"), s("gitcommit")])),
            "{ \"text\", \"gitcommit\" }"
        );
        let mut t = std::collections::BTreeMap::new();
        t.insert("end".to_string(), int(1));
        t.insert("ok".to_string(), Setting::Bool(true));
        assert_eq!(spell(&Setting::Table(t)), "{ [\"end\"] = 1, ok = true }");
    }

    #[test]
    fn a_long_value_is_written_a_part_to_a_line() {
        let long: Vec<Setting> = (0..12).map(|i| s(&format!("module{i}"))).collect();
        let out = set("return {\n}\n", "statusline.layout", &Setting::List(long)).unwrap();
        assert!(
            out.starts_with("return {\n  statusline = {\n    layout = {\n      \"module0\",\n"),
            "{out}"
        );
        assert!(
            out.ends_with("      \"module11\",\n    },\n  },\n}\n"),
            "{out}"
        );
    }

    #[test]
    fn locate_says_the_line_and_the_nearest() {
        assert_eq!(
            locate(FILE, "font.size").unwrap(),
            Located {
                exact: Some(6),
                nearest: 6
            }
        );
        assert_eq!(
            locate(FILE, "font.features").unwrap(),
            Located {
                exact: None,
                nearest: 4
            }
        );
        assert_eq!(
            locate(FILE, "scrolloff").unwrap(),
            Located {
                exact: None,
                nearest: 2
            }
        );
    }

    /// Whatever an edit writes reads back as the value, through the
    /// same sandbox a settings file runs in.
    #[test]
    fn what_is_written_reads_back() {
        let rt = kawoosh_lua::Runtime::new().unwrap().0;
        let mut t = std::collections::BTreeMap::new();
        t.insert("x y".to_string(), s("\u{1}é"));
        t.insert("n".to_string(), Setting::Float(-2.5));
        let value = Setting::Table(t);
        let out = set(FILE, "deep.er.table", &value).unwrap();
        let read = rt.eval_settings("t", &out).unwrap();
        assert_eq!(read.get("deep.er.table"), Some(&value));
        assert_eq!(read.get("font.size"), Some(&int(13)));
        let out = remove(&out, "deep.er.table").unwrap().unwrap();
        assert_eq!(out, FILE);
    }
}
