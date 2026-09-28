//! Keys resolve to named commands through a per-mode trie (mvp.md
//! Decision 4). Notation is neovim's — `j`, `<C-d>`, `gg`, `<leader>t` —
//! because that is the muscle memory being courted. A stroke arrives as
//! kui's `code` (already layout-resolved with the US-QWERTY fallback,
//! kui.md D5) plus modifiers; text never comes through here.

use std::collections::HashMap;

use crate::command::Cond;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Mode {
    Normal,
    Insert,
    Visual,
    /// The command line and the search prompt: `:` and `/`.
    /// An operator is waiting for its motion or text object.
    OperatorPending,
    /// A pane that is not an editor's — the memory pane, the undo
    /// pane, a Lua view with no field under the keys — takes its keys
    /// in this mode, on the engine's resident pane view: the list keys
    /// (`j` `k` `gg` `G` `<C-d>` `<C-u>` `<CR>` `q`) and the pane's
    /// own, and through to normal mode for what every pane shares
    /// ([`Keymap::shared_from_pane`]).
    Pane,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Normal => "NOR",
            Mode::Insert => "INS",
            Mode::Visual => "VIS",
            Mode::OperatorPending => "OP",
            Mode::Pane => "PANE",
        }
    }

    /// The word Lua reads the mode as: `normal`, `insert`, `visual`.
    pub fn word(self) -> &'static str {
        match self {
            Mode::Normal => "normal",
            Mode::Insert => "insert",
            Mode::Visual => "visual",
            Mode::OperatorPending => "operator",
            Mode::Pane => "pane",
        }
    }

    /// The letter `kawoosh.map` and `:map` name the mode by.
    pub fn short(self) -> &'static str {
        match self {
            Mode::Normal => "n",
            Mode::Insert => "i",
            Mode::Visual => "v",
            Mode::OperatorPending => "o",
            Mode::Pane => "p",
        }
    }

    pub fn from_short(s: &str) -> Option<Self> {
        match s {
            "n" | "normal" => Some(Mode::Normal),
            "i" | "insert" => Some(Mode::Insert),
            "v" | "visual" => Some(Mode::Visual),
            "o" | "op" | "operator" => Some(Mode::OperatorPending),
            "p" | "pane" => Some(Mode::Pane),
            _ => None,
        }
    }
}

/// One key press as the keymap sees it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyStroke {
    /// kui's `code`: a character (`"j"`, `"J"`, `":"`) or a name
    /// (`"escape"`, `"enter"`, `"up"`).
    pub code: String,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub sup: bool,
    /// What the press would insert, for insert mode and prompts.
    pub text: Option<String>,
}

impl KeyStroke {
    pub fn plain(code: &str) -> Self {
        Self {
            code: code.into(),
            ctrl: false,
            alt: false,
            shift: false,
            sup: false,
            text: None,
        }
    }

    /// The stroke in map notation: `j`, `<C-d>`, `<A-c>`, `<D-s>`,
    /// `<Esc>`, `<S-Tab>`. A character carries its own shift.
    pub fn notation(&self) -> String {
        let named = named_key(&self.code);
        let base = named
            .map(str::to_string)
            .unwrap_or_else(|| self.code.clone());
        // A chord on a letter carries Shift as the upper-case letter —
        // what a map's `<A-S-j>` normalizes to (`<A-J>`), and what the
        // logical key already is under Ctrl and ⌘. Under Alt kui reports
        // the key with every modifier stripped (⌥o is `o`, not `ø`), so
        // the shift bit has to put the case back, or ⌥⇧j is `<A-j>`.
        let chord = self.ctrl || self.alt || self.sup;
        // A chord's digit keeps its Shift, spelled `<C-S-1>`: the two
        // are different keys where a letter's case says it for them,
        // and the shifted digit arrives as the symbol the layout
        // prints (`!` for 1 on most of them), which is not a spelling
        // anyone wants to bind.
        let shifted_digit = chord && self.shift && named.is_none() && digit_of(&base).is_some();
        let base = match (shifted_digit, digit_of(&base)) {
            (true, Some(d)) => d.to_string(),
            _ if chord && self.shift && named.is_none() && base.len() == 1 => {
                base.to_ascii_uppercase()
            }
            _ => base,
        };
        let mut mods = String::new();
        if self.ctrl {
            mods.push_str("C-");
        }
        if self.alt {
            mods.push_str("A-");
        }
        if self.sup {
            mods.push_str("D-");
        }
        if self.shift && (named.is_some() || shifted_digit) {
            mods.push_str("S-");
        }
        if mods.is_empty() && named.is_none() {
            base
        } else if mods.is_empty() {
            format!("<{base}>")
        } else {
            format!("<{mods}{base}>")
        }
    }
}

