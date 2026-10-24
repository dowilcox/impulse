//! Parity with the golden fixtures shared with the Swift port. The fixtures
//! are read in place from the Swift test tree so both ports answer to one
//! spec; never regenerate them from either port's output.

use std::path::PathBuf;

use impulse_kit::close_risk::{CloseRiskInput, CloseRiskSummary};
use impulse_kit::glob::matches_file_pattern;
use impulse_kit::shell_parser::parse_shell_input;
use serde::Deserialize;
use serde_json::Value;

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../impulse-macos/Tests/ImpulseKitTests/Fixtures")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn glob_matches_fixture() {
    #[derive(Deserialize)]
    struct Case {
        path: String,
        pattern: String,
        matches: bool,
    }
    let cases: Vec<Case> = serde_json::from_str(&fixture("glob.json")).unwrap();
    assert!(cases.len() >= 15);
    for c in cases {
        assert_eq!(
            matches_file_pattern(&c.path, &c.pattern),
            c.matches,
            "path={} pattern={}",
            c.path,
            c.pattern
        );
    }
}

#[test]
fn close_risk_matches_fixture() {
    #[derive(Deserialize)]
    struct Case {
        input: CloseRiskInput,
        summary: CloseRiskSummary,
    }
    let cases: Vec<Case> = serde_json::from_str(&fixture("close_risk.json")).unwrap();
    assert!(cases.len() >= 4);
    for c in cases {
        assert_eq!(c.input.summarize(), c.summary);
    }
}

#[test]
fn shell_parser_matches_fixture() {
    let cases: Vec<Value> = serde_json::from_str(&fixture("shell_parser.json")).unwrap();
    assert!(cases.len() >= 15);
    for expected in cases {
        let input = expected["input"].as_str().unwrap();
        let cursor = expected["cursor"].as_u64().unwrap() as usize;
        let actual = serde_json::to_value(parse_shell_input(input, cursor)).unwrap();
        assert_eq!(actual, expected, "input={input:?} cursor={cursor}");
    }
}

#[test]
fn resolved_themes_match_fixtures() {
    let names = impulse_kit::theme::builtin_theme_names();
    assert_eq!(names.len(), 19);
    for id in names {
        let theme = impulse_kit::theme::builtin_theme(id).unwrap();
        let actual = serde_json::to_value(&theme).unwrap();
        let expected: Value =
            serde_json::from_str(&fixture(&format!("themes/{id}.theme.json"))).unwrap();
        assert_eq!(actual, expected, "theme {id}");
    }
}
