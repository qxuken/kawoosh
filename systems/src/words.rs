//! A buffer's words, indexed off the frame for completion's
//! `buffer_words` source (docs/design/completion.md Decision 6): every
//! identifier of the text as of a snapshot, once each, with the lines
//! it is on, so a completion reads the index at once and ranks by
//! nearness without reading the text again. Built on an io job
//! ([`crate::io::Io::run`]) from a [`Snapshot`], which is a piece
//! tree's handle, not a copy; the copy is the job's.

use kawoosh_doc::{BufferId, Snapshot, Version};

/// The most of a text the index reads: a minified bundle is not a
/// dictionary.
pub const MAX_BYTES: usize = 2 << 20;
/// The most distinct words an index keeps, in the text's order.
pub const MAX_WORDS: usize = 50_000;

/// One word of a text: its letters and the lines it is on, from 0,
/// ascending, each once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Word {
    pub text: String,
    pub lines: Box<[u32]>,
}

/// A buffer's words at a version, in the order they first appear.
#[derive(Clone, Debug)]
pub struct Index {
    pub buffer: BufferId,
    pub version: Version,
    pub words: Vec<Word>,
}

/// Whether `c` is part of an identifier: a letter, a digit, `_`.
pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The words of `snapshot` (its first [`MAX_BYTES`]) at least
/// `min_length` characters long and not starting with a digit.
pub fn index(buffer: BufferId, snapshot: &Snapshot, min_length: usize) -> Index {
    let end = snapshot.len().min(MAX_BYTES);
    let bytes = snapshot.text.collect_range(0..end);
    let text = String::from_utf8_lossy(&bytes);
    Index {
        buffer,
        version: snapshot.version,
        words: words_of(&text, min_length),
    }
}

/// The words of `text`, as [`index`] keeps them.
pub fn words_of(text: &str, min_length: usize) -> Vec<Word> {
    let mut found = Found::default();
    let mut line = 0u32;
    let mut start: Option<usize> = None;
    for (i, c) in text.char_indices() {
        if is_word_char(c) {
            start.get_or_insert(i);
            continue;
        }
        if let Some(s) = start.take() {
            found.push(&text[s..i], line, min_length);
        }
        if c == '\n' {
            line += 1;
        }
    }
    if let Some(s) = start {
        found.push(&text[s..], line, min_length);
    }
    found
        .words
        .into_iter()
        .map(|(w, lines)| Word {
            text: w.to_string(),
            lines: lines.into_boxed_slice(),
        })
        .collect()
}

/// The words met so far, in the order met, and where each is.
#[derive(Default)]
struct Found<'a> {
    at: std::collections::HashMap<&'a str, usize>,
    words: Vec<(&'a str, Vec<u32>)>,
}

impl<'a> Found<'a> {
    fn push(&mut self, w: &'a str, line: u32, min_length: usize) {
        if w.chars().count() < min_length || w.starts_with(|c: char| c.is_ascii_digit()) {
            return;
        }
        match self.at.get(w) {
            Some(&i) => {
                let lines = &mut self.words[i].1;
                if lines.last() != Some(&line) {
                    lines.push(line);
                }
            }
            None if self.words.len() < MAX_WORDS => {
                self.at.insert(w, self.words.len());
                self.words.push((w, vec![line]));
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_once_each_with_their_lines() {
        let w = words_of("helium helper\nhello_world here helper\n\n12ab x helper", 3);
        let got: Vec<(&str, &[u32])> = w.iter().map(|w| (w.text.as_str(), &*w.lines)).collect();
        assert_eq!(
            got,
            [
                ("helium", &[0][..]),
                ("helper", &[0, 1, 3][..]),
                ("hello_world", &[1][..]),
                ("here", &[1][..]),
            ]
        );
        // Letters past ASCII are letters; a word at the very end counts.
        let w = words_of("größe naïve", 3);
        assert_eq!(w[0].text, "größe");
        assert_eq!(w[1].text, "naïve");
    }
}
