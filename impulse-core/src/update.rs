use serde::Deserialize;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const GITHUB_OWNER: &str = "dowilcox";
const GITHUB_REPO: &str = "impulse";
const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60; // 24 hours
const REQUEST_TIMEOUT_SECS: u64 = 5;

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
}

#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub version: String,
    pub current_version: String,
    pub url: String,
}

fn cache_path() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("impulse").join("last_update_check"))
}

fn should_check() -> bool {
    let Some(path) = cache_path() else {
        return true;
    };
    let Ok(contents) = fs::read_to_string(&path) else {
        return true;
    };
    let Ok(last_check) = contents.trim().parse::<u64>() else {
        return true;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    now.saturating_sub(last_check) >= CHECK_INTERVAL_SECS
}

fn write_check_timestamp() {
    let Some(path) = cache_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let _ = fs::write(&path, now.to_string());
}

fn parse_version(tag: &str) -> Option<(u32, u32, u32)> {
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

fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

/// Check GitHub Releases for a newer version.
///
/// Returns `Ok(Some(UpdateInfo))` if a newer version is available,
/// `Ok(None)` if already up to date or checked recently,
/// `Err` on network/parse errors.
///
/// Respects a 24-hour cache interval to avoid hitting the API on every launch.
pub fn check_for_update() -> Result<Option<UpdateInfo>, String> {
    if !should_check() {
        return Ok(None);
    }

    let url = format!(
        "https://api.github.com/repos/{}/{}/releases/latest",
        GITHUB_OWNER, GITHUB_REPO
    );

    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS)))
        .build()
        .new_agent();

    let response = agent
        .get(&url)
        .header("Accept", "application/vnd.github.v3+json")
        .header("User-Agent", &format!("impulse/{}", CURRENT_VERSION))
        .call()
        .map_err(|e| format!("Update check failed: {}", e))?;

    let body = response
        .into_body()
        .read_to_string()
        .map_err(|e| format!("Failed to read response: {}", e))?;

    let release: GitHubRelease =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse release JSON: {}", e))?;

    write_check_timestamp();

    if is_newer(&release.tag_name, CURRENT_VERSION) {
        let version = release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name)
            .to_string();
        log::info!(
            "Update available: {} -> {} (current: {})",
            CURRENT_VERSION,
            version,
            CURRENT_VERSION
        );
        Ok(Some(UpdateInfo {
            version,
            current_version: CURRENT_VERSION.to_string(),
            url: release.html_url,
        }))
    } else {
        log::info!(
            "No update available (current: {}, latest: {})",
            CURRENT_VERSION,
            release.tag_name
        );
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_version() {
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("0.13.2"), Some((0, 13, 2)));
        assert_eq!(parse_version("v0.14.0"), Some((0, 14, 0)));
        assert_eq!(parse_version("invalid"), None);
    }

    #[test]
    fn test_is_newer() {
        assert!(is_newer("v0.14.0", "0.13.2"));
        assert!(is_newer("v1.0.0", "0.99.99"));
        assert!(!is_newer("v0.13.2", "0.13.2"));
        assert!(!is_newer("v0.13.1", "0.13.2"));
    }
}
