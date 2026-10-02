import { convertFileSrc, invoke } from "@tauri-apps/api/core";

/**
 * The only renderer → Rust bridge. Every call names one narrow Rust command;
 * there is no generic SQL, filesystem, or shell passthrough.
 */
export type NativeCommand =
  | "health_check"
  | "get_ui_preferences"
  | "update_ui_preferences"
  | "metadata_search"
  | "metadata_editions"
  | "metadata_cancel"
  | "metadata_import"
  | "library_add_manual_album"
  | "library_set_album_genres"
  | "genre_vocabulary"
  | "csv_import_save_template"
  | "csv_import_open"
  | "csv_import_latest"
  | "csv_import_summary"
  | "csv_import_apply_mapping"
  | "csv_import_rows"
  | "csv_import_enrich"
  | "csv_import_decide"
  | "csv_import_commit"
  | "csv_import_discard"
  | "library_list"
  | "listen_list_remove"
  | "tags_list"
  | "tag_create"
  | "tag_update"
  | "tag_delete"
  | "album_tags_update"
  | "album_detail"
  | "album_edit"
  | "album_unlock"
  | "edition_rename"
  | "album_refresh"
  | "edition_candidates"
  | "edition_add"
  | "edition_switch_preview"
  | "edition_switch"
  | "artwork_preference"
  | "artwork_set_preference"
  | "artwork_fetch"
  | "artwork_fetch_many"
  | "artwork_replace"
  | "artwork_remove"
  | "artwork_restore"
  | "artwork_clear_cache"
  | "rating_set_track"
  | "rating_set_album"
  | "track_set_favourite"
  | "album_review_set"
  | "listen_log"
  | "listen_undo_log"
  | "listen_correct"
  | "listen_delete"
  | "listen_restore"
  | "listen_confirm_coverage"
  | "collection_add"
  | "collection_remove"
  | "listening_settings"
  | "listening_settings_set"
  | "next_up_state"
  | "next_up_options"
  | "next_up_roll"
  | "next_up_reset_pool"
  | "next_up_choose"
  | "next_up_clear"
  | "artist_catalogue"
  | "artist_catalogue_fetch"
  | "artist_tracklists_load"
  | "artist_catalogue_scope_set"
  | "artist_catalogue_add_manual"
  | "artist_catalogue_remove_manual"
  | "artist_catalogue_exclude"
  | "artist_catalogue_add_to_listen_list"
  | "stats_overview"
  | "metadata_cache_stats"
  | "metadata_cache_clear"
  | "profile_export"
  | "profile_import_choose"
  | "profile_import_confirm"
  | "profile_import_cancel";

/** Mirrors `AppError` serialization in `src-tauri/src/error.rs`. */
export interface NativeErrorPayload {
  code: string;
  message: string;
}

export class NativeError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = "NativeError";
    this.code = code;
  }
}

function isErrorPayload(value: unknown): value is NativeErrorPayload {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as Record<string, unknown>).code === "string" &&
    typeof (value as Record<string, unknown>).message === "string"
  );
}

export function toNativeError(raw: unknown): NativeError {
  if (raw instanceof NativeError) return raw;
  if (isErrorPayload(raw)) return new NativeError(raw.code, raw.message);
  if (typeof raw === "string") return new NativeError("transport", raw);
  if (raw instanceof Error) return new NativeError("transport", raw.message);
  return new NativeError("transport", "Unknown native error");
}

export async function callNative(
  command: NativeCommand,
  args?: Record<string, unknown>,
): Promise<unknown> {
  try {
    return await invoke<unknown>(command, args);
  } catch (raw) {
    throw toNativeError(raw);
  }
}

/**
 * URL for cached artwork served by the native `artwork:` protocol. Only images recorded
 * in the database are served; `version` changes whenever the image does.
 */
export function artworkSrc(ownerType: string, ownerId: string, version: string): string {
  return `${convertFileSrc(`${ownerType}/${ownerId}`, "artwork")}?v=${encodeURIComponent(version)}`;
}
