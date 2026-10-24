//! Which server handles a language (the defaults of the Swift
//! `LSPConfig.defaultConfig`), finding its command, and keeping open
//! documents in step with a server.

use std::path::{Path, PathBuf};

use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, DidSaveTextDocument,
};
use lsp_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DidSaveTextDocumentParams, TextDocumentContentChangeEvent, TextDocumentIdentifier,
    TextDocumentItem, Uri, VersionedTextDocumentIdentifier,
};

use crate::client::LspClient;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerSpec {
    pub name: &'static str,
    pub command: &'static str,
    pub args: &'static [&'static str],
}

const fn spec(
    name: &'static str,
    command: &'static str,
    args: &'static [&'static str],
) -> ServerSpec {
    ServerSpec {
        name,
        command,
        args,
    }
}

const RUST_ANALYZER: ServerSpec = spec("rust-analyzer", "rust-analyzer", &[]);
const PYRIGHT: ServerSpec = spec("pyright", "pyright-langserver", &["--stdio"]);
const CLANGD: ServerSpec = spec("clangd", "clangd", &[]);
const TYPESCRIPT: ServerSpec = spec(
    "typescript-language-server",
    "typescript-language-server",
    &["--stdio"],
);
const INTELEPHENSE: ServerSpec = spec("intelephense", "intelephense", &["--stdio"]);
const HTML: ServerSpec = spec(
    "vscode-html-language-server",
    "vscode-html-language-server",
    &["--stdio"],
);
const CSS: ServerSpec = spec(
    "vscode-css-language-server",
    "vscode-css-language-server",
    &["--stdio"],
);
const JSON: ServerSpec = spec(
    "vscode-json-language-server",
    "vscode-json-language-server",
    &["--stdio"],
);
const YAML: ServerSpec = spec("yaml-language-server", "yaml-language-server", &["--stdio"]);
const BASH: ServerSpec = spec("bash-language-server", "bash-language-server", &["start"]);
const GOPLS: ServerSpec = spec("gopls", "gopls", &[]);
const SOURCEKIT: ServerSpec = spec("sourcekit-lsp", "sourcekit-lsp", &[]);

/// The primary server for a language id (`language_from_uri`). The Swift
/// app can run several per language (eslint, tailwind, emmet beside
/// typescript); those come with the full port of `LSPConfig`.
pub fn primary_server(language: &str) -> Option<ServerSpec> {
    Some(match language {
        "rust" => RUST_ANALYZER,
        "python" => PYRIGHT,
        "c" | "cpp" => CLANGD,
        "javascript" | "javascriptreact" | "typescript" | "typescriptreact" => TYPESCRIPT,
        "php" => INTELEPHENSE,
        "html" => HTML,
        "css" | "scss" | "less" => CSS,
        "json" | "jsonc" => JSON,
        "yaml" => YAML,
        "shellscript" => BASH,
        "go" => GOPLS,
        "swift" => SOURCEKIT,
        _ => return None,
    })
}

/// `command` on `PATH`, or as given when it's a path.
pub fn resolve_command(command: &str) -> Option<PathBuf> {
    let candidate = Path::new(command);
    if candidate.components().count() > 1 {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(command))
            .find(|full| full.is_file())
    })
}

/// LSP's language id for Impulse's (they differ only for shell scripts).
pub fn lsp_language_id(language: &str) -> &str {
    language
}

impl LspClient {
    pub fn did_open(&self, uri: Uri, language: &str, version: i32, text: String) {
        let _ = self.notify::<DidOpenTextDocument>(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri,
                language_id: lsp_language_id(language).to_string(),
                version,
                text,
            },
        });
    }

    /// The whole new text (full sync works with every server).
    pub fn did_change(&self, uri: Uri, version: i32, text: String) {
        let _ = self.notify::<DidChangeTextDocument>(DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier { uri, version },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text,
            }],
        });
    }

    pub fn did_save(&self, uri: Uri) {
        let _ = self.notify::<DidSaveTextDocument>(DidSaveTextDocumentParams {
            text_document: TextDocumentIdentifier { uri },
            text: None,
        });
    }

    pub fn did_close(&self, uri: Uri) {
        let _ = self.notify::<DidCloseTextDocument>(DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier { uri },
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_map_to_their_servers() {
        assert_eq!(primary_server("rust").unwrap().command, "rust-analyzer");
        assert_eq!(
            primary_server("typescriptreact").unwrap().command,
            "typescript-language-server"
        );
        assert_eq!(primary_server("python").unwrap().args, &["--stdio"]);
        assert_eq!(primary_server("markdown"), None);
    }

    #[test]
    fn finds_commands_on_path() {
        assert!(resolve_command("sh").is_some());
        assert!(resolve_command("no-such-impulse-command").is_none());
        assert_eq!(resolve_command("/bin/sh"), Some(PathBuf::from("/bin/sh")));
    }
}
