//! A command is two things: a [`Spec`] — its name, its ex spellings,
//! what it takes, whether `!` and `?` mean anything to it, and when it
//! can run — and a body that runs it. The spec is data: the registry
//! holds it, the command line completes from it, `:map` checks against
//! it, Lua reads it as a table. The body is the [`Command`] trait,
//! implemented once per command wherever the command lives — the
//! engine's on `Editor`, the shell's on its own type, a plugin's by the
//! Lua bridge — so that everything about `:bd` is in one place, and
//! nothing about it is in the dispatcher.
//!
//! A subcommand is a command whose name is two words: `history drop`
//! is registered as one, listed by [`Editor::subcommands`], walked by
//! [`Editor::resolve`], completed by the command line under `:history`.

use std::collections::BTreeSet;
use std::fmt;
use std::rc::Rc;

use crate::{Editor, ViewId};

/// How a command was asked for: plain, with `!`, or with `?`. What
/// each means is the command's — `:q!` discards, `:e!` reloads, `:cd?`
/// says where — and its spec documents it ([`Spec::bang`],
/// [`Spec::query`]); a form the spec has no word for is refused before
/// the command sees it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Form {
    #[default]
    Run,
    Bang,
    Query,
}

impl Form {
    pub fn name(self) -> &'static str {
        match self {
            Self::Run => "run",
            Self::Bang => "bang",
            Self::Query => "query",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "run" | "" => Self::Run,
            "bang" | "!" => Self::Bang,
            "query" | "?" => Self::Query,
            _ => return None,
        })
    }

    /// The marker the form is written with on the command line.
    pub fn marker(self) -> &'static str {
        match self {
            Self::Run => "",
            Self::Bang => "!",
            Self::Query => "?",
        }
    }

    /// `name` without its trailing `!` or `?`, and the form that
    /// marker names. A name that is only a marker is not one.
    pub fn split(name: &str) -> (&str, Self) {
        if name.len() > 1 {
            if let Some(n) = name.strip_suffix('!') {
                return (n, Self::Bang);
            }
            if let Some(n) = name.strip_suffix('?') {
                return (n, Self::Query);
            }
        }
        (name, Self::Run)
    }
}

/// What a command runs with.
#[derive(Clone, Debug, PartialEq)]
pub struct Ctx {
    pub view: ViewId,
    pub count: usize,
    pub has_count: bool,
    /// Plain, `!` or `?`.
    pub form: Form,
    /// The arguments past the command's name (and its subcommand's),
    /// a `Path` among them resolved.
    pub args: Vec<String>,
    /// The key that followed, for commands that take a character (`f`,
    /// `r`, `iw`).
    pub arg_char: Option<char>,
}

impl Ctx {
    pub fn new(view: ViewId) -> Self {
        Self {
            view,
            count: 1,
            has_count: false,
            form: Form::Run,
            args: Vec::new(),
            arg_char: None,
        }
    }

    pub fn bang(&self) -> bool {
        self.form == Form::Bang
    }

    pub fn query(&self) -> bool {
        self.form == Form::Query
    }

    pub fn arg(&self, i: usize) -> Option<&str> {
        self.args.get(i).map(String::as_str)
    }
}

/// How an operator takes a motion's range (vim's three).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MotionKind {
    Exclusive,
    Inclusive,
    Linewise,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Kind {
    Motion(MotionKind),
    Operator,
    TextObject,
    #[default]
    Other,
}

/// What an argument of a command is, declared with the command. The
/// engine resolves a `Path` — `~/x`, `../y`, against the working
/// directory — before any command sees it, whoever registered the
/// command: the engine's `:w`, the shell's `:cd`, a plugin's `:oil`.
/// The command line completes each kind from what it names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgKind {
    /// A file or directory, as the user writes it.
    Path,
    /// A buffer, by name.
    Buffer,
    /// A command, by name (`:map`'s last).
    Command,
    /// An option (`:set`).
    Option,
    /// A tool the config registered.
    Tool,
    /// A Lua view.
    View,
    /// Anything.
    Text,
}

