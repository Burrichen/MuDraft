import { callNative, NativeError } from "../transport/native";
import { notifyLibraryChanged } from "./libraryEvents";

/** Mirrors `src-tauri/src/library/lists.rs` and `src-tauri/src/list_import`. */
export type EntryStatus = "listened" | "listen_list" | "in_library" | "missing";

export interface ListProgress {
  total: number;
  /** In the Collection: the only entries that count toward progress. */
  listened: number;
  /** On the Listen List but not yet listened to. Never counted. */
  onListenList: number;
}

export interface ListSummary {
  id: string;
  name: string;
  sourceUrl: string | null;
  progress: ListProgress;
}

export interface ListEntry {
  id: string;
  position: number;
  artistName: string;
  title: string;
  year: number | null;
  status: EntryStatus;
  /** The library album this entry matches, when there is one. */
  albumId: string | null;
}

export interface ListDetail extends ListSummary {
  entries: ListEntry[];
}

export interface NewListEntry {
  artistName: string;
  title: string;
  year?: number | null;
  position?: number | null;
}

export interface ListImport {
  name: string;
  sourceUrl: string;
  entries: (NewListEntry & { year: number | null; position: number })[];
}

const STATUSES: readonly EntryStatus[] = ["listened", "listen_list", "in_library", "missing"];

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null;
}

function invalid(what: string): NativeError {
  return new NativeError("invalid_response", `${what} had an unexpected shape`);
}

function progress(raw: unknown): ListProgress {
  if (
    !isRecord(raw) ||
    typeof raw.total !== "number" ||
    typeof raw.listened !== "number" ||
    typeof raw.onListenList !== "number"
  ) {
    throw invalid("List progress");
  }
  return { total: raw.total, listened: raw.listened, onListenList: raw.onListenList };
}

function summary(raw: unknown): ListSummary {
  if (
    !isRecord(raw) ||
    typeof raw.id !== "string" ||
    typeof raw.name !== "string" ||
    (raw.sourceUrl !== null && typeof raw.sourceUrl !== "string")
  ) {
    throw invalid("List");
  }
  return {
    id: raw.id,
    name: raw.name,
    sourceUrl: raw.sourceUrl,
    progress: progress(raw.progress),
  };
}

function entry(raw: unknown): ListEntry {
  if (
    !isRecord(raw) ||
    typeof raw.id !== "string" ||
    typeof raw.position !== "number" ||
    typeof raw.artistName !== "string" ||
    typeof raw.title !== "string" ||
    (raw.year !== null && typeof raw.year !== "number") ||
    !STATUSES.includes(raw.status as EntryStatus) ||
    (raw.albumId !== null && typeof raw.albumId !== "string")
  ) {
    throw invalid("List entry");
  }
  return raw as unknown as ListEntry;
}

/** Only the fields Rust accepts; anything else would be refused. */
function outgoing(entries: readonly NewListEntry[]) {
  return entries.map((e) => ({
    artistName: e.artistName,
    title: e.title,
    year: e.year ?? null,
    position: e.position ?? null,
  }));
}

export async function listLists(): Promise<ListSummary[]> {
  const raw = await callNative("lists_overview");
  if (!Array.isArray(raw)) throw invalid("Lists");
  return raw.map(summary);
}

export async function getList(listId: string): Promise<ListDetail> {
  const raw = await callNative("list_detail", { listId });
  if (!isRecord(raw) || !Array.isArray(raw.entries)) throw invalid("List");
  return { ...summary(raw), entries: raw.entries.map(entry) };
}

/** Creates the list (with the albums of a reviewed import, if given); resolves to its ID. */
export async function createList(
  name: string,
  sourceUrl: string | null,
  entries: readonly NewListEntry[] = [],
): Promise<string> {
  const raw = await callNative("list_create", {
    name,
    sourceUrl,
    entries: outgoing(entries),
  });
  notifyLibraryChanged();
  if (!isRecord(raw) || typeof raw.id !== "string") throw invalid("New list");
  return raw.id;
}

export async function renameList(listId: string, name: string): Promise<void> {
  await callNative("list_rename", { listId, name });
  notifyLibraryChanged();
}

/** Deletes the list only; albums, the Collection and the Listen List are untouched. */
export async function deleteList(listId: string): Promise<void> {
  await callNative("list_delete", { listId });
  notifyLibraryChanged();
}

export async function addListEntries(
  listId: string,
  entries: readonly NewListEntry[],
): Promise<{ added: number; duplicates: number }> {
  const raw = await callNative("list_entries_add", { listId, entries: outgoing(entries) });
  notifyLibraryChanged();
  if (!isRecord(raw) || typeof raw.added !== "number" || typeof raw.duplicates !== "number") {
    throw invalid("Added albums");
  }
  return { added: raw.added, duplicates: raw.duplicates };
}

export async function removeListEntry(entryId: string): Promise<void> {
  await callNative("list_entry_remove", { entryId });
  notifyLibraryChanged();
}

/** Reads the list on a supported web page. Nothing is saved until the user confirms. */
export async function previewListImport(url: string): Promise<ListImport> {
  const raw = await callNative("list_import_preview", { url });
  if (
    !isRecord(raw) ||
    typeof raw.name !== "string" ||
    typeof raw.sourceUrl !== "string" ||
    !Array.isArray(raw.entries)
  ) {
    throw invalid("List import");
  }
  return raw as unknown as ListImport;
}

export interface ListenListResult {
  /** Albums newly put on the Listen List. */
  added: number;
  /** Albums that aren't in the library yet and need a reviewed MusicBrainz match. */
  needsMatch: number;
}

function listenListResult(raw: unknown): ListenListResult {
  if (!isRecord(raw) || typeof raw.added !== "number" || typeof raw.needsMatch !== "number") {
    throw invalid("Listen List update");
  }
  return { added: raw.added, needsMatch: raw.needsMatch };
}

/** Adds the entry's album if it is already in the library; otherwise reports needsMatch. */
export async function addEntryToListenList(entryId: string): Promise<ListenListResult> {
  const raw = await callNative("list_entry_add_to_listen_list", { entryId });
  notifyLibraryChanged();
  return listenListResult(raw);
}

/**
 * Puts every unlistened album that is already in the library on the Listen List. Albums
 * not in the library are counted in needsMatch, never created without review.
 */
export async function addUnlistenedToListenList(listId: string): Promise<ListenListResult> {
  const raw = await callNative("list_add_unlistened_to_listen_list", { listId });
  notifyLibraryChanged();
  return listenListResult(raw);
}

/** Remember which library album the user chose for an entry. */
export async function linkListEntry(entryId: string, albumId: string): Promise<void> {
  await callNative("list_entry_link", { entryId, albumId });
  notifyLibraryChanged();
}

/**
 * Stage the albums that aren't in the library (a list's, or one entry's) as an import to
 * review against MusicBrainz. The import page picks the staged import up.
 */
export async function reviewMissingOnMusicBrainz(listId: string, entryId?: string): Promise<void> {
  await callNative("list_review_missing", { listId, entryId: entryId ?? null });
}
