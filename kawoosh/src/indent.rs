//! The indenter (docs/design/indent.md Decisions 3 and 4): the ts
//! thread's trees, each kept with the text it was parsed from and the
//! grammar that read it, brought up to the buffer's text on the spot
//! when the buffer has moved past them — the `<CR>` typed before the
//! thread answered — and read by `kawoosh_systems::indent`. Each
//! language's grammar is kept too, as the thread's answers bring it, so
//! a buffer never shown — one a `:wa` formats — is parsed whole when
//! asked about.
//!
//! The same trees answer the syntax text objects (docs/design/nodes.md
//! Decision 9), read by `kawoosh_systems::textobjects`: a `daf` typed
//! right after an edit reads the text as it is, as a `<CR>` does. And
//! `%` on a block's keyword (`kawoosh_systems::blocks`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use kawoosh_doc::{Buffer, BufferId, Version};
use kawoosh_editor::{IndentUnit, SyntaxObject};
use kawoosh_languages::Grammar;
use kawoosh_systems::blocks;
use kawoosh_systems::indent::{self, Unit};
use kawoosh_systems::textobjects;
use kawoosh_systems::ts::{self, Parse};
use tree_sitter::{Parser, Tree};

/// A buffer's tree and what it was read from.
struct Kept {
    language: String,
    version: Version,
    text: text_buffer::Buffer,
    tree: Tree,
    grammar: Arc<Grammar>,
}

#[derive(Default)]
struct Inner {
    kept: HashMap<BufferId, Kept>,
    /// Each language's grammar with an indent, text-object or
    /// injection query, by its name.
    grammars: HashMap<String, Arc<Grammar>>,
    /// Every name a language goes by — its own, its aliases — to its
    /// own, as the registry has them ([`Trees::set_names`]): what an
    /// injection query says (`js`, `sh`) read as the language it means.
    names: HashMap<String, String>,
}

/// The kept trees, shared between the shell (which files the thread's
/// answers) and the editor's indenter (which reads and catches them up).
#[derive(Clone, Default)]
pub struct Trees(Rc<RefCell<Inner>>);

impl Trees {
    /// A thread's answer for `id`, whose language is `language`: its
    /// tree kept, when its grammar has an indent or a text-object
    /// query; else what was kept goes.
    pub fn answered(
        &self,
        id: BufferId,
        language: &str,
        version: Version,
        tree: Option<&Tree>,
        parse: Option<Parse>,
    ) {
        let mut inner = self.0.borrow_mut();
        match (tree, parse) {
            (Some(tree), Some(p)) => {
                inner
                    .grammars
                    .insert(language.to_string(), p.grammar.clone());
                inner.kept.insert(
                    id,
                    Kept {
                        language: language.to_string(),
                        version,
                        text: p.text,
                        tree: tree.clone(),
                        grammar: p.grammar,
                    },
                );
            }
            _ => {
                inner.kept.remove(&id);
            }
        }
    }

    /// The registry's names, each to the language it means: called
    /// when the registry is built and whenever it gains a language.
    pub fn set_names(&self, names: impl IntoIterator<Item = (String, String)>) {
        let mut inner = self.0.borrow_mut();
        inner.names = names.into_iter().collect();
    }

    /// The language of the injection holding byte `at` of buffer `id`
    /// — a `<script>`'s javascript, a fence's rust — the innermost
    /// the grammars at hand can read, named as the registry names it
    /// (an alias the query used resolved; one the registry has no
    /// language for kept as said). `None` where there is no tree, no
    /// injection query, or no injection there: the buffer's own
    /// language, then.
    fn language_at(
        &self,
        parser: &mut Parser,
        id: BufferId,
        buf: &Buffer,
        at: usize,
    ) -> Option<String> {
        let (tree, text, g) = self.current(parser, id, buf)?;
        let inner = self.0.borrow();
        let canonical = |name: &str| {
            inner
                .names
                .get(name)
                .cloned()
                .unwrap_or_else(|| name.to_string())
        };
        let mut found: Option<String> = None;
        let (mut tree, mut g) = (tree, g);
        for _ in 0..ts::INJECTION_DEPTH {
            let Some(inj) = &g.injections else { break };
            let Some((name, range)) = ts::injection_at(inj, tree.root_node(), &text, at) else {
                break;
            };
            let name = canonical(&name);
            found = Some(name.clone());
            // Deeper only where this language's grammar has been seen
            // and injects in turn.
            let Some(next) = inner.grammars.get(&name).cloned() else {
                break;
            };
            if next.injections.is_none() {
                break;
            }
            let Some(sub) = ts::parse_range(parser, &next, &text, range) else {
                break;
            };
            tree = sub;
            g = next;
        }
        found
    }