/// The digit a key stands for when Shift is down: the digit itself,
/// or the symbol a US layout prints above it — which is what a press
/// of ⇧1 reports on most layouts, `!`. None for anything else.
fn digit_of(base: &str) -> Option<char> {
    let mut it = base.chars();
    let (c, rest) = (it.next()?, it.next());
    if rest.is_some() {
        return None;
    }
    match c {
        '0'..='9' => Some(c),
        ')' => Some('0'),
        '!' => Some('1'),
        '@' => Some('2'),
        '#' => Some('3'),
        '$' => Some('4'),
        '%' => Some('5'),
        '^' => Some('6'),
        '&' => Some('7'),
        '*' => Some('8'),
        '(' => Some('9'),
        _ => None,
    }
}

fn named_key(code: &str) -> Option<&'static str> {
    Some(match code {
        "escape" => "Esc",
        "enter" => "CR",
        "tab" => "Tab",
        "backspace" => "BS",
        "delete" => "Del",
        "space" | " " => "Space",
        "up" => "Up",
        "down" => "Down",
        "left" => "Left",
        "right" => "Right",
        "home" => "Home",
        "end" => "End",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        "insert" => "Insert",
        "f1" => "F1",
        "f2" => "F2",
        "f3" => "F3",
        "f4" => "F4",
        "f5" => "F5",
        "f6" => "F6",
        "f7" => "F7",
        "f8" => "F8",
        "f9" => "F9",
        "f10" => "F10",
        "f11" => "F11",
        "f12" => "F12",
        _ => return None,
    })
}

/// The token a `<leader>` in a map is kept as: what it stands for is
/// the keymap's to say at lookup ([`Keymap::set_leader`]), so a leader
/// set after the map was made — from a settings file, reloaded on
/// save — retargets every map at once.
pub const LEADER: &str = "<leader>";

/// Whether `note` is a ctrl- or alt-shift chord: `<C-S-x>`, `<A-S-x>`,
/// or a letter's shift spelled as the letter (`<C-L>`, `<A-J>`).
pub fn is_shift_chord(note: &str) -> bool {
    let Some(inner) = note.strip_prefix('<').and_then(|s| s.strip_suffix('>')) else {
        return false;
    };
    let mut parts: Vec<&str> = inner.split('-').collect();
    let Some(key) = parts.pop() else {
        return false;
    };
    let mods: Vec<&str> = parts;
    let held = mods.iter().any(|m| matches!(*m, "C" | "A" | "M"));
    let shifted = mods.contains(&"S")
        || (key.chars().count() == 1 && key.chars().next().is_some_and(|c| c.is_ascii_uppercase()));
    held && shifted
}

/// Splits `"<C-w>v"` into `["<C-w>", "v"]`; `<leader>` stays [`LEADER`].
pub fn parse_notation(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' {
            let mut inner = String::new();
            let mut closed = false;
            for c in chars.by_ref() {
                if c == '>' {
                    closed = true;
                    break;
                }
                inner.push(c);
            }
            if !closed {
                out.push(format!("<{inner}"));
                break;
            }
            if inner.eq_ignore_ascii_case("leader") {
                out.push(LEADER.into());
            } else if inner.eq_ignore_ascii_case("lt") {
                out.push("<".into());
            } else {
                out.push(normalize_chord(&inner));
            }
        } else if c == ' ' {
            out.push("<Space>".into());
        } else {
            out.push(c.to_string());
        }
    }
    out
}

