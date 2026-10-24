//! Word-based suggestions (Monaco's `wordBasedSuggestions`, which the Swift
//! editor gets for free): with no language server to ask, the editor
//! suggests the words already in the document that start like the one
//! being typed, nearest first.

use std::collections::HashSet;

/// Most suggestions offered at once.
pub const LIMIT: usize = 100;

/// Whether `c` is part of a word: letters, digits, `_` and `$` (Monaco's
/// default word definition, without its number forms).
pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// The word being typed: the word characters right before `offset`.
pub fn typed_prefix(text: &str, offset: usize) -> &str {
    let before = &text[..offset.min(text.len())];
    let start = before
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_word_char(*c))
        .last()
        .map_or(before.len(), |(i, _)| i);
    &before[start..]
}

/// The document's words starting with what's typed before `offset`
/// (ignoring case), nearest to the cursor first, without the one being
/// typed. Nothing for an empty prefix.
pub fn suggestions(text: &str, offset: usize) -> Vec<String> {
    suggestions_with(text, offset, &[])
}

/// `suggestions`, then the words in `others` (other open documents, for
/// Monaco's `matchingDocuments` and `allDocuments`) in the order they
/// appear.
pub fn suggestions_with(text: &str, offset: usize, others: &[&str]) -> Vec<String> {
    let offset = offset.min(text.len());
    let prefix = typed_prefix(text, offset);
    if prefix.is_empty() {
        return Vec::new();
    }
    let lower = prefix.to_lowercase();
    let typed_start = offset - prefix.len();
    let mut words: Vec<(usize, &str)> = Vec::new();
    let mut start = None;
    for (i, c) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        match (is_word_char(c), start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                // The word being typed isn't its own suggestion.
                if s != typed_start {
                    words.push((s, &text[s..i]));
                }
                start = None;
            }
            _ => {}
        }
    }
    let distance = |at: usize| at.abs_diff(offset);
    words.sort_by_key(|(at, _)| distance(*at));
    let elsewhere = others.iter().flat_map(|other| {
        other
            .split(|c: char| !is_word_char(c))
            .filter(|word| !word.is_empty())
    });
    let mut seen = HashSet::new();
    words
        .into_iter()
        .map(|(_, word)| word)
        .chain(elsewhere)
        .filter(|word| {
            word.len() > prefix.len()
                && word.to_lowercase().starts_with(&lower)
                && !word.chars().next().is_some_and(|c| c.is_ascii_digit())
        })
        .filter(|word| seen.insert(*word))
        .take(LIMIT)
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_document_words_nearest_first() {
        let text = "let counter = 1;\nlet count_all = 2;\nprintln!(cou";
        let offset = text.len();
        assert_eq!(typed_prefix(text, offset), "cou");
        assert_eq!(suggestions(text, offset), vec!["count_all", "counter"]);
        // Ignoring case, and never the word itself or digits-first words.
        let text = "Value value v 9v";
        assert_eq!(suggestions(text, 13), vec!["value", "Value"]);
        assert_eq!(suggestions("Value valid va", 14), vec!["valid", "Value"]);
        assert!(suggestions("abc ", 4).is_empty(), "nothing typed");
        // Other documents' words come after this one's, once each.
        assert_eq!(
            suggestions_with("count co", 8, &["copy count", "cobalt"]),
            vec!["count", "copy", "cobalt"]
        );
        assert_eq!(typed_prefix("café_ré", 9), "café_ré");
    }
}
