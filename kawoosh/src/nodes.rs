//! Selections by the syntax tree, helix's way: `<A-o>` selects the node
//! under the caret, then its parent on each press; `<A-i>` goes back
//! in; `<A-n>` / `<A-p>` move to the next and previous sibling. The
//! tree is the one `ts` last answered for the buffer (`inspector.trees`),
//! and it must be the text's own version — an answer is a frame or two
//! behind a keystroke, so a press right after typing says so and does
//! nothing rather than selecting by stale offsets.
//!
//! Every selection is walked (mvp.md Decision 4): each finds its own
//! node. The selections `<A-o>` replaced are kept on a stack per view
//! so `<A-i>` returns to exactly them; with nothing on the stack it
//! takes the node's first named child.

use std::collections::HashMap;

use kawoosh_editor::{Mode, Selection, Selections, Spec, ViewId};
use tree_sitter::Node;

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};

#[derive(Default)]
pub struct NodeSelect {
    /// What each view's selections were before each `<A-o>`, newest last.
    pub stack: HashMap<ViewId, Vec<Selections>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Parent,
    Child,
    Next,
    Prev,
}

/// A selection over `r` as a visual one lies, the head on the last
/// character.
fn over(buf: &kawoosh_doc::Buffer, r: std::ops::Range<usize>) -> Selection {
    let head = if r.end > r.start {
        buf.prev_char(r.end)
    } else {
        r.start
    };
    Selection::new(r.start, head)
}

/// Up from `node` to the first ancestor spanning more than `r`.
fn wider<'t>(mut node: Node<'t>, r: &std::ops::Range<usize>) -> Node<'t> {
    while node.start_byte() == r.start && node.end_byte() == r.end {
        match node.parent() {
            Some(p) => node = p,
            None => break,
        }
    }
    node
}

impl Kawoosh {
    /// The tree of `view`'s buffer at the text as it is, or why not.
    fn tree_for(&self, view: ViewId) -> Result<tree_sitter::Tree, String> {
        let id = self.ed.views[view].buffer;
        let buf = &self.ed.buffers[id];
        match self.inspector.trees.get(&id) {
            Some((v, t)) if *v == buf.version() => Ok(t.clone()),
            Some(_) => Err("the syntax tree is behind the text: again in a moment".into()),
            None => Err(format!("no syntax tree for {}", buf.name)),
        }
    }

    fn node_step(&mut self, view: ViewId, step: Step) {
        let tree = match self.tree_for(view) {
            Ok(t) => t,
            Err(e) => {
                self.ed.message = e;
                return;
            }
        };
        let before = self.ed.views[view].sels.clone();
        if step == Step::Child
            && let Some(prev) = self.nodes.stack.get_mut(&view).and_then(Vec::pop)
        {
            self.ed.views[view].sels = prev;
            self.ed.set_mode(view, Mode::Visual);
            return;
        }
        let id = self.ed.views[view].buffer;
        let visual = self.ed.mode(view) == Mode::Visual;
        let buf = &self.ed.buffers[id];
        let root = tree.root_node();
        let mut items = Vec::with_capacity(before.len());
        for s in before.iter() {
            // The selection's bytes — the caret's character when bare.
            let r = if visual {
                s.start()..buf.next_char(s.end())
            } else {
                s.head..buf.next_char(s.head)
            };
            let Some(node) = root.named_descendant_for_byte_range(r.start, r.end) else {
                items.push(*s);
                continue;
            };
            let target = match step {
                // A bare caret takes the node under it; a selection
                // that is a node already takes the one around it.
                Step::Parent if visual => wider(node, &r),
                Step::Parent => node,
                Step::Child => node.named_child(0).unwrap_or(node),
                Step::Next => node.next_named_sibling().unwrap_or(node),
                Step::Prev => node.prev_named_sibling().unwrap_or(node),
            };
            items.push(over(buf, target.byte_range()));
        }
        let mut sels = Selections {
            items,
            primary: before.primary,
        };
        sels.normalize();
        // A one-character node under a bare caret is the caret's own
        // range: the press still enters visual mode over it.
        if sels == before && visual {
            self.ed.message = match step {
                Step::Parent => "the whole tree is selected".into(),
                Step::Child => "no node inside".into(),
                Step::Next | Step::Prev => "no sibling that way".into(),
            };
            return;
        }
        if step == Step::Parent {
            self.nodes.stack.entry(view).or_default().push(before);
        }
        self.ed.views[view].sels = sels;
        self.ed.set_mode(view, Mode::Visual);
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    fn step(name: &str, doc: &str, step: Step) -> ShellCommand {
        cmd(Spec::new(name).when(&["editor"]).doc(doc), move |k, _| {
            if let Some(v) = k.focused_view() {
                k.node_step(v, step);
            }
        })
    }
    vec![
        step(
            "select node",
            "select the syntax node under the caret, then the one around it (<A-o>)",
            Step::Parent,
        ),
        step(
            "select node child",
            "back to the selection before `select node`, or the node's first child (<A-i>)",
            Step::Child,
        ),
        step(
            "select node next",
            "select the next sibling node (<A-n>)",
            Step::Next,
        ),
        step(
            "select node prev",
            "select the previous sibling node (<A-p>)",
            Step::Prev,
        ),
    ]
}
