//! Finding a server's command, and keeping open documents in step with a
//! server. Which servers run for a language is `config`.

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

/// `command` on the login shell's `PATH` or in the managed servers' folder
/// (the Swift `resolveLspCommandPath`), or as given when it's a path. Not
/// the process's own `PATH`: an app opened from Finder or the Dock has
/// launchd's, which has neither.
pub fn resolve_command(command: &str) -> Option<PathBuf> {
    let candidate = Path::new(command);
    if candidate.components().count() > 1 {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }
    crate::managed::resolve(command)
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

    /// What changed from `old` to `new`, as a range when the server takes
    /// ranges (a keystroke sends a few bytes, not the document).
    pub fn did_change_from(&self, uri: Uri, version: i32, old: &str, new: &str) {
        if !self.incremental_sync() {
            return self.did_change(uri, version, new.to_string());
        }
        let change = crate::edits::content_change(old, new, self.uses_utf32());
        let _ = self.notify::<DidChangeTextDocument>(DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier { uri, version },
            content_changes: vec![change],
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
    fn finds_commands_on_path() {
        assert!(resolve_command("sh").is_some());
        assert!(resolve_command("no-such-impulse-command").is_none());
        assert_eq!(resolve_command("/bin/sh"), Some(PathBuf::from("/bin/sh")));
    }
}
