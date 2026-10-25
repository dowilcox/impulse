//! Narrowing a server's completion list to what's been typed, as Monaco does
//! on the client: servers (rust-analyzer among them) send every candidate
//! and leave the filtering to the editor.

use lsp_types::{CompletionItem, CompletionTextEdit, Position, Range, TextEdit};

use crate::edits::{apply_text_edits, offset_of};

/// The identifier being typed: the word characters right before `offset`.
pub fn typed_prefix(line_before_cursor: &str) -> &str {
    let start = line_before_cursor
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_alphanumeric() || *c == '_')
        .last()
        .map_or(line_before_cursor.len(), |(i, _)| i);
    &line_before_cursor[start..]
}

/// How well `candidate` matches `prefix`, best first; `None` when it doesn't.
fn rank(candidate: &str, prefix: &str) -> Option<u8> {
    if candidate.starts_with(prefix) {
        return Some(0);
    }
    let lower = candidate.to_lowercase();
    let prefix_lower = prefix.to_lowercase();
    if lower.starts_with(&prefix_lower) {
        return Some(1);
    }
    // Every typed character, in order (`ps` finds `push_str`).
    let mut rest = lower.chars();
    prefix_lower
        .chars()
        .all(|wanted| rest.any(|c| c == wanted))
        .then_some(2)
}

/// The items matching `prefix`, best matches first, then in the server's
/// `sortText` order. An empty prefix keeps everything in the server's order.
pub fn filter_completions(items: Vec<CompletionItem>, prefix: &str) -> Vec<CompletionItem> {
    let mut ranked: Vec<(u8, CompletionItem)> = items
        .into_iter()
        .filter_map(|item| {
            let text = item.filter_text.as_deref().unwrap_or(&item.label);
            rank(text, prefix).map(|rank| (rank, item))
        })
        .collect();
    ranked.sort_by(|(a_rank, a), (b_rank, b)| {
        a_rank.cmp(b_rank).then_with(|| {
            let key = |item: &CompletionItem| {
                item.sort_text.clone().unwrap_or_else(|| item.label.clone())
            };
            key(a).cmp(&key(b))
        })
    });
    ranked.into_iter().map(|(_, item)| item).collect()
}

/// Items whose own edits are folded into one: past this, an item's
/// `additionalTextEdits` are dropped (each costs a pass over the text).
const MAX_FOLDED: usize = 64;

/// Every item as one edit in the editor's terms, columns in characters
/// (GPUI Kit's editor reads an edit's columns as characters, applies
/// `insertText` without replacing what's typed, and ignores
/// `additionalTextEdits`):
///
/// - a `textEdit` keeps its range, in characters;
/// - an `insertText` replaces the typed `prefix` (the `prefix_len` bytes
///   before `offset`), as Monaco does;
/// - `additionalTextEdits` (an auto-import) are folded in: the edit spans
///   from the first of them to the last, its text that span after all of
///   them, so the cursor still ends after the completion.
///
/// `text` is the document the server answered for, `utf32` whether the
/// server counts columns in characters.
pub fn editor_completions(
    items: Vec<CompletionItem>,
    text: &str,
    offset: usize,
    prefix_len: usize,
    utf32: bool,
) -> Vec<CompletionItem> {
    let mut folded = 0;
    items
        .into_iter()
        .map(|mut item| {
            // The main edit as server positions.
            let main: Option<TextEdit> = match item.text_edit.take() {
                Some(CompletionTextEdit::Edit(edit)) => Some(edit),
                Some(CompletionTextEdit::InsertAndReplace(edit)) => Some(TextEdit {
                    range: edit.replace,
                    new_text: edit.new_text,
                }),
                None => item.insert_text.take().map(|new_text| {
                    let start = offset.saturating_sub(prefix_len);
                    TextEdit {
                        range: Range::new(
                            server_position(text, start, utf32),
                            server_position(text, offset, utf32),
                        ),
                        new_text,
                    }
                }),
            };
            let Some(main) = main else {
                return item;
            };
            let extra = item.additional_text_edits.take().unwrap_or_default();
            let edit = if extra.is_empty() || folded >= MAX_FOLDED {
                let start = offset_of(text, main.range.start, utf32);
                let end = offset_of(text, main.range.end, utf32).max(start);
                Some((start..end, main.new_text))
            } else {
                folded += 1;
                fold(text, main, extra, utf32)
            };
            if let Some((range, new_text)) = edit {
                item.text_edit = Some(CompletionTextEdit::Edit(TextEdit {
                    range: Range::new(
                        char_position(text, range.start),
                        char_position(text, range.end),
                    ),
                    new_text,
                }));
            }
            item
        })
        .collect()
}

/// `main` and `extra` as one edit: the span from the first edit's start to
/// the last one's end, and what it holds once they're all applied. `None`
/// when they overlap.
fn fold(
    text: &str,
    main: TextEdit,
    extra: Vec<TextEdit>,
    utf32: bool,
) -> Option<(std::ops::Range<usize>, String)> {
    let mut edits = extra;
    edits.push(main);
    let after = apply_text_edits(text, &edits, utf32)?;
    let start = edits
        .iter()
        .map(|e| offset_of(text, e.range.start, utf32))
        .min()?;
    let end = edits
        .iter()
        .map(|e| offset_of(text, e.range.end, utf32))
        .max()?
        .max(start);
    let tail = text.len() - end;
    Some((start..end, after[start..after.len() - tail].to_string()))
}

