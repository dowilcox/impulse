//! "Review Changes" tab: a WebKitGTK WebView hosting impulse-editor's
//! review.html stacked-diff page, with a native header (repo, branch, file
//! count, +/- totals, refresh) above it and a commit bar (message entry +
//! Commit button) below. Mirrors the macOS `DiffReviewTab`.

use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use webkit6::prelude::*;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use impulse_editor::protocol::{ReviewCommand, ReviewEvent, ReviewFileEntry};

use crate::theme::ThemeColors;

/// Widget name identifying review tabs in the tab view.
pub const REVIEW_TAB_NAME: &str = "impulse-review-tab";

thread_local! {
    static HANDLES: RefCell<Vec<(gtk4::Box, Rc<ReviewTabHandle>)>> = const { RefCell::new(Vec::new()) };
}

pub struct ReviewTabHandle {
    webview: webkit6::WebView,
    repo_root: RefCell<String>,
    is_ready: Cell<bool>,
    /// Bumped per reload so a stale async result is dropped.
    load_generation: Cell<u64>,
    /// Repo-relative paths from the last Render, gating discard requests.
    known_paths: RefCell<std::collections::HashSet<String>>,
    repo_label: gtk4::Label,
    branch_label: gtk4::Label,
    count_label: gtk4::Label,
    added_label: gtk4::Label,
    removed_label: gtk4::Label,
    commit_entry: gtk4::Entry,
    commit_btn: gtk4::Button,
    confirmation_label: gtk4::Label,
}

/// Check if a widget is a Review Changes tab.
pub fn is_review_tab(widget: &gtk4::Widget) -> bool {
    widget
        .downcast_ref::<gtk4::Box>()
        .is_some_and(|bx| bx.widget_name() == REVIEW_TAB_NAME)
}

fn handle_for_widget(widget: &gtk4::Widget) -> Option<Rc<ReviewTabHandle>> {
    let bx = widget.downcast_ref::<gtk4::Box>()?;
    HANDLES.with(|handles| {
        handles
            .borrow()
            .iter()
            .find(|(container, _)| container == bx)
            .map(|(_, handle)| handle.clone())
    })
}

/// Re-theme an open review tab (settings change).
pub fn apply_theme(widget: &gtk4::Widget, theme: &ThemeColors) {
    if let Some(handle) = handle_for_widget(widget) {
        handle.apply_theme(theme);
    }
}

/// Reload the changed-file list of an open review tab.
pub fn refresh(widget: &gtk4::Widget) {
    if let Some(handle) = handle_for_widget(widget) {
        handle.reload_and_render();
    }
}

