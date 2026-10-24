//! Keeps a copy of an editor buffer in step with Monaco's content changes
//! (the Swift `BufferEdits`). Offsets and lengths count UTF-16 code units
//! (JavaScript string positions), so a change can start or end inside a
//! character as the user sees it: between a letter and its combining
//! accent, or an emoji and its skin tone. Those must apply exactly too, or
//! the copy drifts from what's on screen.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    /// UTF-16 offset of the replaced range in the text before the event.
    pub offset: usize,
    /// UTF-16 length of the replaced range.
    pub length: usize,
    pub text: String,
}

impl Change {
    pub fn new(offset: usize, length: usize, text: impl Into<String>) -> Self {
        Change {
            offset,
            length,
            text: text.into(),
        }
    }
}

/// Apply one content event's `changes` to `text`. Each change's range is in
/// the text as it was before the event, and ranges don't overlap (Monaco
/// lists them last to first, so applying them in that order one by one
/// gives the same result). Returns false, leaving `text` as it was, when a
/// change doesn't fit: the copy is out of step and needs the whole buffer
/// again.
///
/// The result is built in one pass, so an event with thousands of changes
/// (Replace All) costs about one copy of the text.
pub fn apply(changes: &[Change], text: &mut String) -> bool {
    if changes.is_empty() {
        return true;
    }
    // Front to back. Changes at the same offset keep the order applying
    // Monaco's list one by one would give them: the later one first.
    let mut ordered: Vec<(usize, &Change)> = changes.iter().enumerate().collect();
    ordered.sort_by(|(ia, a), (ib, b)| a.offset.cmp(&b.offset).then(ib.cmp(ia)));
    let ordered: Vec<&Change> = ordered.into_iter().map(|(_, c)| c).collect();

    let total: usize = text.chars().map(char::len_utf16).sum();
    let mut end = 0;
    for change in &ordered {
        if change.offset < end || change.offset + change.length > total {
            return false;
        }
        end = change.offset + change.length;
    }

    // Offsets on character boundaries map to byte offsets; one inside a
    // surrogate pair (half an astral character) can't.
    let mut bounds = Vec::with_capacity(ordered.len());
    let mut chars = text.char_indices();
    let mut unit = 0;
    let mut byte = 0;
    let mut byte_at = |offset: usize| -> Option<usize> {
        while unit < offset {
            let (_, ch) = chars.next()?;
            unit += ch.len_utf16();
            byte += ch.len_utf8();
        }
        (unit == offset).then_some(byte)
    };
    for change in &ordered {
        let (Some(start), Some(stop)) = (
            byte_at(change.offset),
            byte_at(change.offset + change.length),
        ) else {
            apply_splitting_surrogates(&ordered, text);
            return true;
        };
        bounds.push((start, stop));
    }

    let added: usize = ordered.iter().map(|c| c.text.len()).sum();
    let mut result = String::with_capacity(text.len() + added);
    let mut kept = 0;
    for (change, (start, stop)) in ordered.iter().zip(bounds) {
        result.push_str(&text[kept..start]);
        result.push_str(&change.text);
        kept = stop;
    }
    result.push_str(&text[kept..]);
    *text = result;
    true
}

