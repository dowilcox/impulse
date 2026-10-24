//! A server's `WorkspaceEdit` (rename, code actions, `workspace/applyEdit`)
//! as plain operations, and what the user says yes to before it applies (the
//! Swift `ImpulseKit.WorkspaceEdit`).

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use lsp_types::{DocumentChangeOperation, DocumentChanges, OneOf, ResourceOp, TextEdit, Uri};

#[derive(Clone, Debug, PartialEq)]
pub enum Operation {
    Edit {
        uri: Uri,
        edits: Vec<TextEdit>,
    },
    Create {
        uri: Uri,
        overwrite: bool,
        ignore_if_exists: bool,
    },
    Rename {
        old_uri: Uri,
        new_uri: Uri,
        overwrite: bool,
        ignore_if_exists: bool,
    },
    Delete {
        uri: Uri,
        recursive: bool,
        ignore_if_not_exists: bool,
    },
}

/// Something a workspace edit does that the user says yes to first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Confirmation {
    /// Changes a file outside the boundary.
    Changes(PathBuf),
    Creates(PathBuf),
    /// Creates a file over an existing one, leaving it empty.
    Empties(PathBuf),
    Moves(PathBuf, PathBuf),
    Deletes {
        path: PathBuf,
        with_contents: bool,
    },
}

