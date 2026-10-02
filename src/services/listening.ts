import { callNative, NativeError } from "../transport/native";
import { notifyLibraryChanged } from "./libraryEvents";

/** Mirrors `src-tauri/src/library/listening.rs`. */
export type ListenKind = "first" | "relisten" | "unspecified";

export interface LogListen {
  editionId: string;
  /** Local calendar date YYYY-MM-DD; null = unknown. */
  listenedOn: string | null;
  kind: ListenKind;
  earlierUndated: boolean;
  /** null = whole album. */
  trackIds: string[] | null;
  attemptId: string | null;
}

export interface UndoLog {
  listenId: string;
  priorHistoryId: string | null;
  restoreListenList: [string, string] | null;
  createdCollectionFor: string | null;
  completedAttempt: string | null;
}

export interface Logged {
  listenId: string;
  isFull: boolean;
  coverage: "tracks" | "unknown";
  coveredTracks: number;
  addedToCollection: boolean;
  removedFromListenList: boolean;
  completedNextUp: boolean;
  undo: UndoLog;
}

export interface ListenView {
  id: string;
  editionId: string;
  editionName: string;
  listenedOn: string | null;
  kind: ListenKind;
  isFull: boolean;
  coverage: "tracks" | "unknown";
  coveredTracks: number;
  editionTracks: number;
  fromNextUp: boolean;
}

export interface ListeningSummary {
  lastListened: string | null;
  listenCount: number;
  undatedListens: number;
  earlierUndated: boolean;
}

export interface ListenFields {
  listenedOn: string | null;
  kind: ListenKind;
}

/** Today's date on the user's own calendar (never converted through UTC). */
export function localToday(now = new Date()): string {
  const m = String(now.getMonth() + 1).padStart(2, "0");
  const d = String(now.getDate()).padStart(2, "0");
  return `${String(now.getFullYear())}-${m}-${d}`;
}

/** "30 Sep 2026" from a calendar date, without any time-zone shift. */
export function formatCalendarDate(iso: string): string {
  const [y, m, d] = iso.split("-").map(Number);
  if (!y || !m || !d) return iso;
  return new Intl.DateTimeFormat(undefined, { dateStyle: "medium" }).format(new Date(y, m - 1, d));
}

/** "Last listened 30 Sep 2026 · 3 listens", "Listened · date unknown", "Not listened yet". */
export function describeListening(s: ListeningSummary): string {
  const count =
    s.listenCount > 0 ? ` · ${String(s.listenCount)} listen${s.listenCount === 1 ? "" : "s"}` : "";
  const earlier = s.earlierUndated ? " · plus earlier, dates unknown" : "";
  if (s.lastListened)
    return `Last listened ${formatCalendarDate(s.lastListened)}${count}${earlier}`;
  if (s.listenCount > 0 || s.earlierUndated) return `Listened · date unknown${count}${earlier}`;
  return "Not listened yet";
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null;
}

/** `requestId` makes repeats (double clicks, retries) record a single listen. */
export async function logListen(requestId: string, listen: LogListen): Promise<Logged> {
  const raw = await callNative("listen_log", { requestId, listen });
  notifyLibraryChanged();
  if (!isRecord(raw) || typeof raw.listenId !== "string" || !isRecord(raw.undo)) {
    throw new NativeError("invalid_response", "Listen result had an unexpected shape");
  }
  return raw as unknown as Logged;
}

export async function undoLog(undo: UndoLog): Promise<void> {
  await callNative("listen_undo_log", { undo });
  notifyLibraryChanged();
}

/** Resolves to the previous values, for undo. */
export async function correctListen(listenId: string, fields: ListenFields): Promise<ListenFields> {
  const raw = await callNative("listen_correct", { listenId, fields });
  notifyLibraryChanged();
  if (!isRecord(raw) || !("kind" in raw))
    throw new NativeError("invalid_response", "Listen had an unexpected shape");
  return raw as unknown as ListenFields;
}

export async function deleteListen(listenId: string): Promise<void> {
  await callNative("listen_delete", { listenId });
  notifyLibraryChanged();
}

export async function restoreListen(listenId: string): Promise<void> {
  await callNative("listen_restore", { listenId });
  notifyLibraryChanged();
}

export async function confirmCoverage(listenId: string): Promise<number> {
  const raw = await callNative("listen_confirm_coverage", { listenId });
  notifyLibraryChanged();
  return typeof raw === "number" ? raw : 0;
}

export async function addToCollection(editionId: string): Promise<void> {
  await callNative("collection_add", { editionId });
  notifyLibraryChanged();
}

export async function listeningSettings(): Promise<{ fullListenRemovesFromListenList: boolean }> {
  const raw = await callNative("listening_settings");
  if (!isRecord(raw) || typeof raw.fullListenRemovesFromListenList !== "boolean") {
    throw new NativeError("invalid_response", "Settings had an unexpected shape");
  }
  return { fullListenRemovesFromListenList: raw.fullListenRemovesFromListenList };
}

export async function setListeningSettings(
  fullListenRemovesFromListenList: boolean,
): Promise<void> {
  await callNative("listening_settings_set", { settings: { fullListenRemovesFromListenList } });
}