/// A change that splits a surrogate pair can only be applied to UTF-16
/// text; the joined result is valid again once the event's other half
/// lands, and a lone half that's left becomes U+FFFD, the same length.
fn apply_splitting_surrogates(ordered: &[&Change], text: &mut String) {
    let mut units: Vec<u16> = text.encode_utf16().collect();
    for change in ordered.iter().rev() {
        units.splice(
            change.offset..change.offset + change.length,
            change.text.encode_utf16(),
        );
    }
    *text = String::from_utf16_lossy(&units);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Monaco's semantics: the changes applied one by one in the order
    /// given (last to first), on UTF-16 text.
    fn oracle(changes: &[Change], text: &str) -> String {
        let mut units: Vec<u16> = text.encode_utf16().collect();
        for change in changes {
            units.splice(
                change.offset..change.offset + change.length,
                change.text.encode_utf16(),
            );
        }
        String::from_utf16_lossy(&units)
    }

    fn applied(changes: &[Change], text: &str) -> Option<String> {
        let mut copy = text.to_string();
        apply(changes, &mut copy).then_some(copy)
    }

    #[test]
    fn deletes_a_combining_accent_alone() {
        // "é" as e + U+0301: backspace removes only the accent (one unit).
        let text = "cafe\u{301}!";
        assert_eq!(
            applied(&[Change::new(4, 1, "")], text).as_deref(),
            Some("cafe!")
        );
        // Typing the accent back after the e.
        assert_eq!(
            applied(&[Change::new(4, 0, "\u{301}")], "cafe!").as_deref(),
            Some(text)
        );
    }

    #[test]
    fn edits_inside_an_emoji_with_a_skin_tone() {
        // 👍🏽 is two surrogate pairs (4 units); drop just the modifier.
        let text = "a\u{1F44D}\u{1F3FD}b";
        assert_eq!(
            applied(&[Change::new(3, 2, "")], text).as_deref(),
            Some("a\u{1F44D}b")
        );
        assert_eq!(
            applied(&[Change::new(3, 0, "\u{1F3FB}")], "a\u{1F44D}b").as_deref(),
            Some("a\u{1F44D}\u{1F3FB}b")
        );
    }

    #[test]
    fn edits_between_carriage_return_and_line_feed_apply() {
        assert_eq!(
            applied(&[Change::new(2, 0, "x")], "a\r\nb").as_deref(),
            Some("a\rx\nb")
        );
        assert_eq!(
            applied(&[Change::new(1, 1, "")], "a\r\nb").as_deref(),
            Some("a\nb")
        );
    }

    #[test]
    fn many_changes_in_one_event_match_applying_them_one_by_one() {
        // Replace All: Monaco lists the matches last to first.
        let text = "foo e\u{301} 👍🏽 bar\r\n".repeat(200);
        let units: Vec<u16> = text.encode_utf16().collect();
        let offsets: Vec<usize> = (0..units.len().saturating_sub(2))
            .filter(|&i| units[i..i + 3] == [0x66, 0x6F, 0x6F])
            .collect();
        let changes: Vec<Change> = offsets
            .iter()
            .rev()
            .map(|&o| Change::new(o, 3, "qux"))
            .collect();
        assert_eq!(changes.len(), 200);
        assert_eq!(applied(&changes, &text), Some(oracle(&changes, &text)));
        assert_eq!(
            applied(&changes, &text),
            Some("qux e\u{301} 👍🏽 bar\r\n".repeat(200))
        );
    }

    #[test]
    fn insertions_at_the_same_offset_keep_monacos_order() {
        // Applied one by one, the second insert lands before the first.
        let changes = [Change::new(1, 0, "B"), Change::new(1, 0, "A")];
        assert_eq!(applied(&changes, "xy"), Some(oracle(&changes, "xy")));
        assert_eq!(applied(&changes, "xy").as_deref(), Some("xABy"));
        // An insert and a replacement starting at the same place.
        let mixed = [Change::new(1, 2, "R"), Change::new(1, 0, "I")];
        assert_eq!(applied(&mixed, "abcd"), Some(oracle(&mixed, "abcd")));
    }

    #[test]
    fn random_edits_match_utf16_semantics() {
        // A fixed pseudo-random walk over text mixing ASCII, accents, emoji
        // with modifiers, CJK and CRLF, cutting anywhere but inside a
        // surrogate pair.
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = |bound: usize| -> usize {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((seed >> 33) % bound.max(1) as u64) as usize
        };
        let pieces = [
            "a",
            "Z",
            " ",
            "e\u{301}",
            "\u{1F44D}\u{1F3FD}",
            "漢",
            "\r\n",
            "\n",
            "\u{1F1FA}\u{1F1F8}",
            "ñ",
        ];
        let cuttable = |units: &[u16], offset: usize| {
            offset == 0 || offset == units.len() || !(0xDC00..=0xDFFF).contains(&units[offset])
        };
        let mut text = "start".to_string();
        for _ in 0..400 {
            let units: Vec<u16> = text.encode_utf16().collect();
            let mut cuts = std::collections::BTreeSet::new();
            let count = 1 + next(4);
            for _ in 0..count * 2 {
                let offset = next(units.len() + 1);
                if cuttable(&units, offset) {
                    cuts.insert(offset);
                }
            }
            let sorted: Vec<usize> = cuts.into_iter().collect();
            let mut changes = Vec::new();
            let mut index = 0;
            while index + 1 < sorted.len() {
                let insert: String = (0..next(3)).map(|_| pieces[next(pieces.len())]).collect();
                changes.push(Change::new(
                    sorted[index],
                    sorted[index + 1] - sorted[index],
                    insert,
                ));
                index += 2;
            }
            if changes.is_empty() {
                changes = vec![Change::new(0, 0, pieces[next(pieces.len())])];
            }
            changes.reverse();
            let expected = oracle(&changes, &text);
            let result = applied(&changes, &text);
            assert_eq!(result.as_ref(), Some(&expected));
            text = expected;
        }
    }

    #[test]
    fn a_change_that_doesnt_fit_leaves_the_text_alone() {
        let mut text = "abc".to_string();
        assert!(!apply(&[Change::new(2, 5, "x")], &mut text));
        assert!(!apply(
            &[Change::new(0, 2, ""), Change::new(1, 1, "")],
            &mut text
        ));
        assert_eq!(text, "abc");
        assert!(apply(&[], &mut text));
        assert_eq!(text, "abc");
    }

    #[test]
    fn splitting_a_surrogate_pair_keeps_offsets_in_step() {
        // Half of 👍 deleted: the lone half left becomes U+FFFD, still one
        // unit, so later offsets keep lining up with Monaco's.
        let text = "x\u{1F44D}y";
        assert_eq!(
            applied(&[Change::new(2, 1, "")], text).as_deref(),
            Some("x\u{FFFD}y")
        );
        assert_eq!(
            applied(&[Change::new(2, 1, ""), Change::new(0, 1, "z")], text).as_deref(),
            Some("z\u{FFFD}y")
        );
    }
}
