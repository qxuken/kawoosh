//! The tree-sitter system: syntax highlighting as a provider (mvp.md
//! milestone 6, core.md decision 2).
//!
//! A worker thread receives `(snapshot, capture map)` jobs, parses off the
//! UI thread — snapshots are `Send` by construction since the Arc migration
//! — and answers with a `kawoosh_core::Update` covering the whole buffer at
//! the snapshot's version. The journal on the receiving side handles
//! staleness; this system never needs to know whether the user kept typing.
//!
//! MVP scope: full reparse per job (a fresh parse of editor-sized files is
//! low milliseconds), Rust only. Incremental parsing via old trees and
//! damage spans slots in behind the same message types.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender, unbounded};
use kawoosh_core::{BufferId, HighlightId, LayerId, Snapshot, Update};
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    Rust,
}

impl Language {
    /// Detect from a file path; the map grows with the grammar list.
    pub fn detect(path: &std::path::Path) -> Option<Self> {
        match path.extension()?.to_str()? {
            "rs" => Some(Language::Rust),
            _ => None,
        }
    }
}

pub struct Job {
    pub buffer: BufferId,
    pub layer: LayerId,
    pub language: Language,
    pub snapshot: Snapshot,
    /// Capture name → highlight definition, shared with the app's theme.
    pub map: Arc<HashMap<String, HighlightId>>,
}

pub struct Result_ {
    pub buffer: BufferId,
    pub update: Update,
}

/// Start the worker. `wake` is called after each result is queued so the
/// event loop can drain without polling.
pub fn spawn(wake: impl Fn() + Send + 'static) -> (Sender<Job>, Receiver<Result_>) {
    let (job_tx, job_rx) = unbounded::<Job>();
    let (result_tx, result_rx) = unbounded::<Result_>();

    std::thread::spawn(move || {
        let mut parser = Parser::new();
        let rust: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        let rust_query = match Query::new(&rust, tree_sitter_rust::HIGHLIGHTS_QUERY) {
            Ok(query) => query,
            Err(err) => {
                log::error!("rust highlight query failed to compile: {err}");
                return;
            }
        };

        while let Ok(job) = job_rx.recv() {
            let (language, query) = match job.language {
                Language::Rust => (&rust, &rust_query),
            };
            if parser.set_language(language).is_err() {
                continue;
            }

            let text = job.snapshot.collect_range(0..job.snapshot.len());
            let Some(tree) = parser.parse(&text, None) else {
                continue;
            };

            let runs = capture_runs(query, tree.root_node(), &text, &job.map);
            let update = Update {
                layer: job.layer,
                version: job.snapshot.version(),
                span: 0..text.len(),
                runs,
            };
            let _ = result_tx.send(Result_ {
                buffer: job.buffer,
                update,
            });
            wake();
        }
    });

    (job_tx, result_rx)
}

/// Run the highlight query and produce non-overlapping, sorted runs.
/// On nested captures the more specific (later, inner) capture wins.
fn capture_runs(
    query: &Query,
    root: tree_sitter::Node,
    text: &[u8],
    map: &HashMap<String, HighlightId>,
) -> Vec<(Range<usize>, Option<HighlightId>)> {
    let names = query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut raw: Vec<(Range<usize>, HighlightId)> = Vec::new();

    let mut captures = cursor.captures(query, root, text);
    while let Some((matched, index)) = captures.next() {
        let capture = matched.captures[*index];
        let name = names[capture.index as usize];
        let Some(id) = lookup(map, name) else {
            continue;
        };
        let range = capture.node.byte_range();
        if range.is_empty() {
            continue;
        }
        raw.push((range, id));
    }

    // Inner captures override outer ones: sort by (start asc, end desc) so a
    // containing capture comes first, then let later, narrower ranges split
    // it. A simple sweep with a stack of active outers does the splitting.
    raw.sort_by(|a, b| {
        (a.0.start, std::cmp::Reverse(a.0.end)).cmp(&(b.0.start, std::cmp::Reverse(b.0.end)))
    });

    let mut out: Vec<(Range<usize>, Option<HighlightId>)> = Vec::new();
    let mut stack: Vec<(Range<usize>, HighlightId)> = Vec::new();
    let mut cursor_at = 0usize;

    let mut emit = |from: usize,
                    to: usize,
                    id: Option<HighlightId>,
                    out: &mut Vec<(Range<usize>, Option<HighlightId>)>| {
        if from < to {
            out.push((from..to, id));
        }
    };

    for (range, id) in raw {
        // Close finished outers.
        while let Some((top, top_id)) = stack.last().cloned() {
            if top.end <= range.start {
                emit(cursor_at.max(top.start), top.end, Some(top_id), &mut out);
                cursor_at = cursor_at.max(top.end);
                stack.pop();
            } else {
                break;
            }
        }
        // Emit the visible part of the current outer before this inner starts.
        if let Some((_, top_id)) = stack.last().cloned() {
            emit(cursor_at, range.start, Some(top_id), &mut out);
        }
        cursor_at = cursor_at.max(range.start);
        stack.push((range, id));
    }
    while let Some((top, top_id)) = stack.pop() {
        emit(cursor_at.max(top.start), top.end, Some(top_id), &mut out);
        cursor_at = cursor_at.max(top.end);
    }

    out
}