/// `c-D` → `<C-d>`, `esc` → `<Esc>`, `cr` → `<CR>`, `s-tab` → `<S-Tab>`,
/// `d--` → `<D-->` (the minus key under ⌘), `2-leftmouse` →
/// `<2-LeftMouse>` (vim's double click).
fn normalize_chord(inner: &str) -> String {
    // A chord on the minus key ends in the separator and the key.
    let (inner, minus) = match inner.strip_suffix("--") {
        Some(rest) => (rest, true),
        None => (inner, false),
    };
    let mut parts: Vec<&str> = inner.split('-').collect();
    if minus {
        parts.push("-");
    }
    // A mouse gesture's click count leads, as vim writes it.
    let clicks = parts
        .first()
        .filter(|p| parts.len() > 1 && p.len() == 1 && p.as_bytes()[0].is_ascii_digit())
        .map(|p| p.to_string());
    if clicks.is_some() {
        parts.remove(0);
    }
    let (mods, base) = parts.split_at(parts.len() - 1);
    let base = base[0];
    if base.eq_ignore_ascii_case("leftmouse") {
        return match clicks {
            Some(n) => format!("<{n}-LeftMouse>"),
            None => "<LeftMouse>".into(),
        };
    }
    let base_named = match base.to_ascii_lowercase().as_str() {
        "esc" | "escape" => Some("Esc"),
        "cr" | "enter" | "return" => Some("CR"),
        "tab" => Some("Tab"),
        "bs" | "backspace" => Some("BS"),
        "del" | "delete" => Some("Del"),
        "space" => Some("Space"),
        "up" => Some("Up"),
        "down" => Some("Down"),
        "left" => Some("Left"),
        "right" => Some("Right"),
        "home" => Some("Home"),
        "end" => Some("End"),
        "pageup" => Some("PageUp"),
        "pagedown" => Some("PageDown"),
        "insert" => Some("Insert"),
        s if s.len() >= 2 && s.starts_with('f') && s[1..].chars().all(|c| c.is_ascii_digit()) => {
            None
        }
        _ => None,
    };
    let base_s = match base_named {
        Some(n) => n.to_string(),
        None if base.len() > 1 && base.to_ascii_lowercase().starts_with('f') => {
            base.to_ascii_uppercase()
        }
        None => base.to_string(),
    };
    let mut m = String::new();
    let (mut c, mut a, mut d, mut s) = (false, false, false, false);
    for x in mods {
        match x.to_ascii_lowercase().as_str() {
            "c" => c = true,
            "a" | "m" => a = true,
            "d" => d = true,
            "s" => s = true,
            _ => {}
        }
    }
    if c {
        m.push_str("C-");
    }
    if a {
        m.push_str("A-");
    }
    if d {
        m.push_str("D-");
    }
    // A map's `<C-S-1>` keeps its Shift, as the press does.
    let shifted_digit = s && base_named.is_none() && digit_of(&base_s).is_some();
    if s && (base_named.is_some() || shifted_digit) {
        m.push_str("S-");
    }
    if m.is_empty() && base_named.is_none() && base_s.len() == 1 {
        return base_s;
    }
    // A chord's letter is written lower-case so `<C-D>` and `<C-d>` agree.
    let base_s = match digit_of(&base_s) {
        Some(d) if shifted_digit => d.to_string(),
        _ if base_named.is_none() && base_s.len() == 1 && !s => base_s.to_ascii_lowercase(),
        _ if base_named.is_none() && base_s.len() == 1 => base_s.to_ascii_uppercase(),
        _ => base_s,
    };
    format!("<{m}{base_s}>")
}

/// What a key sequence runs. A key can carry several, newest first:
/// the engine takes the first whose `when` holds and whose command can
/// run ([`crate::Editor::pick_binding`]), so `<CR>` bound to
/// `dir enter` (`when` the listing) and, older, to `goto location`
/// (`when` not) is one key doing the right thing in each — and a bare
/// binding on a bare command still shadows everything under it.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Binding {
    pub command: String,
    pub args: Vec<String>,
    /// The binding's own conditions, beside its command's.
    pub when: Vec<Cond>,
}

impl Binding {
    /// `command args...` as the command line would spell it.
    pub fn line(&self) -> String {
        let mut s = self.command.clone();
        for a in &self.args {
            s.push(' ');
            s.push_str(a);
        }
        s
    }
}