/// Build a review tab for the repository containing `repo_root`.
pub fn create_review_tab(repo_root: &str, theme: &'static ThemeColors) -> gtk4::Box {
    let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    container.set_widget_name(REVIEW_TAB_NAME);
    container.set_hexpand(true);
    container.set_vexpand(true);
    container.add_css_class("review-tab");

    // --- Header: repo name, branch, file count, +N/-N, refresh ---
    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
    header.add_css_class("review-header");

    let repo_label = gtk4::Label::new(None);
    repo_label.add_css_class("review-repo");
    header.append(&repo_label);

    let branch_label = gtk4::Label::new(None);
    branch_label.add_css_class("review-branch");
    header.append(&branch_label);

    let count_label = gtk4::Label::new(None);
    count_label.add_css_class("review-count");
    count_label.set_hexpand(true);
    count_label.set_halign(gtk4::Align::Start);
    header.append(&count_label);

    let added_label = gtk4::Label::new(None);
    added_label.add_css_class("review-added");
    header.append(&added_label);

    let removed_label = gtk4::Label::new(None);
    removed_label.add_css_class("review-removed");
    header.append(&removed_label);

    let refresh_btn = gtk4::Button::from_icon_name("view-refresh-symbolic");
    refresh_btn.add_css_class("flat");
    refresh_btn.set_tooltip_text(Some("Refresh"));
    refresh_btn.set_cursor_from_name(Some("pointer"));
    header.append(&refresh_btn);

    container.append(&header);
    container.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

    // --- WebView hosting review.html ---
    let user_content_manager = webkit6::UserContentManager::new();
    let webview = webkit6::WebView::builder()
        .user_content_manager(&user_content_manager)
        .hexpand(true)
        .vexpand(true)
        .build();
    let bg_rgba =
        gtk4::gdk::RGBA::parse(theme.bg).unwrap_or(gtk4::gdk::RGBA::new(0.17, 0.14, 0.27, 1.0));
    webview.set_background_color(&bg_rgba);
    if let Some(wk_settings) = webkit6::prelude::WebViewExt::settings(&webview) {
        wk_settings.set_enable_javascript(true);
        if std::env::var("IMPULSE_DEVTOOLS")
            .ok()
            .is_some_and(|v| v == "1")
        {
            wk_settings.set_enable_developer_extras(true);
        }
        wk_settings.set_allow_file_access_from_file_urls(false);
    }
    container.append(&webview);

    // --- Commit bar: message entry + Commit button ---
    container.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
    let commit_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
    commit_bar.add_css_class("review-commit-bar");

    let commit_entry = gtk4::Entry::new();
    commit_entry.set_placeholder_text(Some("Commit message"));
    commit_entry.set_hexpand(true);
    commit_bar.append(&commit_entry);

    let confirmation_label = gtk4::Label::new(None);
    confirmation_label.add_css_class("review-confirmation");
    confirmation_label.set_visible(false);
    commit_bar.append(&confirmation_label);

    let commit_btn = gtk4::Button::with_label("Commit");
    commit_btn.add_css_class("suggested-action");
    commit_btn.set_sensitive(false);
    commit_bar.append(&commit_btn);
    container.append(&commit_bar);

    let handle = Rc::new(ReviewTabHandle {
        webview: webview.clone(),
        repo_root: RefCell::new(repo_root.to_string()),
        is_ready: Cell::new(false),
        load_generation: Cell::new(0),
        known_paths: RefCell::new(std::collections::HashSet::new()),
        repo_label,
        branch_label,
        count_label,
        added_label,
        removed_label,
        commit_entry: commit_entry.clone(),
        commit_btn: commit_btn.clone(),
        confirmation_label,
    });
    handle.update_repo_label();

    HANDLES.with(|handles| {
        handles
            .borrow_mut()
            .push((container.clone(), handle.clone()))
    });
    {
        let webview = webview.clone();
        container.connect_destroy(move |container| {
            HANDLES.with(|handles| {
                handles.borrow_mut().retain(|(c, _)| c != container);
            });
            if let Some(ucm) = webview.user_content_manager() {
                ucm.unregister_script_message_handler("impulseReview", None);
            }
        });
    }

    // JS -> Rust events.
    let initial_theme = theme;
    user_content_manager.register_script_message_handler("impulseReview", None);
    {
        let handle = handle.clone();
        let container = container.clone();
        user_content_manager.connect_script_message_received(
            Some("impulseReview"),
            move |_ucm, value| {
                let json = value.to_str().to_string();
                let event: ReviewEvent = match serde_json::from_str(&json) {
                    Ok(event) => event,
                    Err(e) => {
                        log::warn!("Failed to parse ReviewEvent: {} (json: {})", e, json);
                        return;
                    }
                };
                match event {
                    ReviewEvent::Ready => {
                        handle.is_ready.set(true);
                        handle.apply_theme(initial_theme);
                        handle.reload_and_render();
                    }
                    ReviewEvent::RequestDiff { path } => handle.load_diff(&path),
                    ReviewEvent::Discard { path } => handle.confirm_and_discard(&container, &path),
                    ReviewEvent::ToggleFile { .. } => {
                        // Expansion is tracked client-side; nothing to do natively.
                    }
                    ReviewEvent::Refresh => handle.reload_and_render(),
                }
            },
        );
    }

    // Block navigation away from the local page.
    webview.connect_decide_policy(|_wv, decision, decision_type| {
        if decision_type == webkit6::PolicyDecisionType::NavigationAction {
            if let Some(nav) = decision.downcast_ref::<webkit6::NavigationPolicyDecision>() {
                if let Some(mut action) = nav.navigation_action() {
                    if let Some(request) = action.request() {
                        if let Some(uri) = request.uri() {
                            let scheme = uri.split(':').next().unwrap_or("");
                            if scheme != "file" && scheme != "about" {
                                decision.ignore();
                                return true;
                            }
                        }
                    }
                }
            }
        }
        false
    });

    // Native chrome wiring.
    {
        let handle = handle.clone();
        refresh_btn.connect_clicked(move |_| handle.reload_and_render());
    }
    {
        let commit_btn = commit_btn.clone();
        commit_entry.connect_changed(move |entry| {
            commit_btn.set_sensitive(!entry.text().trim().is_empty());
        });
    }
    {
        let handle = handle.clone();
        commit_entry.connect_activate(move |_| handle.perform_commit());
    }
    {
        let handle = handle.clone();
        commit_btn.connect_clicked(move |_| handle.perform_commit());
    }

    match impulse_editor::assets::ensure_monaco_extracted() {
        Ok(monaco_dir) => {
            let uri = format!("file://{}/review.html", monaco_dir.display());
            webview.load_uri(&uri);
        }
        Err(e) => {
            log::error!("Failed to extract review assets: {}", e);
            let error_html = format!(
                "<html><body style=\"background:{};color:{};font-family:sans-serif;\
                 display:flex;align-items:center;justify-content:center;height:100vh;\">\
                 <div>Could not load the review editor: {}</div></body></html>",
                theme.bg, theme.fg, e
            );
            webview.load_html(&error_html, None);
        }
    }

    container
}