    /// Only the buffers `live` says are.
    pub fn retain(&self, live: impl Fn(BufferId) -> bool) {
        self.0.borrow_mut().kept.retain(|id, _| live(*id));
    }

    /// Whether buffer `id`, in `language`, can be indented: a tree kept
    /// for it, or its language's grammar to parse it with, and an
    /// indent query in the grammar.
    pub fn serves(&self, id: BufferId, language: &str) -> bool {
        let inner = self.0.borrow();
        inner
            .kept
            .get(&id)
            .filter(|k| k.language == language)
            .map(|k| &k.grammar)
            .or_else(|| inner.grammars.get(language))
            .is_some_and(|g| g.indents.is_some())
    }

    /// The editor's indenter over these trees.
    pub fn indenter(&self) -> Indenter {
        Indenter {
            trees: self.clone(),
            parser: Parser::new(),
        }
    }

    /// The editor's syntax text objects over these trees.
    pub fn objects(&self) -> Objects {
        Objects {
            trees: self.clone(),
            parser: Parser::new(),
        }
    }

    /// `id`'s tree as of `buf`'s text: kept, or caught up — told the
    /// journal's edits since and reparsed, a whole parse when the
    /// journal no longer reaches back — and kept so.
    fn current(
        &self,
        parser: &mut Parser,
        id: BufferId,
        buf: &Buffer,
    ) -> Option<(Tree, text_buffer::Buffer, Arc<Grammar>)> {
        let mut inner = self.0.borrow_mut();
        let Inner { kept, grammars, .. } = &mut *inner;
        let language = &*buf.language;
        // None kept, or kept in another language: parsed whole, when
        // the language's grammar is known.
        if kept.get(&id).is_none_or(|k| k.language != language) {
            let grammar = grammars.get(language)?.clone();
            let snap = buf.snapshot();
            let tree = ts::reparse(parser, &grammar.language, &snap.text, None)?;
            kept.insert(
                id,
                Kept {
                    language: language.to_string(),
                    version: snap.version,
                    text: snap.text,
                    tree,
                    grammar,
                },
            );
        }
        let k = kept.get_mut(&id)?;
        if k.version != buf.version() {
            let snap = buf.snapshot();
            let edits: Option<Vec<_>> = buf
                .journal()
                .edits_since(k.version)
                .ok()
                .map(|it| it.cloned().collect());
            let old = edits.as_deref().map(|e| (k.tree.clone(), &k.text, e));
            let tree = ts::reparse(parser, &k.grammar.language, &snap.text, old)?;
            k.version = snap.version;
            k.text = snap.text;
            k.tree = tree;
        }
        Some((k.tree.clone(), k.text.clone(), k.grammar.clone()))
    }
}

pub struct Indenter {
    trees: Trees,
    parser: Parser,
}

/// The syntax text objects (`kawoosh_editor::SyntaxObjects`): the
/// buffer's tree, caught up, and its grammar's text-object query.
pub struct Objects {
    trees: Trees,
    parser: Parser,
}

impl kawoosh_editor::SyntaxObjects for Objects {
    fn find(
        &mut self,
        id: BufferId,
        buf: &Buffer,
        object: &str,
        within: Range<usize>,
    ) -> Result<Vec<SyntaxObject>, String> {
        let language = &*buf.language;
        let Some((tree, text, g)) = self.trees.current(&mut self.parser, id, buf) else {
            return Err(format!("no syntax tree for {language}"));
        };
        let Some(t) = &g.textobjects else {
            return Err(format!("no syntax text objects for {language}"));
        };
        Ok(textobjects::find(t, &tree, &text, object, within)
            .into_iter()
            .map(|f| SyntaxObject {
                around: f.around,
                inside: f.inside,
            })
            .collect())
    }

