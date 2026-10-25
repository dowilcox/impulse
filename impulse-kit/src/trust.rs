//! Folders whose code Impulse may run on its own (the Swift
//! `WorkspaceTrust`). Some language servers run a project's build scripts
//! and tools (rust-analyzer runs build.rs, ESLint loads the project's config
//! and plugins), formatters on save load project config, and background
//! fetch follows the repository's own git configuration. In a folder that
//! isn't trusted, all of that stays off; what the user runs themselves (the
//! terminal, git actions) is unaffected.
//!
//! Trusting a folder trusts everything inside it, except a folder marked
//! restricted (a task checked out from someone else's branch), which stays
//! restricted until it's trusted itself. Thread-safe: language servers ask
//! from background threads. Kept in the same file, with the same format, as
//! the Swift app: `trusted-folders.json` in the data directory.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

#[derive(Default)]
struct State {
    folders: BTreeSet<String>,
    /// Restricted even inside a trusted folder.
    restricted: BTreeSet<String>,
    enabled: bool,
}

pub struct WorkspaceTrust {
    state: Mutex<State>,
    file: Option<PathBuf>,
    /// Whether the store existed before this launch (it didn't on the first
    /// launch with workspace trust: folders already in use can be trusted
    /// without asking).
    pub existed_before: bool,
}

#[derive(Serialize, Deserialize)]
struct Stored {
    version: u32,
    trusted: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    restricted: Option<Vec<String>>,
}

impl WorkspaceTrust {
    /// `file`: where the trusted folders are kept (`None`: in memory only).
    pub fn new(file: Option<PathBuf>) -> Self {
        let mut state = State {
            enabled: true,
            ..Default::default()
        };
        let mut existed_before = false;
        if let Some(file) = &file {
            existed_before = file.exists();
            if let Some(stored) = std::fs::read(file)
                .ok()
                .and_then(|data| serde_json::from_slice::<Stored>(&data).ok())
            {
                // An empty or relative entry would trust (or block) every
                // folder: it's dropped.
                state.folders = stored
                    .trusted
                    .iter()
                    .map(|p| normalize(p))
                    .filter(|p| is_folder(p))
                    .collect();
                state.restricted = stored
                    .restricted
                    .unwrap_or_default()
                    .iter()
                    .map(|p| normalize(p))
                    .filter(|p| is_folder(p))
                    .collect();
            }
        }
        Self {
            state: Mutex::new(state),
            file,
            existed_before,
        }
    }

    /// Off: every folder is trusted (the "Ask before trusting folders" setting).
    pub fn is_enabled(&self) -> bool {
        self.state.lock().unwrap().enabled
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.state.lock().unwrap().enabled = enabled;
    }

    /// Whether Impulse may run code from `path` (a file or folder).
    pub fn is_trusted(&self, path: &str) -> bool {
        let path = normalize(path);
        let state = self.state.lock().unwrap();
        !state.enabled || trusting_folder(&state, &path).is_some()
    }

    /// The trusted folder `path` is in, if any.
    pub fn trusting_folder(&self, path: &str) -> Option<String> {
        let path = normalize(path);
        trusting_folder(&self.state.lock().unwrap(), &path)
    }

    pub fn trusted_folders(&self) -> Vec<String> {
        self.state.lock().unwrap().folders.iter().cloned().collect()
    }

    pub fn restricted_folders(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap()
            .restricted
            .iter()
            .cloned()
            .collect()
    }

    /// Trust `folder`. A restriction on it goes too; one on a folder inside
    /// it stays (trusting `~/Code` doesn't trust a checkout of someone else's
    /// branch there).
    pub fn trust(&self, folder: &str) {
        let folder = normalize(folder);
        if !is_folder(&folder) {
            return;
        }
        {
            let mut state = self.state.lock().unwrap();
            state.restricted.remove(&folder);
            if trusting_folder(&state, &folder).is_none() {
                // Folders inside it are covered now.
                state.folders.retain(|f| !contains(&folder, f));
                state.folders.insert(folder);
            }
        }
        self.save();
    }

    /// Keep `folder` restricted even inside a trusted folder, until it's
    /// trusted itself.
    pub fn restrict(&self, folder: &str) {
        let folder = normalize(folder);
        if !is_folder(&folder) {
            return;
        }
        {
            let mut state = self.state.lock().unwrap();
            state.folders.retain(|f| !contains(&folder, f));
            state.restricted.insert(folder);
        }
        self.save();
    }

    /// Stop trusting `folder`, and anything trusted inside it. Returns the
    /// folder still trusting it from above (only removing that one would
    /// restrict `folder`), or `None` when it's restricted now.
    pub fn revoke(&self, folder: &str) -> Option<String> {
        let folder = normalize(folder);
        let above = {
            let mut state = self.state.lock().unwrap();
            state.folders.retain(|f| !contains(&folder, f));
            trusting_folder(&state, &folder)
        };
        self.save();
        above
    }

