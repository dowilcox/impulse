//! The app's settings: `settings.json` in the data directory, loaded at
//! launch, watched while Impulse runs (edits made in another editor apply
//! as soon as they're saved), and saved when Impulse changes a setting, as
//! the Swift `SettingsStore` does. A save that finds the file changed on
//! disk merges the two, key by key, and saves again.

use std::time::Duration;

use gpui_kit::{App, Global};
use impulse_kit::settings::{SaveOutcome, Settings, SettingsFile};

pub struct SettingsStore {
    pub settings: Settings,
    file: SettingsFile,
    /// The settings as the file last held them, for merging.
    base: Settings,
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
        cx.set_global(SettingsStore {
            base: settings.clone(),
            settings,
            file,
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

    /// The file changed outside Impulse: take its settings, unless it no
    /// longer parses (then keep what Impulse has).
    fn reload(cx: &mut App) {
        let store = cx.global_mut::<SettingsStore>();
        if let Some(settings) = store.file.load(true) {
            store.base = settings.clone();
            if settings != store.settings {
                store.settings = settings;
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