impl ArgKind {
    /// The kind by its name — what a plugin writes in `args = { "path" }`.
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "path" | "file" | "dir" => Self::Path,
            "buffer" => Self::Buffer,
            "command" => Self::Command,
            "option" => Self::Option,
            "tool" => Self::Tool,
            "view" => Self::View,
            "text" | "string" => Self::Text,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Path => "path",
            Self::Buffer => "buffer",
            Self::Command => "command",
            Self::Option => "option",
            Self::Tool => "tool",
            Self::View => "view",
            Self::Text => "text",
        }
    }
}

/// A command's arguments: one kind per position; with `rest`, the last
/// kind takes every argument past it (`:echo` is `text...`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Args {
    pub kinds: Vec<ArgKind>,
    pub rest: bool,
}

impl Args {
    pub fn new(kinds: &[ArgKind]) -> Self {
        Self {
            kinds: kinds.to_vec(),
            rest: false,
        }
    }

    /// `kinds`, the last of them repeated.
    pub fn rest(kinds: &[ArgKind]) -> Self {
        Self {
            kinds: kinds.to_vec(),
            rest: true,
        }
    }

    /// Parses `"path"`, `"text..."`: a trailing `...` on the last name
    /// is `rest`. An unknown name is an error naming it.
    pub fn parse(names: &[String]) -> Result<Self, String> {
        let mut kinds = Vec::new();
        let mut rest = false;
        for (i, n) in names.iter().enumerate() {
            let (n, more) = match n.strip_suffix("...") {
                Some(n) => (n, true),
                None => (n.as_str(), false),
            };
            if more && i + 1 != names.len() {
                return Err(format!("`{n}...` must be the last argument"));
            }
            rest |= more;
            kinds.push(ArgKind::parse(n).ok_or_else(|| format!("unknown argument kind `{n}`"))?);
        }
        Ok(Self { kinds, rest })
    }

    /// The names as `parse` takes them.
    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.kinds.iter().map(|k| k.name().to_string()).collect();
        if self.rest
            && let Some(last) = v.last_mut()
        {
            last.push_str("...");
        }
        v
    }

    /// The kind of argument `i` (from 0), if the command takes one.
    pub fn kind_at(&self, i: usize) -> Option<ArgKind> {
        self.kinds
            .get(i)
            .copied()
            .or_else(|| (self.rest && !self.kinds.is_empty()).then(|| *self.kinds.last().unwrap()))
    }
}

/// One condition of a command's `when`: a fact that must hold, or with
/// `!`, must not. A fact is a name — `store`, `terminal`, `lsp`,
/// `modified`, `language:oil` — that the engine answers from the view
/// ([`Editor::holds`]) or that the shell or a plugin published
/// ([`Editor::fact`]). Written `"terminal"` / `"!terminal"`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cond {
    pub fact: String,
    pub holds: bool,
}

impl Cond {
    pub fn parse(s: &str) -> Self {
        match s.strip_prefix('!') {
            Some(f) => Self {
                fact: f.to_string(),
                holds: false,
            },
            None => Self {
                fact: s.to_string(),
                holds: true,
            },
        }
    }
}

impl fmt::Display for Cond {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.holds {
            f.write_str("!")?;
        }
        f.write_str(&self.fact)
    }
}

/// What a fact is answered from: the published set, the mode, and the
/// buffer in question — one shape for the engine ([`Editor::holds`])
/// and for a snapshot of it (Lua's `kawoosh.can`), so the two agree.
#[derive(Clone, Copy, Debug, Default)]
pub struct Facts<'a> {
    pub published: Option<&'a BTreeSet<String>>,
    pub visual: bool,
    /// The buffer's name, language, whether modified, whether a file.
    pub buffer: Option<(&'a str, &'a str, bool, bool)>,
}

impl Facts<'_> {
    /// Whether `fact` holds: published, or one answered here —
    /// `visual`, `modified`, `file`, `buffer:<name>`, `language:<name>`.
    pub fn holds(&self, fact: &str) -> bool {
        if self.published.is_some_and(|p| p.contains(fact)) {
            return true;
        }
        if fact == "visual" {
            return self.visual;
        }
        let Some((name, language, modified, file)) = self.buffer else {
            return false;
        };
        match fact.split_once(':') {
            Some(("buffer", n)) => name == n,
            Some(("language", l)) => language == l,
            None => match fact {
                "modified" => modified,
                "file" => file,
                _ => false,
            },
            _ => false,
        }
    }
}

