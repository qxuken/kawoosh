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

/// A buffer's words at a version, in the order they first appear,
/// and apart from them the words inside its masks — never offered,
/// from this buffer or any other source (secrets.md Decision 3).
#[derive(Clone)]
pub struct Index {
    pub buffer: BufferId,
    pub version: Version,
    pub words: Vec<Word>,
    pub masked: Vec<String>,
}

impl std::fmt::Debug for Index {
    /// The masked words are not said.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Index")
            .field("buffer", &self.buffer)
            .field("version", &self.version)
            .field("words", &self.words.len())
            .field("masked", &self.masked.len())
            .finish()
    }
}

/// Whether `c` is part of an identifier: a letter, a digit, `_`.
pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The words of `snapshot` (its first [`MAX_BYTES`]) not starting with a
/// digit, every length: a reader filters by its own shortest. A word
/// that reaches into one of `masks` (byte ranges, sorted) is not a word
/// of the index but one of its `masked`. The copy of the text is wiped
/// before it goes.
pub fn index(buffer: BufferId, snapshot: &Snapshot, masks: &[std::ops::Range<usize>]) -> Index {
    let end = snapshot.len().min(MAX_BYTES);
    let mut bytes = snapshot.text.collect_range(0..end);
    let (words, masked) = match String::from_utf8_lossy(&bytes) {
        std::borrow::Cow::Borrowed(text) => split_masked(text, masks),
        std::borrow::Cow::Owned(mut text) => {
            let out = split_masked(&text, masks);
            text_buffer::wipe_string(&mut text);
            out
        }
    };
    if !masks.is_empty() {
        text_buffer::wipe(&mut bytes);
    }
    Index {
        buffer,
        version: snapshot.version,
        words,
        masked,
    }
}

/// [`words_of`] of every length, the words in `masks` put apart.
fn split_masked(text: &str, masks: &[std::ops::Range<usize>]) -> (Vec<Word>, Vec<String>) {
    let mut found = Found::default();
    let mut masked: Vec<String> = Vec::new();
    let mut m = 0;
    scan(text, |w, at, line| {
        while m < masks.len() && masks[m].end <= at {
            m += 1;
        }
        let inside = masks.get(m).is_some_and(|r| r.start < at + w.len());
        if inside {
            if !masked.iter().any(|x| x == w) {
                masked.push(w.to_string());
            }
        } else {
            found.push(w, line, 1);
        }
    });
    // Masked once is masked: its plain occurrences say it as well.
    let (words, gone): (Vec<Word>, Vec<Word>) = found
        .words()
        .into_iter()
        .partition(|w| !masked.contains(&w.text));
    for mut w in gone {
        text_buffer::wipe_string(&mut w.text);
    }
    (words, masked)
}

/// The words of `text` at least `min_length` characters long and not
/// starting with a digit, once each with their lines.
pub fn words_of(text: &str, min_length: usize) -> Vec<Word> {
    let mut found = Found::default();
    scan(text, |w, _, line| found.push(w, line, min_length));
    found.words()
}

/// Every identifier of `text`: its text, where it starts, its line.
fn scan<'a>(text: &'a str, mut each: impl FnMut(&'a str, usize, u32)) {
    let mut line = 0u32;
    let mut start: Option<usize> = None;
    for (i, c) in text.char_indices() {
        if is_word_char(c) {
            start.get_or_insert(i);
            continue;
        }
        if let Some(s) = start.take() {
            each(&text[s..i], s, line);
        }
        if c == '\n' {
            line += 1;
        }
    }
    if let Some(s) = start {
        each(&text[s..], s, line);
    }
}

/// The words met so far, in the order met, and where each is.
#[derive(Default)]
struct Found<'a> {
    at: std::collections::HashMap<&'a str, usize>,
    words: Vec<(&'a str, Vec<u32>)>,
}

impl<'a> Found<'a> {
    fn words(self) -> Vec<Word> {
        self.words
            .into_iter()
            .map(|(w, lines)| Word {
                text: w.to_string(),
                lines: lines.into_boxed_slice(),
            })
            .collect()
    }

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

    /// A word that reaches into a mask is the index's masked, not its
    /// word, wherever else it is too.
    #[test]
    fn a_masked_word_is_kept_apart() {
        let text = "key = MIIsecret\nsee MIIsecret and keyword\n";
        let mask: std::ops::Range<usize> = 6..15;
        let (words, masked) = split_masked(text, std::slice::from_ref(&mask));
        let got: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(got, ["key", "see", "and", "keyword"]);
        assert_eq!(masked, ["MIIsecret"]);
    }
}
