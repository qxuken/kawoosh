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
    // A letter's case is its Shift, as a press spells it
    // ([`KeyStroke::notation`]): `<C-S-h>` and `<C-H>` are ctrl-shift-h,
    // `<C-h>` is not — so a key `:map list` shows binds that key again.
    let base_s = match digit_of(&base_s) {
        Some(d) if shifted_digit => d.to_string(),
        _ if base_named.is_none() && base_s.len() == 1 && s => base_s.to_ascii_uppercase(),
        _ => base_s,
    };
    if m.is_empty() && base_named.is_none() && base_s.len() == 1 {
        return base_s;
    }
    format!("<{m}{base_s}>")
}

/// What a key sequence runs. A key can carry several, newest first:
/// the engine takes the first whose `when` holds and whose command can
/// run ([`crate::Editor::pick_binding`]), so `r` bound to `terminal
/// again` (`when` the pane's line has ended) and, older, to `terminal
/// raw` is one key doing the right thing in each — and a bare binding
/// on a bare command still shadows everything under it.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Binding {
    pub command: String,
    pub args: Vec<String>,
    /// The binding's own conditions, beside its command's.
    pub when: Vec<Cond>,
    /// The place the binding is local to — a fact, `language:dir`,
    /// `lua:picker`, `buffer#ID` ([`Keymap::bind_local`]) — or none
    /// for a global one.
    pub scope: Option<String>,
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

/// What a key sequence is in the places asked: bound (the bindings
/// that can take it, the innermost place's first), the start of longer
/// ones, or nothing.
#[derive(Debug)]
pub enum Lookup {
    /// The bindings on the sequence, newest first — a local place's
    /// before the global ones under it, for a binding that cannot run
    /// or passes to hand the key down.
    Exact(Vec<Binding>),
    Prefix,
    None,
}

impl Lookup {
    pub fn is_none(&self) -> bool {
        matches!(self, Lookup::None)
    }
}

/// One trie's answer, before the places are merged.
enum Hit<'a> {
    Exact(&'a [Binding]),
    Prefix,
    None,
}

/// The maps local to one place (docs/design/local-maps.md).
#[derive(Default, Debug)]
struct Local {
    modes: HashMap<Mode, Node>,
    /// When the place got its first map: among places of one rank the
    /// newest is asked first.
    made: u64,
}

/// A place's rank, the innermost first: a field's own, the prompt, one
/// buffer, a buffer by name, a language, a Lua view, and every other
/// fact (local-maps.md Decision 3).
fn rank(scope: &str) -> u8 {
    if scope.starts_with("field:") {
        0
    } else if scope == "prompt" {
        1
    } else if scope.starts_with("buffer#") {
        2
    } else if scope.starts_with("buffer:") {
        3
    } else if scope.starts_with("language:") {
        4
    } else if scope.starts_with("lua:") {
        5
    } else {
        6
    }
}

