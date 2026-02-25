pub(crate) mod context;
mod dialogs;
mod keybinding_setup;
mod sidebar_signals;
mod tab_management;

use dialogs::{show_command_palette, show_go_to_line_dialog, show_quick_open};

use gtk4::gio;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use vte4::prelude::*;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::editor;
use crate::keybindings;
use crate::lsp_completion::{LspRequest, LspResponse};
use crate::sidebar;
use crate::status_bar;
use crate::terminal;
use crate::terminal_container;

#[derive(Clone)]
struct Command {
    name: String,
    shortcut: String,
    action: Rc<dyn Fn()>,
}

/// Information about a closed tab, used for the "reopen closed tab" feature.
#[derive(Clone, Debug)]
enum ClosedTab {
    /// An editor tab with a file path.
    Editor(String),
    /// An image preview tab with a file path.
    ImagePreview(String),
}

/// Maximum number of closed tabs to remember.
const MAX_CLOSED_TABS: usize = 20;

pub fn build_window(app: &adw::Application) {
    // Pre-warm a WebView with Monaco so the first editor tab opens instantly.
    crate::editor_webview::warm_up_editor();

    let settings = Rc::new(RefCell::new(crate::settings::load()));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Impulse")
        .default_width(settings.borrow().window_width)
        .default_height(settings.borrow().window_height)
        .build();

    // Shared font size state (user-facing size in points, e.g. 11)
    let font_size: Rc<Cell<i32>> = Rc::new(Cell::new(settings.borrow().font_size));

    // Shared copy-on-select flag checked by terminal selection-changed signal handlers
    let copy_on_select_flag: Rc<Cell<bool>> =
        Rc::new(Cell::new(settings.borrow().terminal_copy_on_select));

    // Pre-compute shell spawn parameters once (shell path, env vars, temp files)
    let shell_cache = Rc::new(terminal::ShellSpawnCache::new());

    // Stack of recently closed tabs for "reopen closed tab" (Ctrl+Shift+T)
    let closed_tabs: Rc<RefCell<Vec<ClosedTab>>> = Rc::new(RefCell::new(Vec::new()));

    // --- LSP Bridge: GTK <-> Tokio ---
    // Channel for sending requests from GTK to the LSP tokio runtime
    let (lsp_request_tx, mut lsp_request_rx) = tokio::sync::mpsc::channel::<LspRequest>(256);
    let lsp_request_tx = Rc::new(lsp_request_tx);

    // Channel for sending responses from the LSP runtime back to GTK
    // We use std::sync::mpsc since glib::MainContext::channel is not available in this version.
    // A glib timeout polls the receiver periodically.
    let (lsp_gtk_tx, lsp_gtk_rx) = std::sync::mpsc::channel::<LspResponse>();
    let lsp_gtk_rx = Rc::new(RefCell::new(lsp_gtk_rx));

    // Spawn the tokio runtime in a background thread
    {
        let initial_dir = if !settings.borrow().last_directory.is_empty()
            && std::path::Path::new(&settings.borrow().last_directory).is_dir()
        {
            settings.borrow().last_directory.clone()
        } else {
            impulse_core::shell::get_home_directory().unwrap_or_else(|_| "/".to_string())
        };
        let root_uri = file_path_to_uri(std::path::Path::new(&initial_dir))
            .unwrap_or_else(|| "file:///".to_string());
        let gtk_tx = lsp_gtk_tx.clone();

        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("Failed to create tokio runtime for LSP");

            rt.block_on(async move {
                // Create the event channel from core LspEvents
                let (event_tx, mut event_rx) =
                    tokio::sync::mpsc::unbounded_channel::<impulse_core::lsp::LspEvent>();

                let registry = std::sync::Arc::new(
                    impulse_core::lsp::LspRegistry::new(root_uri, event_tx),
                );

                // Task to forward LspEvents to the GTK main loop
                let gtk_tx_events = gtk_tx.clone();
                let registry_for_exit = registry.clone();
                tokio::spawn(async move {
                    while let Some(event) = event_rx.recv().await {
                        let response = match event {
                            impulse_core::lsp::LspEvent::Diagnostics {
                                uri,
                                version,
                                diagnostics,
                            } => {
                                let diags = diagnostics
                                    .into_iter()
                                    .map(|d| {
                                        let range = d.range;
                                        let severity = match d.severity {
                                            Some(lsp_types::DiagnosticSeverity::ERROR) => {
                                                crate::lsp_completion::DiagnosticSeverity::Error
                                            }
                                            Some(lsp_types::DiagnosticSeverity::WARNING) => {
                                                crate::lsp_completion::DiagnosticSeverity::Warning
                                            }
                                            Some(lsp_types::DiagnosticSeverity::INFORMATION) => {
                                                crate::lsp_completion::DiagnosticSeverity::Information
                                            }
                                            _ => crate::lsp_completion::DiagnosticSeverity::Hint,
                                        };
                                        crate::lsp_completion::DiagnosticInfo {
                                            line: range.start.line,
                                            character: range.start.character,
                                            end_line: range.end.line,
                                            end_character: range.end.character,
                                            severity,
                                            message: d.message,
                                        }
                                    })
                                    .collect();
                                LspResponse::Diagnostics {
                                    uri,
                                    version,
                                    diagnostics: diags,
                                }
                            }
                            impulse_core::lsp::LspEvent::Initialized {
                                client_key,
                                server_id,
                            } => LspResponse::ServerInitialized {
                                client_key,
                                server_id,
                            },
                            impulse_core::lsp::LspEvent::ServerError {
                                client_key,
                                server_id,
                                message,
                            } => LspResponse::ServerError {
                                client_key,
                                server_id,
                                message,
                            },
                            impulse_core::lsp::LspEvent::ServerExited {
                                client_key,
                                server_id,
                            } => {
                                registry_for_exit.remove_client(&client_key).await;
                                LspResponse::ServerExited {
                                    client_key,
                                    server_id,
                                }
                            }
                        };
                        if gtk_tx_events.send(response).is_err() {
                            break;
                        }
                    }
                });

                // Main request processing loop.
                // Requests are processed in-order to keep didOpen/didChange/completion
                // sequencing deterministic per document.
                let gtk_tx_req = gtk_tx;
                while let Some(request) = lsp_request_rx.recv().await {
                    let gtk_tx = gtk_tx_req.clone();
                    match request {
                        LspRequest::DidOpen {
                            uri,
                            language_id,
                            version,
                            text,
                        } => {
                            let clients = registry.get_clients(&language_id, &uri).await;
                            for client in clients {
                                let _ = client.did_open(&uri, &language_id, version, &text);
                            }
                        }
                        LspRequest::DidChange { uri, version, text } => {
                            let lang = language_from_uri(&uri);
                            let clients = registry.get_clients(&lang, &uri).await;
                            for client in clients {
                                let _ = client.did_change(&uri, version, &text);
                            }
                        }
                        LspRequest::DidSave { uri } => {
                            let lang = language_from_uri(&uri);
                            let clients = registry.get_clients(&lang, &uri).await;
                            for client in clients {
                                let _ = client.did_save(&uri);
                            }
                        }
                        LspRequest::DidClose { uri } => {
                            let lang = language_from_uri(&uri);
                            let clients = registry.get_clients(&lang, &uri).await;
                            for client in clients {
                                let _ = client.did_close(&uri);
                            }
                        }
                        LspRequest::Completion {
                            request_id,
                            uri,
                            version,
                            line,
                            character,
                        } => {
                            let lang = language_from_uri(&uri);
                            let clients = registry.get_clients(&lang, &uri).await;
                            let mut seen = std::collections::HashSet::<String>::new();
                            let mut completions = Vec::new();
                            for client in clients {
                                if let Ok(items) = client.completion(&uri, line, character).await {
                                    for item in items {
                                        let dedupe_key = format!(
                                            "{}|{}|{}",
                                            item.label,
                                            item.detail.clone().unwrap_or_default(),
                                            item.insert_text.clone().unwrap_or_default()
                                        );
                                        if seen.insert(dedupe_key) {
                                            completions.push(completion_item_to_info(item));
                                        }
                                    }
                                }
                            }
                            let _ = gtk_tx.send(LspResponse::CompletionResult {
                                request_id,
                                uri,
                                version,
                                items: completions,
                            });
                        }
                        LspRequest::Hover {
                            request_id,
                            uri,
                            version,
                            line,
                            character,
                        } => {
                            let lang = language_from_uri(&uri);
                            let clients = registry.get_clients(&lang, &uri).await;
                            for client in clients {
                                if let Ok(Some(hover)) = client.hover(&uri, line, character).await {
                                    let content = crate::lsp_hover::hover_content_to_string(&hover);
                                    let _ = gtk_tx.send(LspResponse::HoverResult {
                                        request_id,
                                        uri: uri.clone(),
                                        version,
                                        contents: content,
                                    });
                                    break;
                                }
                            }
                        }
                        LspRequest::Definition {
                            request_id,
                            uri,
                            version,
                            line,
                            character,
                        } => {
                            let lang = language_from_uri(&uri);
                            let clients = registry.get_clients(&lang, &uri).await;
                            for client in clients {
                                if let Ok(Some(def)) = client.definition(&uri, line, character).await {
                                    let location = match def {
                                        lsp_types::GotoDefinitionResponse::Scalar(loc) => Some(loc),
                                        lsp_types::GotoDefinitionResponse::Array(locs) => {
                                            locs.into_iter().next()
                                        }
                                        lsp_types::GotoDefinitionResponse::Link(links) => links
                                            .into_iter()
                                            .next()
                                            .map(|l| lsp_types::Location {
                                                uri: l.target_uri,
                                                range: l.target_selection_range,
                                            }),
                                    };
                                    if let Some(loc) = location {
                                        let _ = gtk_tx.send(LspResponse::DefinitionResult {
                                            request_id,
                                            source_uri: uri.clone(),
                                            source_version: version,
                                            uri: loc.uri.to_string(),
                                            line: loc.range.start.line,
                                            character: loc.range.start.character,
                                        });
                                        break;
                                    }
                                }
                            }
                        }
                        LspRequest::Formatting {
                            request_id,
                            uri,
                            version,
                            tab_size,
                            insert_spaces,
                        } => {
                            let lang = language_from_uri(&uri);
                            let clients = registry.get_clients(&lang, &uri).await;
                            for client in clients {
                                if let Ok(edits) = client.formatting(&uri, tab_size, insert_spaces).await {
                                    let infos = edits
                                        .into_iter()
                                        .map(|e| crate::lsp_completion::TextEditInfo {
                                            start_line: e.range.start.line,
                                            start_character: e.range.start.character,
                                            end_line: e.range.end.line,
                                            end_character: e.range.end.character,
                                            new_text: e.new_text,
                                        })
                                        .collect();
                                    let _ = gtk_tx.send(LspResponse::FormattingResult {
                                        request_id,
                                        uri: uri.clone(),
                                        version,
                                        edits: infos,
                                    });
                                    break;
                                }
                            }
                        }
                        LspRequest::SignatureHelp {
                            request_id,
                            uri,
                            version,
                            line,
                            character,
                        } => {
                            let lang = language_from_uri(&uri);
                            let clients = registry.get_clients(&lang, &uri).await;
                            for client in clients {
                                if let Ok(result) = client.signature_help(&uri, line, character).await {
                                    let info = result.map(|sh| {
                                        crate::lsp_completion::SignatureHelpInfo {
                                            active_signature: sh.active_signature.unwrap_or(0),
                                            active_parameter: sh.active_parameter.unwrap_or(0),
                                            signatures: sh.signatures.into_iter().map(|sig| {
                                                let params = sig.parameters.unwrap_or_default().into_iter().map(|p| {
                                                    let label = match p.label {
                                                        lsp_types::ParameterLabel::Simple(s) => s,
                                                        lsp_types::ParameterLabel::LabelOffsets([start, end]) => {
                                                            sig.label.get(start as usize..end as usize)
                                                                .unwrap_or("")
                                                                .to_string()
                                                        }
                                                    };
                                                    let doc = p.documentation.map(|d| match d {
                                                        lsp_types::Documentation::String(s) => s,
                                                        lsp_types::Documentation::MarkupContent(m) => m.value,
                                                    });
                                                    crate::lsp_completion::ParameterInfo { label, documentation: doc }
                                                }).collect();
                                                let doc = sig.documentation.map(|d| match d {
                                                    lsp_types::Documentation::String(s) => s,
                                                    lsp_types::Documentation::MarkupContent(m) => m.value,
                                                });
                                                crate::lsp_completion::SignatureInfo {
                                                    label: sig.label,
                                                    documentation: doc,
                                                    parameters: params,
                                                }
                                            }).collect(),
                                        }
                                    });
                                    let _ = gtk_tx.send(LspResponse::SignatureHelpResult {
                                        request_id,
                                        uri: uri.clone(),
                                        version,
                                        signature_help: info,
                                    });
                                    break;
                                }
                            }
                        }
                        LspRequest::References {
                            request_id,
                            uri,
                            version,
                            line,
                            character,
                        } => {
                            let lang = language_from_uri(&uri);
                            let clients = registry.get_clients(&lang, &uri).await;
                            for client in clients {
                                if let Ok(locs) = client.references(&uri, line, character).await {
                                    let infos = locs
                                        .into_iter()
                                        .map(|l| crate::lsp_completion::LocationInfo {
                                            uri: l.uri.to_string(),
                                            start_line: l.range.start.line,
                                            start_character: l.range.start.character,
                                            end_line: l.range.end.line,
                                            end_character: l.range.end.character,
                                        })
                                        .collect();
                                    let _ = gtk_tx.send(LspResponse::ReferencesResult {
                                        request_id,
                                        uri: uri.clone(),
                                        version,
                                        locations: infos,
                                    });
                                    break;
                                }
                            }
                        }
                        LspRequest::CodeAction {
                            request_id,
                            uri,
                            version,
                            start_line,
                            start_column,
                            end_line,
                            end_column,
                            diagnostics,
                        } => {
                            let lang = language_from_uri(&uri);
                            let clients = registry.get_clients(&lang, &uri).await;
                            // Convert DiagnosticInfo to lsp_types::Diagnostic
                            let lsp_diags: Vec<lsp_types::Diagnostic> = diagnostics
                                .into_iter()
                                .map(|d| lsp_types::Diagnostic {
                                    range: lsp_types::Range {
                                        start: lsp_types::Position {
                                            line: d.line,
                                            character: d.character,
                                        },
                                        end: lsp_types::Position {
                                            line: d.end_line,
                                            character: d.end_character,
                                        },
                                    },
                                    severity: Some(match d.severity {
                                        crate::lsp_completion::DiagnosticSeverity::Error => {
                                            lsp_types::DiagnosticSeverity::ERROR
                                        }
                                        crate::lsp_completion::DiagnosticSeverity::Warning => {
                                            lsp_types::DiagnosticSeverity::WARNING
                                        }
                                        crate::lsp_completion::DiagnosticSeverity::Information => {
                                            lsp_types::DiagnosticSeverity::INFORMATION
                                        }
                                        crate::lsp_completion::DiagnosticSeverity::Hint => {
                                            lsp_types::DiagnosticSeverity::HINT
                                        }
                                    }),
                                    message: d.message,
                                    ..Default::default()
                                })
                                .collect();
                            for client in clients {
                                if let Ok(actions) = client
                                    .code_action(
                                        &uri, start_line, start_column, end_line, end_column,
                                        lsp_diags.clone(),
                                    )
                                    .await
                                {
                                    let infos = actions
                                        .into_iter()
                                        .filter_map(|a| match a {
                                            lsp_types::CodeActionOrCommand::CodeAction(ca) => {
                                                let edits = ca
                                                    .edit
                                                    .and_then(|we| we.changes)
                                                    .into_iter()
                                                    .flat_map(|changes| {
                                                        changes.into_iter().flat_map(|(u, edits)| {
                                                            let uri_str = u.to_string();
                                                            edits.into_iter().map(move |e| {
                                                                crate::lsp_completion::WorkspaceTextEditInfo {
                                                                    uri: uri_str.clone(),
                                                                    start_line: e.range.start.line,
                                                                    start_character: e.range.start.character,
                                                                    end_line: e.range.end.line,
                                                                    end_character: e.range.end.character,
                                                                    new_text: e.new_text,
                                                                }
                                                            })
                                                        })
                                                    })
                                                    .collect();
                                                Some(crate::lsp_completion::CodeActionInfo {
                                                    title: ca.title,
                                                    kind: ca.kind.map(|k| k.as_str().to_string()),
                                                    edits,
                                                    is_preferred: ca.is_preferred.unwrap_or(false),
                                                })
                                            }
                                            _ => None,
                                        })
                                        .collect();
                                    let _ = gtk_tx.send(LspResponse::CodeActionResult {
                                        request_id,
                                        uri: uri.clone(),
                                        version,
                                        actions: infos,
                                    });
                                    break;
                                }
                            }
                        }
                        LspRequest::Rename {
                            request_id,
                            uri,
                            version,
                            line,
                            character,
                            new_name,
                        } => {
                            let lang = language_from_uri(&uri);
                            let clients = registry.get_clients(&lang, &uri).await;
                            for client in clients {
                                if let Ok(Some(we)) = client.rename(&uri, line, character, &new_name).await {
                                    let edits: Vec<crate::lsp_completion::WorkspaceTextEditInfo> = we
                                        .changes
                                        .into_iter()
                                        .flat_map(|changes| {
                                            changes.into_iter().flat_map(|(u, edits)| {
                                                let uri_str = u.to_string();
                                                edits.into_iter().map(move |e| {
                                                    crate::lsp_completion::WorkspaceTextEditInfo {
                                                        uri: uri_str.clone(),
                                                        start_line: e.range.start.line,
                                                        start_character: e.range.start.character,
                                                        end_line: e.range.end.line,
                                                        end_character: e.range.end.character,
                                                        new_text: e.new_text,
                                                    }
                                                })
                                            })
                                        })
                                        .collect();
                                    let _ = gtk_tx.send(LspResponse::RenameResult {
                                        request_id,
                                        uri: uri.clone(),
                                        version,
                                        edits,
                                    });
                                    break;
                                }
                            }
                        }
                        LspRequest::PrepareRename {
                            request_id,
                            uri,
                            version,
                            line,
                            character,
                        } => {
                            let lang = language_from_uri(&uri);
                            let clients = registry.get_clients(&lang, &uri).await;
                            for client in clients {
                                if let Ok(result) = client.prepare_rename(&uri, line, character).await {
                                    let (range, placeholder) = match result {
                                        Some(lsp_types::PrepareRenameResponse::Range(r)) => {
                                            (Some(crate::lsp_completion::RangeInfo {
                                                start_line: r.start.line,
                                                start_character: r.start.character,
                                                end_line: r.end.line,
                                                end_character: r.end.character,
                                            }), None)
                                        }
                                        Some(lsp_types::PrepareRenameResponse::RangeWithPlaceholder {
                                            range,
                                            placeholder,
                                        }) => {
                                            (Some(crate::lsp_completion::RangeInfo {
                                                start_line: range.start.line,
                                                start_character: range.start.character,
                                                end_line: range.end.line,
                                                end_character: range.end.character,
                                            }), Some(placeholder))
                                        }
                                        Some(lsp_types::PrepareRenameResponse::DefaultBehavior { .. }) | None => {
                                            (None, None)
                                        }
                                    };
                                    let _ = gtk_tx.send(LspResponse::PrepareRenameResult {
                                        request_id,
                                        uri: uri.clone(),
                                        version,
                                        range,
                                        placeholder,
                                    });
                                    break;
                                }
                            }
                        }
                        LspRequest::Shutdown => {
                            registry.shutdown_all().await;
                        }
                    }
                }
            });
        });
    }

    // Shared document version counter for LSP
    let lsp_doc_versions: Rc<RefCell<std::collections::HashMap<String, i32>>> =
        Rc::new(RefCell::new(std::collections::HashMap::new()));
    let lsp_request_seq: Rc<Cell<u64>> = Rc::new(Cell::new(1));
    let latest_completion_req: Rc<RefCell<std::collections::HashMap<String, u64>>> =
        Rc::new(RefCell::new(std::collections::HashMap::new()));
    let latest_hover_req: Rc<RefCell<std::collections::HashMap<String, u64>>> =
        Rc::new(RefCell::new(std::collections::HashMap::new()));
    let latest_definition_req: Rc<RefCell<std::collections::HashMap<String, u64>>> =
        Rc::new(RefCell::new(std::collections::HashMap::new()));
    // Maps internal LSP seq → Monaco's request_id for definition requests,
    // so we can resolve the correct Monaco promise when the LSP responds.
    let definition_monaco_ids: Rc<RefCell<std::collections::HashMap<u64, u64>>> =
        Rc::new(RefCell::new(std::collections::HashMap::new()));
    let latest_formatting_req: Rc<RefCell<std::collections::HashMap<String, u64>>> =
        Rc::new(RefCell::new(std::collections::HashMap::new()));
    let latest_signature_help_req: Rc<RefCell<std::collections::HashMap<String, u64>>> =
        Rc::new(RefCell::new(std::collections::HashMap::new()));
    let latest_references_req: Rc<RefCell<std::collections::HashMap<String, u64>>> =
        Rc::new(RefCell::new(std::collections::HashMap::new()));
    let latest_code_action_req: Rc<RefCell<std::collections::HashMap<String, u64>>> =
        Rc::new(RefCell::new(std::collections::HashMap::new()));
    let latest_rename_req: Rc<RefCell<std::collections::HashMap<String, u64>>> =
        Rc::new(RefCell::new(std::collections::HashMap::new()));
    let lsp_error_toast_dedupe: Rc<RefCell<HashSet<String>>> =
        Rc::new(RefCell::new(HashSet::new()));
    let (lsp_install_result_tx, lsp_install_result_rx) =
        std::sync::mpsc::channel::<Result<String, String>>();
    let lsp_install_result_rx = Rc::new(RefCell::new(lsp_install_result_rx));

    // Track the current CSS provider so we can swap themes at runtime
    let css_provider: Rc<RefCell<gtk4::CssProvider>> = {
        let theme = crate::theme::get_theme(&settings.borrow().color_scheme);
        Rc::new(RefCell::new(crate::theme::load_css(theme)))
    };

    // Main vertical layout
    let main_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    // Header bar with tab bar
    let header = adw::HeaderBar::new();
    let tab_bar = adw::TabBar::new();
    let tab_view = adw::TabView::new();
    tab_bar.set_view(Some(&tab_view));
    tab_bar.set_autohide(false);
    tab_bar.set_cursor_from_name(Some("pointer"));

    // Tab context menu
    let tab_menu = gio::Menu::new();
    tab_menu.append(Some("New Tab"), Some("tab.new"));
    tab_menu.append(Some("Pin/Unpin Tab"), Some("tab.pin"));
    tab_menu.append(Some("Close Tab"), Some("tab.close"));
    tab_menu.append(Some("Close Other Tabs"), Some("tab.close-others"));
    tab_view.set_menu_model(Some(&tab_menu));

    header.set_title_widget(Some(&tab_bar));

    // Toggle sidebar button (leftmost)
    let sidebar_btn = gtk4::ToggleButton::builder()
        .tooltip_text("Toggle Sidebar (Ctrl+Shift+B)")
        .active(settings.borrow().sidebar_visible)
        .build();
    sidebar_btn.set_cursor_from_name(Some("pointer"));
    header.pack_start(&sidebar_btn);

    // New tab button
    let new_tab_btn = gtk4::Button::new();
    new_tab_btn.set_tooltip_text(Some("New Tab (Ctrl+T)"));
    new_tab_btn.set_cursor_from_name(Some("pointer"));
    header.pack_end(&new_tab_btn);

    // Settings button (right side of header, click handler wired below after tab_view setup)
    let settings_btn = gtk4::Button::new();
    settings_btn.set_tooltip_text(Some("Settings"));
    settings_btn.set_cursor_from_name(Some("pointer"));
    header.pack_start(&settings_btn);

    main_box.append(&header);

    // Horizontal pane: sidebar + tab view
    let paned = gtk4::Paned::new(gtk4::Orientation::Horizontal);
    paned.set_vexpand(true);
    paned.set_position(settings.borrow().sidebar_width);
    paned.set_shrink_start_child(false);
    paned.set_shrink_end_child(false);

    // Sidebar
    let initial_theme = crate::theme::get_theme(&settings.borrow().color_scheme);
    let (sidebar_widget, sidebar_state) = sidebar::build_sidebar(&settings, initial_theme);
    sidebar_widget.set_visible(settings.borrow().sidebar_visible);
    paned.set_start_child(Some(&sidebar_widget));

    // Set header button icons from shared SVG icon cache
    {
        let cache = sidebar_state.icon_cache.borrow();
        if let Some(t) = cache.get_toolbar_icon("toolbar-sidebar") {
            sidebar_btn.set_child(Some(&gtk4::Image::from_paintable(Some(t))));
        }
        if let Some(t) = cache.get_toolbar_icon("toolbar-plus") {
            new_tab_btn.set_child(Some(&gtk4::Image::from_paintable(Some(t))));
        }
        if let Some(t) = cache.get_toolbar_icon("settings") {
            settings_btn.set_child(Some(&gtk4::Image::from_paintable(Some(t))));
        }
    }

    // Terminal search bar (hidden by default)
    let search_revealer = gtk4::Revealer::new();
    search_revealer.set_reveal_child(false);
    search_revealer.set_transition_type(gtk4::RevealerTransitionType::SlideDown);

    let search_bar_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    search_bar_box.add_css_class("terminal-search-bar");

    let find_entry = gtk4::SearchEntry::new();
    find_entry.set_placeholder_text(Some("Find in terminal..."));
    find_entry.set_hexpand(true);

    let find_prev_btn = gtk4::Button::from_icon_name("go-up-symbolic");
    find_prev_btn.set_tooltip_text(Some("Previous Match"));
    let find_next_btn = gtk4::Button::from_icon_name("go-down-symbolic");
    find_next_btn.set_tooltip_text(Some("Next Match"));
    let find_close_btn = gtk4::Button::from_icon_name("window-close-symbolic");
    find_close_btn.set_tooltip_text(Some("Close"));

    search_bar_box.append(&find_entry);
    search_bar_box.append(&find_prev_btn);
    search_bar_box.append(&find_next_btn);
    search_bar_box.append(&find_close_btn);
    search_revealer.set_child(Some(&search_bar_box));

    // Editor search/replace is handled by Monaco's built-in Ctrl+F/Ctrl+H.

    // Tab view in the end pane, wrapped with terminal search bar above
    let right_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    right_box.append(&search_revealer);
    right_box.append(&tab_view);
    tab_view.set_vexpand(true);
    paned.set_end_child(Some(&right_box));

    main_box.append(&paned);

    // Status bar
    let status_bar = status_bar::new_shared();
    main_box.append(&status_bar.borrow().widget);

    let toast_overlay = adw::ToastOverlay::new();
    toast_overlay.set_child(Some(&main_box));
    window.set_content(Some(&toast_overlay));

    // Load initial directory (use last saved directory if available)
    let initial_dir = if !settings.borrow().last_directory.is_empty()
        && std::path::Path::new(&settings.borrow().last_directory).is_dir()
    {
        settings.borrow().last_directory.clone()
    } else {
        impulse_core::shell::get_home_directory().unwrap_or_else(|_| "/".to_string())
    };
    sidebar_state.load_directory(&initial_dir);
    status_bar.borrow().update_cwd(&initial_dir);

    // Initialize project search root to current directory
    *sidebar_state.project_search.current_root.borrow_mut() = initial_dir.clone();

    // Shared state
    let sidebar_state = Rc::new(sidebar_state);

    let lsp_state = context::LspState {
        request_tx: lsp_request_tx.clone(),
        doc_versions: lsp_doc_versions.clone(),
        request_seq: lsp_request_seq.clone(),
        latest_completion_req: latest_completion_req.clone(),
        latest_hover_req: latest_hover_req.clone(),
        latest_definition_req: latest_definition_req.clone(),
        definition_monaco_ids: definition_monaco_ids.clone(),
        error_toast_dedupe: lsp_error_toast_dedupe.clone(),
        latest_formatting_req: latest_formatting_req.clone(),
        latest_signature_help_req: latest_signature_help_req.clone(),
        latest_references_req: latest_references_req.clone(),
        latest_code_action_req: latest_code_action_req.clone(),
        latest_rename_req: latest_rename_req.clone(),
    };

    let open_editor_paths: Rc<RefCell<HashSet<String>>> = Rc::new(RefCell::new(HashSet::new()));
    let editor_tab_pages: Rc<RefCell<HashMap<String, adw::TabPage>>> =
        Rc::new(RefCell::new(HashMap::new()));

    let ctx = context::WindowContext {
        window: window.clone(),
        tab_view: tab_view.clone(),
        sidebar_state: sidebar_state.clone(),
        settings: settings.clone(),
        lsp: lsp_state,
        toast_overlay: toast_overlay.clone(),
        status_bar: status_bar.clone(),
        open_editor_paths,
        editor_tab_pages,
    };

    sidebar_signals::wire_sidebar_signals(&ctx);

    let setup_terminal_signals =
        tab_management::make_setup_terminal_signals(&tab_view, &status_bar, &sidebar_state);

    let create_tab = tab_management::make_create_tab(
        &tab_view,
        &setup_terminal_signals,
        &settings,
        &copy_on_select_flag,
        &shell_cache,
        &sidebar_state.icon_cache,
    );

    // Create first tab
    (create_tab.clone())();

    // Restore previously open editor tabs
    for file_path in &settings.borrow().open_files.clone() {
        if std::path::Path::new(file_path).exists() {
            if let Some(cb) = sidebar_state.on_file_activated.borrow().as_ref() {
                cb(file_path);
            }
        }
    }

    // Switch back to first tab (terminal) after restoring editor tabs
    if tab_view.n_pages() > 0 {
        tab_view.set_selected_page(&tab_view.nth_page(0));
    }

    // New tab button
    {
        let create_tab = create_tab.clone();
        new_tab_btn.connect_clicked(move |_| {
            create_tab();
        });
    }

    // Toggle sidebar
    {
        let sidebar_widget = sidebar_widget.clone();
        sidebar_btn.connect_toggled(move |btn: &gtk4::ToggleButton| {
            sidebar_widget.set_visible(btn.is_active());
        });
    }

    tab_management::setup_tab_context_menu(&window, &tab_view, &create_tab);

    tab_management::setup_lsp_response_polling(&ctx, &lsp_gtk_rx, &lsp_install_result_rx);

    // Shared closure for reopening the most recently closed editor/image tab.
    // Defined early so it can be used by both the capture-phase key handler and
    // the shortcut controller.
    let reopen_tab: Rc<dyn Fn()> = {
        let closed_tabs = closed_tabs.clone();
        let sidebar_state = sidebar_state.clone();
        Rc::new(move || {
            let closed = closed_tabs.borrow_mut().pop();
            if let Some(entry) = closed {
                let path = match &entry {
                    ClosedTab::Editor(p) | ClosedTab::ImagePreview(p) => p.clone(),
                };
                // Only reopen if the file still exists on disk
                if std::path::Path::new(&path).exists() {
                    if let Some(cb) = sidebar_state.on_file_activated.borrow().as_ref() {
                        cb(&path);
                    }
                }
            }
        })
    };

    let term_ctx = context::TerminalContext {
        copy_on_select: copy_on_select_flag.clone(),
        font_size: font_size.clone(),
        shell_cache: shell_cache.clone(),
    };

    keybinding_setup::setup_capture_phase_keys(
        &ctx,
        &term_ctx,
        &sidebar_btn,
        &setup_terminal_signals,
        &create_tab,
        &reopen_tab,
    );

    // Wire the markdown preview toggle button in the status bar
    {
        let tab_view = tab_view.clone();
        let settings = settings.clone();
        let status_bar_for_click = status_bar.clone();
        let preview_btn = status_bar.borrow().preview_button.clone();
        preview_btn.connect_clicked(move |_| {
            if let Some(page) = tab_view.selected_page() {
                let child = page.child();
                if editor::is_editor(&child) {
                    let s = settings.borrow();
                    let theme = crate::theme::get_theme(&s.color_scheme);
                    if let Some(is_previewing) =
                        editor::toggle_markdown_preview(child.upcast_ref(), theme)
                    {
                        status_bar_for_click
                            .borrow()
                            .show_preview_button(is_previewing);
                    }
                }
            }
        });
    }

    let kb_overrides = settings.borrow().keybinding_overrides.clone();

    // Shared closure to open settings and apply changes live
    let open_settings: Rc<dyn Fn()> = {
        let window_ref = window.clone();
        let settings = settings.clone();
        let tab_view = tab_view.clone();
        let css_provider = css_provider.clone();
        let copy_on_select_flag = copy_on_select_flag.clone();
        let font_size = font_size.clone();
        let sidebar_state = sidebar_state.clone();
        Rc::new(move || {
            let tab_view = tab_view.clone();
            let css_provider = css_provider.clone();
            let copy_on_select_flag = copy_on_select_flag.clone();
            let font_size = font_size.clone();
            let sidebar_state = sidebar_state.clone();
            crate::settings_page::show_settings_window(&window_ref, &settings, move |s| {
                // Keep the font_size Cell in sync so the close handler
                // doesn't overwrite the user's settings-page changes.
                font_size.set(s.font_size);
                // Swap theme CSS
                let new_theme = crate::theme::get_theme(&s.color_scheme);
                let display = gtk4::gdk::Display::default().expect("No display");
                gtk4::style_context_remove_provider_for_display(&display, &*css_provider.borrow());
                let new_provider = crate::theme::load_css(new_theme);
                *css_provider.borrow_mut() = new_provider;

                // Update sidebar file icons for the new theme
                sidebar_state.update_theme(new_theme);

                // Apply to all open tabs
                for i in 0..tab_view.n_pages() {
                    let page = tab_view.nth_page(i);
                    let child = page.child();
                    if let Some(term) = crate::terminal_container::get_active_terminal(&child) {
                        crate::terminal::apply_settings(&term, s, new_theme, &copy_on_select_flag);
                    } else if crate::editor::is_editor(&child) {
                        crate::editor::apply_settings(child.upcast_ref::<gtk4::Widget>(), s);
                        crate::editor::apply_theme(child.upcast_ref::<gtk4::Widget>(), new_theme);
                        // Re-render markdown preview if currently previewing
                        crate::editor::refresh_markdown_preview(
                            child.upcast_ref::<gtk4::Widget>(),
                            new_theme,
                        );
                    }
                }
            });
        })
    };

    // Wire the settings button
    {
        let open_settings = open_settings.clone();
        settings_btn.connect_clicked(move |_| {
            open_settings();
        });
    }

    // Build command list for the command palette
    let commands = {
        let create_tab = create_tab.clone();
        let tab_view = tab_view.clone();
        let sidebar_btn = sidebar_btn.clone();
        let window_ref = window.clone();
        let sidebar_state = sidebar_state.clone();
        let setup_terminal_signals = setup_terminal_signals.clone();
        let toast_overlay = toast_overlay.clone();
        let lsp_install_result_tx = lsp_install_result_tx.clone();

        vec![
            Command {
                name: "New Terminal Tab".to_string(),
                shortcut: keybindings::accel_to_display(&keybindings::get_accel(
                    "new_tab",
                    &kb_overrides,
                )),
                action: Rc::new({
                    let create_tab = create_tab.clone();
                    move || create_tab()
                }),
            },
            Command {
                name: "Close Tab".to_string(),
                shortcut: keybindings::accel_to_display(&keybindings::get_accel(
                    "close_tab",
                    &kb_overrides,
                )),
                action: Rc::new({
                    let tab_view = tab_view.clone();
                    move || {
                        if let Some(page) = tab_view.selected_page() {
                            tab_view.close_page(&page);
                        }
                    }
                }),
            },
            Command {
                name: "Reopen Closed Tab".to_string(),
                shortcut: keybindings::accel_to_display(&keybindings::get_accel(
                    "reopen_tab",
                    &kb_overrides,
                )),
                action: Rc::new({
                    let reopen_tab = reopen_tab.clone();
                    move || reopen_tab()
                }),
            },
            Command {
                name: "Toggle Sidebar".to_string(),
                shortcut: keybindings::accel_to_display(&keybindings::get_accel(
                    "toggle_sidebar",
                    &kb_overrides,
                )),
                action: Rc::new({
                    let sidebar_btn = sidebar_btn.clone();
                    move || sidebar_btn.set_active(!sidebar_btn.is_active())
                }),
            },
            Command {
                name: "Quick Open File".to_string(),
                shortcut: "".to_string(),
                action: Rc::new({
                    let window_ref = window_ref.clone();
                    let sidebar_state = sidebar_state.clone();
                    move || show_quick_open(&window_ref, &sidebar_state)
                }),
            },
            Command {
                name: "Find in Project".to_string(),
                shortcut: keybindings::accel_to_display(&keybindings::get_accel(
                    "project_search",
                    &kb_overrides,
                )),
                action: Rc::new({
                    let sidebar_btn = sidebar_btn.clone();
                    let sidebar_state = sidebar_state.clone();
                    move || {
                        // Show sidebar and switch to search tab
                        if !sidebar_btn.is_active() {
                            sidebar_btn.set_active(true);
                        }
                        sidebar_state.search_btn.set_active(true);
                        sidebar_state.project_search.search_entry.grab_focus();
                    }
                }),
            },
            Command {
                name: "Toggle Fullscreen".to_string(),
                shortcut: keybindings::accel_to_display(&keybindings::get_accel(
                    "fullscreen",
                    &kb_overrides,
                )),
                action: Rc::new({
                    let window_ref = window_ref.clone();
                    move || {
                        if window_ref.is_fullscreen() {
                            window_ref.unfullscreen();
                        } else {
                            window_ref.fullscreen();
                        }
                    }
                }),
            },
            Command {
                name: "New Window".to_string(),
                shortcut: keybindings::accel_to_display(&keybindings::get_accel(
                    "new_window",
                    &kb_overrides,
                )),
                action: Rc::new({
                    let app = app.clone();
                    move || build_window(&app)
                }),
            },
            Command {
                name: "Split Terminal Horizontally".to_string(),
                shortcut: keybindings::accel_to_display(&keybindings::get_accel(
                    "split_horizontal",
                    &kb_overrides,
                )),
                action: Rc::new({
                    let tab_view = tab_view.clone();
                    let setup_terminal_signals = setup_terminal_signals.clone();
                    let settings = settings.clone();
                    let copy_on_select_flag = copy_on_select_flag.clone();
                    let shell_cache = shell_cache.clone();
                    move || {
                        if let Some(page) = tab_view.selected_page() {
                            let child = page.child();
                            let setup = setup_terminal_signals.clone();
                            let s = settings.borrow();
                            let theme = crate::theme::get_theme(&s.color_scheme);
                            terminal_container::split_terminal(
                                &child,
                                gtk4::Orientation::Horizontal,
                                &|term| {
                                    setup(term);
                                },
                                &s,
                                theme,
                                copy_on_select_flag.clone(),
                                &shell_cache,
                            );
                        }
                    }
                }),
            },
            Command {
                name: "Split Terminal Vertically".to_string(),
                shortcut: keybindings::accel_to_display(&keybindings::get_accel(
                    "split_vertical",
                    &kb_overrides,
                )),
                action: Rc::new({
                    let tab_view = tab_view.clone();
                    let setup_terminal_signals = setup_terminal_signals.clone();
                    let settings = settings.clone();
                    let copy_on_select_flag = copy_on_select_flag.clone();
                    let shell_cache = shell_cache.clone();
                    move || {
                        if let Some(page) = tab_view.selected_page() {
                            let child = page.child();
                            let setup = setup_terminal_signals.clone();
                            let s = settings.borrow();
                            let theme = crate::theme::get_theme(&s.color_scheme);
                            terminal_container::split_terminal(
                                &child,
                                gtk4::Orientation::Vertical,
                                &|term| {
                                    setup(term);
                                },
                                &s,
                                theme,
                                copy_on_select_flag.clone(),
                                &shell_cache,
                            );
                        }
                    }
                }),
            },
            Command {
                name: "Open Settings".to_string(),
                shortcut: keybindings::accel_to_display(&keybindings::get_accel(
                    "open_settings",
                    &kb_overrides,
                )),
                action: Rc::new({
                    let open_settings = open_settings.clone();
                    move || {
                        open_settings();
                    }
                }),
            },
            Command {
                name: "Toggle Markdown Preview".to_string(),
                shortcut: keybindings::accel_to_display(&keybindings::get_accel(
                    "toggle_markdown_preview",
                    &kb_overrides,
                )),
                action: Rc::new({
                    let tab_view = tab_view.clone();
                    let settings = settings.clone();
                    let status_bar = status_bar.clone();
                    move || {
                        if let Some(page) = tab_view.selected_page() {
                            let child = page.child();
                            if editor::is_editor(&child) {
                                let s = settings.borrow();
                                let theme = crate::theme::get_theme(&s.color_scheme);
                                if let Some(is_previewing) =
                                    editor::toggle_markdown_preview(child.upcast_ref(), theme)
                                {
                                    status_bar.borrow().show_preview_button(is_previewing);
                                }
                            }
                        }
                    }
                }),
            },
            Command {
                name: "Install Web LSP Servers".to_string(),
                shortcut: "".to_string(),
                action: Rc::new({
                    let toast_overlay = toast_overlay.clone();
                    let lsp_install_result_tx = lsp_install_result_tx.clone();
                    move || {
                        let start_toast = adw::Toast::new(
                            "Installing web LSP servers (TypeScript, PHP, HTML/CSS, etc.)...",
                        );
                        start_toast.set_timeout(3);
                        toast_overlay.add_toast(start_toast);

                        let tx = lsp_install_result_tx.clone();
                        std::thread::spawn(move || {
                            if let Err(e) =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    let result =
                                        impulse_core::lsp::install_managed_web_lsp_servers().map(
                                            |bin_dir| {
                                                format!(
                                                    "Installed managed LSP servers to {}",
                                                    bin_dir.display()
                                                )
                                            },
                                        );
                                    let _ = tx.send(result);
                                }))
                            {
                                log::error!("Background thread panicked: {:?}", e);
                            }
                        });
                    }
                }),
            },
        ]
    };

    keybinding_setup::setup_shortcut_controller(
        &ctx,
        &term_ctx,
        app,
        &sidebar_btn,
        &setup_terminal_signals,
        &open_settings,
        &search_revealer,
        &find_entry,
        &commands,
        &create_tab,
        &reopen_tab,
    );

    // --- Terminal search bar wiring ---

    // Search entry text changed -> set regex on active terminal
    {
        let tab_view_ref = tab_view.clone();
        find_entry.connect_search_changed(move |entry| {
            run_guarded_ui("terminal-search-changed", || {
                let text = entry.text().to_string();
                if let Some(page) = tab_view_ref.selected_page() {
                    let child = page.child();
                    if let Some(term) = find_vte_terminal(&child) {
                        if text.is_empty() {
                            term.search_set_regex(None::<&vte4::Regex>, 0);
                        } else {
                            let escaped = regex_escape(&text);
                            if let Ok(regex) = vte4::Regex::for_search(&escaped, 0) {
                                term.search_set_regex(Some(&regex), 0);
                                term.search_find_next();
                            }
                        }
                    }
                }
            });
        });
    }

    // Next button
    {
        let tab_view_ref = tab_view.clone();
        find_next_btn.connect_clicked(move |_| {
            if let Some(page) = tab_view_ref.selected_page() {
                if let Some(term) = find_vte_terminal(&page.child()) {
                    term.search_find_next();
                }
            }
        });
    }

    // Previous button
    {
        let tab_view_ref = tab_view.clone();
        find_prev_btn.connect_clicked(move |_| {
            if let Some(page) = tab_view_ref.selected_page() {
                if let Some(term) = find_vte_terminal(&page.child()) {
                    term.search_find_previous();
                }
            }
        });
    }

    // Close button
    {
        let search_revealer_ref = search_revealer.clone();
        let tab_view_ref = tab_view.clone();
        find_close_btn.connect_clicked(move |_| {
            search_revealer_ref.set_reveal_child(false);
            if let Some(page) = tab_view_ref.selected_page() {
                if let Some(term) = find_vte_terminal(&page.child()) {
                    term.search_set_regex(None::<&vte4::Regex>, 0);
                    term.grab_focus();
                }
            }
        });
    }

    // Escape in search entry closes the bar
    {
        let search_revealer_ref = search_revealer.clone();
        let tab_view_ref = tab_view.clone();
        let key_controller = gtk4::EventControllerKey::new();
        key_controller.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                search_revealer_ref.set_reveal_child(false);
                if let Some(page) = tab_view_ref.selected_page() {
                    if let Some(term) = find_vte_terminal(&page.child()) {
                        term.search_set_regex(None::<&vte4::Regex>, 0);
                        term.grab_focus();
                    }
                }
                return gtk4::glib::Propagation::Stop;
            }
            gtk4::glib::Propagation::Proceed
        });
        find_entry.add_controller(key_controller);
    }

    // Hide terminal search bar when switching to an editor tab
    {
        let search_revealer = search_revealer.clone();
        tab_view.connect_selected_page_notify(move |tv| {
            if let Some(page) = tv.selected_page() {
                let child = page.child();
                if editor::is_editor(&child) && search_revealer.reveals_child() {
                    search_revealer.set_reveal_child(false);
                }
            }
        });
    }

    // Set the initial active tab for tree state tracking
    if let Some(page) = tab_view.selected_page() {
        sidebar_state.set_active_tab(&page.child());
    }

    tab_management::setup_tab_switch_handler(&tab_view, &status_bar, &sidebar_state);

    tab_management::setup_tab_close_handler(&ctx, &create_tab, &closed_tabs);

    // Save settings when window is closed
    {
        let paned = paned.clone();
        let sidebar_btn = sidebar_btn.clone();
        let sidebar_state = sidebar_state.clone();
        let tab_view_ref = tab_view.clone();
        let font_size = font_size.clone();
        let settings = settings.clone();
        let lsp_tx = lsp_request_tx.clone();
        window.connect_close_request(move |window| {
            // Shutdown LSP servers
            if let Err(e) = lsp_tx.try_send(LspRequest::Shutdown) {
                log::warn!("LSP request channel full, dropping shutdown request: {}", e);
            }
            // Collect open editor file paths
            let mut open_files = Vec::new();
            let n = tab_view_ref.n_pages();
            for i in 0..n {
                let page = tab_view_ref.nth_page(i);
                let child = page.child();
                if editor::is_editor(&child) {
                    let path = child.widget_name().to_string();
                    if !path.is_empty() && path != "GtkBox" {
                        open_files.push(path);
                    }
                }
            }

            // Merge window state into existing settings (preserve all user prefs)
            {
                let mut s = settings.borrow_mut();
                s.window_width = window.width();
                s.window_height = window.height();
                s.sidebar_visible = sidebar_btn.is_active();
                s.sidebar_width = paned.position();
                s.last_directory = sidebar_state.current_path.borrow().clone();
                s.font_size = font_size.get();
                s.open_files = open_files;
            }
            crate::settings::save(&settings.borrow());
            gtk4::glib::Propagation::Proceed
        });
    }

    window.present();
}