#[derive(Default, Debug)]
struct Node {
    children: HashMap<String, Node>,
    /// Newest first.
    bindings: Vec<Binding>,
}

#[derive(Debug)]
pub enum Lookup<'a> {
    /// The bindings on the sequence, newest first.
    Exact(&'a [Binding]),
    Prefix,
    None,
}

#[derive(Default, Debug)]
pub struct Keymap {
    modes: HashMap<Mode, Node>,
    /// The key `<leader>` stands for, in notation (`<Space>`, `,`).
    leader: String,
    /// Bumped on every bind and unbind, for a reader that lists the
    /// bindings only when they changed.
    version: u64,
    /// What a prefix is for, by its notation (`<leader>b`: `buffers`)
    /// — the which-key's name for a group.
    groups: HashMap<String, String>,
}

impl Keymap {
    pub fn new() -> Self {
        Self {
            modes: HashMap::new(),
            leader: "<Space>".into(),
            version: 0,
            groups: HashMap::new(),
        }
    }

    /// Names what the keys of `prefix` (map notation) open, for the
    /// which-key: `describe("<leader>b", "buffers")`.
    pub fn describe(&mut self, prefix: &str, name: &str) {
        self.groups
            .insert(parse_notation(prefix).concat(), name.to_string());
        self.version += 1;
    }

    /// The name [`Keymap::describe`] gave `prefix`, the keys as pressed:
    /// one that is the leader's is found under `<leader>` too.
    pub fn group_name(&self, prefix: &[String]) -> Option<&str> {
        if let Some(n) = self.groups.get(&prefix.concat()) {
            return Some(n);
        }
        let as_leader: String = prefix
            .iter()
            .map(|k| {
                if *k == self.leader {
                    LEADER
                } else {
                    k.as_str()
                }
            })
            .collect();
        self.groups.get(&as_leader).map(String::as_str)
    }

    /// Every name [`Keymap::describe`] gave, by its prefix as bound
    /// (`<leader>b`), sorted — for a map of the keys whole.
    pub fn groups(&self) -> Vec<(&str, &str)> {
        let mut v: Vec<(&str, &str)> = self
            .groups
            .iter()
            .map(|(k, n)| (k.as_str(), n.as_str()))
            .collect();
        v.sort_unstable();
        v
    }

    /// The key `<leader>` stands for.
    pub fn leader(&self) -> &str {
        &self.leader
    }

    /// Makes `<leader>` stand for `key`, given in map notation — `","`,
    /// `"<Space>"`, `" "`. One key: a sequence is refused.
    pub fn set_leader(&mut self, key: &str) -> Result<(), String> {
        match parse_notation(key).as_slice() {
            [one] => {
                self.leader = one.clone();
                Ok(())
            }
            [] => Err("leader: no key".into()),
            _ => Err(format!("leader: one key, not a sequence ({key})")),
        }
    }

    /// Binds `keys` (map notation) in `mode` to `command args...`. A
    /// binding on a prefix of another shadows it: the longer one is
    /// unreachable, which is neovim's behaviour too (timeout aside).
    pub fn bind(&mut self, mode: Mode, keys: &str, command: &str) {
        self.bind_when(mode, keys, command, &[]);
    }

