//! Android app updates. The Tauri updater has no Android support, so Android asks the same
//! published `latest.json` the desktop apps use whether a newer version exists, and opens the
//! matching APK download in the browser. Android itself refuses to install an APK that isn't
//! signed with the installed app's key, so a wrong or tampered file cannot replace MuDraft.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::error::{AppError, AppResult};

const MANIFEST_URL: &str =
    "https://github.com/Burrichen/MuDraft/releases/latest/download/latest.json";
/// The release asset each version's Android build is published as.
const APK_URL: &str = "https://github.com/Burrichen/MuDraft/releases/download";
const MAX_MANIFEST_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableUpdate {
    pub version: String,
    pub current_version: String,
    pub notes: String,
}

#[derive(Deserialize)]
struct Manifest {
    version: String,
    #[serde(default)]
    notes: String,
}

/// `major.minor.patch` (an optional `v` prefix is allowed); anything else is not a version.
fn parse_version(v: &str) -> Option<[u64; 3]> {
    let mut parts = v.trim().trim_start_matches('v').split('.');
    let mut next = || -> Option<u64> {
        let p = parts.next()?;
        (!p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) && p.len() <= 9)
            .then(|| p.parse().ok())?
    };
    let out = [next()?, next()?, next()?];
    parts.next().is_none().then_some(out)
}

fn read_manifest(body: &[u8], current: &str) -> AppResult<Option<AvailableUpdate>> {
    let manifest: Manifest = serde_json::from_slice(body)
        .map_err(|_| AppError::Provider("the update information isn't readable".into()))?;
    let (Some(latest), Some(have)) = (parse_version(&manifest.version), parse_version(current))
    else {
        return Err(AppError::Provider(
            "the update information has an unexpected version".into(),
        ));
    };
    Ok((latest > have).then(|| AvailableUpdate {
        version: manifest.version.trim().trim_start_matches('v').to_owned(),
        current_version: current.to_owned(),
        notes: manifest.notes.trim().to_owned(),
    }))
}

/// Is a newer MuDraft published? `None` when this is the newest.
#[tauri::command]
pub async fn app_update_check() -> AppResult<Option<AvailableUpdate>> {
    let client = crate::net::client_builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| AppError::Internal(format!("cannot build HTTP client: {e}")))?;
    let response = client
        .get(MANIFEST_URL)
        .send()
        .await
        .map_err(|e| AppError::Offline(e.to_string()))?;
    if !response.status().is_success() {
        return Err(AppError::Provider(format!(
            "GitHub answered {} for the update information",
            response.status().as_u16()
        )));
    }
    let body = response
        .bytes()
        .await
        .map_err(|e| AppError::Offline(e.to_string()))?;
    if body.len() > MAX_MANIFEST_BYTES {
        return Err(AppError::Provider(
            "the update information is too large".into(),
        ));
    }
    read_manifest(&body, env!("CARGO_PKG_VERSION"))
}

fn apk_url(version: &str) -> AppResult<String> {
    let [a, b, c] = parse_version(version)
        .ok_or_else(|| AppError::validation("version", "isn't a version number"))?;
    Ok(format!(
        "{APK_URL}/v{a}.{b}.{c}/MuDraft_{a}.{b}.{c}_android.apk"
    ))
}

/// Open the APK download for `version` in the browser; the user opens the finished download
/// to install it over this version.
#[tauri::command]
pub async fn app_update_open(app: AppHandle, version: String) -> AppResult<()> {
    let url = apk_url(&version)?;
    #[cfg(target_os = "android")]
    {
        use tauri_plugin_opener::OpenerExt;
        app.opener()
            .open_url(url, None::<&str>)
            .map_err(|e| AppError::Internal(format!("couldn't open the download: {e}")))
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, url);
        Err(AppError::validation(
            "update",
            "this build updates itself; there is no APK to open",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(parse_version("1.10.0") > parse_version("1.9.9"));
        assert_eq!(parse_version("v1.4.0"), Some([1, 4, 0]));
        for bad in ["", "1.4", "1.4.0.1", "1.4.x", "1.4.0-beta", "../1.0.0"] {
            assert_eq!(parse_version(bad), None, "{bad}");
        }
    }

    #[test]
    fn only_a_newer_published_version_is_offered() {
        let manifest = br#"{"version":"1.5.0","notes":"  - Faster\n","platforms":{}}"#;
        let update = read_manifest(manifest, "1.4.0").unwrap().unwrap();
        assert_eq!(update.version, "1.5.0");
        assert_eq!(update.current_version, "1.4.0");
        assert_eq!(update.notes, "- Faster");
        assert_eq!(read_manifest(manifest, "1.5.0").unwrap(), None);
        assert_eq!(read_manifest(manifest, "2.0.0").unwrap(), None);
        assert!(read_manifest(b"not json", "1.4.0").is_err());
        assert!(read_manifest(br#"{"version":"latest"}"#, "1.4.0").is_err());
    }

    #[test]
    fn the_download_url_is_built_only_from_a_valid_version() {
        assert_eq!(
            apk_url("v1.5.0").unwrap(),
            "https://github.com/Burrichen/MuDraft/releases/download/v1.5.0/MuDraft_1.5.0_android.apk"
        );
        assert!(apk_url("1.5.0/../../x").is_err());
        assert!(apk_url("1.5").is_err());
    }
}
