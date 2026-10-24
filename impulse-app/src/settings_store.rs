//! The app's settings: `settings.json` in the data directory, loaded at
//! launch, watched while Impulse runs (edits made in another editor apply
//! as soon as they're saved), and saved when Impulse changes a setting, as
//! the Swift `SettingsStore` does. A save that finds the file changed on
//! disk merges the two, key by key, and saves again.

use std::time::Duration;

use gpui_kit::{App, Global};
use impulse_kit::settings::{LoadWarning, SaveOutcome, Settings, SettingsFile};

pub struct SettingsStore {
    pub settings: Settings,
    file: SettingsFile,
    /// The settings as the file last held them, for merging.
    base: Settings,
    /// Counts the loads that failed, so a banner dismissed for one comes
    /// back when the file breaks again.
    warning_serial: u64,
}

impl Global for SettingsStore {}

impl SettingsStore {
    /// Load settings.json (`impulse-dev` for debug builds) and watch it.
    pub fn init(cx: &mut App) {
        let mut file =
            SettingsFile::new(impulse_kit::settings::settings_path(cfg!(debug_assertions)));
        if std::env::var_os("IMPULSE_NO_PERSISTENCE").is_some() {
            file.persistence = false;
        }
        let settings = file.load(false).unwrap_or_default();
        if let Some(warning) = &file.warning {
            log::warn!(
                "settings.json didn't load ({}); using the defaults",
                warning.message
            );
        }
        let warning_serial = u64::from(file.warning.is_some());
        cx.set_global(SettingsStore {
            base: settings.clone(),
            settings,
            file,
            warning_serial,
        });
        cx.spawn(async move |cx| loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            cx.update(|cx| {
                if cx.global::<SettingsStore>().file.changed_on_disk() {
                    Self::reload(cx);
                }
            });
        })
        .detach();
    }

    pub fn get(cx: &App) -> &Settings {
        &cx.global::<SettingsStore>().settings
    }

    /// Why settings.json didn't load, with the count of failed loads so
    /// far, while it doesn't.
    pub fn warning(cx: &App) -> Option<(u64, &LoadWarning)> {
        let store = cx.global::<SettingsStore>();
        store
            .file
            .warning
            .as_ref()
            .map(|warning| (store.warning_serial, warning))
    }

    /// The file changed outside Impulse: take its settings, unless it no
    /// longer parses (then keep what Impulse has and say so).
    fn reload(cx: &mut App) {
        let store = cx.global_mut::<SettingsStore>();
        let had_warning = store.file.warning.is_some();
        match store.file.load(true) {
            Some(settings) => {
                store.base = settings.clone();
                if settings != store.settings || had_warning {
                    store.settings = settings;
                    cx.refresh_windows();
                }
            }
            None => {
                if store.file.warning.is_some() {
                    store.warning_serial += 1;
                }
                cx.refresh_windows();
            }
        }
    }

    /// Change settings and save them.
    pub fn update(cx: &mut App, change: impl FnOnce(&mut Settings)) {
        let store = cx.global_mut::<SettingsStore>();
        let mut settings = store.settings.clone();
        change(&mut settings);
        settings.validate();
        if settings == store.settings {
            return;
        }
        store.settings = settings;
        store.save();
        cx.refresh_windows();
    }

    fn save(&mut self) {
        match self.file.save(&self.settings) {
            SaveOutcome::Saved => self.base = self.settings.clone(),
            SaveOutcome::ChangedOnDisk => {
                if let Some(theirs) = self.file.load(true) {
                    self.settings = Settings::merged(&self.settings, &theirs, &self.base);
                    self.base = theirs;
                    if self.file.save(&self.settings) == SaveOutcome::Saved {
                        self.base = self.settings.clone();
                    }
                }
            }
            SaveOutcome::Failed => {}
        }
    }
}