impl Confirmation {
    /// One line for the confirmation dialog.
    pub fn describe(&self) -> String {
        match self {
            Confirmation::Changes(path) => format!("Change {}", path.display()),
            Confirmation::Creates(path) => format!("Create {}", path.display()),
            Confirmation::Empties(path) => format!("Empty {}", path.display()),
            Confirmation::Moves(from, to) => {
                format!("Move {} to {}", from.display(), to.display())
            }
            Confirmation::Deletes {
                path,
                with_contents,
            } => format!(
                "Delete {}{}",
                path.display(),
                if *with_contents {
                    " and everything in it"
                } else {
                    ""
                }
            ),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkspaceEdit {
    pub operations: Vec<Operation>,
    /// The document versions the server computed its edits against, by URI
    /// (`documentChanges` only; none means any version). An open document at
    /// another version has changed since: the edits would land in the wrong
    /// places.
    pub versions: HashMap<Uri, i32>,
}

pub fn uri_path(uri: &Uri) -> PathBuf {
    PathBuf::from(impulse_kit::uri::uri_to_file_path(uri.as_str()))
}

impl From<&lsp_types::WorkspaceEdit> for WorkspaceEdit {
    /// `documentChanges` (in order) when present, else `changes` (sorted by
    /// URI: a JSON object has no order).
    fn from(edit: &lsp_types::WorkspaceEdit) -> Self {
        let mut out = WorkspaceEdit::default();
        let text_document_edit =
            |out: &mut WorkspaceEdit, document: &lsp_types::TextDocumentEdit| {
                let uri = document.text_document.uri.clone();
                if let Some(version) = document.text_document.version {
                    out.versions.insert(uri.clone(), version);
                }
                out.operations.push(Operation::Edit {
                    uri,
                    edits: document
                        .edits
                        .iter()
                        .map(|edit| match edit {
                            OneOf::Left(edit) => edit.clone(),
                            OneOf::Right(annotated) => annotated.text_edit.clone(),
                        })
                        .collect(),
                });
            };
        match &edit.document_changes {
            Some(DocumentChanges::Edits(documents)) => {
                for document in documents {
                    text_document_edit(&mut out, document);
                }
            }
            Some(DocumentChanges::Operations(operations)) => {
                for operation in operations {
                    match operation {
                        DocumentChangeOperation::Edit(document) => {
                            text_document_edit(&mut out, document)
                        }
                        DocumentChangeOperation::Op(ResourceOp::Create(create)) => {
                            let options = create.options.as_ref();
                            out.operations.push(Operation::Create {
                                uri: create.uri.clone(),
                                overwrite: options.and_then(|o| o.overwrite).unwrap_or(false),
                                ignore_if_exists: options
                                    .and_then(|o| o.ignore_if_exists)
                                    .unwrap_or(false),
                            });
                        }
                        DocumentChangeOperation::Op(ResourceOp::Rename(rename)) => {
                            let options = rename.options.as_ref();
                            out.operations.push(Operation::Rename {
                                old_uri: rename.old_uri.clone(),
                                new_uri: rename.new_uri.clone(),
                                overwrite: options.and_then(|o| o.overwrite).unwrap_or(false),
                                ignore_if_exists: options
                                    .and_then(|o| o.ignore_if_exists)
                                    .unwrap_or(false),
                            });
                        }
                        DocumentChangeOperation::Op(ResourceOp::Delete(delete)) => {
                            let options = delete.options.as_ref();
                            out.operations.push(Operation::Delete {
                                uri: delete.uri.clone(),
                                recursive: options.and_then(|o| o.recursive).unwrap_or(false),
                                ignore_if_not_exists: options
                                    .and_then(|o| o.ignore_if_not_exists)
                                    .unwrap_or(false),
                            });
                        }
                    }
                }
            }
            None => {
                let mut changes: Vec<(&Uri, &Vec<TextEdit>)> =
                    edit.changes.iter().flatten().collect();
                changes.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
                for (uri, edits) in changes {
                    out.operations.push(Operation::Edit {
                        uri: uri.clone(),
                        edits: edits.clone(),
                    });
                }
            }
        }
        out
    }
}

impl WorkspaceEdit {
    /// Every file the edit touches, in order, without repeats.
    pub fn uris(&self) -> Vec<Uri> {
        let mut out: Vec<Uri> = Vec::new();
        for operation in &self.operations {
            let touched = match operation {
                Operation::Edit { uri, .. }
                | Operation::Create { uri, .. }
                | Operation::Delete { uri, .. } => vec![uri],
                Operation::Rename {
                    old_uri, new_uri, ..
                } => vec![old_uri, new_uri],
            };
            for uri in touched {
                if !out.contains(uri) {
                    out.push(uri.clone());
                }
            }
        }
        out
    }

    /// Text edits only, grouped by file (several entries for one file merge).
    pub fn text_edits(&self) -> Vec<(Uri, Vec<TextEdit>)> {
        let mut out: Vec<(Uri, Vec<TextEdit>)> = Vec::new();
        for operation in &self.operations {
            if let Operation::Edit { uri, edits } = operation {
                match out.iter_mut().find(|(u, _)| u == uri) {
                    Some((_, existing)) => existing.extend(edits.iter().cloned()),
                    None => out.push((uri.clone(), edits.clone())),
                }
            }
        }
        out
    }

    pub fn has_resource_operations(&self) -> bool {
        self.operations
            .iter()
            .any(|op| !matches!(op, Operation::Edit { .. }))
    }

    /// What the user is asked about before the edit applies (nothing: it
    /// just applies): every file it creates, moves or deletes, wherever it
    /// is, and every file outside `boundary` it changes (without a boundary,
    /// every file it changes).
    pub fn confirmations(&self, boundary: Option<&Path>) -> Vec<Confirmation> {
        let mut out = Vec::new();
        for operation in &self.operations {
            match operation {
                Operation::Edit { uri, edits } => {
                    let file = uri_path(uri);
                    if !edits.is_empty() && boundary.is_none_or(|b| !is_inside(&file, b)) {
                        out.push(Confirmation::Changes(file));
                    }
                }
                Operation::Create {
                    uri,
                    overwrite,
                    ignore_if_exists,
                } => {
                    let file = uri_path(uri);
                    let exists = file.exists();
                    if exists && (*ignore_if_exists || !*overwrite) {
                        continue;
                    }
                    out.push(if exists {
                        Confirmation::Empties(file)
                    } else {
                        Confirmation::Creates(file)
                    });
                }
                Operation::Rename {
                    old_uri, new_uri, ..
                } => out.push(Confirmation::Moves(uri_path(old_uri), uri_path(new_uri))),
                Operation::Delete { uri, recursive, .. } => {
                    let file = uri_path(uri);
                    out.push(Confirmation::Deletes {
                        with_contents: file.is_dir() && *recursive,
                        path: file,
                    });
                }
            }
        }
        out
    }
}

/// `path` with `..` taken out and symlinks resolved as far as it exists (a
/// file still to be created resolves through its folder).
pub fn resolved_path(path: &Path) -> PathBuf {
    let mut lexical = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                lexical.pop();
            }
            other => lexical.push(other),
        }
    }
    PathBuf::from(impulse_kit::trust::normalize(&lexical.to_string_lossy()))
}

/// Whether `path` is `folder` or inside it, once `..` is taken out and
/// symlinks are resolved: a link inside the folder that points out of it is
/// outside.
pub fn is_inside(path: &Path, folder: &Path) -> bool {
    let folder = resolved_path(folder);
    let path = resolved_path(path);
    folder == Path::new("/") || path.starts_with(&folder)
}

