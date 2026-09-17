//! Keys resolve to named commands through a per-mode trie (mvp.md
//! Decision 4). Notation is neovim's — `j`, `<C-d>`, `gg`, `<leader>t` —
//! because that is the muscle memory being courted. A stroke arrives as
//! kui's `code` (already layout-resolved with the US-QWERTY fallback,
//! kui.md D5) plus modifiers; text never comes through here.

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Mode {
    Normal,
    Insert,
    Visual,
    /// The command line and the search prompt: `:` and `/`.
    Command,
    /// An operator is waiting for its motion or text object.
    OperatorPending,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Normal => "NOR",
            Mode::Insert => "INS",
            Mode::Visual => "VIS",
            Mode::Command => "CMD",
            Mode::OperatorPending => "OP",
        }
    }

    pub fn from_short(s: &str) -> Option<Self> {
        match s {
            "n" | "normal" => Some(Mode::Normal),
            "i" | "insert" => Some(Mode::Insert),
            "v" | "visual" => Some(Mode::Visual),
            "c" | "command" => Some(Mode::Command),
            "o" | "op" | "operator" => Some(Mode::OperatorPending),
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
        if self.shift && named.is_some() {
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

/// `c-D` → `<C-d>`, `esc` → `<Esc>`, `cr` → `<CR>`, `s-tab` → `<S-Tab>`.
fn normalize_chord(inner: &str) -> String {
    let parts: Vec<&str> = inner.split('-').collect();
    let (mods, base) = parts.split_at(parts.len() - 1);
    let base = base[0];
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
    if s && base_named.is_some() {
        m.push_str("S-");
    }
    if m.is_empty() && base_named.is_none() && base_s.len() == 1 {
        return base_s;
    }
    // A chord's letter is written lower-case so `<C-D>` and `<C-d>` agree.
    let base_s = if base_named.is_none() && base_s.len() == 1 && !s {
        base_s.to_ascii_lowercase()
    } else if base_named.is_none() && base_s.len() == 1 {
        base_s.to_ascii_uppercase()
    } else {
        base_s
    };
    format!("<{m}{base_s}>")
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Binding {
    pub command: String,
    pub args: Vec<String>,
}

#[derive(Default, Debug)]
struct Node {
    children: HashMap<String, Node>,
    binding: Option<Binding>,
}

#[derive(Debug)]
pub enum Lookup<'a> {
    Exact(&'a Binding),
    Prefix,
    None,
}

#[derive(Default, Debug)]
pub struct Keymap {
    modes: HashMap<Mode, Node>,
    /// The key `<leader>` stands for, in notation (`<Space>`, `,`).
    leader: String,
}

impl Keymap {
    pub fn new() -> Self {
        Self {
            modes: HashMap::new(),
            leader: "<Space>".into(),
        }
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
        let seq = parse_notation(keys);
        let mut parts = command.split_whitespace();
        let name = parts.next().unwrap_or_default().to_string();
        let args = parts.map(str::to_string).collect();
        let mut node = self.modes.entry(mode).or_default();
        for k in seq {
            node = node.children.entry(k).or_default();
        }
        node.binding = Some(Binding {
            command: name,
            args,
        });
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
        node.binding = None;
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
            return match &node.binding {
                // A binding with longer bindings beneath it: the shorter
                // wins at once, like neovim without `timeoutlen`.
                Some(b) => Lookup::Exact(b),
                None if node.children.is_empty() => Lookup::None,
                None => Lookup::Prefix,
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

    /// Every binding in `mode`, for `:map` listings and the Lua API.
    pub fn bindings(&self, mode: Mode) -> Vec<(String, Binding)> {
        let mut out = Vec::new();
        if let Some(root) = self.modes.get(&mode) {
            walk(root, String::new(), &mut out);
        }
        out.sort();
        out
    }
}

fn walk(node: &Node, prefix: String, out: &mut Vec<(String, Binding)>) {
    if let Some(b) = &node.binding {
        out.push((prefix.clone(), b.clone()));
    }
    for (k, n) in &node.children {
        walk(n, format!("{prefix}{k}"), out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    #[test]
    fn the_leader_is_resolved_at_lookup() {
        let mut km = Keymap::new();
        km.bind(Mode::Normal, "<leader>t", "todo");
        km.bind(Mode::Normal, "<leader>cd", "chdir");
        km.bind(Mode::Normal, "<Space>x", "explicit");
        let keys = |s: &str| parse_notation(s);
        assert!(
            matches!(km.lookup(Mode::Normal, &keys(" t")), Lookup::Exact(b) if b.command == "todo")
        );
        assert!(matches!(
            km.lookup(Mode::Normal, &keys(" c")),
            Lookup::Prefix
        ));
        assert!(
            matches!(km.lookup(Mode::Normal, &keys(" cd")), Lookup::Exact(b) if b.command == "chdir")
        );
        assert!(
            matches!(km.lookup(Mode::Normal, &keys(" x")), Lookup::Exact(b) if b.command == "explicit")
        );
        assert!(matches!(
            km.lookup(Mode::Normal, &keys(" ")),
            Lookup::Prefix
        ));
        assert!(matches!(km.lookup(Mode::Normal, &keys(",t")), Lookup::None));
        km.set_leader(",").unwrap();
        assert_eq!(km.leader(), ",");
        // A bound `,` waits while a leader map is open past it.
        km.bind(Mode::Normal, ",", "keep_primary");
        assert!(matches!(
            km.lookup(Mode::Normal, &keys(",")),
            Lookup::Prefix
        ));
        assert!(matches!(km.lookup(Mode::Normal, &keys(",q")), Lookup::None));
        assert!(
            matches!(km.lookup(Mode::Normal, &keys(",t")), Lookup::Exact(b) if b.command == "todo")
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
            matches!(km.lookup(Mode::Normal, &gg), Lookup::Exact(b) if b.command == "goto_start")
        );
        let cd = ["<C-d>".to_string()];
        assert!(matches!(km.lookup(Mode::Normal, &cd), Lookup::Exact(_)));
        km.bind(Mode::Normal, "<C-w>w", "pane_next");
        let cw_cw = ["<C-w>".to_string(), "<C-w>".to_string()];
        assert!(matches!(km.lookup(Mode::Normal, &cw_cw), Lookup::None));
        assert!(
            matches!(km.lookup_lenient(Mode::Normal, &cw_cw), Lookup::Exact(b) if b.command == "pane_next")
        );
        let x = ["x".to_string()];
        assert!(matches!(km.lookup(Mode::Normal, &x), Lookup::None));
    }
}