    fn partner(
        &mut self,
        id: BufferId,
        buf: &Buffer,
        at: usize,
    ) -> Option<(Range<usize>, Range<usize>)> {
        let (tree, _, _) = self.trees.current(&mut self.parser, id, buf)?;
        blocks::partner(&tree, at)
    }

    fn language_at(&mut self, id: BufferId, buf: &Buffer, at: usize) -> Option<String> {
        self.trees.language_at(&mut self.parser, id, buf, at)
    }
}

fn unit(u: &IndentUnit) -> Unit {
    Unit {
        text: u.text.clone(),
        width: u.width,
        tabstop: u.tabstop,
    }
}

impl kawoosh_editor::Indenter for Indenter {
    fn new_line(
        &mut self,
        id: BufferId,
        buf: &Buffer,
        at: usize,
        u: &IndentUnit,
    ) -> Option<String> {
        let (tree, text, g) = self.trees.current(&mut self.parser, id, buf)?;
        indent::for_new_line(g.indents.as_ref()?, &tree, &text, at, &unit(u))
    }

    fn lines(
        &mut self,
        id: BufferId,
        buf: &Buffer,
        lines: Range<usize>,
        u: &IndentUnit,
    ) -> Option<Vec<Option<String>>> {
        let (tree, text, g) = self.trees.current(&mut self.parser, id, buf)?;
        Some(indent::for_lines(
            g.indents.as_ref()?,
            &tree,
            &text,
            lines,
            &unit(u),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kawoosh_editor::{Editor, Indenter as _};

    /// A tree the thread answered for an older text, asked about a line
    /// break in the new one: told the journal's edits and reparsed, so
    /// what was typed since counts (Decision 4).
    #[test]
    fn a_tree_behind_the_text_catches_up() {
        let l = kawoosh_languages::LANGUAGES
            .iter()
            .find(|l| l.name == "python")
            .unwrap();
        let grammar = Arc::new((l.grammar.unwrap())().unwrap());
        let mut ed = Editor::new();
        let mut b = Buffer::new("a.py", "x = 1\n");
        b.language = "python".into();
        let id = ed.add_buffer(b);
        let buf = &ed.buffers[id];
        let mut parser = Parser::new();
        parser.set_language(&grammar.language).unwrap();
        let tree = parser.parse("x = 1\n", None).unwrap();
        let trees = Trees::default();
        trees.answered(
            id,
            "python",
            buf.version(),
            Some(&tree),
            Some(Parse {
                text: buf.snapshot().text,
                grammar,
            }),
        );
        let buf = &mut ed.buffers[id];
        buf.replace(0..0, "def f():\n    if a:\n");
        let unit = IndentUnit {
            text: "    ".into(),
            width: 4,
            tabstop: 4,
        };
        let mut ind = trees.indenter();
        let at = "def f():\n    if a:".len();
        assert_eq!(
            ind.new_line(id, buf, at, &unit).as_deref(),
            Some("        ")
        );
        // Kept caught up: a second edit reparses from there.
        buf.replace(at..at, "\n        return 1");
        let at = buf.len() - "\nx = 1\n".len();
        assert_eq!(ind.new_line(id, buf, at, &unit).as_deref(), Some("    "));
        // A buffer the thread never answered for, in a language it has:
        // parsed whole when asked.
        let mut b = Buffer::new("b.py", "x = [\n1,\n]\n");
        b.language = "python".into();
        let other = ed.add_buffer(b);
        assert!(trees.serves(other, "python"));
        assert_eq!(
            ind.lines(other, &ed.buffers[other], 0..3, &unit),
            Some(vec![
                Some(String::new()),
                Some("    ".into()),
                Some(String::new())
            ])
        );
        // One in a language never seen has no answer.
        let third = ed.add_buffer(Buffer::new("c.txt", "x"));
        assert!(!trees.serves(third, "text"));
        assert_eq!(ind.new_line(third, &ed.buffers[third], 0, &unit), None);
    }
}
