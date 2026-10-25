//! Against a real server: rust-analyzer on a small crate. Skipped (with a
//! note) where rust-analyzer isn't installed.

use std::process::Command;
use std::time::{Duration, Instant};

use futures::executor::block_on;
use impulse_lsp::{path_uri, resolve_command, LspClient, LspConfig, ServerEvent};
use lsp_types::request::{Completion, HoverRequest};
use lsp_types::{
    CompletionParams, CompletionResponse, DiagnosticSeverity, HoverParams, Position,
    TextDocumentIdentifier, TextDocumentPositionParams,
};

fn rust_analyzer() -> Option<std::path::PathBuf> {
    let config = LspConfig::default();
    let path = resolve_command(&config.servers_for("rust")[0].1.command)?;
    // A rustup proxy without the component installed exits with an error.
    let works = Command::new(&path)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    works.then_some(path)
}

/// rust-analyzer answers "content modified" while it's still loading the
/// workspace; editors ask again, so this does too.
fn retry<T>(mut request: impl FnMut() -> Result<T, String>) -> T {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match request() {
            Ok(value) => return value,
            Err(error) if error == "content modified" && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(error) => panic!("{error}"),
        }
    }
}

#[test]
fn diagnostics_hover_and_completion() {
    let Some(command) = rust_analyzer() else {
        eprintln!("rust-analyzer isn't installed; skipping");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    let main = dir.path().join("src/main.rs");
    let text = "fn main() {\n    let count: u32 = \"three\";\n    let name = String::new();\n    name.\n}\n";
    std::fs::write(&main, text).unwrap();

    let (events_tx, events) = async_channel::unbounded();
    let client = block_on(LspClient::start(
        "rust-analyzer",
        &command,
        &[],
        None,
        dir.path(),
        events_tx,
    ))
    .unwrap();
    assert!(client.capabilities().hover_provider.is_some());
    let uri = path_uri(&main).unwrap();
    client.did_open(uri.clone(), "rust", 1, text.to_string());

    // rust-analyzer's own type check reports the mismatch on line 2, once
    // the workspace is loaded (an earlier, quicker report says "expected
    // u32" and comes before hovers can answer).
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut mismatch = None;
    while mismatch.is_none() && Instant::now() < deadline {
        match events.recv_blocking() {
            Ok(ServerEvent::Diagnostics(_, params)) if params.uri == uri => {
                mismatch = params.diagnostics.into_iter().find(|d| {
                    d.severity == Some(DiagnosticSeverity::ERROR)
                        && d.range.start.line == 1
                        && d.message.contains("mismatched")
                });
            }
            Ok(ServerEvent::Exited { .. }) | Err(_) => {
                eprintln!("server exited");
                break;
            }
            Ok(_) => {}
        }
    }
    let mismatch = mismatch.expect("the type error on line 2");
    assert!(
        mismatch.message.contains("mismatched"),
        "{}",
        mismatch.message
    );

    let position = |line, character| TextDocumentPositionParams {
        text_document: TextDocumentIdentifier { uri: uri.clone() },
        position: Position { line, character },
    };
    let hover = retry(|| {
        block_on(client.request::<HoverRequest>(HoverParams {
            text_document_position_params: position(2, 9),
            work_done_progress_params: Default::default(),
        }))
    })
    .expect("hover on `name`");
    assert!(format!("{:?}", hover.contents).contains("String"));

    let completions = retry(|| {
        block_on(client.request::<Completion>(CompletionParams {
            text_document_position: position(3, 9),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: None,
        }))
    });
    let labels: Vec<String> = match completions {
        Some(CompletionResponse::Array(items)) => items.into_iter().map(|i| i.label).collect(),
        Some(CompletionResponse::List(list)) => list.items.into_iter().map(|i| i.label).collect(),
        None => Vec::new(),
    };
    assert!(
        labels.iter().any(|l| l.starts_with("push_str")),
        "String methods in {labels:?}"
    );

    // An edit sent as a changed range (rust-analyzer syncs incrementally):
    // the server's text is the editor's, so a new line's hover is right.
    assert!(client.incremental_sync());
    let edited = text.replace("    name.\n", "    let total = name.len();\n    name.\n");
    client.did_change_from(uri.clone(), 2, text, &edited);
    let hover = retry(|| {
        block_on(client.request::<HoverRequest>(HoverParams {
            text_document_position_params: position(3, 9),
            work_done_progress_params: Default::default(),
        }))
    })
    .expect("hover on `total`");
    assert!(format!("{:?}", hover.contents).contains("usize"));

    block_on(client.shutdown());
}
