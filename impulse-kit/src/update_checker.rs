//! Checking GitHub Releases for a newer Impulse (the Swift `UpdateChecker`,
//! from `impulse-core/src/update.rs`). At most one check a day: the time of
//! the last one is kept in `<cache dir>/impulse/last_update_check`.
//!
//! The HTTP request itself is the caller's: `check_for_update` hands it an
//! `UpdateRequest` and takes back the body of a successful response, so
//! this crate stays free of networking.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

const GITHUB_OWNER: &str = "dowilcox";
const GITHUB_REPO: &str = "impulse";
const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateInfo {
    /// The newer version, without a leading `v`.
    pub version: String,
    pub current_version: String,
    /// The release's page.
    pub url: String,
}

/// The GET request to make for the latest release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub timeout: Duration,
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
}

/// Where the time of the last check is kept.
pub fn cache_path() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("impulse").join("last_update_check"))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn should_check(cache: Option<&Path>, now: u64) -> bool {
    let Some(last_check) = cache
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|contents| contents.trim().parse::<u64>().ok())
    else {
        return true;
    };
    now >= last_check && now - last_check >= CHECK_INTERVAL_SECS
}

fn write_check_timestamp(cache: Option<&Path>, now: u64) {
    let Some(path) = cache else { return };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, now.to_string());
}

/// `(major, minor, patch)` from `v1.2.3` or `1.2.3`.
pub fn parse_version(tag: &str) -> Option<(u32, u32, u32)> {
    let v = tag.strip_prefix('v').unwrap_or(tag);
    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    Some((
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
    ))
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

/// The request for the latest release, sent as `impulse/<current_version>`.
pub fn latest_release_request(current_version: &str) -> UpdateRequest {
    UpdateRequest {
        url: format!("https://api.github.com/repos/{GITHUB_OWNER}/{GITHUB_REPO}/releases/latest"),
        headers: vec![
            ("Accept".into(), "application/vnd.github.v3+json".into()),
            ("User-Agent".into(), format!("impulse/{current_version}")),
        ],
        timeout: REQUEST_TIMEOUT,
    }
}

/// Check GitHub Releases for a newer version. Blocking (as `fetch` is):
/// call it off the UI thread. `fetch` makes the request and returns the
/// body of a 2xx response, `None` otherwise. Returns `None` when up to
/// date, checked within the last day, or on any error.
pub fn check_for_update(
    current_version: &str,
    fetch: impl FnOnce(&UpdateRequest) -> Option<String>,
) -> Option<UpdateInfo> {
    check_for_update_with_cache(cache_path().as_deref(), now_secs(), current_version, fetch)
}

/// `check_for_update` with the cache file and the time given.
pub fn check_for_update_with_cache(
    cache: Option<&Path>,
    now: u64,
    current_version: &str,
    fetch: impl FnOnce(&UpdateRequest) -> Option<String>,
) -> Option<UpdateInfo> {
    if !should_check(cache, now) {
        return None;
    }
    let body = fetch(&latest_release_request(current_version))?;
    let release: GitHubRelease = serde_json::from_str(&body).ok()?;

    write_check_timestamp(cache, now);

    if !is_newer(&release.tag_name, current_version) {
        log::info!(
            "No update available (current: {current_version}, latest: {})",
            release.tag_name
        );
        return None;
    }
    let version = release
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&release.tag_name)
        .to_string();
    log::info!("Update available: {current_version} -> {version}");
    Some(UpdateInfo {
        version,
        current_version: current_version.to_string(),
        url: release.html_url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versions() {
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("0.13.2"), Some((0, 13, 2)));
        assert_eq!(parse_version("v0.14.0"), Some((0, 14, 0)));
        assert_eq!(parse_version("invalid"), None);
        assert_eq!(parse_version("1.2"), None);
    }

    #[test]
    fn compares_versions() {
        assert!(is_newer("v0.14.0", "0.13.2"));
        assert!(is_newer("v1.0.0", "0.99.99"));
        assert!(!is_newer("v0.13.2", "0.13.2"));
        assert!(!is_newer("v0.13.1", "0.13.2"));
    }

    #[test]
    fn checks_at_most_once_a_day() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("impulse/last_update_check");
        let release = r#"{"tag_name": "v0.40.0", "html_url": "https://example.com/r"}"#;
        let mut asked = Vec::new();
        let found = check_for_update_with_cache(Some(&cache), 1_000_000, "0.36.0", |request| {
            asked.push(request.url.clone());
            Some(release.to_string())
        });
        assert_eq!(
            found,
            Some(UpdateInfo {
                version: "0.40.0".into(),
                current_version: "0.36.0".into(),
                url: "https://example.com/r".into(),
            })
        );
        assert_eq!(
            asked,
            vec!["https://api.github.com/repos/dowilcox/impulse/releases/latest"]
        );
        assert_eq!(fs::read_to_string(&cache).unwrap(), "1000000");

        // Within the day: no request at all.
        let again = check_for_update_with_cache(Some(&cache), 1_000_000 + 3600, "0.36.0", |_| {
            panic!("asked again within a day")
        });
        assert_eq!(again, None);

        // A day later, up to date.
        let later = check_for_update_with_cache(
            Some(&cache),
            1_000_000 + CHECK_INTERVAL_SECS,
            "0.40.0",
            |_| Some(release.to_string()),
        );
        assert_eq!(later, None);
    }

    #[test]
    fn failed_requests_dont_count_as_a_check() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("last_update_check");
        assert_eq!(
            check_for_update_with_cache(Some(&cache), 5, "0.36.0", |_| None),
            None
        );
        assert_eq!(
            check_for_update_with_cache(Some(&cache), 5, "0.36.0", |_| Some("<html>".into())),
            None
        );
        assert!(!cache.exists());
    }
}
