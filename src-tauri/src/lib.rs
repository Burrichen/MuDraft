//! MuDraft native core. `db`, `domain`, `error`, and `library` form the persistence and
//! domain contract that Tauri commands build on; they are public so integration tests
//! exercise exactly that surface.

pub mod artwork;
mod commands;
pub mod csv_import;
pub mod db;
pub mod domain;
pub mod error;
pub mod library;
pub mod metadata;
mod paths;
mod state;

use tauri::Manager;

use crate::paths::StorageProfile;
use crate::state::AppState;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let state = init_state(app.handle());
            let metadata =
                commands::metadata::MetadataService::musicbrainz(state.database_path().as_deref())?;
            let artwork = artwork::ArtworkService::new(std::sync::Arc::new(
                artwork::http::ReqwestImageTransport::new()?,
            ));
            app.manage(state);
            app.manage(metadata);
            app.manage(artwork);
            Ok(())
        })
        // Cached artwork, served only from files recorded in the database.
        .register_uri_scheme_protocol("artwork", |ctx, request| {
            serve_artwork(ctx.app_handle(), request.uri().path())
        })
        .invoke_handler(tauri::generate_handler![
            commands::health_check,
            commands::get_ui_preferences,
            commands::update_ui_preferences,
            commands::metadata::metadata_search,
            commands::metadata::metadata_editions,
            commands::metadata::metadata_cancel,
            commands::metadata::metadata_import,
            commands::metadata::library_add_manual_album,
            commands::metadata::library_set_album_genres,
            commands::metadata::genre_vocabulary,
            commands::csv_import::csv_import_save_template,
            commands::csv_import::csv_import_open,
            commands::csv_import::csv_import_latest,
            commands::csv_import::csv_import_summary,
            commands::csv_import::csv_import_apply_mapping,
            commands::csv_import::csv_import_rows,
            commands::csv_import::csv_import_enrich,
            commands::csv_import::csv_import_decide,
            commands::csv_import::csv_import_commit,
            commands::csv_import::csv_import_discard,
            commands::library::library_list,
            commands::library::listen_list_remove,
            commands::library::tags_list,
            commands::library::tag_create,
            commands::library::tag_update,
            commands::library::tag_delete,
            commands::library::album_tags_update,
            commands::album::album_detail,
            commands::album::album_edit,
            commands::album::album_unlock,
            commands::album::edition_rename,
            commands::album::album_refresh,
            commands::album::edition_candidates,
            commands::album::edition_add,
            commands::album::edition_switch_preview,
            commands::album::edition_switch,
            commands::album::artwork_preference,
            commands::album::artwork_set_preference,
            commands::album::artwork_fetch,
            commands::album::artwork_fetch_many,
            commands::album::artwork_replace,
            commands::album::artwork_remove,
            commands::album::artwork_restore,
            commands::album::artwork_clear_cache,
            commands::album::rating_set_track,
            commands::album::rating_set_album,
            commands::album::track_set_favourite,
            commands::album::album_review_set,
            commands::listening::listen_log,
            commands::listening::listen_undo_log,
            commands::listening::listen_correct,
            commands::listening::listen_delete,
            commands::listening::listen_restore,
            commands::listening::listen_confirm_coverage,
            commands::listening::collection_add,
            commands::listening::collection_remove,
            commands::listening::listening_settings,
            commands::listening::listening_settings_set,
            commands::next_up::next_up_state,
            commands::next_up::next_up_options,
            commands::next_up::next_up_roll,
            commands::next_up::next_up_reset_pool,
            commands::next_up::next_up_choose,
            commands::next_up::next_up_clear,
            commands::discography::artist_catalogue,
            commands::discography::artist_catalogue_fetch,
            commands::discography::artist_tracklists_load,
            commands::discography::artist_catalogue_scope_set,
            commands::discography::artist_catalogue_add_manual,
            commands::discography::artist_catalogue_remove_manual,
            commands::discography::artist_catalogue_exclude,
            commands::discography::artist_catalogue_add_to_listen_list,
            commands::stats::stats_overview,
        ])
        .run(tauri::generate_context!())
        .expect("error while running MuDraft");
}

fn init_state(app: &tauri::AppHandle) -> AppState {
    let profile = StorageProfile::current();
    let os_dir = match app.path().app_local_data_dir() {
        Ok(dir) => dir,
        Err(e) => {
            let err = error::AppError::StorageUnavailable(format!("no per-user data dir: {e}"));
            return AppState::failed(profile, Default::default(), err);
        }
    };
    let override_dir = std::env::var_os(paths::DATA_DIR_ENV);
    match paths::resolve_data_dir(&os_dir, profile, override_dir) {
        Ok(dir) => {
            let state = AppState::open(profile, dir);
            if let Err(e) = state.with_db(|_| Ok(())) {
                eprintln!("MuDraft storage error [{}]: {e}", e.code());
            }
            state
        }
        Err(e) => AppState::failed(profile, os_dir, e),
    }
}

/// `artwork://localhost/<owner_type>%2F<owner_id>` (Windows: `http://artwork.localhost/…`).
fn serve_artwork(app: &tauri::AppHandle, path: &str) -> tauri::http::Response<Vec<u8>> {
    let decoded = path
        .trim_start_matches('/')
        .replace("%2F", "/")
        .replace("%2f", "/");
    let mut parts = decoded.splitn(2, '/');
    let (owner_type, owner_id) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let served = app
        .try_state::<AppState>()
        .ok_or_else(|| error::AppError::Internal("not ready".into()))
        .and_then(|state| artwork::serve(&state, owner_type, owner_id));
    let builder = tauri::http::Response::builder().header("Cache-Control", "no-cache");
    match served {
        Ok((bytes, mime)) => builder
            .status(200)
            .header("Content-Type", mime)
            .header("X-Content-Type-Options", "nosniff")
            .body(bytes),
        Err(_) => builder.status(404).body(Vec::new()),
    }
    .unwrap_or_else(|_| tauri::http::Response::new(Vec::new()))
}
