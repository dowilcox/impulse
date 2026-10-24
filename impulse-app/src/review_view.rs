//! Review: a repository's changes as cards of hunks, as the Swift
//! `ReviewSurface` shows them: a header (what's compared, file count and
//! line stats, viewed count, unified or split, whitespace, context), a file
//! navigator, and one list of rows: each file's header, its hunks' headers
//! and lines (added and removed tinted, word-level changes stronger, line
//! numbers in gutters), and the end of its card. Files, hunks and lines can
//! be staged, unstaged or reverted where the scope allows it, with the
//! Swift app's keys: j/k hunks, n/p files, s/u/x stage, unstage, revert the
//! focused hunk, v viewed, c comment, o open, Return or Space fold. Review
//! comments are in `comments`.

mod comments;
mod navigator;
mod selection;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use gpui_kit::{
    div, list, prelude::*, px, AnyElement, App, Context, FocusHandle, FontWeight, Hsla,
    KeyDownEvent, ListAlignment, ListState, SharedString, StyledText, TextRun, Window,
};
use impulse_git::hunks::{DiffLine, DiffLineKind};
use impulse_git::scoped_diff::{DiffOptions, FileDiff};
use impulse_git::types::{ChangeStatus, DiffScope, FileChange};
use impulse_kit::chrome::{metrics, ChromePalette, Rgba as KitRgba};
use impulse_kit::theme::ResolvedTheme;

use crate::workbench_chrome::{hsla, icon};
use gpui_kit::component::menu::ContextMenuExt as _;

/// More context than any file shown has lines: the whole file.
const WHOLE_FILE: i32 = 100_000;
const NAVIGATOR_WIDTH: f32 = 250.0;
const CARD_INSET: f32 = 12.0;
/// The hover group of a diff line's column (its comment button shows).
const LINE_GROUP: &str = "review-line";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Layout {
    Unified,
    Split,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Stage,
    Unstage,
    Revert,
}

/// One changed file and its view state.
struct ReviewFile {
    change: FileChange,
    diff: Option<FileDiff>,
    error: Option<String>,
    loading: bool,
    expanded: bool,
    viewed: bool,
    /// Viewed earlier, but the diff changed since.
    changed_since_viewed: bool,
    /// A stage or revert is running on it.
    busy: bool,
    /// Its review comments, and which no longer match their lines.
    comments: Vec<impulse_kit::review_comments::ReviewComment>,
    outdated: HashSet<String>,
    /// Lines picked in one of its hunks (`selection`).
    selection: Option<selection::LineSelection>,
}

/// One row of the list.
#[derive(Clone, PartialEq)]
enum Row {
    PageTop,
    FileHeader(usize),
    Notice(usize, String),
    /// Above a file's outdated comments.
    OutdatedTitle(usize),
    /// A comment, by id.
    Comment(usize, String),
    /// The comment being written in this file.
    Composer(usize),
    HunkHeader(usize, usize),
    Line(usize, usize, usize),
    /// Split layout: an old-side line beside a new-side line.
    Split(usize, usize, Option<usize>, Option<usize>),
    FileEnd(usize),
}

/// Diff colors (the Swift `ReviewColors`).
#[derive(Clone)]
struct Colors {
    page: KitRgba,
    header: KitRgba,
    hunk_header: KitRgba,
    border: KitRgba,
    text: KitRgba,
    text2: KitRgba,
    text3: KitRgba,
    accent: KitRgba,
    added: KitRgba,
    removed: KitRgba,
    added_bg: KitRgba,
    removed_bg: KitRgba,
    added_word: KitRgba,
    removed_word: KitRgba,
    hover: KitRgba,
}

impl Colors {
    fn new(theme: &ResolvedTheme, palette: &ChromePalette) -> Self {
        let bg = KitRgba::from_hex(&theme.bg);
        let fg = KitRgba::from_hex(&theme.fg);
        let accent = KitRgba::from_hex(&theme.accent);
        let light = theme.is_light;
        let added = KitRgba::from_hex(&theme.git_added);
        let removed = KitRgba::from_hex(&theme.git_deleted);
        Colors {
            page: bg,
            header: palette.panel,
            hunk_header: bg.mix(accent, 0.06),
            border: palette.hairline,
            text: fg,
            text2: KitRgba::from_hex(&theme.fg_muted),
            text3: KitRgba::from_hex(&theme.fg_comment),
            accent,
            added,
            removed,
            added_bg: added.opacity(if light { 0.12 } else { 0.1 }),
            removed_bg: removed.opacity(if light { 0.12 } else { 0.11 }),
            added_word: added.opacity(if light { 0.3 } else { 0.28 }),
            removed_word: removed.opacity(0.3),
            hover: fg.opacity(if light { 0.06 } else { 0.07 }),
        }
    }
}

/// Which files were marked viewed, by the diff content they were viewed
/// at, so a file that changes again comes back unviewed.
fn viewed_path() -> PathBuf {
    impulse_kit::paths::data_dir(cfg!(debug_assertions)).join("review-viewed.json")
}

fn load_viewed(key: &str) -> HashMap<String, String> {
    std::fs::read(viewed_path())
        .ok()
        .and_then(|d| serde_json::from_slice::<HashMap<String, HashMap<String, String>>>(&d).ok())
        .and_then(|mut all| all.remove(key))
        .unwrap_or_default()
}

fn save_viewed(key: &str, map: &HashMap<String, String>) {
    let mut all: HashMap<String, HashMap<String, String>> = std::fs::read(viewed_path())
        .ok()
        .and_then(|d| serde_json::from_slice(&d).ok())
        .unwrap_or_default();
    all.insert(key.to_string(), map.clone());
    if let Ok(text) = serde_json::to_vec_pretty(&all) {
        let _ = impulse_kit::settings::write_private_file(&viewed_path(), &text);
    }
}

