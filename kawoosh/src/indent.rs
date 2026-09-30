//! The indenter (docs/design/indent.md Decisions 3 and 4): the ts
//! thread's trees, each kept with the text it was parsed from and the
//! grammar that read it, brought up to the buffer's text on the spot
//! when the buffer has moved past them — the `<CR>` typed before the
//! thread answered — and read by `kawoosh_systems::indent`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use kawoosh_doc::{Buffer, BufferId, Version};
use kawoosh_editor::IndentUnit;
use kawoosh_languages::Grammar;
use kawoosh_systems::indent::{self, Unit};
use kawoosh_systems::ts::{self, Parse};
use tree_sitter::{Parser, Tree};

/// A buffer's tree and what it was read from.
struct Kept {
    version: Version,
    text: text_buffer::Buffer,
    tree: Tree,
    grammar: Arc<Grammar>,
}

/// The kept trees, shared between the shell (which files the thread's
/// answers) and the editor's indenter (which reads and catches them up).
#[derive(Clone, Default)]
pub struct Trees(Rc<RefCell<HashMap<BufferId, Kept>>>);

impl Trees {
    /// A thread's answer for `id`: its tree kept, when its grammar has
    /// an indent query; else what was kept goes.
    pub fn answered(
        &self,
        id: BufferId,
        version: Version,
        tree: Option<&Tree>,
        parse: Option<Parse>,
    ) {
        let mut kept = self.0.borrow_mut();
        match (tree, parse) {
            (Some(tree), Some(p)) => {
                kept.insert(
                    id,
                    Kept {
                        version,
                        text: p.text,
                        tree: tree.clone(),
                        grammar: p.grammar,
                    },
                );
            }
            _ => {
                kept.remove(&id);
            }
        }
    }

    /// Only the buffers `live` says are.
    pub fn retain(&self, live: impl Fn(BufferId) -> bool) {
        self.0.borrow_mut().retain(|id, _| live(*id));
    }

    /// The editor's indenter over these trees.
    pub fn indenter(&self) -> Indenter {
        Indenter {
            trees: self.clone(),
            parser: Parser::new(),
        }
    }
}

pub struct Indenter {
    trees: Trees,
    parser: Parser,
}

impl Indenter {
    /// `id`'s tree as of `buf`'s text: kept, or caught up — told the
    /// journal's edits since and reparsed, a whole parse when the
    /// journal no longer reaches back — and kept so.
    fn current(
        &mut self,
        id: BufferId,
        buf: &Buffer,
    ) -> Option<(Tree, text_buffer::Buffer, Arc<Grammar>)> {
        let mut kept = self.trees.0.borrow_mut();
        let k = kept.get_mut(&id)?;
        if k.version != buf.version() {
            let snap = buf.snapshot();
            let edits: Option<Vec<_>> = buf
                .journal()
                .edits_since(k.version)
                .ok()
                .map(|it| it.cloned().collect());
            let old = edits.as_deref().map(|e| (k.tree.clone(), &k.text, e));
            let tree = ts::reparse(&mut self.parser, &k.grammar.language, &snap.text, old)?;
            k.version = snap.version;
            k.text = snap.text;
            k.tree = tree;
        }
        Some((k.tree.clone(), k.text.clone(), k.grammar.clone()))
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
        let (tree, text, g) = self.current(id, buf)?;
        indent::for_new_line(g.indents.as_ref()?, &tree, &text, at, &unit(u))
    }

    fn lines(
        &mut self,
        id: BufferId,
        buf: &Buffer,
        lines: Range<usize>,
        u: &IndentUnit,
    ) -> Option<Vec<Option<String>>> {
        let (tree, text, g) = self.current(id, buf)?;
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
        let id = ed.add_buffer(Buffer::new("a.py", "x = 1\n"));
        let buf = &ed.buffers[id];
        let mut parser = Parser::new();
        parser.set_language(&grammar.language).unwrap();
        let tree = parser.parse("x = 1\n", None).unwrap();
        let trees = Trees::default();
        trees.answered(
            id,
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
        // A buffer with no tree kept has no answer.
        let other = ed.add_buffer(Buffer::new("b.py", ""));
        assert_eq!(ind.new_line(other, &ed.buffers[other], 0, &unit), None);
    }
}