/// Where a rename or code action the user picked may change files without
/// asking: the repository the file is in, or the trusted folder it's in when
/// that's narrower (only part of the repository is trusted) or there's no
/// repository; failing both, the language server's project root.
pub fn user_edit_boundary(
    trusted_folder: Option<&Path>,
    repository_root: Option<&Path>,
    server_root: Option<&Path>,
) -> Option<PathBuf> {
    match (trusted_folder, repository_root) {
        (Some(trusted), Some(repository)) => Some(if is_inside(repository, trusted) {
            repository.to_path_buf()
        } else {
            trusted.to_path_buf()
        }),
        (Some(trusted), None) => Some(trusted.to_path_buf()),
        (None, Some(repository)) => Some(repository.to_path_buf()),
        (None, None) => server_root.map(Path::to_path_buf),
    }
}

/// The repository `path` is in: the nearest folder with `.git`.
pub fn repository_root(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|dir| dir.join(".git").exists())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn parse(json: &str) -> WorkspaceEdit {
        let edit: lsp_types::WorkspaceEdit = serde_json::from_str(json).unwrap();
        WorkspaceEdit::from(&edit)
    }

    fn uri(s: &str) -> Uri {
        Uri::from_str(s).unwrap()
    }

    fn file_uri(path: &Path) -> Uri {
        crate::path_uri(path).unwrap()
    }

    fn insert() -> Vec<TextEdit> {
        vec![TextEdit {
            range: lsp_types::Range::default(),
            new_text: "x".into(),
        }]
    }

    #[test]
    fn parses_changes_sorted_by_uri() {
        let parsed = parse(
            r#"{"changes": {
              "file:///b.swift": [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}}, "newText": "B"}],
              "file:///a.swift": [{"range": {"start": {"line": 1, "character": 2}, "end": {"line": 1, "character": 3}}, "newText": "A"}]
            }}"#,
        );
        assert_eq!(
            parsed.uris(),
            [uri("file:///a.swift"), uri("file:///b.swift")]
        );
        let edits = parsed.text_edits();
        assert_eq!(edits[0].0, uri("file:///a.swift"));
        assert_eq!(edits[0].1[0].new_text, "A");
        assert!(!parsed.has_resource_operations());
    }

    #[test]
    fn parses_document_changes_with_resource_operations() {
        let parsed = parse(
            r#"{"documentChanges": [
              {"kind": "create", "uri": "file:///new.swift", "options": {"ignoreIfExists": true}},
              {"textDocument": {"uri": "file:///new.swift", "version": null},
               "edits": [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}, "newText": "hi", "annotationId": "x"}]},
              {"kind": "rename", "oldUri": "file:///old.swift", "newUri": "file:///renamed.swift"},
              {"kind": "delete", "uri": "file:///gone", "options": {"recursive": true}}
            ]}"#,
        );
        assert_eq!(parsed.operations.len(), 4);
        assert_eq!(
            parsed.operations[0],
            Operation::Create {
                uri: uri("file:///new.swift"),
                overwrite: false,
                ignore_if_exists: true
            }
        );
        assert!(
            matches!(&parsed.operations[1], Operation::Edit { edits, .. } if edits[0].new_text == "hi")
        );
        assert_eq!(
            parsed.uris(),
            [
                uri("file:///new.swift"),
                uri("file:///old.swift"),
                uri("file:///renamed.swift"),
                uri("file:///gone")
            ]
        );
        assert!(parsed.has_resource_operations());
        assert!(parsed.versions.is_empty(), "a null version is any version");
    }

    #[test]
    fn keeps_the_versions_edits_were_computed_for() {
        let parsed = parse(
            r#"{"documentChanges": [
              {"textDocument": {"uri": "file:///a.ts", "version": 7}, "edits": []},
              {"textDocument": {"uri": "file:///b.ts", "version": null}, "edits": []}
            ]}"#,
        );
        assert_eq!(parsed.versions, HashMap::from([(uri("file:///a.ts"), 7)]));
    }

    /// A Cargo workspace: two crates in one repository, and a folder beside it.
    fn repository() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let base = tempfile::tempdir().unwrap();
        let repo = base.path().join("repo");
        for folder in [".git", "crates/app/src", "crates/lib/src"] {
            std::fs::create_dir_all(repo.join(folder)).unwrap();
        }
        for krate in ["app", "lib"] {
            std::fs::write(repo.join(format!("crates/{krate}/Cargo.toml")), "").unwrap();
            std::fs::write(repo.join(format!("crates/{krate}/src/lib.rs")), "").unwrap();
        }
        let outside = base.path().join("other");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("notes.rs"), "").unwrap();
        (base, repo, outside)
    }

    #[test]
    fn a_rename_across_crates_doesnt_ask() {
        let (_base, repo, outside) = repository();
        let edit = |paths: &[PathBuf]| WorkspaceEdit {
            operations: paths
                .iter()
                .map(|p| Operation::Edit {
                    uri: file_uri(p),
                    edits: insert(),
                })
                .collect(),
            versions: HashMap::new(),
        };
        let rename = edit(&[
            repo.join("crates/lib/src/lib.rs"),
            repo.join("crates/app/src/lib.rs"),
        ]);
        assert!(rename.confirmations(Some(&repo)).is_empty());
        // Bounded by the crate (a server's own change), the sibling asks.
        assert_eq!(
            rename.confirmations(Some(&repo.join("crates/lib"))),
            [Confirmation::Changes(repo.join("crates/app/src/lib.rs"))]
        );
        // Past the repository, and through `..`, asks either way.
        let reaching = edit(&[
            outside.join("notes.rs"),
            repo.join("crates/../../other/notes.rs"),
        ]);
        assert_eq!(reaching.confirmations(Some(&repo)).len(), 2);
        // Without a boundary, every changed file asks.
        assert_eq!(rename.confirmations(None).len(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_out_of_the_repository_is_outside() {
        let (_base, repo, outside) = repository();
        std::os::unix::fs::symlink(&outside, repo.join("linked")).unwrap();
        assert!(!is_inside(&repo.join("linked/notes.rs"), &repo));
        assert!(
            !is_inside(&repo.join("linked/new.rs"), &repo),
            "a file still to be created, through the link"
        );
        assert!(is_inside(&repo.join("crates/app/new.rs"), &repo));
    }

    #[test]
    fn files_created_moved_or_deleted_always_ask() {
        let (_base, repo, _) = repository();
        let lib = repo.join("crates/lib/src/lib.rs");
        let edit = WorkspaceEdit {
            operations: vec![
                Operation::Create {
                    uri: file_uri(&repo.join("crates/lib/src/new.rs")),
                    overwrite: false,
                    ignore_if_exists: false,
                },
                Operation::Create {
                    uri: file_uri(&lib),
                    overwrite: true,
                    ignore_if_exists: false,
                },
                Operation::Create {
                    uri: file_uri(&lib),
                    overwrite: false,
                    ignore_if_exists: true,
                },
                Operation::Rename {
                    old_uri: file_uri(&lib),
                    new_uri: file_uri(&repo.join("crates/lib/src/core.rs")),
                    overwrite: false,
                    ignore_if_exists: false,
                },
                Operation::Delete {
                    uri: file_uri(&repo.join("crates/app")),
                    recursive: true,
                    ignore_if_not_exists: false,
                },
                Operation::Delete {
                    uri: file_uri(&repo.join("crates/app/src/lib.rs")),
                    recursive: false,
                    ignore_if_not_exists: false,
                },
            ],
            versions: HashMap::new(),
        };
        assert_eq!(
            edit.confirmations(Some(&repo)),
            [
                Confirmation::Creates(repo.join("crates/lib/src/new.rs")),
                Confirmation::Empties(lib.clone()),
                Confirmation::Moves(lib, repo.join("crates/lib/src/core.rs")),
                Confirmation::Deletes {
                    path: repo.join("crates/app"),
                    with_contents: true
                },
                Confirmation::Deletes {
                    path: repo.join("crates/app/src/lib.rs"),
                    with_contents: false
                },
            ]
        );
    }

    #[test]
    fn the_boundary_for_the_users_own_edits() {
        let (_base, repo, _) = repository();
        let krate = repo.join("crates/app");
        let parent = repo.parent().unwrap().to_path_buf();
        let b = |t: Option<&PathBuf>, r: Option<&PathBuf>| {
            user_edit_boundary(t.map(|p| p.as_path()), r.map(|p| p.as_path()), Some(&krate))
        };
        assert_eq!(b(Some(&repo), Some(&repo)), Some(repo.clone()));
        // A trusted parent of the repository: the repository.
        assert_eq!(b(Some(&parent), Some(&repo)), Some(repo.clone()));
        // Only a crate trusted: the crate, not the rest of the repository.
        assert_eq!(b(Some(&krate), Some(&repo)), Some(krate.clone()));
        assert_eq!(b(Some(&parent), None), Some(parent.clone()));
        assert_eq!(b(None, Some(&repo)), Some(repo.clone()));
        assert_eq!(b(None, None), Some(krate.clone()));
        assert_eq!(repository_root(&krate.join("src/lib.rs")), Some(repo));
    }
}