impl ReviewTabHandle {
    fn send_command(&self, command: &ReviewCommand) {
        if !self.is_ready.get() {
            return;
        }
        let json = match serde_json::to_string(command) {
            Ok(json) => json,
            Err(e) => {
                log::error!("Failed to serialize ReviewCommand: {}", e);
                return;
            }
        };
        let script = format!("window.__applyReviewCommand({json});");
        self.webview.evaluate_javascript(
            &script,
            None,
            None,
            None::<&gtk4::gio::Cancellable>,
            |_| {},
        );
    }

    fn apply_theme(&self, theme: &ThemeColors) {
        let bg_rgba =
            gtk4::gdk::RGBA::parse(theme.bg).unwrap_or(gtk4::gdk::RGBA::new(0.17, 0.14, 0.27, 1.0));
        self.webview.set_background_color(&bg_rgba);
        self.send_command(&ReviewCommand::SetTheme {
            theme: Box::new(crate::editor_webview::theme_to_monaco(theme)),
        });
    }

    fn update_repo_label(&self) {
        let repo_root = self.repo_root.borrow();
        let name = std::path::Path::new(repo_root.as_str())
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Review Changes".to_string());
        self.repo_label.set_text(&name);
    }

    /// Reload the changed-file list off the main thread and push a Render.
    fn reload_and_render(self: &Rc<Self>) {
        let generation = self.load_generation.get() + 1;
        self.load_generation.set(generation);
        let repo = self.repo_root.borrow().clone();
        let handle = self.clone();
        gtk4::glib::spawn_future_local(async move {
            let repo_for_task = repo.clone();
            let result = gtk4::gio::spawn_blocking(move || {
                impulse_core::git::list_changed_files(&repo_for_task)
            })
            .await;
            if handle.load_generation.get() != generation {
                return;
            }
            match result {
                Ok(Ok(change_set)) => handle.apply_change_set(change_set),
                _ => {
                    // Repo disappeared / error — render an empty set.
                    handle.known_paths.borrow_mut().clear();
                    handle.branch_label.set_text("");
                    handle.count_label.set_text("0 files");
                    handle.added_label.set_text("+0");
                    handle.removed_label.set_text("-0");
                    handle.send_command(&ReviewCommand::Render { files: Vec::new() });
                }
            }
        });
    }

    fn apply_change_set(&self, change_set: impulse_core::git::ChangeSet) {
        let entries: Vec<ReviewFileEntry> = change_set
            .files
            .iter()
            .map(|f| ReviewFileEntry {
                path: f.path.clone(),
                status: f.status.clone(),
                old_path: f.old_path.clone(),
                added: f.added,
                removed: f.removed,
                is_binary: f.is_binary,
            })
            .collect();
        *self.known_paths.borrow_mut() = change_set.files.iter().map(|f| f.path.clone()).collect();
        self.branch_label
            .set_text(change_set.branch.as_deref().unwrap_or(""));
        self.count_label.set_text(&if change_set.files.len() == 1 {
            "1 file".to_string()
        } else {
            format!("{} files", change_set.files.len())
        });
        self.added_label
            .set_text(&format!("+{}", change_set.total_added));
        self.removed_label
            .set_text(&format!("-{}", change_set.total_removed));
        self.send_command(&ReviewCommand::Render { files: entries });
    }