/// `constant.builtin` falls back to `constant`, then nothing.
fn lookup(map: &HashMap<String, HighlightId>, name: &str) -> Option<HighlightId> {
    if let Some(id) = map.get(name) {
        return Some(*id);
    }
    let head = name.split('.').next()?;
    map.get(head).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kawoosh_core::{Core, Highlight, LayerSpec};

    fn theme(core: &mut Core) -> Arc<HashMap<String, HighlightId>> {
        let mut map = HashMap::new();
        for name in ["keyword", "function", "string", "comment", "type"] {
            map.insert(
                name.to_string(),
                core.create_highlight(Highlight::default()),
            );
        }
        Arc::new(map)
    }

    #[test]
    fn highlights_rust_through_the_provider_path() {
        let mut core = Core::default();
        let layer = core.create_layer(LayerSpec::derived("syntax"));
        let map = theme(&mut core);

        let buffer = core.create_buffer();
        core.set_text(buffer, b"fn main() { let s = \"hi\"; } // end\n");
        let snapshot = core.buffer(buffer).unwrap().snapshot();

        let (jobs, results) = spawn(|| {});
        jobs.send(Job {
            buffer,
            layer,
            language: Language::Rust,
            snapshot,
            map: map.clone(),
        })
        .unwrap();

        let result = results
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        let applied = core.apply(buffer, result.update).unwrap();
        assert!(!applied.transformed);

        // "fn" is a keyword: offset 0..2 carries the keyword highlight.
        let buf = core.buffer(buffer).unwrap();
        let keyword = map["keyword"];
        let at0 = buf.highlights_at(0);
        assert!(
            at0.iter().any(|(_, h)| *h == keyword),
            "expected keyword at 0, got {at0:?}"
        );

        // The string literal region carries the string highlight.
        let string = map["string"];
        let at_str = buf.highlights_at(21);
        assert!(
            at_str.iter().any(|(_, h)| *h == string),
            "expected string at 21, got {at_str:?}"
        );
    }

    #[test]
    fn stale_snapshot_is_transformed_by_the_journal() {
        let mut core = Core::default();
        let layer = core.create_layer(LayerSpec::derived("syntax"));
        let map = theme(&mut core);

        let buffer = core.create_buffer();
        core.set_text(buffer, b"fn main() {}\n");
        let snapshot = core.buffer(buffer).unwrap().snapshot();

        // Edit after the snapshot: prepend a comment line.
        core.insert(buffer, 0, b"// c\n");

        let (jobs, results) = spawn(|| {});
        jobs.send(Job {
            buffer,
            layer,
            language: Language::Rust,
            snapshot,
            map: map.clone(),
        })
        .unwrap();

        let result = results
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        let applied = core.apply(buffer, result.update).unwrap();
        assert!(applied.transformed, "journal carried the update forward");

        // "fn" moved to offset 5; the keyword highlight moved with it.
        let buf = core.buffer(buffer).unwrap();
        let keyword = map["keyword"];
        assert!(
            buf.highlights_at(5).iter().any(|(_, h)| *h == keyword),
            "keyword followed the text"
        );
    }
}