/// Everything about a command that is not its behaviour.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Spec {
    /// `buffer_delete`; `history drop` for a subcommand.
    pub name: String,
    /// The ex spellings: `bd`, `bdelete`. Resolved before anything
    /// else, so an alias is as good as the name on the command line,
    /// in a keymap, from Lua.
    pub aliases: Vec<String>,
    pub args: Args,
    /// What `!` means to it, when it means anything.
    pub bang: Option<String>,
    /// What `?` means to it, when it means anything.
    pub query: Option<String>,
    /// The conditions under which it runs, all of them.
    pub when: Vec<Cond>,
    pub kind: Kind,
    /// True for commands that read one more key as an argument.
    pub takes_char: bool,
    /// One line on what it does.
    pub doc: String,
}

impl Spec {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            ..Default::default()
        }
    }

    pub fn alias(mut self, aliases: &[&str]) -> Self {
        self.aliases.extend(aliases.iter().map(|a| a.to_string()));
        self
    }

    pub fn args(mut self, args: Args) -> Self {
        self.args = args;
        self
    }

    pub fn bang(mut self, doc: &str) -> Self {
        self.bang = Some(doc.to_string());
        self
    }

    pub fn query(mut self, doc: &str) -> Self {
        self.query = Some(doc.to_string());
        self
    }

    /// `["store", "!terminal"]`.
    pub fn when(mut self, conds: &[&str]) -> Self {
        self.when.extend(conds.iter().map(|c| Cond::parse(c)));
        self
    }

    pub fn kind(mut self, kind: Kind) -> Self {
        self.kind = kind;
        self
    }

    pub fn takes_char(mut self) -> Self {
        self.takes_char = true;
        self
    }

    pub fn doc(mut self, doc: &str) -> Self {
        self.doc = doc.to_string();
        self
    }

    /// Whether `form` is one this command has a word for.
    pub fn takes(&self, form: Form) -> bool {
        match form {
            Form::Run => true,
            Form::Bang => self.bang.is_some(),
            Form::Query => self.query.is_some(),
        }
    }

    /// Whether every condition of `when` holds; the error is the reason,
    /// for the message.
    pub fn check(&self, facts: &Facts<'_>) -> Result<(), String> {
        for c in &self.when {
            if facts.holds(&c.fact) != c.holds {
                return Err(match c.holds {
                    true => format!("{} needs {}", self.name, c.fact),
                    false => format!("{} is not for {}", self.name, c.fact),
                });
            }
        }
        Ok(())
    }

    /// `history` for `history drop`; none for a top-level command.
    pub fn parent(&self) -> Option<&str> {
        self.name.rsplit_once(' ').map(|(p, _)| p)
    }

    /// `drop` for `history drop`; the name for a top-level command.
    pub fn word(&self) -> &str {
        self.name
            .rsplit_once(' ')
            .map(|(_, w)| w)
            .unwrap_or(&self.name)
    }
}

/// A command's behaviour, on the host it acts on: the engine's on
/// [`Editor`], the shell's on the shell.
pub trait Command<H = Editor> {
    fn spec(&self) -> Spec;
    fn run(&self, host: &mut H, ctx: &Ctx);
}

/// A command's body as a closure.
pub type Body<H> = Rc<dyn Fn(&mut H, &Ctx)>;

/// A command out of a spec and a closure — the way most are written.
pub struct FnCommand<H> {
    spec: Spec,
    body: Body<H>,
}

impl<H> FnCommand<H> {
    pub fn new(spec: Spec, body: impl Fn(&mut H, &Ctx) + 'static) -> Self {
        Self {
            spec,
            body: Rc::new(body),
        }
    }
}

impl<H> Clone for FnCommand<H> {
    fn clone(&self) -> Self {
        Self {
            spec: self.spec.clone(),
            body: self.body.clone(),
        }
    }
}

impl<H> Command<H> for FnCommand<H> {
    fn spec(&self) -> Spec {
        self.spec.clone()
    }

    fn run(&self, host: &mut H, ctx: &Ctx) {
        (self.body)(host, ctx)
    }
}

/// A command line taken apart: the command it names (aliases resolved,
/// subcommand words consumed), the form, and what is left as arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invocation {
    pub name: String,
    pub form: Form,
    pub args: Vec<String>,
}