fn apply_font_size_to_all_terminals(tab_view: &adw::TabView, size: i32, font_family: &str) {
    let family = if font_family.is_empty() {
        "JetBrains Mono"
    } else {
        font_family
    };
    let n = tab_view.n_pages();
    for i in 0..n {
        let page = tab_view.nth_page(i);
        for term in terminal_container::collect_terminals(&page.child()) {
            let mut font_desc = gtk4::pango::FontDescription::from_string(family);
            font_desc.set_size(size * 1024);
            term.set_font_desc(Some(&font_desc));
        }
    }
}

pub fn send_diff_decorations(handle: &crate::editor_webview::MonacoEditorHandle, file_path: &str) {
    let file_path_owned = file_path.to_string();
    gtk4::glib::spawn_future_local(async move {
        let fp = file_path_owned.clone();
        let result = gtk4::gio::spawn_blocking(move || impulse_core::git::get_file_diff(&fp)).await;
        let decorations = match result {
            Ok(Ok(diff)) => {
                let mut decos: Vec<impulse_editor::protocol::DiffDecoration> = diff
                    .changed_lines
                    .iter()
                    .filter_map(|(&line, status)| {
                        let diff_status = match status {
                            impulse_core::git::DiffLineStatus::Added => {
                                impulse_editor::protocol::DiffStatus::Added
                            }
                            impulse_core::git::DiffLineStatus::Modified => {
                                impulse_editor::protocol::DiffStatus::Modified
                            }
                            impulse_core::git::DiffLineStatus::Unchanged => return None,
                        };
                        Some(impulse_editor::protocol::DiffDecoration {
                            line,
                            status: diff_status,
                        })
                    })
                    .collect();
                for &line in &diff.deleted_lines {
                    decos.push(impulse_editor::protocol::DiffDecoration {
                        line,
                        status: impulse_editor::protocol::DiffStatus::Deleted,
                    });
                }
                decos
            }
            _ => vec![],
        };
        // Re-lookup the handle on the main thread (the Rc may have been dropped during async)
        if let Some(handle) = crate::editor::get_handle(&file_path_owned) {
            handle.apply_diff_decorations(decorations);
        }
    });
}

