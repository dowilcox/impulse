//! Checking for a newer Impulse at launch (the Swift `UpdateChecker` call
//! in `AppDelegate`): at most once a day when `check_for_updates` is on,
//! through `impulse_kit::update_checker`, the request made with `curl` so
//! the app needs no HTTP stack of its own. A newer release shows as
//! "Update x.y.z" in every window's status bar, opening its page.

use gpui_kit::{App, AppContext as _, Global};
use impulse_kit::child_process::ChildProcess;
use impulse_kit::update_checker::{self, UpdateInfo, UpdateRequest};

/// The newer release, once one is known.
#[derive(Default)]
pub(crate) struct AvailableUpdate(pub Option<UpdateInfo>);

impl Global for AvailableUpdate {}

pub(crate) fn start(cx: &mut App) {
    cx.set_global(AvailableUpdate::default());
    if !crate::settings_store::SettingsStore::get(cx).check_for_updates {
        return;
    }
    let task = cx.background_spawn(async {
        update_checker::check_for_update(env!("CARGO_PKG_VERSION"), fetch)
    });
    cx.spawn(async move |cx| {
        let Some(update) = task.await else {
            return;
        };
        cx.update(|cx| {
            cx.set_global(AvailableUpdate(Some(update)));
            cx.refresh_windows();
        });
    })
    .detach();
}

/// The body of a successful response, by `curl`.
fn fetch(request: &UpdateRequest) -> Option<String> {
    let mut process = ChildProcess::new("curl")
        .args(["-fsSL", "--max-time"])
        .arg(request.timeout.as_secs().max(1).to_string());
    for (name, value) in &request.headers {
        process = process.arg("-H").arg(format!("{name}: {value}"));
    }
    let output = process
        .arg("--")
        .arg(&request.url)
        .timeout(request.timeout + std::time::Duration::from_secs(5))
        .run()
        .ok()?;
    (output.status == 0 && !output.timed_out).then(|| output.stdout_text())
}