    /// [`Keymap::bind`] with the binding's own conditions (`"!terminal"`
    /// negates). The new binding goes in front of the key's others;
    /// one equal to it moves to the front.
    pub fn bind_when(&mut self, mode: Mode, keys: &str, command: &str, when: &[Cond]) {
        let seq = parse_notation(keys);
        let mut parts = command.split_whitespace();
        let name = parts.next().unwrap_or_default().to_string();
        let args = parts.map(str::to_string).collect();
        let mut node = self.modes.entry(mode).or_default();
        for k in seq {
            node = node.children.entry(k).or_default();
        }
        let b = Binding {
            command: name,
            args,
            when: when.to_vec(),
        };
        node.bindings.retain(|o| *o != b);
        node.bindings.insert(0, b);
        self.version += 1;
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn unbind(&mut self, mode: Mode, keys: &str) {
        let seq = parse_notation(keys);
        let Some(mut node) = self.modes.get_mut(&mode) else {
            return;
        };
        for k in seq {
            match node.children.get_mut(&k) {
                Some(n) => node = n,
                None => return,
            }
        }
        node.bindings.clear();
        self.version += 1;
    }

    /// Whether longer bindings lie beneath `keys` in `mode` — a key
    /// that is a prefix as well as a binding.
    /// A pressed key that is the leader's follows the `<leader>` branch
    /// too, as [`Keymap::lookup`] does: a plugin's key on Space, gated
    /// off where it is pressed, left `<leader>f` open behind it.
    pub fn has_deeper(&self, mode: Mode, keys: &[String]) -> bool {
        self.modes
            .get(&mode)
            .is_some_and(|root| self.deeper(root, keys))
    }

    fn deeper(&self, node: &Node, keys: &[String]) -> bool {
        let Some((k, rest)) = keys.split_first() else {
            return !node.children.is_empty();
        };
        node.children.get(k).is_some_and(|n| self.deeper(n, rest))
            || (*k == self.leader
                && node
                    .children
                    .get(LEADER)
                    .is_some_and(|n| self.deeper(n, rest)))
    }

    pub fn lookup(&self, mode: Mode, keys: &[String]) -> Lookup<'_> {
        match self.modes.get(&mode) {
            Some(root) => self.walk(root, keys),
            None => Lookup::None,
        }
    }

    /// The trie from `node` down `keys`. A pressed key that is the
    /// leader's follows the `<leader>` branch as well as its own — an
    /// explicit `<Space>x` and a `<leader>y` both reachable with Space
    /// as the leader. An exact match on the key's own branch wins over
    /// the leader's, but a leader map open past the key keeps the
    /// sequence open rather than firing the bare key: a `,` bound and
    /// chosen as leader waits for what follows, as vim's would.
    fn walk<'a>(&'a self, node: &'a Node, keys: &[String]) -> Lookup<'a> {
        let Some((k, rest)) = keys.split_first() else {
            // A binding with longer bindings beneath it: the shorter
            // wins at once, like neovim without `timeoutlen`.
            return if !node.bindings.is_empty() {
                Lookup::Exact(&node.bindings)
            } else if node.children.is_empty() {
                Lookup::None
            } else {
                Lookup::Prefix
            };
        };
        let own = match node.children.get(k) {
            Some(n) => self.walk(n, rest),
            None => Lookup::None,
        };
        let leader = match node.children.get(LEADER) {
            Some(n) if *k == self.leader => self.walk(n, rest),
            _ => Lookup::None,
        };
        match (own, leader) {
            (Lookup::Exact(b), Lookup::None) => Lookup::Exact(b),
            (Lookup::Exact(_), Lookup::Prefix) => Lookup::Prefix,
            (Lookup::Exact(b), Lookup::Exact(_)) => Lookup::Exact(b),
            (Lookup::Prefix, Lookup::Exact(b)) => Lookup::Exact(b),
            (Lookup::Prefix, _) | (Lookup::None, Lookup::Prefix) => Lookup::Prefix,
            (Lookup::None, l) => l,
        }
    }

    /// [`Keymap::lookup`], then — when nothing matched and the last key is
    /// a ctrl chord — the same sequence with the chord's letter bare, so
    /// `<C-w><C-w>` is `<C-w>w` and `<C-w><C-v>` is `<C-w>v`, as in vim.
    /// Whether a sequence begun in pane mode is one every pane shares
    /// with normal mode — the `<C-w>` cluster, the leader's groups,
    /// `:`, the next-and-previous cluster (`]t` `[t` `]q` `[q`), and a
    /// ctrl- or alt-shift chord (the pane cluster from a terminal too)
    /// — so a miss in pane mode looks it up there. Anything else
    /// (`dd`, `i`) is not: a list does not edit.
    pub fn shared_from_pane(&self, keys: &[String]) -> bool {
        let Some(first) = keys.first() else {
            return false;
        };
        if matches!(first.as_str(), "<C-w>" | ":" | "]" | "[")
            || first == LEADER
            || *first == self.leader
        {
            return true;
        }
        is_shift_chord(first)
    }