/// Runs all matching commands-on-save for the given file path.
/// Returns `true` if any successful command had `reload_file` set.
fn run_commands_on_save(path: &str, commands: &[crate::settings::CommandOnSave]) -> bool {
    let mut needs_reload = false;
    for cmd in commands {
        if crate::settings::matches_file_pattern(path, &cmd.file_pattern) {
            let mut command = std::process::Command::new(&cmd.command);
            command.args(&cmd.args);
            command.arg("--").arg(path);
            match command.output() {
                Ok(output) => {
                    if !output.status.success() {
                        log::warn!(
                            "Command '{}' failed for {}: {}",
                            cmd.name,
                            path,
                            String::from_utf8_lossy(&output.stderr)
                        );
                    } else {
                        log::info!("Command '{}' succeeded for {}", cmd.name, path);
                        if cmd.reload_file {
                            needs_reload = true;
                        }
                    }
                }
                Err(e) => log::warn!("Failed to run command '{}': {}", cmd.name, e),
            }
        }
    }
    needs_reload
}

fn add_shortcut(controller: &gtk4::ShortcutController, accel: &str, callback: impl Fn() + 'static) {
    let trigger = gtk4::ShortcutTrigger::parse_string(accel);
    let action = gtk4::CallbackAction::new(move |_widget, _args| {
        callback();
        gtk4::glib::Propagation::Stop
    });
    if let Some(trigger) = trigger {
        let shortcut = gtk4::Shortcut::new(Some(trigger), Some(action));
        controller.add_shortcut(shortcut);
    }
}