/// The registry: every command's spec, and its body when it runs here.
/// A command the shell runs has its spec here and no body; running it
/// is an effect.
#[derive(Default)]
pub struct Registry {
    entries: std::collections::HashMap<String, Entry>,
    aliases: std::collections::HashMap<String, String>,
    /// What the shell and plugins say holds: `store`, `terminal`, `lsp`.
    pub facts: BTreeSet<String>,
    /// Bumped on every add or declare, so a reader (Lua's snapshot)
    /// copies the specs only when they changed.
    version: u64,
}

#[derive(Clone)]
struct Entry {
    spec: Spec,
    body: Option<Rc<dyn Command<Editor>>>,
}

impl Registry {
    /// Adds `cmd`, its spec and body; a spec declared before for the
    /// name gives way.
    pub fn add(&mut self, cmd: impl Command<Editor> + 'static) {
        let spec = cmd.spec();
        self.put(spec, Some(Rc::new(cmd)));
    }

    /// Declares a command that runs elsewhere — the shell's: its spec
    /// is known here, resolved and completed like any other, and
    /// running it is [`crate::Effect::Shell`]. A declaration never
    /// takes the body from a command that has one.
    pub fn declare(&mut self, spec: Spec) {
        let body = self.entries.get(&spec.name).and_then(|e| e.body.clone());
        self.put(spec, body);
    }