    /// Write the list (so the store exists even with nothing trusted).
    pub fn persist(&self) {
        self.save();
    }

    pub fn revoke_all(&self) {
        {
            let mut state = self.state.lock().unwrap();
            state.folders.clear();
            state.restricted.clear();
        }
        self.save();
    }

    /// Written while the state is locked, so two changes on different
    /// threads can't land in the wrong order (one restriction lost), through
    /// a temporary file of this process's own.
    fn save(&self) {
        let Some(file) = &self.file else { return };
        let state = self.state.lock().unwrap();
        let stored = Stored {
            version: 1,
            trusted: state.folders.iter().cloned().collect(),
            restricted: Some(state.restricted.iter().cloned().collect()),
        };
        let Ok(data) = serde_json::to_vec(&stored) else {
            return;
        };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let temp = file.with_extension(format!("json.{}.tmp", std::process::id()));
        if std::fs::write(&temp, data).is_ok() {
            let _ = std::fs::rename(&temp, file);
        }
    }
}

/// Absolute, with `~` expanded, symlinks resolved (so /tmp and /private/tmp
/// are the same folder) and `.`/`..` removed; no trailing slash. A path that
/// doesn't exist yet resolves as far as it does.
pub fn normalize(path: &str) -> String {
    let expanded = match path.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            format!("{}{rest}", dirs::home_dir().unwrap_or_default().display())
        }
        _ => path.to_string(),
    };
    let path = Path::new(&expanded);
    // Resolve the longest part that exists, then add the rest lexically.
    let mut existing = path.to_path_buf();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    let resolved = loop {
        if let Ok(real) = std::fs::canonicalize(&existing) {
            break real;
        }
        match (existing.file_name(), existing.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name.to_os_string());
                existing = parent.to_path_buf();
            }
            _ => break existing,
        }
    };
    let mut out = PathBuf::new();
    for component in resolved.components().chain(
        rest.iter()
            .rev()
            .flat_map(|name| Path::new(name).components()),
    ) {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    let text = out.to_string_lossy().into_owned();
    if text.len() > 1 && text.ends_with('/') {
        text[..text.len() - 1].to_string()
    } else {
        text
    }
}

/// A folder the list can hold: absolute (an empty or relative entry would
/// match every path).
fn is_folder(folder: &str) -> bool {
    folder.starts_with('/')
}

/// `folder` is `path` or one of its ancestors.
fn contains(folder: &str, path: &str) -> bool {
    is_folder(folder)
        && (folder == "/" || path == folder || path.starts_with(&format!("{folder}/")))
}