fn find_vte_terminal(widget: &gtk4::Widget) -> Option<vte4::Terminal> {
    if let Some(term) = widget.downcast_ref::<vte4::Terminal>() {
        return Some(term.clone());
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(term) = find_vte_terminal(&c) {
            return Some(term);
        }
        child = c.next_sibling();
    }
    None
}

fn regex_escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len() * 2);
    for c in text.chars() {
        match c {
            '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' => {
                escaped.push('\\');
                escaped.push(c);
            }
            _ => escaped.push(c),
        }
    }
    escaped
}

pub(crate) fn run_guarded_ui<F: FnOnce()>(label: &str, f: F) {
    if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        let msg = if let Some(s) = payload.downcast_ref::<&str>() {
            *s
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.as_str()
        } else {
            "non-string panic payload"
        };
        log::error!("UI callback panic in '{}': {}", label, msg);
    }
}

fn completion_text_edit_to_info(
    edit: lsp_types::CompletionTextEdit,
) -> crate::lsp_completion::TextEditInfo {
    match edit {
        lsp_types::CompletionTextEdit::Edit(edit) => crate::lsp_completion::TextEditInfo {
            start_line: edit.range.start.line,
            start_character: edit.range.start.character,
            end_line: edit.range.end.line,
            end_character: edit.range.end.character,
            new_text: edit.new_text,
        },
        lsp_types::CompletionTextEdit::InsertAndReplace(edit) => {
            crate::lsp_completion::TextEditInfo {
                start_line: edit.replace.start.line,
                start_character: edit.replace.start.character,
                end_line: edit.replace.end.line,
                end_character: edit.replace.end.character,
                new_text: edit.new_text,
            }
        }
    }
}

