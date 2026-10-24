//! Narrowing a server's completion list to what's been typed, as Monaco does
//! on the client: servers (rust-analyzer among them) send every candidate
//! and leave the filtering to the editor.

use lsp_types::CompletionItem;

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
}