/// The server's position of byte `offset` (`\n` ends lines in the
/// editor's text).
fn server_position(text: &str, offset: usize, utf32: bool) -> Position {
    crate::edits::position_of(text, offset, utf32)
}

/// The editor's position of byte `offset`: its line, and characters into it.
fn char_position(text: &str, offset: usize) -> Position {
    crate::edits::position_of(text, offset, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(label: &str, sort: &str) -> CompletionItem {
        CompletionItem {
            label: label.into(),
            sort_text: Some(sort.into()),
            ..Default::default()
        }
    }

    fn labels(items: &[CompletionItem]) -> Vec<&str> {
        items.iter().map(|i| i.label.as_str()).collect()
    }

    #[test]
    fn prefix_is_the_word_before_the_cursor() {
        assert_eq!(typed_prefix("    name.pu"), "pu");
        assert_eq!(typed_prefix("let x = "), "");
        assert_eq!(typed_prefix("café_au"), "café_au");
    }

    #[test]
    fn keeps_matches_best_first() {
        let items = vec![
            item("rmatch_indices", "1"),
            item("push_str", "3"),
            item("push", "2"),
            item("Pull", "0"),
            item("pop", "4"),
        ];
        let filtered = filter_completions(items, "pu");
        assert_eq!(labels(&filtered), ["push", "push_str", "Pull"]);
    }

    #[test]
    fn subsequences_match_after_prefixes() {
        let items = vec![
            item("parse_str", "1"),
            item("push_str", "2"),
            item("ps", "3"),
        ];
        assert_eq!(
            labels(&filter_completions(items, "ps")),
            ["ps", "parse_str", "push_str"]
        );
    }

    #[test]
    fn filter_text_wins_over_the_label() {
        let mut fancy = item("len() -> usize", "1");
        fancy.filter_text = Some("len".into());
        assert_eq!(
            labels(&filter_completions(vec![fancy], "le")),
            ["len() -> usize"]
        );
    }

    #[test]
    fn empty_prefix_keeps_the_server_order() {
        let items = vec![item("b", "2"), item("a", "1")];
        assert_eq!(labels(&filter_completions(items, "")), ["a", "b"]);
    }

    fn edit_of(item: &CompletionItem) -> (Range, String) {
        match item.text_edit.as_ref() {
            Some(CompletionTextEdit::Edit(e)) => (e.range, e.new_text.clone()),
            _ => panic!("no edit"),
        }
    }

    #[test]
    fn insert_text_replaces_what_is_typed() {
        let text = "fn main() {\n    pri\n}\n";
        let offset = text.find("pri").unwrap() + 3;
        let item = CompletionItem {
            label: "println!".into(),
            insert_text: Some("println!".into()),
            ..Default::default()
        };
        let out = editor_completions(vec![item], text, offset, 3, false);
        let (range, new_text) = edit_of(&out[0]);
        assert_eq!(range, Range::new(Position::new(1, 4), Position::new(1, 7)));
        assert_eq!(new_text, "println!");
        assert!(out[0].insert_text.is_none());
    }

    #[test]
    fn edits_turn_to_characters() {
        // A UTF-16 server: the emoji is two units, one character.
        let text = "😀 pri";
        let item = CompletionItem {
            label: "print".into(),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range: Range::new(Position::new(0, 3), Position::new(0, 6)),
                new_text: "print".into(),
            })),
            ..Default::default()
        };
        let out = editor_completions(vec![item], text, text.len(), 3, false);
        let (range, _) = edit_of(&out[0]);
        assert_eq!(range, Range::new(Position::new(0, 2), Position::new(0, 5)));
    }

    #[test]
    fn additional_edits_fold_into_one() {
        let text = "fn main() {\n    Has\n}\n";
        let offset = text.find("Has").unwrap() + 3;
        let item = CompletionItem {
            label: "HashMap".into(),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range: Range::new(Position::new(1, 4), Position::new(1, 7)),
                new_text: "HashMap".into(),
            })),
            additional_text_edits: Some(vec![TextEdit {
                range: Range::new(Position::new(0, 0), Position::new(0, 0)),
                new_text: "use std::collections::HashMap;\n\n".into(),
            }]),
            ..Default::default()
        };
        let out = editor_completions(vec![item], text, offset, 3, false);
        let (range, new_text) = edit_of(&out[0]);
        assert_eq!(range, Range::new(Position::new(0, 0), Position::new(1, 7)));
        let start = 0;
        let end = offset;
        let result = format!("{}{new_text}{}", &text[..start], &text[end..]);
        assert_eq!(
            result,
            "use std::collections::HashMap;\n\nfn main() {\n    HashMap\n}\n"
        );
        assert!(out[0].additional_text_edits.is_none());
    }
}