fn completion_item_to_info(
    item: lsp_types::CompletionItem,
) -> crate::lsp_completion::CompletionInfo {
    let text_edit = item.text_edit.map(completion_text_edit_to_info);
    let additional_text_edits = item
        .additional_text_edits
        .unwrap_or_default()
        .into_iter()
        .map(|edit| crate::lsp_completion::TextEditInfo {
            start_line: edit.range.start.line,
            start_character: edit.range.start.character,
            end_line: edit.range.end.line,
            end_character: edit.range.end.character,
            new_text: edit.new_text,
        })
        .collect();

    crate::lsp_completion::CompletionInfo {
        label: item.label,
        detail: item.detail,
        insert_text: item.insert_text,
        insert_text_format: item.insert_text_format,
        text_edit,
        additional_text_edits,
        kind: format!(
            "{:?}",
            item.kind.unwrap_or(lsp_types::CompletionItemKind::TEXT)
        ),
    }
}

/// Get the working directory from the active tab: the terminal's CWD (via OSC 7)
/// or the parent directory of the file in an editor tab.
fn get_active_cwd(tab_view: &adw::TabView) -> Option<String> {
    let page = tab_view.selected_page()?;
    let child = page.child();

    // Try terminal CWD first
    if let Some(term) = terminal_container::get_active_terminal(&child) {
        if let Some(uri) = term.current_directory_uri() {
            let path = uri_to_file_path(uri.as_ref());
            if !path.is_empty() {
                return Some(path);
            }
        }
    }

    // Try editor file's parent directory
    if editor::is_editor(&child) {
        let file_path = child.widget_name().to_string();
        if let Some(parent) = std::path::Path::new(&file_path).parent() {
            return Some(parent.to_string_lossy().to_string());
        }
    }

    None
}

fn file_path_to_uri(path: &std::path::Path) -> Option<String> {
    impulse_core::util::file_path_to_uri(path)
}

fn uri_to_file_path(uri: &str) -> String {
    impulse_core::util::uri_to_file_path(uri)
}

fn language_from_uri(uri: &str) -> String {
    impulse_core::util::language_from_uri(uri)
}
