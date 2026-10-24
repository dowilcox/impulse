//! Text matching behind the editor's occurrence and selection highlights
//! (Monaco's word highlighter without a language server, and its
//! selection highlighter): the word at the cursor, its whole-word
//! occurrences, and the other places a selected piece of text appears.

use std::ops::Range;

/// At most this many ranges are marked, as Monaco caps its decorations.
pub const LIMIT: usize = 1000;

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The word touching byte `offset` (letters, digits and `_`), if any.
pub fn word_at(text: &str, offset: usize) -> Option<Range<usize>> {
    let offset = offset.min(text.len());
    if !text.is_char_boundary(offset) {
        return None;
    }
    let start = text[..offset]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_word(*c))
        .last()
        .map_or(offset, |(i, _)| i);
    let end = offset
        + text[offset..]
            .char_indices()
            .find(|(_, c)| !is_word(*c))
            .map_or(text.len() - offset, |(i, _)| i);
    (start < end).then_some(start..end)
}

/// Every place `word` appears as a whole word.
pub fn whole_word_matches(text: &str, word: &str) -> Vec<Range<usize>> {
    if word.is_empty() {
        return Vec::new();
    }
    text.match_indices(word)
        .filter(|(i, _)| {
            let before = text[..*i].chars().next_back().is_none_or(|c| !is_word(c));
            let after = text[i + word.len()..]
                .chars()
                .next()
                .is_none_or(|c| !is_word(c));
            before && after
        })
        .map(|(i, _)| i..i + word.len())
        .take(LIMIT)
        .collect()
}

/// The other places the selected text appears, for a selection on one line
/// that isn't only whitespace; nothing otherwise.
pub fn selection_matches(text: &str, selection: Range<usize>) -> Vec<Range<usize>> {
    let Some(needle) = text.get(selection.clone()) else {
        return Vec::new();
    };
    if needle.trim().is_empty() || needle.contains('\n') || needle.len() > 200 {
        return Vec::new();
    }
    text.match_indices(needle)
        .map(|(i, _)| i..i + needle.len())
        .filter(|range| *range != selection)
        .take(LIMIT)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_word_at_the_cursor() {
        let text = "let foo_bar = foo_bar + 1;";
        assert_eq!(word_at(text, 4), Some(4..11));
        assert_eq!(word_at(text, 11), Some(4..11));
        assert_eq!(word_at(text, 12), None);
        assert_eq!(word_at("héllo wörld", 8), Some(7..13));
    }

    #[test]
    fn matches_whole_words_only() {
        let text = "foo foobar foo_x (foo)";
        assert_eq!(whole_word_matches(text, "foo"), vec![0..3, 18..21]);
    }

    #[test]
    fn selection_matches_skip_the_selection_itself() {
        let text = "a.b(); a.b(); ab";
        assert_eq!(selection_matches(text, 0..3), vec![7..10]);
        assert!(selection_matches("x\ny x\ny", 0..3).is_empty());
        assert!(selection_matches("a  b  c", 1..3).is_empty());
    }
}
