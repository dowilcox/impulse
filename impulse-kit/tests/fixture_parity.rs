//! Parity with the golden fixtures (`tests/fixtures/`), generated from the
//! original Rust at tag `v0.29.0` and the spec the Swift port answered to;
//! never regenerate them from this code's output.

use std::path::PathBuf;

use impulse_kit::close_risk::{CloseRiskInput, CloseRiskSummary};
use impulse_kit::glob::matches_file_pattern;
use impulse_kit::shell_parser::parse_shell_input;
use serde::Deserialize;
use serde_json::Value;

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
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

#[test]
fn uris_and_languages_match_fixtures() {
    #[derive(Deserialize)]
    struct Pair {
        path: String,
        uri: String,
    }
    #[derive(Deserialize)]
    struct Lang {
        uri: String,
        language: String,
    }
    use impulse_kit::uri::{file_path_to_uri, language_from_uri, uri_to_file_path};
    let to_uri: Vec<Pair> = serde_json::from_str(&fixture("path_to_uri.json")).unwrap();
    for c in to_uri {
        assert_eq!(
            file_path_to_uri(std::path::Path::new(&c.path)).as_deref(),
            Some(c.uri.as_str())
        );
    }
    let to_path: Vec<Pair> = serde_json::from_str(&fixture("uri_to_path.json")).unwrap();
    for c in to_path {
        assert_eq!(uri_to_file_path(&c.uri), c.path, "uri={}", c.uri);
    }
    let langs: Vec<Lang> = serde_json::from_str(&fixture("language_from_uri.json")).unwrap();
    for c in langs {
        assert_eq!(language_from_uri(&c.uri), c.language, "uri={}", c.uri);
    }
}
