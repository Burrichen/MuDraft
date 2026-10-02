import { callNative, NativeError } from "../transport/native";
import type { ArtworkRef } from "./album";
import { notifyLibraryChanged } from "./libraryEvents";
import type { ListeningSummary } from "./listening";
import type { RatingSummary } from "./ratings";

/** Mirrors `src-tauri/src/library/{listing,tags}.rs`. */
export type LibrarySource = "listen_list" | "collection";
export type SortOrder =
  | "added_newest"
  | "added_oldest"
  | "title"
  | "artist"
  | "year_newest"
  | "year_oldest"
  | "recently_listened"
  | "rating_highest"
  | "rating_lowest";

export interface ListQuery {
  search?: string;
  sort: SortOrder;
  genres: string[];
  tagIds: string[];
}

export interface TagRef {
  id: string;
  name: string;
  color: string;
  builtin: boolean;
}

export interface ListItem {
  albumId: string;
  editionId: string;
  title: string;
  artists: { id: string; name: string }[];
  credit: string;
  originalYear: number | null;
  editionName: string;
  editionCount: number;
  genres: string[];
  tags: TagRef[];
  addedAt: string;
  artwork: ArtworkRef | null;
  rating: RatingSummary;
  listening: ListeningSummary;
}

export interface ListResult {
  items: ListItem[];
  total: number;
  genres: { name: string; count: number }[];
  tags: (TagRef & { count: number })[];
}

export interface TagInfo extends TagRef {
  albumCount: number;
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null;
}

function invalid(what: string): NativeError {
  return new NativeError("invalid_response", `${what} had an unexpected shape`);
}

function tagInfo(raw: unknown): TagInfo {
  if (
    !isRecord(raw) ||
    typeof raw.id !== "string" ||
    typeof raw.name !== "string" ||
    typeof raw.color !== "string"
  ) {
    throw invalid("Tag");
  }
  return raw as unknown as TagInfo;
}

export async function listLibrary(source: LibrarySource, query: ListQuery): Promise<ListResult> {
  const raw = await callNative("library_list", {
    source,
    query: {
      search: query.search ?? null,
      sort: query.sort,
      genres: query.genres,
      tagIds: query.tagIds,
    },
  });
  if (!isRecord(raw) || !Array.isArray(raw.items) || typeof raw.total !== "number")
    throw invalid("Album list");
  return raw as unknown as ListResult;
}

export async function removeFromListenList(albumIds: readonly string[]): Promise<number> {
  const raw = await callNative("listen_list_remove", { albumIds });
  notifyLibraryChanged();
  return typeof raw === "number" ? raw : 0;
}

export async function listTags(): Promise<TagInfo[]> {
  const raw = await callNative("tags_list");
  if (!Array.isArray(raw)) throw invalid("Tag list");
  return raw.map(tagInfo);
}

export async function createTag(name: string, color?: string): Promise<TagInfo> {
  const tag = tagInfo(await callNative("tag_create", { name, color: color ?? null }));
  notifyLibraryChanged();
  return tag;
}

export async function updateTag(
  tagId: string,
  change: { name?: string; color?: string },
): Promise<TagInfo> {
  const tag = tagInfo(
    await callNative("tag_update", {
      tagId,
      name: change.name ?? null,
      color: change.color ?? null,
    }),
  );
  notifyLibraryChanged();
  return tag;
}

/** Resolves to the number of albums that lost the tag. */
export async function deleteTag(tagId: string): Promise<number> {
  const raw = await callNative("tag_delete", { tagId });
  notifyLibraryChanged();
  return typeof raw === "number" ? raw : 0;
}

export async function updateAlbumTags(
  albumIds: readonly string[],
  change: { add?: readonly string[]; remove?: readonly string[] },
): Promise<{ added: number; removed: number }> {
  const raw = await callNative("album_tags_update", {
    albumIds,
    add: change.add ?? [],
    remove: change.remove ?? [],
  });
  notifyLibraryChanged();
  if (!isRecord(raw) || typeof raw.added !== "number") throw invalid("Tag update");
  return raw as unknown as { added: number; removed: number };
}
