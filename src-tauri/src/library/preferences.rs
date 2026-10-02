//! Interface preferences that survive restart: sidebar state, last route, album layout.
//! Stored in the same `setting` table as other settings, under `ui.*` keys.

use rusqlite::{Connection, Transaction};
use serde::{Deserialize, Serialize};

use super::settings::{read_value, write_value};
use crate::domain::ids::parse_uuid;
use crate::error::{AppError, AppResult};

const SIDEBAR_COLLAPSED: &str = "ui.sidebar_collapsed";
const LAST_ROUTE: &str = "ui.last_route";
const ALBUM_LAYOUT: &str = "ui.album_layout";

pub const DEFAULT_ROUTE: &str = "/listen-list";
const PRIMARY_ROUTES: &[&str] = &[
    "/listen-list",
    "/next-up",
    "/collection",
    "/stats",
    "/settings",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlbumLayout {
    Grid,
    List,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiPreferences {
    pub sidebar_collapsed: bool,
    pub last_route: String,
    pub album_layout: AlbumLayout,
}

impl Default for UiPreferences {
    fn default() -> Self {
        Self {
            sidebar_collapsed: false,
            last_route: DEFAULT_ROUTE.to_owned(),
            album_layout: AlbumLayout::Grid,
        }
    }
}

/// Partial update; absent fields are left unchanged. Unknown fields are rejected.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiPreferencesPatch {
    pub sidebar_collapsed: Option<bool>,
    pub last_route: Option<String>,
    pub album_layout: Option<AlbumLayout>,
}

pub fn load(conn: &Connection) -> AppResult<UiPreferences> {
    let d = UiPreferences::default();
    let last_route = match read_value::<String>(conn, LAST_ROUTE, "a route")? {
        // A route saved by an older build that no longer exists falls back to the default
        // view; the stored value is left alone until the user navigates.
        Some(route) => normalize_route(&route).unwrap_or(d.last_route),
        None => d.last_route,
    };
    Ok(UiPreferences {
        sidebar_collapsed: read_value(conn, SIDEBAR_COLLAPSED, "true/false")?
            .unwrap_or(d.sidebar_collapsed),
        last_route,
        album_layout: read_value(conn, ALBUM_LAYOUT, "\"grid\" or \"list\"")?
            .unwrap_or(d.album_layout),
    })
}

/// Validate every field first, then write them together.
pub fn update(tx: &Transaction<'_>, patch: &UiPreferencesPatch) -> AppResult<UiPreferences> {
    let route = patch
        .last_route
        .as_deref()
        .map(normalize_route)
        .transpose()?;
    if let Some(v) = patch.sidebar_collapsed {
        write_value(tx, SIDEBAR_COLLAPSED, v)?;
    }
    if let Some(route) = route {
        write_value(tx, LAST_ROUTE, route)?;
    }
    if let Some(layout) = patch.album_layout {
        write_value(tx, ALBUM_LAYOUT, layout)?;
    }
    load(tx)
}

/// Only in-app routes are accepted: the five sections, or an album/artist page by ID.
pub fn normalize_route(raw: &str) -> AppResult<String> {
    let path = raw.trim();
    if PRIMARY_ROUTES.contains(&path) {
        return Ok(path.to_owned());
    }
    let nested = |prefix: &str| {
        path.strip_prefix(prefix)
            .filter(|id| !id.contains('/'))
            .map(|id| parse_uuid("route", id).map(|id| format!("{prefix}{id}")))
    };
    nested("/albums/")
        .or_else(|| nested("/artists/"))
        .unwrap_or_else(|| {
            Err(AppError::validation(
                "route",
                format!("'{raw}' is not a MuDraft page"),
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    #[test]
    fn routes_are_restricted_to_app_pages() {
        assert_eq!(normalize_route("/stats").unwrap(), "/stats");
        let id = crate::domain::ids::new_id();
        assert_eq!(
            normalize_route(&format!("/albums/{}", id.to_uppercase())).unwrap(),
            format!("/albums/{id}")
        );
        for bad in [
            "",
            "/",
            "/diary",
            "https://example.com",
            "/albums/",
            "/albums/x",
            "/albums/{id}/x",
            "/stats?x=1",
        ] {
            assert_eq!(
                normalize_route(bad).unwrap_err().code(),
                "validation",
                "{bad}"
            );
        }
    }

    #[test]
    fn preferences_default_update_partially_and_persist() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite3");
        let mut db = Database::open(&path).unwrap();
        assert_eq!(db.read(load).unwrap(), UiPreferences::default());

        let patch = UiPreferencesPatch {
            sidebar_collapsed: Some(true),
            last_route: Some("/collection".into()),
            ..Default::default()
        };
        let updated = db.write(|tx| update(tx, &patch)).unwrap();
        assert!(updated.sidebar_collapsed);
        assert_eq!(updated.album_layout, AlbumLayout::Grid);
        drop(db);

        let db = Database::open(&path).unwrap();
        let loaded = db.read(load).unwrap();
        assert_eq!(loaded, updated);
        assert_eq!(loaded.last_route, "/collection");
    }

    #[test]
    fn invalid_patch_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Database::open(&dir.path().join("t.sqlite3")).unwrap();
        let patch = UiPreferencesPatch {
            sidebar_collapsed: Some(true),
            last_route: Some("/nope".into()),
            album_layout: None,
        };
        assert_eq!(
            db.write(|tx| update(tx, &patch)).unwrap_err().code(),
            "validation"
        );
        assert!(!db.read(load).unwrap().sidebar_collapsed);
        let unknown = serde_json::from_str::<UiPreferencesPatch>(r#"{"theme":"light"}"#);
        assert!(unknown.is_err());
    }

    #[test]
    fn stale_route_falls_back_but_wrong_type_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Database::open(&dir.path().join("t.sqlite3")).unwrap();
        db.write(|tx| write_value(tx, LAST_ROUTE, "/diary"))
            .unwrap();
        assert_eq!(db.read(load).unwrap().last_route, DEFAULT_ROUTE);
        db.write(|tx| write_value(tx, SIDEBAR_COLLAPSED, "yes"))
            .unwrap();
        assert_eq!(db.read(load).unwrap_err().code(), "validation");
    }
}
