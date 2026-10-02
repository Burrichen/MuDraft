use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{AppError, AppResult};

pub const DB_FILE_NAME: &str = "mudraft.sqlite3";
/// Debug-build-only override, used for automated desktop runs against throwaway storage.
pub const DATA_DIR_ENV: &str = "MUDRAFT_DATA_DIR";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageProfile {
    Production,
    Development,
}

impl StorageProfile {
    /// Release builds always use production storage; debug builds never do.
    pub fn current() -> Self {
        if cfg!(debug_assertions) {
            Self::Development
        } else {
            Self::Production
        }
    }
}

/// Resolve the data directory from the OS per-user local data dir
/// (`~/Library/Application Support/<id>` on macOS, `%LOCALAPPDATA%\<id>` on Windows).
/// Development storage is a sibling `<id>.dev` directory so dev runs can never touch real data.
pub fn resolve_data_dir(
    os_app_local_data_dir: &Path,
    profile: StorageProfile,
    override_dir: Option<OsString>,
) -> AppResult<PathBuf> {
    if profile == StorageProfile::Development
        && let Some(raw) = override_dir.filter(|v| !v.is_empty())
    {
        let dir = PathBuf::from(raw);
        if !dir.is_absolute() {
            return Err(AppError::InvalidPath(format!(
                "{DATA_DIR_ENV} must be an absolute path, got {}",
                dir.display()
            )));
        }
        return Ok(dir);
    }

    match profile {
        StorageProfile::Production => Ok(os_app_local_data_dir.to_path_buf()),
        StorageProfile::Development => {
            let name = os_app_local_data_dir.file_name().ok_or_else(|| {
                AppError::InvalidPath(format!(
                    "OS data directory has no final component: {}",
                    os_app_local_data_dir.display()
                ))
            })?;
            let mut dev_name = name.to_os_string();
            dev_name.push(".dev");
            Ok(os_app_local_data_dir.with_file_name(dev_name))
        }
    }
}

pub fn ensure_dir(dir: &Path) -> AppResult<()> {
    std::fs::create_dir_all(dir)
        .map_err(|e| AppError::StorageUnavailable(format!("cannot create {}: {e}", dir.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> PathBuf {
        std::env::temp_dir().join("app.mudraft.desktop")
    }

    #[test]
    fn production_uses_os_dir_and_ignores_override() {
        let dir = resolve_data_dir(
            &base(),
            StorageProfile::Production,
            Some(base().join("elsewhere").into()),
        )
        .unwrap();
        assert_eq!(dir, base());
    }

    #[test]
    fn development_uses_sibling_dev_dir() {
        let dir = resolve_data_dir(&base(), StorageProfile::Development, None).unwrap();
        assert_eq!(dir, std::env::temp_dir().join("app.mudraft.desktop.dev"));
        assert_ne!(dir, base());
    }

    #[test]
    fn development_override_must_be_absolute() {
        let err = resolve_data_dir(
            &base(),
            StorageProfile::Development,
            Some("relative/dir".into()),
        )
        .unwrap_err();
        assert_eq!(err.code(), "invalid_path");

        let abs = std::env::temp_dir().join("mudraft-e2e");
        let dir = resolve_data_dir(
            &base(),
            StorageProfile::Development,
            Some(abs.clone().into()),
        )
        .unwrap();
        assert_eq!(dir, abs);
    }
}