/// The trusted folder `path` is in, the innermost one, unless a restricted
/// folder at or inside it holds `path`.
fn trusting_folder(state: &State, path: &str) -> Option<String> {
    let folder = state
        .folders
        .iter()
        .filter(|f| contains(f, path))
        .max_by_key(|f| f.len())?;
    let blocked = state
        .restricted
        .iter()
        .any(|r| contains(r, path) && contains(folder, r));
    (!blocked).then(|| folder.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch {
        _dir: tempfile::TempDir,
        root: String,
    }

    fn scratch() -> Scratch {
        let dir = tempfile::tempdir().unwrap();
        for sub in ["Code/app/src", "Code/other", "Code/app-2"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        let root = dir.path().to_string_lossy().into_owned();
        Scratch { _dir: dir, root }
    }

    #[test]
    fn a_trusted_folder_covers_whats_inside() {
        let s = scratch();
        let trust = WorkspaceTrust::new(None);
        let app = format!("{}/Code/app", s.root);
        assert!(!trust.is_trusted(&app));
        trust.trust(&app);
        assert!(trust.is_trusted(&app));
        assert!(trust.is_trusted(&format!("{app}/src")));
        assert!(
            trust.is_trusted(&format!("{app}/src/main.rs")),
            "files that don't exist yet too"
        );
        assert!(
            !trust.is_trusted(&format!("{}/Code/app-2", s.root)),
            "a name prefix isn't a folder"
        );
        assert!(!trust.is_trusted(&format!("{}/Code", s.root)));
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_the_same_folder() {
        let s = scratch();
        let link = format!("{}/link", s.root);
        std::os::unix::fs::symlink(format!("{}/Code/app", s.root), &link).unwrap();
        let trust = WorkspaceTrust::new(None);
        trust.trust(&link);
        assert!(trust.is_trusted(&format!("{}/Code/app/src", s.root)));
        assert!(trust.is_trusted(&format!("{}/Code/app/../app/src", s.root)));
    }

    #[test]
    fn revoking_and_parent_folders() {
        let s = scratch();
        let code = format!("{}/Code", s.root);
        let app = format!("{code}/app");
        let trust = WorkspaceTrust::new(None);
        trust.trust(&app);
        trust.trust(&code);
        assert_eq!(
            trust.trusted_folders(),
            [normalize(&code)],
            "the parent replaces it"
        );
        assert_eq!(
            trust.revoke(&app),
            Some(normalize(&code)),
            "still trusted from above"
        );
        assert_eq!(trust.revoke(&code), None);
        assert!(!trust.is_trusted(&app));
    }

    #[test]
    fn a_restricted_folder_stays_restricted_inside_a_trusted_one() {
        let s = scratch();
        let code = format!("{}/Code", s.root);
        let task = format!("{code}/app/src");
        let trust = WorkspaceTrust::new(None);
        trust.trust(&code);
        trust.restrict(&task);
        assert!(!trust.is_trusted(&task));
        assert!(!trust.is_trusted(&format!("{task}/main.rs")));
        assert_eq!(trust.trusting_folder(&task), None);
        assert!(
            trust.is_trusted(&format!("{code}/app")),
            "only the restricted folder"
        );
        assert!(trust.is_trusted(&format!("{code}/other")));

        // Trusting the folder above again doesn't lift it.
        trust.trust(&code);
        assert!(!trust.is_trusted(&task));
        // Trusting it does.
        trust.trust(&task);
        assert!(trust.is_trusted(&task));
        assert!(trust.restricted_folders().is_empty());
    }

    #[test]
    fn a_folder_inside_a_restricted_one_can_be_trusted() {
        let s = scratch();
        let code = format!("{}/Code", s.root);
        let trust = WorkspaceTrust::new(None);
        trust.trust(&code);
        trust.restrict(&format!("{code}/app"));
        trust.trust(&format!("{code}/app/src"));
        assert!(trust.is_trusted(&format!("{code}/app/src")));
        assert!(!trust.is_trusted(&format!("{code}/app")));
        assert_eq!(
            trust.revoke(&format!("{code}/app/src")),
            None,
            "restricted again, not trusted from above"
        );
    }

    #[test]
    fn restrictions_are_kept_between_launches() {
        let s = scratch();
        let file = PathBuf::from(format!("{}/data/trusted-folders.json", s.root));
        let code = format!("{}/Code", s.root);
        let first = WorkspaceTrust::new(Some(file.clone()));
        first.trust(&code);
        first.restrict(&format!("{code}/other"));
        let second = WorkspaceTrust::new(Some(file.clone()));
        assert!(second.is_trusted(&format!("{code}/app")));
        assert!(!second.is_trusted(&format!("{code}/other")));
        second.revoke_all();
        assert!(WorkspaceTrust::new(Some(file))
            .restricted_folders()
            .is_empty());
    }

    #[test]
    fn turned_off_trusts_everything() {
        let trust = WorkspaceTrust::new(None);
        assert!(!trust.is_trusted("/somewhere/else"));
        trust.set_enabled(false);
        assert!(trust.is_trusted("/somewhere/else"));
    }

    #[test]
    fn keeps_the_list_between_launches() {
        let s = scratch();
        let file = PathBuf::from(format!("{}/data/trusted-folders.json", s.root));
        let other = format!("{}/Code/other", s.root);
        let first = WorkspaceTrust::new(Some(file.clone()));
        assert!(!first.existed_before);
        first.trust(&other);
        let second = WorkspaceTrust::new(Some(file.clone()));
        assert!(second.existed_before);
        assert!(second.is_trusted(&other));
        second.revoke_all();
        assert!(!WorkspaceTrust::new(Some(file)).is_trusted(&other));
    }

    #[test]
    fn reads_the_swift_apps_file() {
        let s = scratch();
        let file = PathBuf::from(format!("{}/trusted-folders.json", s.root));
        let code = normalize(&format!("{}/Code", s.root));
        std::fs::write(
            &file,
            format!(r#"{{"version":1,"trusted":["{code}"],"restricted":["{code}/other"]}}"#),
        )
        .unwrap();
        let trust = WorkspaceTrust::new(Some(file));
        assert!(trust.is_trusted(&format!("{code}/app")));
        assert!(!trust.is_trusted(&format!("{code}/other")));
    }

    #[test]
    fn an_empty_entry_trusts_nothing() {
        let dir = std::env::temp_dir().join(format!("impulse-trust-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("trusted-folders.json");
        std::fs::write(&file, r#"{"version":1,"trusted":["", "relative"]}"#).unwrap();
        let trust = WorkspaceTrust::new(Some(file));
        assert!(!trust.is_trusted("/usr"));
        trust.trust("");
        assert!(!trust.is_trusted("/usr"));
        assert!(trust.trusted_folders().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