    fn put(&mut self, spec: Spec, body: Option<Rc<dyn Command<Editor>>>) {
        if let Some(old) = self.entries.get(&spec.name) {
            for a in &old.spec.aliases {
                self.aliases.remove(a);
            }
        }
        for a in &spec.aliases {
            self.aliases.insert(a.clone(), spec.name.clone());
        }
        self.entries.insert(spec.name.clone(), Entry { spec, body });
        self.version += 1;
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn spec(&self, name: &str) -> Option<&Spec> {
        self.entries.get(name).map(|e| &e.spec)
    }

    pub fn spec_mut(&mut self, name: &str) -> Option<&mut Spec> {
        self.entries.get_mut(name).map(|e| &mut e.spec)
    }

    pub(crate) fn body(&self, name: &str) -> Option<Rc<dyn Command<Editor>>> {
        self.entries.get(name).and_then(|e| e.body.clone())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    /// Every spec, by name.
    pub fn specs(&self) -> Vec<&Spec> {
        let mut v: Vec<&Spec> = self.entries.values().map(|e| &e.spec).collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }

    /// Every top-level name, sorted: a command's, or the first word of
    /// a subcommand's (`tab` for `tab new`, which needs no `tab`).
    pub fn names(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self
            .entries
            .keys()
            .map(|n| n.split(' ').next().unwrap_or(n))
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Whether `name` is a command or a word on the way to one.
    pub fn is_name(&self, name: &str) -> bool {
        self.entries.contains_key(name) || self.has_children(name)
    }

    fn has_children(&self, name: &str) -> bool {
        let prefix = format!("{name} ");
        self.entries.keys().any(|n| n.starts_with(&prefix))
    }

    /// Every alias, sorted.
    pub fn alias_names(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.aliases.keys().map(String::as_str).collect();
        v.sort_unstable();
        v
    }

    /// The name an alias stands for, or `name` itself.
    pub fn canonical<'a>(&'a self, name: &'a str) -> &'a str {
        self.aliases.get(name).map(String::as_str).unwrap_or(name)
    }

    /// The words that follow `name`, sorted, each once: `char`, `to`,
    /// `word` under `delete` — a subcommand's, or the next word on the
    /// way to one (`to` of `delete to end`).
    pub fn subcommands(&self, name: &str) -> Vec<&str> {
        let prefix = format!("{name} ");
        let mut v: Vec<&str> = self
            .entries
            .keys()
            .filter_map(|n| n.strip_prefix(&prefix))
            .map(|rest| rest.split(' ').next().unwrap_or(rest))
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Takes `name` (an alias, perhaps, with `!` or `?` on its end)
    /// and `args` apart: the alias is resolved, then each leading
    /// argument that names a subcommand, or a word on the way to one,
    /// is consumed into the name — `history` + `[drop, k]` is `history
    /// drop` + `[k]`, `delete` + `[to, end]` is `delete to end` — a
    /// marker on any of those words setting the form (`:history clear!`
    /// and `:history! clear` alike). A name nothing is known by stays
    /// as written, for the shell to answer.
    pub fn resolve(&self, name: &str, args: &[String]) -> Invocation {
        let (head, mut form) = Form::split(name);
        let mut name = self.canonical(head).to_string();
        let mut i = 0;
        while let Some(word) = args.get(i) {
            let (w, f) = Form::split(word);
            let sub = format!("{name} {w}");
            if !self.is_name(&sub) {
                break;
            }
            name = sub;
            if f != Form::Run {
                form = f;
            }
            i += 1;
        }
        Invocation {
            name,
            form,
            args: args[i..].to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg() -> Registry {
        let mut r = Registry::default();
        r.declare(
            Spec::new("history")
                .alias(&["hist"])
                .bang("discard held rows too"),
        );
        r.declare(Spec::new("history drop").args(Args::new(&[ArgKind::Text])));
        r.declare(Spec::new("history clear").bang("held rows too"));
        r.declare(Spec::new("quit").alias(&["q"]).bang("discard changes"));
        r
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn resolves_aliases_forms_and_subcommands() {
        let r = reg();
        assert_eq!(
            r.resolve("q!", &[]),
            Invocation {
                name: "quit".into(),
                form: Form::Bang,
                args: vec![]
            }
        );
        assert_eq!(
            r.resolve("hist", &s(&["drop", "file:/a"])),
            Invocation {
                name: "history drop".into(),
                form: Form::Run,
                args: s(&["file:/a"])
            }
        );
        // The marker on the subcommand's word, or on the parent's.
        assert_eq!(r.resolve("history", &s(&["clear!"])).form, Form::Bang);
        assert_eq!(r.resolve("history!", &s(&["clear"])).form, Form::Bang);
        assert_eq!(r.resolve("history!", &s(&["clear"])).name, "history clear");
        // A word that is no subcommand is an argument.
        assert_eq!(
            r.resolve("history", &s(&["nope", "x"])).args,
            s(&["nope", "x"])
        );
        // Unknown stays as written.
        assert_eq!(r.resolve("zap?", &s(&["a"])).name, "zap");
        assert_eq!(r.resolve("zap?", &s(&["a"])).form, Form::Query);
        // A bare marker is not a name.
        assert_eq!(Form::split("!"), ("!", Form::Run));
    }

    #[test]
    fn subcommands_and_names() {
        let r = reg();
        assert_eq!(r.subcommands("history"), ["clear", "drop"]);
        assert_eq!(r.names(), ["history", "quit"]);
        // A word on the way to a subcommand needs no command of its
        // own: it is walked, listed under its parent, and lists on.
        let mut r2 = Registry::default();
        r2.declare(Spec::new("delete to end"));
        r2.declare(Spec::new("delete char"));
        assert_eq!(r2.names(), ["delete"]);
        assert_eq!(r2.subcommands("delete"), ["char", "to"]);
        assert_eq!(r2.subcommands("delete to"), ["end"]);
        assert_eq!(
            r2.resolve("delete", &s(&["to", "end", "x"])),
            Invocation {
                name: "delete to end".into(),
                form: Form::Run,
                args: s(&["x"])
            }
        );
        assert!(r2.is_name("delete to") && !r2.spec("delete to").is_some());
        assert_eq!(r.names(), ["history", "quit"]);
        assert_eq!(r.alias_names(), ["hist", "q"]);
        assert_eq!(r.spec("history drop").unwrap().parent(), Some("history"));
        // Re-declaring a name drops its old aliases.
        let mut r = r;
        r.declare(Spec::new("quit").alias(&["qq"]));
        assert_eq!(r.canonical("q"), "q");
        assert_eq!(r.canonical("qq"), "quit");
    }

    #[test]
    fn conds_and_args_round_trip() {
        assert_eq!(Cond::parse("!terminal").to_string(), "!terminal");
        assert!(Cond::parse("store").holds);
        let a = Args::parse(&s(&["path", "text..."])).unwrap();
        assert_eq!(a.names(), s(&["path", "text..."]));
        assert!(Spec::new("x").bang("b").takes(Form::Bang));
        assert!(!Spec::new("x").takes(Form::Query));
    }
}