#[derive(Default, Debug)]
pub struct Keymap {
    modes: HashMap<Mode, Node>,
    /// The maps local to a place, by its fact ([`Keymap::bind_local`]).
    locals: HashMap<String, Local>,
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
            locals: HashMap::new(),
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
        self.bind_in(None, mode, keys, command, when);
    }

    /// [`Keymap::bind_when`] local to a place: `scope` is a fact —
    /// `lua:picker`, `field:lua:picker/q`, `language:dir`,
    /// `buffer:*compile*`, `buffer#ID`, `terminal` — and the binding is
    /// found only where it holds, before the global ones, which it
    /// shadows there (docs/design/local-maps.md).
    pub fn bind_local(
        &mut self,
        scope: &str,
        mode: Mode,
        keys: &str,
        command: &str,
        when: &[Cond],
    ) {
        self.bind_in(Some(scope), mode, keys, command, when);
    }

    fn bind_in(
        &mut self,
        scope: Option<&str>,
        mode: Mode,
        keys: &str,
        command: &str,
        when: &[Cond],
    ) {
        let seq = parse_notation(keys);
        let mut parts = command.split_whitespace();
        let name = parts.next().unwrap_or_default().to_string();
        let args = parts.map(str::to_string).collect();
        let made = self.version;
        let modes = match scope {
            Some(s) => {
                &mut self
                    .locals
                    .entry(s.to_string())
                    .or_insert_with(|| Local {
                        modes: HashMap::new(),
                        made,
                    })
                    .modes
            }
            None => &mut self.modes,
        };
        let mut node = modes.entry(mode).or_default();
        for k in seq {
            node = node.children.entry(k).or_default();
        }
        let b = Binding {
            command: name,
            args,
            when: when.to_vec(),
            scope: scope.map(str::to_string),
        };
        node.bindings.retain(|o| *o != b);
        node.bindings.insert(0, b);
        self.version += 1;
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn unbind(&mut self, mode: Mode, keys: &str) {
        self.unbind_in(None, mode, keys);
    }

    /// The bindings of `keys` local to `scope` gone; the global ones and
    /// the longer ones beneath stay.
    pub fn unbind_local(&mut self, scope: &str, mode: Mode, keys: &str) {
        self.unbind_in(Some(scope), mode, keys);
    }

    fn unbind_in(&mut self, scope: Option<&str>, mode: Mode, keys: &str) {
        let seq = parse_notation(keys);
        let modes = match scope {
            Some(s) => match self.locals.get_mut(s) {
                Some(l) => &mut l.modes,
                None => return,
            },
            None => &mut self.modes,
        };
        let Some(mut node) = modes.get_mut(&mode) else {
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

    /// Every map local to `scope` gone — a buffer's `buffer#ID` with
    /// the buffer.
    pub fn drop_scope(&mut self, scope: &str) {
        if self.locals.remove(scope).is_some() {
            self.version += 1;
        }
    }

    /// The places with maps of their own for which `holds` says yes,
    /// the innermost first: by rank ([`rank`]), then the newest place.
    /// What a lookup on a view asks ([`crate::Editor::key_scopes`]).
    pub fn scopes_holding(&self, holds: impl Fn(&str) -> bool) -> Vec<String> {
        let mut v: Vec<(&String, &Local)> = self.locals.iter().filter(|(s, _)| holds(s)).collect();
        v.sort_by(|a, b| {
            rank(a.0)
                .cmp(&rank(b.0))
                .then(b.1.made.cmp(&a.1.made))
                .then(a.0.cmp(b.0))
        });
        v.into_iter().map(|(s, _)| s.clone()).collect()
    }

    /// The tries a lookup in `mode` walks, in order: each place's of
    /// `scopes` (innermost first), then the global one.
    fn tries<'a>(
        &'a self,
        scopes: &'a [String],
        mode: Mode,
    ) -> impl Iterator<Item = &'a Node> + 'a {
        scopes
            .iter()
            .filter_map(move |s| self.locals.get(s).and_then(|l| l.modes.get(&mode)))
            .chain(self.modes.get(&mode))
    }

    /// Whether longer bindings lie beneath `keys` in `mode` — a key
    /// that is a prefix as well as a binding.
    /// A pressed key that is the leader's follows the `<leader>` branch
    /// too, as [`Keymap::lookup`] does: a plugin's key on Space, gated
    /// off where it is pressed, left `<leader>f` open behind it.
    pub fn has_deeper(&self, mode: Mode, keys: &[String]) -> bool {
        self.deeper_in(&[], mode, keys)
    }

    /// [`Keymap::has_deeper`] in the places of `scopes` as well as the
    /// global map.
    pub fn deeper_in(&self, scopes: &[String], mode: Mode, keys: &[String]) -> bool {
        self.tries(scopes, mode).any(|root| self.deeper(root, keys))
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

    /// `keys` in the global map of `mode`.
    pub fn lookup(&self, mode: Mode, keys: &[String]) -> Lookup {
        self.lookup_in(&[], mode, keys)
    }

    /// `keys` in the places of `scopes`, innermost first, then the
    /// global map (local-maps.md Decision 3): the first that knows the
    /// keys decides — bound, a local binding shadowing the global one
    /// and every longer one under it; a prefix, a local prefix
    /// shadowing a shorter global binding. A bound sequence carries the
    /// bindings of the places under it too, so one that cannot run
    /// here, or passes, hands the key down.
    pub fn lookup_in(&self, scopes: &[String], mode: Mode, keys: &[String]) -> Lookup {
        let mut out = Lookup::None;
        for root in self.tries(scopes, mode) {
            match self.walk(root, keys) {
                Hit::None => {}
                Hit::Prefix => {
                    if matches!(out, Lookup::None) {
                        out = Lookup::Prefix;
                    }
                }
                Hit::Exact(bs) => {
                    if let Lookup::Exact(v) = &mut out {
                        v.extend_from_slice(bs);
                    } else if matches!(out, Lookup::None) {
                        out = Lookup::Exact(bs.to_vec());
                    }
                }
            }
        }
        out
    }

    /// The trie from `node` down `keys`. A pressed key that is the
    /// leader's follows the `<leader>` branch as well as its own — an
    /// explicit `<Space>x` and a `<leader>y` both reachable with Space
    /// as the leader. An exact match on the key's own branch wins over
    /// the leader's, but a leader map open past the key keeps the
    /// sequence open rather than firing the bare key: a `,` bound and
    /// chosen as leader waits for what follows, as vim's would.
    fn walk<'a>(&'a self, node: &'a Node, keys: &[String]) -> Hit<'a> {
        let Some((k, rest)) = keys.split_first() else {
            // A binding with longer bindings beneath it: the shorter
            // wins at once, like neovim without `timeoutlen`.
            return if !node.bindings.is_empty() {
                Hit::Exact(&node.bindings)
            } else if node.children.is_empty() {
                Hit::None
            } else {
                Hit::Prefix
            };
        };
        let own = match node.children.get(k) {
            Some(n) => self.walk(n, rest),
            None => Hit::None,
        };
        let leader = match node.children.get(LEADER) {
            Some(n) if *k == self.leader => self.walk(n, rest),
            _ => Hit::None,
        };
        match (own, leader) {
            (Hit::Exact(b), Hit::None) => Hit::Exact(b),
            (Hit::Exact(_), Hit::Prefix) => Hit::Prefix,
            (Hit::Exact(b), Hit::Exact(_)) => Hit::Exact(b),
            (Hit::Prefix, Hit::Exact(b)) => Hit::Exact(b),
            (Hit::Prefix, _) | (Hit::None, Hit::Prefix) => Hit::Prefix,
            (Hit::None, l) => l,
        }
    }

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

    /// [`Keymap::lookup`], then — when nothing matched and the last key is
    /// a ctrl chord — the same sequence with the chord's letter bare, so
    /// `<C-w><C-w>` is `<C-w>w` and `<C-w><C-v>` is `<C-w>v`, as in vim.
    pub fn lookup_lenient(&self, mode: Mode, keys: &[String]) -> Lookup {
        self.lookup_lenient_in(&[], mode, keys)
    }

    /// [`Keymap::lookup_lenient`] in the places of `scopes` as well.
    pub fn lookup_lenient_in(&self, scopes: &[String], mode: Mode, keys: &[String]) -> Lookup {
        match self.lookup_in(scopes, mode, keys) {
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
                self.lookup_in(scopes, mode, &alt)
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
        self.next_keys_in(&[], mode, prefix)
    }

    /// [`Keymap::next_keys`] in the places of `scopes` as well: a key's
    /// bindings the innermost place's first, then the global ones.
    pub fn next_keys_in(
        &self,
        scopes: &[String],
        mode: Mode,
        prefix: &[String],
    ) -> Vec<(String, Vec<Binding>)> {
        let mut out: Vec<(String, Vec<Binding>)> = Vec::new();
        for root in self.tries(scopes, mode) {
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
            // Within one trie a key's own branch wins over the leader's,
            // as lookup has it; across places they add up.
            let mut seen: Vec<&String> = Vec::new();
            for n in nodes {
                for (k, c) in &n.children {
                    if seen.contains(&k) {
                        continue;
                    }
                    seen.push(k);
                    match out.iter_mut().find(|(o, _)| o == k) {
                        Some((_, bs)) => bs.extend(c.bindings.iter().cloned()),
                        None => out.push((k.clone(), c.bindings.clone())),
                    }
                }
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Every binding in `mode`, for `:map` listings and the Lua API — a
    /// key with several listed once per binding, newest first, a local
    /// one saying its place ([`Binding::scope`]).
    pub fn bindings(&self, mode: Mode) -> Vec<(String, Binding)> {
        self.binding_strokes(mode)
            .into_iter()
            .map(|(keys, b)| (keys.concat(), b))
            .collect()
    }

    /// [`Keymap::bindings`] with each key sequence as its strokes, as
    /// they are stored and pressed — `["<C-H>"]` for ctrl-shift-h — so
    /// a reader need not split the joined notation again.
    pub fn binding_strokes(&self, mode: Mode) -> Vec<(Vec<String>, Binding)> {
        let mut out = Vec::new();
        if let Some(root) = self.modes.get(&mode) {
            walk(root, &mut Vec::new(), &mut out);
        }
        let mut scopes: Vec<&String> = self.locals.keys().collect();
        scopes.sort();
        for s in scopes {
            if let Some(root) = self.locals[s].modes.get(&mode) {
                walk(root, &mut Vec::new(), &mut out);
            }
        }
        out.sort_by_key(|a| a.0.concat());
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

    /// A chord's upper-case letter is its Shift, read from a map as a
    /// press spells it: `<C-H>` is ctrl-shift-h, the key `:map list`
    /// shows for `<C-S-h>`, and not the shell's `<C-h>` — until
    /// 2026-09-28 a map's `<C-H>` was lower-cased into it, so a key
    /// copied from the listing bound another.
    #[test]
    fn a_chords_upper_case_letter_is_its_shift() {
        for (map, press) in [
            ("<C-H>", ("h", true, false, false)),
            ("<A-J>", ("j", false, true, false)),
            ("<D-L>", ("l", false, false, true)),
        ] {
            let mut k = KeyStroke::plain(press.0);
            k.ctrl = press.1;
            k.alt = press.2;
            k.sup = press.3;
            k.shift = true;
            assert_eq!(parse_notation(map), [k.notation()], "{map}");
            // And the notation parses back to itself.
            assert_eq!(parse_notation(&k.notation()), [k.notation()]);
        }
        assert_eq!(parse_notation("<C-S-h>"), ["<C-H>"]);
        assert_eq!(parse_notation("<C-h>"), ["<C-h>"]);
        assert_eq!(
            parse_notation("<S-j>"),
            ["J"],
            "a shifted letter is the letter"
        );
        let mut km = Keymap::new();
        km.bind(Mode::Normal, "<C-H>", "pane left");
        km.bind(Mode::Normal, "<C-h>", "move left");
        let got: Vec<(Vec<String>, String)> = km
            .binding_strokes(Mode::Normal)
            .into_iter()
            .map(|(k, b)| (k, b.command))
            .collect();
        assert_eq!(
            got,
            [
                (vec!["<C-H>".to_string()], "pane".to_string()),
                (vec!["<C-h>".to_string()], "move".to_string())
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
        assert_eq!(parse_notation("<c-d>"), ["<C-d>"]);
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
            matches!(km.lookup(Mode::Normal, &keys(" t")), Lookup::Exact(bs) if bs[0].command == "todo")
        );
        assert!(matches!(
            km.lookup(Mode::Normal, &keys(" c")),
            Lookup::Prefix
        ));
        assert!(
            matches!(km.lookup(Mode::Normal, &keys(" cd")), Lookup::Exact(bs) if bs[0].command == "chdir")
        );
        assert!(
            matches!(km.lookup(Mode::Normal, &keys(" x")), Lookup::Exact(bs) if bs[0].command == "explicit")
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
            matches!(km.lookup(Mode::Normal, &keys(",t")), Lookup::Exact(bs) if bs[0].command == "todo")
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

    /// A map local to a place is found only where the place holds, and
    /// there before the global one of its keys, which it hands the key
    /// to (local-maps.md Decision 3).
    #[test]
    fn a_local_map_is_found_only_where_it_holds() {
        let mut km = Keymap::new();
        km.bind(Mode::Normal, "<CR>", "goto location");
        km.bind_local("language:dir", Mode::Normal, "<CR>", "dir enter", &[]);
        let cr = parse_notation("<CR>");
        let got = |l: Lookup| match l {
            Lookup::Exact(bs) => bs.iter().map(|b| b.line()).collect::<Vec<_>>(),
            Lookup::Prefix => vec!["prefix".to_string()],
            Lookup::None => vec![],
        };
        assert_eq!(got(km.lookup(Mode::Normal, &cr)), ["goto location"]);
        let dir = ["language:dir".to_string()];
        assert_eq!(
            got(km.lookup_in(&dir, Mode::Normal, &cr)),
            ["dir enter", "goto location"],
            "the place's first, the global one under it"
        );
        // A key only the place has is nothing anywhere else.
        km.bind_local("lua:launcher", Mode::Normal, "z", "launcher key z", &[]);
        assert!(matches!(
            km.lookup(Mode::Normal, &parse_notation("z")),
            Lookup::None
        ));
        // Its binding says where it lives.
        let listed: Vec<(String, Option<String>)> = km
            .bindings(Mode::Normal)
            .into_iter()
            .map(|(k, b)| (k, b.scope))
            .collect();
        assert!(
            listed.contains(&("z".into(), Some("lua:launcher".into()))),
            "{listed:?}"
        );
        km.unbind_local("language:dir", Mode::Normal, "<CR>");
        assert_eq!(
            got(km.lookup_in(&dir, Mode::Normal, &cr)),
            ["goto location"]
        );
        km.drop_scope("lua:launcher");
        let launcher = ["lua:launcher".to_string()];
        assert!(
            km.lookup_in(&launcher, Mode::Normal, &parse_notation("z"))
                .is_none()
        );
    }

    /// A local binding shadows the longer global ones under its keys (the
    /// launcher's `g` over `gg`), and a local prefix a shorter global
    /// binding (the listing's `ma` over `m`).
    #[test]
    fn a_local_map_shadows_both_ways() {
        let mut km = Keymap::new();
        km.bind(Mode::Normal, "gg", "goto start");
        km.bind(Mode::Normal, "m", "mark");
        km.bind_local(
            "field:lua:launcher/q",
            Mode::Normal,
            "g",
            "launcher key g",
            &[],
        );
        km.bind_local("language:dir", Mode::Normal, "ma", "dir sort name", &[]);
        let g = parse_notation("g");
        let m = parse_notation("m");
        assert!(matches!(km.lookup(Mode::Normal, &g), Lookup::Prefix));
        let field = ["field:lua:launcher/q".to_string()];
        assert!(
            matches!(km.lookup_in(&field, Mode::Normal, &g), Lookup::Exact(bs) if bs.len() == 1 && bs[0].line() == "launcher key g")
        );
        assert!(
            km.deeper_in(&field, Mode::Normal, &g),
            "`gg` is still under it"
        );
        assert!(matches!(km.lookup(Mode::Normal, &m), Lookup::Exact(_)));
        let dir = ["language:dir".to_string()];
        assert!(matches!(
            km.lookup_in(&dir, Mode::Normal, &m),
            Lookup::Prefix
        ));
        assert!(
            matches!(km.lookup_in(&dir, Mode::Normal, &parse_notation("ma")), Lookup::Exact(bs) if bs[0].line() == "dir sort name")
        );
        // The which-key's rows: the place's keys beside the global ones.
        let rows: Vec<String> = km
            .next_keys_in(&dir, Mode::Normal, &m)
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        assert_eq!(rows, ["a"]);
        assert!(km.next_keys(Mode::Normal, &m).is_empty());
    }

    /// The places a view is in, the innermost first: a field's own, the
    /// prompt, one buffer, a buffer by name, a language, a Lua view,
    /// then every other fact, the newest place first.
    #[test]
    fn the_places_are_asked_innermost_first() {
        let mut km = Keymap::new();
        for s in [
            "terminal",
            "lua:picker",
            "language:dir",
            "exited",
            "buffer:*compile*",
            "buffer#7",
            "prompt",
            "field:cmdline",
            "memory",
        ] {
            km.bind_local(s, Mode::Normal, "x", "x", &[]);
        }
        assert_eq!(
            km.scopes_holding(|s| s != "memory"),
            [
                "field:cmdline",
                "prompt",
                "buffer#7",
                "buffer:*compile*",
                "language:dir",
                "lua:picker",
                "exited",
                "terminal"
            ]
        );
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
            matches!(km.lookup(Mode::Normal, &gg), Lookup::Exact(bs) if bs[0].command == "goto_start")
        );
        let cd = ["<C-d>".to_string()];
        assert!(matches!(km.lookup(Mode::Normal, &cd), Lookup::Exact(_)));
        km.bind(Mode::Normal, "<C-w>w", "pane next");
        let cw_cw = ["<C-w>".to_string(), "<C-w>".to_string()];
        assert!(matches!(km.lookup(Mode::Normal, &cw_cw), Lookup::None));
        assert!(
            matches!(km.lookup_lenient(Mode::Normal, &cw_cw), Lookup::Exact(bs) if bs[0].line() == "pane next")
        );
        let x = ["x".to_string()];
        assert!(matches!(km.lookup(Mode::Normal, &x), Lookup::None));
    }
}