    pub fn lookup_lenient(&self, mode: Mode, keys: &[String]) -> Lookup<'_> {
        match self.lookup(mode, keys) {
            Lookup::None if keys.len() > 1 => {
                let Some(last) = keys.last() else {
                    return Lookup::None;
                };
                let Some(bare) = last
                    .strip_prefix("<C-")
                    .and_then(|s| s.strip_suffix('>'))
                    .filter(|s| s.chars().count() == 1)
                else {
                    return Lookup::None;
                };
                let mut alt = keys[..keys.len() - 1].to_vec();
                alt.push(bare.to_string());
                self.lookup(mode, &alt)
            }
            l => l,
        }
    }

    /// What can follow `prefix` in `mode`, sorted: each next key with
    /// its bindings, newest first — none for a key that is a prefix of
    /// its own — a which-key's rows, which picks the binding that can
    /// run ([`crate::Editor::pick_binding`]). A pressed key that is the
    /// leader's follows the `<leader>` branch as well as its own, as
    /// lookup does.
    pub fn next_keys(&self, mode: Mode, prefix: &[String]) -> Vec<(String, Vec<Binding>)> {
        let Some(root) = self.modes.get(&mode) else {
            return Vec::new();
        };
        let mut nodes = vec![root];
        for k in prefix {
            let mut next = Vec::new();
            for n in nodes {
                if let Some(c) = n.children.get(k) {
                    next.push(c);
                }
                if *k == self.leader
                    && let Some(c) = n.children.get(LEADER)
                {
                    next.push(c);
                }
            }
            nodes = next;
        }
        let mut out: Vec<(String, Vec<Binding>)> = Vec::new();
        for n in nodes {
            for (k, c) in &n.children {
                if out.iter().any(|(o, _)| o == k) {
                    continue;
                }
                out.push((k.clone(), c.bindings.clone()));
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Every binding in `mode`, for `:map` listings and the Lua API — a
    /// key with several listed once per binding, newest first.
    pub fn bindings(&self, mode: Mode) -> Vec<(String, Binding)> {
        self.binding_strokes(mode)
            .into_iter()
            .map(|(keys, b)| (keys.concat(), b))
            .collect()
    }

    /// [`Keymap::bindings`] with each key sequence as its strokes, as
    /// they are stored and pressed — `["<C-H>"]` for ctrl-shift-h,
    /// which the joined notation cannot be parsed back into (a map's
    /// `<C-H>` is `<C-h>`).
    pub fn binding_strokes(&self, mode: Mode) -> Vec<(Vec<String>, Binding)> {
        let mut out = Vec::new();
        if let Some(root) = self.modes.get(&mode) {
            walk(root, &mut Vec::new(), &mut out);
        }
        out.sort_by(|a, b| a.0.concat().cmp(&b.0.concat()));
        out
    }
}

fn walk(node: &Node, prefix: &mut Vec<String>, out: &mut Vec<(Vec<String>, Binding)>) {
    for b in &node.bindings {
        out.push((prefix.clone(), b.clone()));
    }
    for (k, n) in &node.children {
        prefix.push(k.clone());
        walk(n, prefix, out);
        prefix.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A chord's digit keeps its Shift, and the symbol a layout prints
    /// over the digit is that digit: `<C-S-1>` is bindable and is not
    /// `<C-1>`, which is what lets a map spell the two apart
    /// (2026-09-22, for kawoosh's column keys on Windows).
    #[test]
    fn a_chords_digit_keeps_its_shift() {
        let mut k = KeyStroke::plain("!");
        k.ctrl = true;
        k.shift = true;
        assert_eq!(k.notation(), "<C-S-1>", "the shifted symbol is its digit");
        let mut k = KeyStroke::plain("1");
        k.ctrl = true;
        k.shift = true;
        assert_eq!(k.notation(), "<C-S-1>", "and so is the digit itself");
        let mut k = KeyStroke::plain("1");
        k.ctrl = true;
        assert_eq!(k.notation(), "<C-1>", "without Shift it is the plain one");
        let mut k = KeyStroke::plain("1");
        k.sup = true;
        assert_eq!(k.notation(), "<D-1>");
        // A map spells them the same way, so a binding matches a press.
        assert_eq!(parse_notation("<C-S-1>"), ["<C-S-1>"]);
        assert_eq!(parse_notation("<C-1>"), ["<C-1>"]);
        assert_eq!(parse_notation("<D-1>"), ["<D-1>"]);
        // A `!` nobody shifted is still a `!`.
        let mut k = KeyStroke::plain("!");
        k.ctrl = true;
        assert_eq!(k.notation(), "<C-!>");
    }

    /// The groups' names come back whole, by the prefix they were
    /// given under, sorted.
    #[test]
    fn the_group_names_are_listed() {
        let mut km = Keymap::new();
        km.describe("<leader>t", "tabs");
        km.describe("<leader>b", "buffers");
        km.describe("<C-w>", "panes");
        assert_eq!(
            km.groups(),
            [
                ("<C-w>", "panes"),
                ("<leader>b", "buffers"),
                ("<leader>t", "tabs")
            ]
        );
    }

    /// The minus key under a chord and vim's mouse gestures spell the
    /// same from a map as from a press (2026-09-23: ⌘- for the font,
    /// a double click in a listing).
    #[test]
    fn the_minus_chord_and_a_double_click_are_spelled() {
        let press = |code: &str, shift: bool| {
            let mut k = KeyStroke::plain(code);
            k.sup = true;
            k.shift = shift;
            k.notation()
        };
        assert_eq!(press("-", false), "<D-->");
        assert_eq!(press("=", false), "<D-=>");
        assert_eq!(press("+", true), "<D-+>");
        assert_eq!(press("_", true), "<D-_>");
        assert_eq!(parse_notation("<D-->"), ["<D-->"]);
        assert_eq!(parse_notation("<C-->"), ["<C-->"]);
        assert_eq!(parse_notation("<D-=>"), ["<D-=>"]);
        assert_eq!(parse_notation("<D-+>"), ["<D-+>"]);
        assert_eq!(parse_notation("<D-_>"), ["<D-_>"]);
        assert_eq!(parse_notation("<2-LeftMouse>"), ["<2-LeftMouse>"]);
        assert_eq!(parse_notation("<2-leftmouse>"), ["<2-LeftMouse>"]);
        assert_eq!(parse_notation("<LeftMouse>"), ["<LeftMouse>"]);
    }

    #[test]
    fn notation_round_trips() {
        assert_eq!(KeyStroke::plain("j").notation(), "j");
        assert_eq!(KeyStroke::plain("J").notation(), "J");
        assert_eq!(KeyStroke::plain("escape").notation(), "<Esc>");
        let mut k = KeyStroke::plain("d");
        k.ctrl = true;
        assert_eq!(k.notation(), "<C-d>");
        let mut k = KeyStroke::plain("tab");
        k.shift = true;
        assert_eq!(k.notation(), "<S-Tab>");
        // ⌥⇧j as kui reports it — the modifier-stripped `j` with the
        // shift bit — is what `<A-S-j>` normalizes to.
        let mut k = KeyStroke::plain("j");
        k.alt = true;
        k.shift = true;
        assert_eq!(k.notation(), "<A-J>");
        assert_eq!(parse_notation("<A-S-j>"), ["<A-J>"]);
        let mut k = KeyStroke::plain("j");
        k.alt = true;
        assert_eq!(k.notation(), "<A-j>");
        assert_eq!(parse_notation("gg"), ["g", "g"]);
        assert_eq!(parse_notation("<C-w>v"), ["<C-w>", "v"]);
        assert_eq!(parse_notation("<c-D>"), ["<C-d>"]);
        assert_eq!(parse_notation("<C-S-v>"), ["<C-V>"]);
        assert_eq!(parse_notation("<leader>t"), [LEADER, "t"]);
        assert_eq!(parse_notation("<Esc>"), ["<Esc>"]);
        assert_eq!(parse_notation("<cr>"), ["<CR>"]);
    }

    /// `<leader>` is resolved when a key is looked up, not when the map
    /// is made: the leader changes and every map follows; an explicit
    /// map on the leader's key lives beside the leader maps.
    /// The leader's key has longer bindings beneath it through the
    /// `<leader>` branch, as `lookup` walks it: a plugin's pane-mode key
    /// on Space, gated off in another pane, must leave `<leader>f` open
    /// there, which reads `has_deeper` (roadmap step 52 met it).
    #[test]
    fn the_leader_key_is_deeper_through_the_leader_branch() {
        let mut km = Keymap::new();
        km.bind(Mode::Normal, "<leader>f", "files");
        let space = parse_notation(" ");
        assert!(km.has_deeper(Mode::Normal, &space));
        assert!(!km.has_deeper(Mode::Normal, &parse_notation(" f")));
        assert!(!km.has_deeper(Mode::Normal, &parse_notation("x")));
    }

    #[test]
    fn the_leader_is_resolved_at_lookup() {
        let mut km = Keymap::new();
        km.bind(Mode::Normal, "<leader>t", "todo");
        km.bind(Mode::Normal, "<leader>cd", "chdir");
        km.bind(Mode::Normal, "<Space>x", "explicit");
        let keys = |s: &str| parse_notation(s);
        assert!(
            matches!(km.lookup(Mode::Normal, &keys(" t")), Lookup::Exact([b, ..]) if b.command == "todo")
        );
        assert!(matches!(
            km.lookup(Mode::Normal, &keys(" c")),
            Lookup::Prefix
        ));
        assert!(
            matches!(km.lookup(Mode::Normal, &keys(" cd")), Lookup::Exact([b, ..]) if b.command == "chdir")
        );
        assert!(
            matches!(km.lookup(Mode::Normal, &keys(" x")), Lookup::Exact([b, ..]) if b.command == "explicit")
        );
        assert!(matches!(
            km.lookup(Mode::Normal, &keys(" ")),
            Lookup::Prefix
        ));
        assert!(matches!(km.lookup(Mode::Normal, &keys(",t")), Lookup::None));
        km.set_leader(",").unwrap();
        assert_eq!(km.leader(), ",");
        // A bound `,` waits while a leader map is open past it.
        km.bind(Mode::Normal, ",", "cursor primary");
        assert!(matches!(
            km.lookup(Mode::Normal, &keys(",")),
            Lookup::Prefix
        ));
        assert!(matches!(km.lookup(Mode::Normal, &keys(",q")), Lookup::None));
        assert!(
            matches!(km.lookup(Mode::Normal, &keys(",t")), Lookup::Exact([b, ..]) if b.command == "todo")
        );
        assert!(
            matches!(km.lookup(Mode::Normal, &keys(" t")), Lookup::None),
            "Space is no leader now"
        );
        assert!(
            matches!(km.lookup(Mode::Normal, &keys(" x")), Lookup::Exact(_)),
            "the explicit map stays"
        );
        km.set_leader("<Space>").unwrap();
        assert_eq!(km.leader(), "<Space>");
        km.set_leader(" ").unwrap();
        assert_eq!(km.leader(), "<Space>");
        assert!(km.set_leader("ab").is_err());
        assert!(km.set_leader("").is_err());
        // The listing says `<leader>`, whatever it stands for.
        let listed: Vec<String> = km
            .bindings(Mode::Normal)
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        assert!(listed.contains(&"<leader>t".to_string()), "{listed:?}");
    }

    #[test]
    fn trie_lookup() {
        let mut km = Keymap::new();
        km.bind(Mode::Normal, "gg", "goto_start");
        km.bind(Mode::Normal, "ge", "goto_end");
        km.bind(Mode::Normal, "<C-d>", "half_down");
        let g = ["g".to_string()];
        assert!(matches!(km.lookup(Mode::Normal, &g), Lookup::Prefix));
        let gg = ["g".to_string(), "g".to_string()];
        assert!(
            matches!(km.lookup(Mode::Normal, &gg), Lookup::Exact([b, ..]) if b.command == "goto_start")
        );
        let cd = ["<C-d>".to_string()];
        assert!(matches!(km.lookup(Mode::Normal, &cd), Lookup::Exact(_)));
        km.bind(Mode::Normal, "<C-w>w", "pane next");
        let cw_cw = ["<C-w>".to_string(), "<C-w>".to_string()];
        assert!(matches!(km.lookup(Mode::Normal, &cw_cw), Lookup::None));
        assert!(
            matches!(km.lookup_lenient(Mode::Normal, &cw_cw), Lookup::Exact([b, ..]) if b.line() == "pane next")
        );
        let x = ["x".to_string()];
        assert!(matches!(km.lookup(Mode::Normal, &x), Lookup::None));
    }
}
