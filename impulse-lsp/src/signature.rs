//! Signature help as the editor shows it (Monaco's parameter hints): the
//! signature to show and where its active parameter is in its label.

use std::ops::Range;

use lsp_types::{ParameterLabel, SignatureHelp, SignatureInformation};

/// The signature to show: the active one, else the first.
pub fn active_signature(help: &SignatureHelp) -> Option<(usize, &SignatureInformation)> {
    let index = help.active_signature.unwrap_or(0) as usize;
    let index = index.min(help.signatures.len().checked_sub(1)?);
    Some((index, &help.signatures[index]))
}

/// The byte range of the active parameter in the signature's label: the
/// signature's own active parameter, else the help's.
pub fn active_parameter_range(
    help: &SignatureHelp,
    signature: &SignatureInformation,
) -> Option<Range<usize>> {
    let active = signature.active_parameter.or(help.active_parameter)? as usize;
    let parameter = signature.parameters.as_ref()?.get(active)?;
    let label = &signature.label;
    match &parameter.label {
        ParameterLabel::Simple(text) => {
            if text.is_empty() {
                return None;
            }
            // The parameter's own text, past the function name.
            let from = label.find('(').map_or(0, |i| i + 1);
            label[from..]
                .find(text.as_str())
                .map(|i| from + i..from + i + text.len())
        }
        ParameterLabel::LabelOffsets([start, end]) => {
            let byte = |utf16: u32| {
                let mut units = 0;
                for (i, c) in label.char_indices() {
                    if units >= utf16 {
                        return Some(i);
                    }
                    units += c.len_utf16() as u32;
                }
                (units >= utf16).then_some(label.len())
            };
            let (start, end) = (byte(*start)?, byte(*end)?);
            (start < end).then_some(start..end)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::ParameterInformation;

    fn help(label: &str, parameters: Vec<ParameterLabel>, active: u32) -> SignatureHelp {
        SignatureHelp {
            signatures: vec![SignatureInformation {
                label: label.into(),
                documentation: None,
                parameters: Some(
                    parameters
                        .into_iter()
                        .map(|label| ParameterInformation {
                            label,
                            documentation: None,
                        })
                        .collect(),
                ),
                active_parameter: None,
            }],
            active_signature: Some(0),
            active_parameter: Some(active),
        }
    }

    #[test]
    fn finds_simple_labels_after_the_name() {
        let h = help(
            "fn add(a: i32, b: i32)",
            vec![
                ParameterLabel::Simple("a: i32".into()),
                ParameterLabel::Simple("b: i32".into()),
            ],
            1,
        );
        let (_, sig) = active_signature(&h).unwrap();
        assert_eq!(active_parameter_range(&h, sig), Some(15..21));
    }

    #[test]
    fn reads_utf16_offsets() {
        let h = help(
            "é(x, y)",
            vec![
                ParameterLabel::LabelOffsets([2, 3]),
                ParameterLabel::LabelOffsets([5, 6]),
            ],
            0,
        );
        let (_, sig) = active_signature(&h).unwrap();
        // "é" is 2 bytes but 1 UTF-16 unit.
        assert_eq!(active_parameter_range(&h, sig), Some(3..4));
    }

    #[test]
    fn nothing_without_signatures_or_parameters() {
        let empty = SignatureHelp {
            signatures: vec![],
            active_signature: None,
            active_parameter: None,
        };
        assert!(active_signature(&empty).is_none());
        let h = help("f()", vec![], 0);
        let (_, sig) = active_signature(&h).unwrap();
        assert_eq!(active_parameter_range(&h, sig), None);
    }
}