    /// Fetch unified-diff hunks for a repo-relative path off the main thread
    /// and push a SetHunks command.
    fn load_diff(self: &Rc<Self>, path: &str) {
        let repo = self.repo_root.borrow().clone();
        let path = path.to_string();
        let generation = self.load_generation.get();
        let handle = self.clone();
        gtk4::glib::spawn_future_local(async move {
            let repo_for_task = repo.clone();
            let path_for_task = path.clone();
            let result = gtk4::gio::spawn_blocking(move || {
                impulse_core::git::file_hunks(&repo_for_task, &path_for_task)
            })
            .await;
            if handle.load_generation.get() != generation {
                return;
            }
            let hunks = match result {
                Ok(Ok(hunks)) => hunks,
                // Send an empty result so review.js stops showing the spinner.
                _ => impulse_core::git::FileHunks {
                    language: "plaintext".to_string(),
                    is_binary: false,
                    too_large: false,
                    truncated: false,
                    added: 0,
                    removed: 0,
                    hunks: Vec::new(),
                },
            };
            handle.send_command(&ReviewCommand::SetHunks { path, hunks });
        });
    }

    fn confirm_and_discard(self: &Rc<Self>, container: &gtk4::Box, path: &str) {
        if !self.known_paths.borrow().contains(path) {
            return;
        }
        let dialog = adw::AlertDialog::new(
            Some(&format!("Discard changes to {path}?")),
            Some("This reverts the file to its last committed state. This cannot be undone."),
        );
        dialog.add_responses(&[("cancel", "Cancel"), ("discard", "Discard")]);
        dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");

        let handle = self.clone();
        let path = path.to_string();
        dialog.connect_response(None, move |_, response| {
            if response != "discard" {
                return;
            }
            let repo = handle.repo_root.borrow().clone();
            let path = path.clone();
            let handle = handle.clone();
            gtk4::glib::spawn_future_local(async move {
                let result = gtk4::gio::spawn_blocking(move || {
                    impulse_core::git::discard_path(&repo, &path)
                })
                .await;
                if !matches!(result, Ok(Ok(()))) {
                    log::warn!("Discard failed: {:?}", result);
                }
                // Reload + re-render regardless: even on partial failure the
                // file list should reflect the current state.
                handle.reload_and_render();
            });
        });
        dialog.present(Some(container));
    }

    fn perform_commit(self: &Rc<Self>) {
        let message = self.commit_entry.text().trim().to_string();
        if message.is_empty() {
            self.commit_entry.grab_focus();
            return;
        }
        let repo = self.repo_root.borrow().clone();
        let handle = self.clone();
        self.commit_btn.set_sensitive(false);
        gtk4::glib::spawn_future_local(async move {
            let result =
                gtk4::gio::spawn_blocking(move || impulse_core::git::commit_all(&repo, &message))
                    .await;
            match result {
                Ok(Ok(oid)) => {
                    handle.commit_entry.set_text("");
                    let short: String = oid.chars().take(7).collect();
                    handle
                        .confirmation_label
                        .set_text(&format!("Committed {short}"));
                    handle.confirmation_label.set_visible(true);
                    let confirmation = handle.confirmation_label.clone();
                    gtk4::glib::timeout_add_seconds_local_once(3, move || {
                        confirmation.set_visible(false);
                    });
                    handle.reload_and_render();
                }
                Ok(Err(e)) => {
                    handle.commit_btn.set_sensitive(true);
                    handle
                        .confirmation_label
                        .set_text(&format!("Commit failed: {e}"));
                    handle.confirmation_label.set_visible(true);
                }
                Err(_) => {
                    handle.commit_btn.set_sensitive(true);
                }
            }
        });
    }
}