/// Identity of a diff's content, for "viewed" (the Swift `contentHash`).
fn content_hash(diff: &FileDiff) -> String {
    use sha2::{Digest, Sha256};
    let changes = if diff.hunks.is_empty() {
        String::new()
    } else {
        impulse_git::scoped_diff::change_identity(&diff.hunks)
    };
    let text = format!(
        "{}|{}|{}|{changes}",
        diff.path, diff.is_binary, diff.too_large
    );
    Sha256::digest(text.as_bytes())[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// What a scope is called in the header.
pub fn scope_title(scope: &DiffScope) -> String {
    match scope {
        DiffScope::Uncommitted => "Uncommitted changes".into(),
        DiffScope::Unstaged => "Unstaged".into(),
        DiffScope::Staged => "Staged".into(),
        DiffScope::Branch { base } => format!("vs {base}"),
        DiffScope::Commit { sha } if sha == "HEAD" => "Last commit".into(),
        DiffScope::Commit { sha } => format!("Commit {}", sha.chars().take(7).collect::<String>()),
        DiffScope::Range { from, to } => format!("{from}…{to}"),
        DiffScope::Stash { index } => format!("stash@{{{index}}}"),
        // "Agent turn at 14:05", "Since your review at 09:30".
        DiffScope::Snapshot { from, to } => impulse_kit::git::diff_scope::DiffScope::Snapshot {
            from: from.clone(),
            to: to.clone(),
        }
        .title(),
    }
}

fn empty_message(scope: &DiffScope) -> String {
    match scope {
        DiffScope::Unstaged => "No unstaged changes.".into(),
        DiffScope::Staged => "Nothing is staged.".into(),
        DiffScope::Uncommitted => "No uncommitted changes.".into(),
        DiffScope::Branch { base } => format!("No changes compared to {base}."),
        _ => "No changes.".into(),
    }
}

/// Context lines on both sides; each run of removed lines paired with the
/// following run of added lines (`ReviewRowBuilder.splitPairs`).
fn split_pairs(lines: &[DiffLine]) -> Vec<(Option<usize>, Option<usize>)> {
    let mut pairs = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind == DiffLineKind::Context {
            pairs.push((Some(i), Some(i)));
            i += 1;
            continue;
        }
        let (mut removed, mut added) = (Vec::new(), Vec::new());
        while i < lines.len() && lines[i].kind == DiffLineKind::Removed {
            removed.push(i);
            i += 1;
        }
        while i < lines.len() && lines[i].kind == DiffLineKind::Added {
            added.push(i);
            i += 1;
        }
        for k in 0..removed.len().max(added.len()) {
            pairs.push((removed.get(k).copied(), added.get(k).copied()));
        }
    }
    pairs
}

/// Unstaged when there is unstaged work, else staged, else everything
/// uncommitted (`ReviewSurface.defaultScope`).
pub fn default_scope(root: &str) -> DiffScope {
    let has = |scope: DiffScope| {
        impulse_git::scoped_diff::changed_files_in_scope(root, &scope)
            .map(|c| !c.is_empty())
            .unwrap_or(false)
    };
    if has(DiffScope::Unstaged) {
        DiffScope::Unstaged
    } else if has(DiffScope::Staged) {
        DiffScope::Staged
    } else {
        DiffScope::Uncommitted
    }
}

pub struct ReviewView {
    pub root: String,
    /// The navigator's filter field (`navigator`), made on first render.
    nav_filter: Option<gpui_kit::Entity<gpui_kit::component::input::InputState>>,
    _nav_filter_events: Option<gpui_kit::Subscription>,
    pub scope: DiffScope,
    options: DiffOptions,
    default_context: i32,
    layout: Layout,
    files: Vec<ReviewFile>,
    rows: Vec<Row>,
    list: ListState,
    focus: FocusHandle,
    colors: Colors,
    palette: ChromePalette,
    font_family: SharedString,
    font_size: f32,
    /// The focused hunk (file, hunk), for keys.
    focused: Option<(usize, usize)>,
    loading: bool,
    message: Option<String>,
    base_branch: Option<String>,
    generation: u64,
    /// A file to show once the list has it (absolute path).
    pending_reveal: Option<std::path::PathBuf>,
    /// Inside History: the commit is chosen there, so no scope bar.
    pub embedded: bool,
    /// The repository's review comments, and when their file last changed.
    comments: impulse_kit::review_comments::CommentStore,
    comments_stamp: Option<std::time::SystemTime>,
    composer: Option<comments::Composer>,
    editing: Option<comments::Editing>,
    _refresh: gpui_kit::Task<()>,
}

/// What the review asks the window to do.
pub enum ReviewEvent {
    OpenFile(PathBuf, Option<u32>),
}

impl gpui_kit::EventEmitter<ReviewEvent> for ReviewView {}

impl ReviewView {
    pub fn new(
        root: String,
        scope: DiffScope,
        theme: &ResolvedTheme,
        palette: &ChromePalette,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = crate::settings_store::SettingsStore::get(cx).clone();
        let context = settings.review_context_lines as i32;
        let base_branch = impulse_git::scoped_diff::default_base_branch(&root);
        // Changes on disk show without a refresh: the working tree is read
        // again every two seconds while the review is open.
        let refresh = cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(2))
                .await;
            if this
                .update(cx, |view, cx| view.refresh_if_live(cx))
                .is_err()
            {
                break;
            }
        });
        let mut view = ReviewView {
            root,
            scope,
            options: DiffOptions {
                context_lines: context,
                ignore_whitespace: false,
            },
            default_context: context,
            layout: Layout::Unified,
            files: Vec::new(),
            rows: Vec::new(),
            list: ListState::new(0, ListAlignment::Top, px(800.)),
            focus: cx.focus_handle(),
            colors: Colors::new(theme, palette),
            palette: palette.clone(),
            font_family: settings.font_family.clone().into(),
            font_size: (settings.font_size as f32 * 6.0 / 7.0).round().max(8.0),
            focused: None,
            loading: false,
            message: None,
            base_branch,
            generation: 0,
            embedded: false,
            pending_reveal: None,
            comments: impulse_kit::review_comments::CommentStore::load(PathBuf::new()),
            comments_stamp: None,
            composer: None,
            nav_filter: None,
            _nav_filter_events: None,
            editing: None,
            _refresh: refresh,
        };
        view.reload_comments(cx);
        view.refresh(cx);
        view
    }

    pub fn title(&self) -> String {
        let name = std::path::Path::new(&self.root)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.is_empty() {
            "Review".into()
        } else {
            format!("Review · {name}")
        }
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    pub fn set_theme(
        &mut self,
        theme: &ResolvedTheme,
        palette: &ChromePalette,
        cx: &mut Context<Self>,
    ) {
        self.colors = Colors::new(theme, palette);
        self.palette = palette.clone();
        let settings = crate::settings_store::SettingsStore::get(cx);
        self.font_family = settings.font_family.clone().into();
        self.font_size = (settings.font_size as f32 * 6.0 / 7.0).round().max(8.0);
        cx.notify();
    }

    fn scope_key(&self) -> String {
        format!("{}:{}", self.root, scope_title(&self.scope))
    }

    /// Read the changed files and every diff again.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let (root, scope, options) = (self.root.clone(), self.scope.clone(), self.options);
        self.loading = true;
        cx.notify();
        let task = cx.background_spawn(async move {
            let changes = impulse_git::scoped_diff::changed_files_in_scope(&root, &scope)
                .map_err(|e| e.to_string())?;
            let diffs: Vec<Result<FileDiff, String>> = changes
                .iter()
                .map(|c| {
                    impulse_git::scoped_diff::file_diff(
                        &root,
                        &c.path,
                        c.old_path.as_deref(),
                        &scope,
                        &options,
                    )
                    .map_err(|e| e.to_string())
                })
                .collect();
            Ok::<_, String>((changes, diffs))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |view, cx| {
                if view.generation != generation {
                    return;
                }
                view.loading = false;
                match result {
                    Ok((changes, diffs)) => {
                        view.merge(changes, diffs);
                        view.rebuild_rows();
                        if let Some(path) = view.pending_reveal.take() {
                            view.reveal_path(&path, cx);
                        }
                    }
                    Err(error) => {
                        view.files.clear();
                        view.message = Some(error);
                    }
                }
                view.rebuild_rows();
                cx.notify();
            });
        })
        .detach();
    }

    /// The live scopes read again in the background; fixed ones (a commit)
    /// never change.
    fn refresh_if_live(&mut self, cx: &mut Context<Self>) {
        self.reload_comments_if_changed(cx);
        if !self.loading && (self.scope.includes_working_tree() || self.scope.supports_staging()) {
            self.refresh_quietly(cx);
        }
    }

    /// A refresh that only touches the rows when something changed.
    fn refresh_quietly(&mut self, cx: &mut Context<Self>) {
        let (root, scope, options) = (self.root.clone(), self.scope.clone(), self.options);
        let generation = self.generation;
        let task = cx.background_spawn(async move {
            let changes = impulse_git::scoped_diff::changed_files_in_scope(&root, &scope).ok()?;
            let diffs: Vec<Result<FileDiff, String>> = changes
                .iter()
                .map(|c| {
                    impulse_git::scoped_diff::file_diff(
                        &root,
                        &c.path,
                        c.old_path.as_deref(),
                        &scope,
                        &options,
                    )
                    .map_err(|e| e.to_string())
                })
                .collect();
            Some((changes, diffs))
        });
        cx.spawn(async move |this, cx| {
            let Some((changes, diffs)) = task.await else {
                return;
            };
            let _ = this.update(cx, |view, cx| {
                if view.generation != generation {
                    return;
                }
                let same =
                    view.files.len() == changes.len()
                        && view.files.iter().zip(&diffs).all(|(file, diff)| {
                            match (&file.diff, diff) {
                                (Some(a), Ok(b)) => a == b,
                                _ => false,
                            }
                        });
                // While a comment is typed its row isn't rebuilt under it.
                if !same && !view.is_composing() {
                    view.merge(changes, diffs);
                    view.rebuild_rows();
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// New file list and diffs, keeping each file's view state.
    fn merge(&mut self, changes: Vec<FileChange>, diffs: Vec<Result<FileDiff, String>>) {
        let viewed = load_viewed(&self.scope_key());
        let previous: HashMap<String, (bool, bool)> = self
            .files
            .iter()
            .map(|f| (f.change.path.clone(), (f.expanded, f.viewed)))
            .collect();
        self.files = changes
            .into_iter()
            .zip(diffs)
            .map(|(change, diff)| {
                let (diff, error) = match diff {
                    Ok(diff) => (Some(diff), None),
                    Err(error) => (None, Some(error)),
                };
                let hash = diff.as_ref().map(content_hash);
                let viewed_at = viewed.get(&change.path);
                let is_viewed = viewed_at.is_some() && viewed_at == hash.as_ref();
                let expanded = previous
                    .get(&change.path)
                    .map(|(e, _)| *e)
                    .unwrap_or(!is_viewed);
                ReviewFile {
                    changed_since_viewed: viewed_at.is_some() && !is_viewed,
                    viewed: is_viewed,
                    expanded,
                    loading: false,
                    busy: false,
                    error,
                    diff,
                    change,
                    comments: Vec::new(),
                    outdated: HashSet::new(),
                    selection: None,
                }
            })
            .collect();
        self.assign_comments();
        self.message = self.files.is_empty().then(|| empty_message(&self.scope));
        if let Some((file, hunk)) = self.focused {
            let valid = self
                .files
                .get(file)
                .and_then(|f| f.diff.as_ref())
                .is_some_and(|d| hunk < d.hunks.len());
            if !valid {
                self.focused = None;
            }
        }
        // A composer whose line went away goes with it.
        if let Some(composer) = &self.composer {
            let valid = self
                .files
                .iter()
                .find(|f| f.change.path == composer.path)
                .and_then(|f| f.diff.as_ref())
                .and_then(|d| d.hunks.get(composer.hunk))
                .is_some_and(|h| composer.line_index < h.lines.len());
            if !valid {
                self.composer = None;
            }
        }
    }

    fn rebuild_rows(&mut self) {
        let mut rows = vec![Row::PageTop];
        for (index, file) in self.files.iter().enumerate() {
            rows.push(Row::FileHeader(index));
            if file.expanded {
                match &file.diff {
                    None => rows.push(Row::Notice(
                        index,
                        file.error.clone().unwrap_or_else(|| "Loading…".into()),
                    )),
                    Some(diff) if diff.is_binary => {
                        rows.push(Row::Notice(index, "Binary file, no diff shown".into()))
                    }
                    Some(diff) if diff.too_large => rows.push(Row::Notice(
                        index,
                        "File too large to display. Open it in the editor.".into(),
                    )),
                    Some(diff) if diff.hunks.is_empty() => {
                        rows.push(Row::Notice(index, "No textual changes".into()))
                    }
                    Some(diff) => {
                        // Comments whose lines changed float at the top.
                        rows.extend(self.outdated_rows(index));
                        for (h, hunk) in diff.hunks.iter().enumerate() {
                            rows.push(Row::HunkHeader(index, h));
                            let after = |l: usize| self.rows_after(index, h, l, &hunk.lines[l]);
                            match self.layout {
                                Layout::Unified => {
                                    for l in 0..hunk.lines.len() {
                                        rows.push(Row::Line(index, h, l));
                                        rows.extend(after(l));
                                    }
                                }
                                Layout::Split => {
                                    for (old, new) in split_pairs(&hunk.lines) {
                                        rows.push(Row::Split(index, h, old, new));
                                        rows.extend(old.map(after).unwrap_or_default());
                                        if new != old {
                                            rows.extend(new.map(after).unwrap_or_default());
                                        }
                                    }
                                }
                            }
                        }
                        if diff.truncated {
                            rows.push(Row::Notice(
                                index,
                                "Diff truncated: the file has more changes than shown. Stage or revert it as a whole file.".into(),
                            ));
                        }
                    }
                }
            }
            rows.push(Row::FileEnd(index));
        }
        if rows != self.rows {
            // Only the rows that changed, so the list stays where it was
            // scrolled (a comment added, a hunk staged).
            let prefix = rows
                .iter()
                .zip(&self.rows)
                .take_while(|(a, b)| a == b)
                .count();
            let suffix = rows[prefix..]
                .iter()
                .rev()
                .zip(self.rows[prefix..].iter().rev())
                .take_while(|(a, b)| a == b)
                .count();
            self.list.splice(
                prefix..self.rows.len() - suffix,
                rows.len() - prefix - suffix,
            );
            self.rows = rows;
        }
    }

    fn capabilities(&self) -> (bool, bool, bool) {
        match self.scope {
            DiffScope::Unstaged => (true, false, true),
            DiffScope::Staged => (false, true, false),
            _ => (false, false, false),
        }
    }

    pub fn set_scope(&mut self, scope: DiffScope, cx: &mut Context<Self>) {
        self.scope = scope;
        self.files.clear();
        self.focused = None;
        self.rebuild_rows();
        self.refresh(cx);
    }

    fn set_viewed(&mut self, index: usize, viewed: bool, cx: &mut Context<Self>) {
        let key = self.scope_key();
        let Some(file) = self.files.get_mut(index) else {
            return;
        };
        file.viewed = viewed;
        file.changed_since_viewed = false;
        // Viewed files fold, as on GitHub.
        file.expanded = !viewed;
        let mut map = load_viewed(&key);
        match (viewed, file.diff.as_ref()) {
            (true, Some(diff)) => {
                map.insert(file.change.path.clone(), content_hash(diff));
            }
            _ => {
                map.remove(&file.change.path);
            }
        }
        save_viewed(&key, &map);
        self.rebuild_rows();
        cx.notify();
    }

    fn toggle_expanded(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(file) = self.files.get_mut(index) {
            file.expanded = !file.expanded;
            self.rebuild_rows();
            cx.notify();
        }
    }

    /// Stage, unstage or revert a whole file, or one hunk of it.
    fn act(&mut self, action: Action, index: usize, hunk: Option<usize>, cx: &mut Context<Self>) {
        let (stage, unstage, revert) = self.capabilities();
        let allowed = match action {
            Action::Stage => stage,
            Action::Unstage => unstage,
            Action::Revert => revert,
        };
        // The hunk's selected lines only, when it holds the selection.
        let lines = hunk.and_then(|h| self.selected_lines(index, h));
        let Some(file) = self.files.get_mut(index).filter(|f| allowed && !f.busy) else {
            return;
        };
        if hunk.is_some() && file.diff.as_ref().is_some_and(|d| d.truncated) {
            self.message = Some(impulse_git::operations::TRUNCATED_DIFF_MESSAGE.to_string());
            cx.notify();
            return;
        }
        file.selection = None;
        file.busy = true;
        let root = self.root.clone();
        let path = file.change.path.clone();
        let old_path = file.change.old_path.clone();
        let untracked = file.change.status == ChangeStatus::Untracked;
        let options = self.options;
        let expected: HashMap<usize, String> = hunk
            .and_then(|h| {
                file.diff
                    .as_ref()
                    .and_then(|d| d.hunk_ids.get(h))
                    .map(|id| (h, id.clone()))
            })
            .into_iter()
            .collect();
        cx.notify();
        let task = cx.background_spawn(async move {
            use impulse_git::operations as ops;
            let result = match (action, hunk) {
                (Action::Stage, None) => ops::stage(&[path.as_str()], &root),
                (Action::Unstage, None) => ops::unstage(&[path.as_str()], &root, false),
                (Action::Revert, None) if untracked => {
                    // An untracked file "reverted" is a file removed: to the trash.
                    trash::delete(std::path::Path::new(&root).join(&path))
                        .map_err(|e| impulse_git::OperationError::Invalid(e.to_string()))
                }
                (Action::Revert, None) => ops::discard_working_tree(&[path.as_str()], &root),
                (action, Some(h)) => {
                    let target = match action {
                        Action::Stage => ops::PatchTarget::Stage,
                        Action::Unstage => ops::PatchTarget::Unstage,
                        Action::Revert => ops::PatchTarget::Discard,
                    };
                    let selection = match &lines {
                        Some(lines) => impulse_git::patch::PatchSelection::lines(lines, h),
                        None => impulse_git::patch::PatchSelection::whole_hunks(&[h]),
                    };
                    ops::apply(
                        target,
                        &selection,
                        &path,
                        old_path.as_deref(),
                        &expected,
                        &options,
                        &root,
                    )
                }
            };
            result.map_err(|e| e.message())
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |view, cx| {
                if let Some(file) = view.files.get_mut(index) {
                    file.busy = false;
                }
                if let Err(error) = result {
                    view.message = Some(error);
                }
                view.refresh(cx);
            });
        })
        .detach();
    }

    // MARK: Keys

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &event.keystroke;
        let m = &k.modifiers;
        // Keys typed into a comment are the comment's.
        if !self.focus.is_focused(window) {
            return;
        }
        // ⌘C copies the selected lines; ⌘Y stages, ⇧⌘Y unstages, ⌥⌘Z
        // reverts the focused hunk (Ctrl on Linux).
        let command = if cfg!(target_os = "macos") {
            m.platform && !m.control
        } else {
            m.control && !m.platform
        };
        if command {
            let handled = match (k.key.as_str(), m.shift, m.alt) {
                ("c", false, false) => self.copy_selection(cx),
                ("y", shift, false) => {
                    if let Some((file, hunk)) = self.focused {
                        let action = if shift {
                            Action::Unstage
                        } else {
                            Action::Stage
                        };
                        self.act(action, file, Some(hunk), cx);
                    }
                    true
                }
                ("z", false, true) => {
                    if let Some((file, hunk)) = self.focused {
                        self.act(Action::Revert, file, Some(hunk), cx);
                    }
                    true
                }
                _ => false,
            };
            if handled {
                cx.stop_propagation();
            }
            return;
        }
        if m.control || m.alt || m.platform {
            return;
        }
        if k.key == "escape" {
            if self.clear_selections(cx) {
                cx.stop_propagation();
            }
            return;
        }
        let key = k.key_char.clone().unwrap_or_else(|| k.key.clone());
        if self.press(&key, window, cx) {
            cx.stop_propagation();
        }
    }

    /// One of Review's letter keys; false when `key` isn't one.
    fn press(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let focused = self.focused;
        match key {
            "j" => self.move_hunk(1, cx),
            "k" => self.move_hunk(-1, cx),
            "n" => self.move_file(1, false, cx),
            "p" => self.move_file(-1, false, cx),
            "N" => self.move_file(1, true, cx),
            "s" | "u" | "x" => {
                if let Some((file, hunk)) = focused {
                    let action = match key {
                        "s" => Action::Stage,
                        "u" => Action::Unstage,
                        _ => Action::Revert,
                    };
                    self.act(action, file, Some(hunk), cx);
                }
            }
            "v" => {
                if let Some((file, _)) = focused {
                    let viewed = !self.files[file].viewed;
                    self.set_viewed(file, viewed, cx);
                }
            }
            "o" => {
                if let Some((file, hunk)) = focused {
                    self.open(file, Some(hunk), cx);
                }
            }
            "c" => {
                if let Some((file, hunk)) = focused {
                    self.open_composer(file, hunk, None, window, cx);
                }
            }
            "t" | "T" | "/" => self.focus_filter(window, cx),
            "enter" | " " | "space" => {
                if let Some((file, _)) = focused {
                    self.toggle_expanded(file, cx);
                }
            }
            _ => return false,
        }
        true
    }

    /// The snapshot's `review-…` actions (the Swift `debugAction`): `split`,
    /// `keys=` letters, `select=path:hunk:from-to`, `comment=` text on the
    /// focused hunk, `reveal=` a path, `edit` the first comment,
    /// `composer`, `scope=uncommitted` or `scope=branch:<base>`.
    pub fn debug_action(
        &mut self,
        action: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let index_of = |view: &Self, path: &str| {
            view.files
                .iter()
                .position(|f| f.change.path == path)
                .ok_or_else(|| format!("review: no file {path}"))
        };
        if action == "split" {
            self.layout = Layout::Split;
            self.rebuild_rows();
        } else if let Some(keys) = action.strip_prefix("keys=") {
            for key in keys.chars() {
                self.press(&key.to_string(), window, cx);
            }
        } else if let Some(spec) = action.strip_prefix("select=") {
            let parts: Vec<&str> = spec.split(':').collect();
            let [path, hunk, range] = parts.as_slice() else {
                return Err(format!("review: select={spec}"));
            };
            let f = index_of(self, path)?;
            let hunk: usize = hunk.parse().map_err(|_| format!("review: select={spec}"))?;
            let (from, to) = range.split_once('-').unwrap_or((range, range));
            let (from, to): (usize, usize) = (
                from.parse().map_err(|_| format!("review: select={spec}"))?,
                to.parse().map_err(|_| format!("review: select={spec}"))?,
            );
            self.toggle_line(f, hunk, from, false, cx);
            self.toggle_line(f, hunk, to, true, cx);
        } else if let Some(text) = action.strip_prefix("comment=") {
            let (f, h) = self
                .focused
                .ok_or("review: comment= needs a focused hunk")?;
            self.open_composer(f, h, None, window, cx);
            let input = self.composer.as_ref().map(|c| c.input.clone());
            if let Some(input) = input {
                input.update(cx, |input, cx| {
                    input.set_value(text.to_string(), window, cx)
                });
            }
            self.save_composer(window, cx);
        } else if let Some(path) = action.strip_prefix("reveal=") {
            let f = index_of(self, path)?;
            self.reveal_file(f, cx);
            let hunks = self.files[f].diff.as_ref().map_or(0, |d| d.hunks.len());
            if hunks > 0 {
                self.focused = Some((f, 0));
            }
        } else if action == "edit" {
            let id = self
                .files
                .iter()
                .flat_map(|f| &f.comments)
                .next()
                .map(|c| c.id.clone());
            let id = id.ok_or("review: no comment to edit")?;
            self.edit_comment(id, window, cx);
        } else if action == "composer" {
            let (f, h) = self
                .focused
                .ok_or("review: composer needs a focused hunk")?;
            self.open_composer(f, h, None, window, cx);
        } else if action == "scope=uncommitted" {
            self.set_scope(DiffScope::Uncommitted, cx);
        } else if let Some(base) = action.strip_prefix("scope=branch:") {
            self.set_scope(
                DiffScope::Branch {
                    base: base.to_string(),
                },
                cx,
            );
        } else {
            return Err(format!("review: unknown action {action}"));
        }
        cx.notify();
        Ok(())
    }

    fn open(&mut self, index: usize, hunk: Option<usize>, cx: &mut Context<Self>) {
        let Some(file) = self.files.get(index) else {
            return;
        };
        let line = hunk.and_then(|h| {
            let hunk = file.diff.as_ref()?.hunks.get(h)?;
            let line = hunk
                .lines
                .iter()
                .find(|l| l.kind != DiffLineKind::Context)
                .or(hunk.lines.first())?;
            line.new_lineno.or(line.old_lineno)
        });
        let path = PathBuf::from(&self.root).join(&file.change.path);
        cx.emit(ReviewEvent::OpenFile(path, line));
    }

    fn move_hunk(&mut self, delta: i32, cx: &mut Context<Self>) {
        let mut stops: Vec<(usize, usize)> = Vec::new();
        for (f, file) in self.files.iter().enumerate() {
            if let (true, Some(diff)) = (file.expanded, &file.diff) {
                stops.extend((0..diff.hunks.len()).map(|h| (f, h)));
            }
        }
        if stops.is_empty() {
            return;
        }
        let next = match self
            .focused
            .and_then(|f| stops.iter().position(|s| *s == f))
        {
            Some(i) => (i as i32 + delta).clamp(0, stops.len() as i32 - 1) as usize,
            None if delta > 0 => 0,
            None => stops.len() - 1,
        };
        self.focused = Some(stops[next]);
        if let Some(row) = self
            .rows
            .iter()
            .position(|r| *r == Row::HunkHeader(stops[next].0, stops[next].1))
        {
            self.list.scroll_to_reveal_item(row);
        }
        cx.notify();
    }

    fn move_file(&mut self, delta: i32, unviewed_only: bool, cx: &mut Context<Self>) {
        let count = self.files.len() as i32;
        let mut index =
            self.focused
                .map(|(f, _)| f as i32)
                .unwrap_or(if delta > 0 { -1 } else { count });
        for _ in 0..count {
            index += delta;
            if index < 0 || index >= count {
                return;
            }
            if !unviewed_only || !self.files[index as usize].viewed {
                self.reveal_file(index as usize, cx);
                return;
            }
        }
    }

    /// Scroll to the file at `path` (absolute), now or once it's loaded.
    pub fn reveal_path(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        let relative = path
            .strip_prefix(&self.root)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        match self.files.iter().position(|f| f.change.path == relative) {
            Some(index) if !self.loading => self.reveal_file(index, cx),
            _ => self.pending_reveal = Some(path.to_path_buf()),
        }
    }

    fn reveal_file(&mut self, index: usize, cx: &mut Context<Self>) {
        let has_hunks = self.files[index]
            .diff
            .as_ref()
            .is_some_and(|d| !d.hunks.is_empty());
        self.focused = has_hunks.then_some((index, 0));
        if let Some(row) = self.rows.iter().position(|r| *r == Row::FileHeader(index)) {
            self.list.scroll_to(gpui_kit::ListOffset {
                item_ix: row,
                offset_in_item: px(0.),
            });
        }
        cx.notify();
    }

    // MARK: Drawing

    fn c(&self, color: KitRgba) -> Hsla {
        hsla(color)
    }

    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = &self.palette;
        let (added, removed) = self.files.iter().fold((0, 0), |(a, r), f| {
            let (fa, fr) = f
                .diff
                .as_ref()
                .map(|d| (d.added, d.removed))
                .unwrap_or((f.change.added.unwrap_or(0), f.change.removed.unwrap_or(0)));
            (a + fa, r + fr)
        });
        let viewed = self.files.iter().filter(|f| f.viewed).count();
        let count = self.files.len();
        let mut scopes = vec![
            ("Unstaged", DiffScope::Unstaged),
            ("Staged", DiffScope::Staged),
            ("All", DiffScope::Uncommitted),
        ];
        if let Some(base) = &self.base_branch {
            scopes.push(("vs base", DiffScope::Branch { base: base.clone() }));
        }
        scopes.push(("Last commit", DiffScope::Commit { sha: "HEAD".into() }));
        let segment = |id: &'static str, label: String, selected: bool| {
            div()
                .id(id)
                .px(px(8.))
                .h(px(22.))
                .flex()
                .items_center()
                .rounded(px(metrics::RADIUS_SMALL))
                .text_size(px(11.5))
                .cursor_pointer()
                .when(selected, |d| d.bg(hsla(p.raised)).text_color(hsla(p.text)))
                .when(!selected, |d| {
                    d.text_color(hsla(p.text_secondary))
                        .hover(|s| s.bg(hsla(p.hover)))
                })
                .child(label)
        };
        let mut scope_bar = div().flex().items_center().gap(px(2.));
        for (i, (label, scope)) in scopes.into_iter().enumerate() {
            let selected = scope == self.scope;
            let title = if label == "vs base" {
                scope_title(&scope)
            } else {
                label.to_string()
            };
            scope_bar = scope_bar.child(
                segment(["s0", "s1", "s2", "s3", "s4"][i.min(4)], title, selected)
                    .on_click(cx.listener(move |view, _, _, cx| view.set_scope(scope.clone(), cx))),
            );
        }
        let context_title = match self.options.context_lines {
            WHOLE_FILE => "Whole file".to_string(),
            1 => "1 line".to_string(),
            n => format!("{n} lines"),
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .h(px(36.))
            .px(px(10.))
            .bg(hsla(p.panel))
            .border_b_1()
            .border_color(hsla(p.hairline))
            .child(icon("file-diff", 13., p.text))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(12.))
                    .child(scope_title(&self.scope)),
            )
            .when(!self.embedded, |d| d.child(scope_bar))
            .when(count > 0, |d| {
                d.child(
                    div()
                        .text_size(px(11.5))
                        .text_color(hsla(p.text_secondary))
                        .child(format!("{count} file{}", if count == 1 { "" } else { "s" })),
                )
                .child(stat(added, removed, p))
            })
            .when(self.loading, |d| {
                d.child(
                    div()
                        .text_size(px(11.))
                        .text_color(hsla(p.text_tertiary))
                        .child("Loading…"),
                )
            })
            .child(div().flex_1())
            .when(count > 0, |d| {
                d.child(
                    div()
                        .font_family("JetBrains Mono")
                        .text_size(px(11.))
                        .text_color(hsla(if viewed == count {
                            p.success
                        } else {
                            p.text_secondary
                        }))
                        .child(format!("{viewed}/{count} viewed")),
                )
            })
            .child(
                div()
                    .flex()
                    .p(px(2.))
                    .rounded(px(metrics::RADIUS))
                    .bg(hsla(p.overlay))
                    .child(
                        segment("unified", "Unified".into(), self.layout == Layout::Unified)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.layout = Layout::Unified;
                                view.rebuild_rows();
                                cx.notify();
                            })),
                    )
                    .child(
                        segment("split", "Split".into(), self.layout == Layout::Split).on_click(
                            cx.listener(|view, _, _, cx| {
                                view.layout = Layout::Split;
                                view.rebuild_rows();
                                cx.notify();
                            }),
                        ),
                    ),
            )
            .child(
                div()
                    .id("whitespace")
                    .p(px(4.))
                    .rounded(px(metrics::RADIUS_SMALL))
                    .cursor_pointer()
                    .hover(|s| s.bg(hsla(p.hover)))
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.options.ignore_whitespace = !view.options.ignore_whitespace;
                        view.refresh(cx);
                    }))
                    .child(icon(
                        "space",
                        13.,
                        if self.options.ignore_whitespace {
                            p.accent
                        } else {
                            p.text_secondary
                        },
                    )),
            )
            .child(
                div()
                    .id("context")
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .px(px(6.))
                    .h(px(22.))
                    .rounded(px(metrics::RADIUS_SMALL))
                    .cursor_pointer()
                    .text_size(px(11.))
                    .text_color(hsla(if self.options.context_lines > self.default_context {
                        p.accent
                    } else {
                        p.text_secondary
                    }))
                    .hover(|s| s.bg(hsla(p.hover)))
                    .on_click(cx.listener(|view, _, _, cx| {
                        // The Swift context menu's steps, in turn.
                        let mut steps = vec![view.default_context, 3, 10, 25, WHOLE_FILE];
                        steps.sort();
                        steps.dedup();
                        let current = view.options.context_lines;
                        view.options.context_lines =
                            *steps.iter().find(|s| **s > current).unwrap_or(&steps[0]);
                        view.refresh(cx);
                    }))
                    .child(icon("chevrons-up-down", 13., p.text_secondary))
                    .child(context_title),
            )
            .child(self.comments_menu(cx))
            .child(
                div()
                    .id("refresh")
                    .p(px(4.))
                    .rounded(px(metrics::RADIUS_SMALL))
                    .cursor_pointer()
                    .hover(|s| s.bg(hsla(p.hover)))
                    .on_click(cx.listener(|view, _, _, cx| view.refresh(cx)))
                    .child(icon("refresh-cw", 13., p.text_secondary)),
            )
    }

    fn render_row(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Every row spans the list, its card inset from the edges.
        div()
            .w_full()
            .px(px(CARD_INSET))
            .child(self.row_content(index, window, cx))
            .into_any_element()
    }

    fn row_content(&mut self, index: usize, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(row) = self.rows.get(index).cloned() else {
            return div().into_any_element();
        };
        let colors = self.colors.clone();
        let card = |d: gpui_kit::Div| {
            d.w_full()
                .border_l_1()
                .border_r_1()
                .border_color(hsla(colors.border))
                .bg(hsla(colors.page))
        };
        match row {
            Row::PageTop => div().h(px(10.)).into_any_element(),
            Row::FileHeader(f) => self.file_header(f, cx),
            Row::FileEnd(_) => div()
                .h(px(12.))
                .child(div().w_full().h(px(1.)).bg(hsla(colors.border)))
                .into_any_element(),
            Row::Notice(_, text) => card(div())
                .h(px(42.))
                .flex()
                .items_center()
                .px(px(14.))
                .text_size(px(12.))
                .text_color(hsla(colors.text2))
                .child(text)
                .into_any_element(),
            Row::HunkHeader(f, h) => {
                let header = self.files[f]
                    .diff
                    .as_ref()
                    .and_then(|d| d.hunks.get(h))
                    .map(|h| h.header.clone())
                    .unwrap_or_default();
                let focused = self.focused == Some((f, h));
                let truncated = self.files[f].diff.as_ref().is_some_and(|d| d.truncated);
                let (can_stage, can_unstage, can_revert) = self.capabilities();
                let (stage, unstage, revert) = if truncated {
                    (false, false, false)
                } else {
                    (can_stage, can_unstage, can_revert)
                };
                let busy = self.files[f].busy;
                let selected = self.selected_lines(f, h).map_or(0, |l| l.len());
                let what = match selected {
                    0 => "hunk".to_string(),
                    1 => "1 line".to_string(),
                    n => format!("{n} lines"),
                };
                let mut actions = div().flex().items_center().gap(px(4.));
                let p = &self.palette;
                if truncated && (can_stage || can_unstage || can_revert) {
                    actions = actions.child(
                        div()
                            .text_size(px(11.))
                            .text_color(hsla(p.text_tertiary))
                            .child("Whole file only"),
                    );
                }
                let button = |id: &'static str, label: String, primary: bool| {
                    div()
                        .id(id)
                        .px(px(8.))
                        .h(px(18.))
                        .flex()
                        .items_center()
                        .rounded(px(metrics::RADIUS_SMALL))
                        .text_size(px(11.))
                        .cursor_pointer()
                        .when(primary, |d| {
                            d.bg(hsla(p.accent)).text_color(hsla(p.text_on_accent))
                        })
                        .when(!primary, |d| {
                            d.border_1()
                                .border_color(hsla(p.hairline_strong))
                                .text_color(hsla(p.text_secondary))
                        })
                        .child(label)
                };
                if !busy {
                    if stage {
                        actions = actions.child(
                            button("stage-hunk", format!("Stage {what}"), true).on_click(
                                cx.listener(move |v, _, _, cx| {
                                    v.act(Action::Stage, f, Some(h), cx)
                                }),
                            ),
                        );
                    }
                    if unstage {
                        actions = actions.child(
                            button("unstage-hunk", format!("Unstage {what}"), true).on_click(
                                cx.listener(move |v, _, _, cx| {
                                    v.act(Action::Unstage, f, Some(h), cx)
                                }),
                            ),
                        );
                    }
                    if revert {
                        actions = actions.child(
                            button("revert-hunk", format!("Revert {what}"), false).on_click(
                                cx.listener(move |v, _, _, cx| {
                                    v.act(Action::Revert, f, Some(h), cx)
                                }),
                            ),
                        );
                    }
                }
                actions = actions.child(self.hunk_comment_button(f, h, cx));
                card(div())
                    .id(("hunk", index))
                    .h(px(26.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(10.))
                    .bg(hsla(colors.hunk_header))
                    .border_t_1()
                    .when(focused, |d| {
                        d.border_l_2().border_color(hsla(colors.accent))
                    })
                    .on_click(cx.listener(move |v, _, _, cx| {
                        v.focused = Some((f, h));
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .font_family(self.font_family.clone())
                            .text_size(px(self.font_size - 1.))
                            .text_color(hsla(colors.text3))
                            .child(header),
                    )
                    .child(actions)
                    .into_any_element()
            }
            Row::Line(f, h, l) => self.line_row(f, h, &[(Some(l), Side::Single)], cx),
            Row::Split(f, h, old, new) => {
                self.line_row(f, h, &[(old, Side::Old), (new, Side::New)], cx)
            }
            Row::OutdatedTitle(_) => self.outdated_title_row(),
            Row::Comment(f, id) => self.comment_row(f, &id, cx),
            Row::Composer(_) => self.composer_row(cx),
        }
    }

    fn file_header(&self, f: usize, cx: &mut Context<Self>) -> AnyElement {
        let p = &self.palette;
        let file = &self.files[f];
        let path = &file.change.path;
        let (dir, name) = match path.rfind('/') {
            Some(i) => (format!("{}/", &path[..i]), path[i + 1..].to_string()),
            None => (String::new(), path.clone()),
        };
        let (added, removed) = file.diff.as_ref().map(|d| (d.added, d.removed)).unwrap_or((
            file.change.added.unwrap_or(0),
            file.change.removed.unwrap_or(0),
        ));
        let (stage, unstage, revert) = self.capabilities();
        let viewed = file.viewed;
        let action = |id: &'static str, label: &'static str, primary: bool| {
            div()
                .id(id)
                .px(px(8.))
                .h(px(22.))
                .flex()
                .items_center()
                .rounded(px(metrics::RADIUS_SMALL))
                .text_size(px(11.5))
                .cursor_pointer()
                .when(primary, |d| {
                    d.bg(hsla(p.accent)).text_color(hsla(p.text_on_accent))
                })
                .when(!primary, |d| {
                    d.border_1()
                        .border_color(hsla(p.hairline_strong))
                        .text_color(hsla(p.text_secondary))
                })
                .child(label)
        };
        div()
            .id(("file-header", f))
            .w_full()
            .h(px(34.))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(10.))
            .bg(hsla(self.colors.header))
            .border_1()
            .border_color(hsla(self.colors.border))
            .rounded_t(px(metrics::RADIUS))
            .cursor_pointer()
            .on_click(cx.listener(move |v, _, _, cx| v.toggle_expanded(f, cx)))
            .child(icon(
                if file.expanded {
                    "chevron-down"
                } else {
                    "chevron-right"
                },
                12.,
                p.text_tertiary,
            ))
            .child(status_letter(file.change.status, p))
            .child(
                div()
                    .flex()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(px(12.))
                    .when_some(file.change.old_path.clone(), |d, old| {
                        d.child(
                            div()
                                .text_color(hsla(p.text_tertiary))
                                .line_through()
                                .child(old),
                        )
                        .child(div().px(px(4.)).child(icon(
                            "chevron-right",
                            10.,
                            p.text_tertiary,
                        )))
                    })
                    .child(div().text_color(hsla(p.text_tertiary)).child(dir))
                    .child(div().text_color(hsla(p.text)).child(name)),
            )
            .when(file.changed_since_viewed, |d| {
                d.child(
                    div()
                        .text_size(px(10.))
                        .text_color(hsla(p.warning))
                        .child("changed since viewed"),
                )
            })
            .child(div().flex_1())
            .child(if file.diff.as_ref().is_some_and(|d| d.is_binary) {
                div()
                    .font_family("JetBrains Mono")
                    .text_size(px(10.5))
                    .text_color(hsla(p.text_tertiary))
                    .child("binary")
                    .into_any_element()
            } else {
                stat(added, removed, p).into_any_element()
            })
            .child(
                div()
                    .id(("viewed", f))
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .px(px(6.))
                    .h(px(22.))
                    .rounded(px(metrics::RADIUS_SMALL))
                    .hover(|s| s.bg(hsla(p.hover)))
                    .on_click(cx.listener(move |v, _, _, cx| {
                        cx.stop_propagation();
                        v.set_viewed(f, !viewed, cx)
                    }))
                    .child(
                        div()
                            .size(px(13.))
                            .rounded(px(3.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(viewed, |d| d.bg(hsla(p.accent)))
                            .when(!viewed, |d| {
                                d.border_1().border_color(hsla(p.hairline_strong))
                            })
                            .when(viewed, |d| d.child(icon("check", 10., p.text_on_accent))),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(hsla(p.text_secondary))
                            .child("Viewed"),
                    ),
            )
            .when(stage && !file.busy, |d| {
                d.child(action("stage-file", "Stage", true).on_click(cx.listener(
                    move |v, _, _, cx| {
                        cx.stop_propagation();
                        v.act(Action::Stage, f, None, cx)
                    },
                )))
            })
            .when(unstage && !file.busy, |d| {
                d.child(
                    action("unstage-file", "Unstage", true).on_click(cx.listener(
                        move |v, _, _, cx| {
                            cx.stop_propagation();
                            v.act(Action::Unstage, f, None, cx)
                        },
                    )),
                )
            })
            .when(revert && !file.busy, |d| {
                d.child(action("revert-file", "Revert", false).on_click(cx.listener(
                    move |v, _, _, cx| {
                        cx.stop_propagation();
                        v.act(Action::Revert, f, None, cx)
                    },
                )))
            })
            .child(
                div()
                    .id(("open", f))
                    .p(px(3.))
                    .rounded(px(metrics::RADIUS_SMALL))
                    .hover(|s| s.bg(hsla(p.hover)))
                    .on_click(cx.listener(move |v, _, _, cx| {
                        cx.stop_propagation();
                        v.open(f, Some(0), cx)
                    }))
                    .child(icon("external-link", 12., p.text_secondary)),
            )
            .into_any_element()
    }

    /// A diff line (unified), or an old and a new one side by side (split).
    fn line_row(
        &self,
        f: usize,
        h: usize,
        sides: &[(Option<usize>, Side)],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = &self.colors;
        let Some(hunk) = self.files[f].diff.as_ref().and_then(|d| d.hunks.get(h)) else {
            return div().into_any_element();
        };
        let line_height = (self.font_size * 1.6).max(19.).ceil();
        let gutter_width = (self.font_size * 0.62 * 5.0 + 12.0).ceil().max(46.);
        let marker_width = (self.font_size * 0.62 + 8.0).ceil().max(18.);
        let focused = self.focused == Some((f, h));
        let busy = self.files[f].busy;
        let mut row = div()
            .w_full()
            .h(px(line_height))
            .flex()
            .border_l_1()
            .border_r_1()
            .border_color(hsla(colors.border))
            .bg(hsla(colors.page))
            .relative()
            .font_family(self.font_family.clone())
            .text_size(px(self.font_size));
        for (i, (index, side)) in sides.iter().enumerate() {
            let line = index
                .and_then(|l| hunk.lines.get(l))
                .filter(|line| match side {
                    Side::Old => line.kind != DiffLineKind::Added,
                    Side::New => line.kind != DiffLineKind::Removed,
                    Side::Single => true,
                });
            let mut column = div().flex().flex_1().min_w_0().h_full().overflow_hidden();
            if i > 0 {
                column = column.border_l_1().border_color(hsla(colors.border));
            }
            let Some(line) = line else {
                row = row.child(column.bg(hsla(colors.hover)));
                continue;
            };
            let (tint, word, marker, marker_color) = match line.kind {
                DiffLineKind::Added => {
                    (Some(colors.added_bg), colors.added_word, "+", colors.added)
                }
                DiffLineKind::Removed => (
                    Some(colors.removed_bg),
                    colors.removed_word,
                    "−",
                    colors.removed,
                ),
                DiffLineKind::Context => (None, colors.hover, " ", colors.text3),
            };
            if let Some(tint) = tint {
                column = column.bg(hsla(tint));
            }
            let numbers: Vec<Option<u32>> = match side {
                Side::Single => vec![line.old_lineno, line.new_lineno],
                Side::Old => vec![line.old_lineno],
                Side::New => vec![line.new_lineno],
            };
            // The gutter: numbers and marker. A click there selects a
            // changed line (Shift: extends); elsewhere it focuses the hunk.
            let selectable = line.kind != DiffLineKind::Context && !busy;
            let mut gutter = div().id(("gutter", i)).flex_none().flex().h_full();
            for number in numbers {
                gutter = gutter.child(
                    div()
                        .flex_none()
                        .w(px(gutter_width))
                        .pr(px(8.))
                        .flex()
                        .justify_end()
                        .items_center()
                        .text_size(px(self.font_size - 1.))
                        .text_color(hsla(colors.text3))
                        .child(number.map(|n| n.to_string()).unwrap_or_default()),
                );
            }
            gutter = gutter.child(
                div()
                    .flex_none()
                    .w(px(marker_width))
                    .flex()
                    .justify_center()
                    .items_center()
                    .text_color(hsla(marker_color))
                    .child(marker),
            );
            if let (Some(l), true) = (*index, selectable) {
                gutter = gutter
                    .cursor_pointer()
                    .hover(|d| d.bg(hsla(colors.hover)))
                    .on_click(cx.listener(move |v, event: &gpui_kit::ClickEvent, _, cx| {
                        cx.stop_propagation();
                        v.toggle_line(f, h, l, event.modifiers().shift, cx);
                    }));
            }
            if index.is_some_and(|l| self.is_selected(f, h, l)) {
                column = column.relative().child(
                    div()
                        .absolute()
                        .inset_0()
                        .bg(hsla(colors.accent).opacity(0.22))
                        .child(div().w(px(3.)).h_full().bg(hsla(colors.accent))),
                );
            }
            column = column.child(gutter).child(
                div()
                    .flex()
                    .items_center()
                    .min_w_0()
                    .whitespace_nowrap()
                    .child(code_text(line, colors.text, word)),
            );
            if let (Some(l), false) = (index, busy) {
                column = column
                    .relative()
                    .group(LINE_GROUP)
                    .child(self.gutter_comment_button(f, h, *l, i, cx));
            }
            let column = match index {
                Some(l) => {
                    let (l, view) = (*l, cx.entity().downgrade());
                    column
                        .id(("line", i))
                        .on_click(cx.listener(move |v, _, _, cx| {
                            v.focused = Some((f, h));
                            cx.notify();
                        }))
                        .context_menu(move |menu, _, cx| {
                            let Some(entity) = view.upgrade() else {
                                return menu;
                            };
                            entity.read(cx).line_menu(menu, f, h, l, view.clone())
                        })
                        .into_any_element()
                }
                None => column.into_any_element(),
            };
            row = row.child(column);
        }
        if focused {
            row = row.child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .w(px(2.))
                    .h_full()
                    .bg(hsla(colors.accent)),
            );
        }
        row.into_any_element()
    }
}

#[derive(Clone, Copy)]
enum Side {
    Single,
    Old,
    New,
}

/// A line's code, with its word-level changes highlighted (spans are UTF-16
/// offsets).
fn code_text(line: &DiffLine, text: KitRgba, word: KitRgba) -> StyledText {
    let content = line.content.replace('\t', "    ");
    if line.spans.is_empty() || line.content.contains('\t') {
        return StyledText::new(content);
    }
    // UTF-16 offset -> byte offset.
    let mut map = Vec::with_capacity(content.len() + 1);
    let mut units = 0u32;
    for (byte, ch) in content.char_indices() {
        map.push((units, byte));
        units += ch.len_utf16() as u32;
    }
    map.push((units, content.len()));
    let byte_at = |unit: u32| {
        map.iter()
            .find(|(u, _)| *u >= unit)
            .map(|(_, b)| *b)
            .unwrap_or(content.len())
    };
    let font = gpui_kit::font("JetBrains Mono");
    let run = |len: usize, background: Option<Hsla>| TextRun {
        len,
        font: font.clone(),
        color: hsla(text),
        background_color: background,
        underline: None,
        strikethrough: None,
    };
    let mut runs = Vec::new();
    let mut at = 0;
    for span in &line.spans {
        let (start, end) = (byte_at(span.start), byte_at(span.end));
        if start > at {
            runs.push(run(start - at, None));
        }
        if end > start.max(at) {
            runs.push(run(end - start.max(at), Some(hsla(word))));
        }
        at = end.max(at);
    }
    if at < content.len() {
        runs.push(run(content.len() - at, None));
    }
    StyledText::new(content).with_runs(runs)
}

fn status_letter(status: ChangeStatus, p: &ChromePalette) -> impl IntoElement {
    let color = match status {
        ChangeStatus::Added | ChangeStatus::Untracked => p.git_added,
        ChangeStatus::Deleted => p.git_deleted,
        ChangeStatus::Renamed => p.git_renamed,
        ChangeStatus::Conflicted => p.git_conflict,
        _ => p.git_modified,
    };
    div()
        .w(px(14.))
        .flex_none()
        .font_family("JetBrains Mono")
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(hsla(color))
        .child(status.letter())
}

fn stat(added: usize, removed: usize, p: &ChromePalette) -> impl IntoElement {
    div()
        .flex()
        .gap(px(6.))
        .font_family("JetBrains Mono")
        .text_size(px(11.))
        .child(
            div()
                .text_color(hsla(p.git_added))
                .child(format!("+{added}")),
        )
        .when(removed > 0, |d| {
            d.child(
                div()
                    .text_color(hsla(p.git_deleted))
                    .child(format!("−{removed}")),
            )
        })
}

impl Render for ReviewView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette.clone();
        let wide = window.viewport_size().width >= px(900.);
        let body: AnyElement = match &self.message {
            Some(message) if self.files.is_empty() => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(hsla(p.text_secondary))
                .child(message.clone())
                .into_any_element(),
            _ => list(self.list.clone(), cx.processor(Self::render_row))
                .flex_1()
                .h_full()
                .into_any_element(),
        };
        div()
            .id("review")
            .key_context("Review")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .flex()
            .flex_col()
            .size_full()
            .bg(hsla(self.colors.page))
            .child(self.header(cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .when(wide && !self.files.is_empty(), |d| {
                        d.child(self.navigator(window, cx))
                    })
                    .child(body),
            )
            .when_some(
                self.message.clone().filter(|_| !self.files.is_empty()),
                |d, message| {
                    d.child(
                        div()
                            .px(px(12.))
                            .py(px(6.))
                            .text_size(px(12.))
                            .text_color(hsla(p.danger))
                            .child(message),
                    )
                },
            )
    }
}

impl gpui_kit::Focusable for ReviewView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
