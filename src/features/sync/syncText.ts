import type {
  ChangeSummary,
  RemoteCheck,
  SyncConflict,
  SyncCounts,
  SyncProgress,
  SyncStatus,
} from "../../services/sync";

export const plural = (n: number, one: string, many = `${one}s`) =>
  `${String(n)} ${n === 1 ? one : many}`;

export type Tone = "idle" | "ok" | "pending" | "busy" | "warning" | "error";

export interface StatusView {
  label:
    | "Not Connected"
    | "Up to Date"
    | "Changes Pending"
    | "Syncing"
    | "Conflicts"
    | "Offline"
    | "Error";
  tone: Tone;
  detail: string;
}

export function summaryText(s: ChangeSummary | null | undefined): string {
  if (!s) return "";
  const parts = [
    s.albums && plural(s.albums, "album"),
    s.listens && plural(s.listens, "listen"),
    s.ratings && plural(s.ratings, "rating"),
    s.reviews && plural(s.reviews, "review"),
    s.tags && plural(s.tags, "tag change"),
    s.artwork && plural(s.artwork, "artwork change"),
    s.other && plural(s.other, "other change"),
  ].filter(Boolean);
  return parts.join(", ");
}

export function isEmptySummary(s: ChangeSummary | null | undefined): boolean {
  return !s || summaryText(s) === "";
}

export function countsText(c: SyncCounts): string {
  return [
    plural(c.albums, "album"),
    plural(c.listens, "listen"),
    `${String(c.collection)} in the Collection`,
    plural(c.ratedTracks, "rated track"),
    plural(c.reviews, "review"),
    plural(c.tags, "tag"),
  ].join(", ");
}

export function isEmptyLibrary(c: SyncCounts): boolean {
  return c.artists + c.albums + c.listens + c.tags + c.collection === 0;
}

export function formatWhen(iso: string | null): string {
  if (!iso) return "Never";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" }).format(d);
}

const OFFLINE = new Set(["sync_offline", "timeout"]);

/** The headline status. Success is only ever shown from stored, completed syncs. */
export function deriveStatus(
  status: SyncStatus | null,
  busy: boolean,
  check: RemoteCheck | null,
): StatusView {
  if (busy) return { label: "Syncing", tone: "busy", detail: "Working with GitHub…" };
  if (!status || !status.configured)
    return { label: "Not Connected", tone: "idle", detail: "Choose a repository to sync with." };
  if (!status.hasToken)
    return {
      label: "Not Connected",
      tone: "idle",
      detail: "Add a GitHub token for this computer.",
    };
  if (status.unresolvedConflicts > 0)
    return {
      label: "Conflicts",
      tone: "warning",
      detail: `${plural(status.unresolvedConflicts, "conflict")} to resolve. Nothing has changed yet.`,
    };
  if (status.lastError)
    return OFFLINE.has(status.lastError.code)
      ? {
          label: "Offline",
          tone: "error",
          detail: "GitHub couldn’t be reached. Your changes are safe on this computer.",
        }
      : { label: "Error", tone: "error", detail: "The last sync didn’t finish." };
  if (!status.linked)
    return {
      label: "Not Connected",
      tone: "idle",
      detail: "Run the first sync to connect this computer.",
    };
  const remote = check?.sameSync && check.revision !== status.revision ? check.remoteChanges : null;
  if (status.pendingLocalChanges || !isEmptySummary(remote))
    return {
      label: "Changes Pending",
      tone: "pending",
      detail: "Sync Now to bring both sides together.",
    };
  return { label: "Up to Date", tone: "ok", detail: "Last sync finished on both sides." };
}

export function progressText(p: SyncProgress | null): string {
  switch (p?.phase) {
    case "reading":
      return "Reading GitHub…";
    case "downloading":
      return "Downloading changes…";
    case "merging":
      return "Combining changes…";
    case "backingUp":
      return "Backing up this computer’s library…";
    case "uploading":
      return "Uploading changes…";
    case "publishing":
      return "Saving to GitHub…";
    case "applying":
      return "Applying changes on this computer…";
    default:
      return "Starting…";
  }
}

export interface Recovery {
  title: string;
  advice: string;
  action: "retry" | "token" | "repository" | "disconnect" | null;
}

/** What went wrong and what to do about it. Never suggests anything that loses data. */
export function recovery(code: string, message: string): Recovery {
  const m = message.toLowerCase();
  switch (code) {
    case "auth_required":
      return {
        title: "GitHub didn’t accept a token",
        advice:
          "It may have expired or been revoked, or none is saved on this computer. Create a new fine-grained token for the repository with Contents set to “Read and write”, and save it here. Your library is unchanged.",
        action: "token",
      };
    case "sync_setup":
      if (m.includes("public"))
        return {
          title: "The repository is public",
          advice:
            "MuDraft only syncs to a private repository. On GitHub, open the repository’s Settings, change its visibility to Private, then try again.",
          action: "retry",
        };
      if (m.includes("newer"))
        return {
          title: "Made by a newer MuDraft",
          advice:
            "The sync data was written by a newer version of MuDraft. Update MuDraft on this computer, then sync again. Nothing was changed.",
          action: null,
        };
      if (m.includes("wasn't found") || m.includes("refused access"))
        return {
          title: "Repository not available",
          advice:
            "Check the owner and repository name, that the repository exists and is private, and that this computer’s token was given access to it with Contents “Read and write”.",
          action: "repository",
        };
      if (m.includes("different sync data"))
        return {
          title: "The repository was reset",
          advice:
            "It now holds different sync data. Disconnect, connect again, and choose how to combine the libraries. Your library here is unchanged.",
          action: "disconnect",
        };
      if (m.includes("other files"))
        return {
          title: "Not a MuDraft repository",
          advice:
            "The repository holds other files, so MuDraft won’t write to it. Create a new private repository just for MuDraft sync.",
          action: "repository",
        };
      if (m.includes("damaged"))
        return {
          title: "Sync data on GitHub failed its checks",
          advice:
            "Nothing was changed here. If files in the repository were edited by hand, restore them on GitHub (its history keeps every version).",
          action: null,
        };
      return { title: "The repository isn’t ready", advice: message, action: "repository" };
    case "sync_offline":
    case "timeout":
      return {
        title: "Can’t reach GitHub",
        advice:
          "Check your internet connection and try again. Your changes are safe on this computer and will sync next time.",
        action: "retry",
      };
    case "rate_limited":
      return {
        title: "GitHub is limiting requests",
        advice: "Wait a few minutes, then try again. Nothing was changed.",
        action: "retry",
      };
    case "disk_full":
      return {
        title: "Not enough free disk space",
        advice: `${message} Free some space on this computer, then try again.`,
        action: "retry",
      };
    case "backup_failed":
      return {
        title: "Couldn’t back up first",
        advice:
          "MuDraft always backs up your library before applying changes and couldn’t this time, so nothing was changed. Check that the data folder is writable and has space.",
        action: "retry",
      };
    case "storage_unavailable":
    case "storage_corrupt":
      return { title: "Couldn’t apply the changes here", advice: message, action: "retry" };
    case "conflict":
      return { title: "Something changed during sync", advice: message, action: "retry" };
    default:
      return { title: "Sync didn’t finish", advice: message, action: "retry" };
  }
}

// ---------------------------------------------------------------- conflicts

const FIELD_LABELS: Record<string, string> = {
  title: "Title",
  name: "Name",
  sort_name: "Sort name",
  disambiguation: "Disambiguation",
  description: "Description",
  original_date: "Original release date",
  release_date: "Release date",
  color: "Colour",
  rating: "Rating",
  review: "Review",
  is_favourite: "Liked",
  listened_on: "Listen date",
  kind: "Listen type",
  is_full: "Full listen",
  coverage: "Tracks covered",
  deleted_at: "Deleted",
  edition_id: "Edition",
  attempt_id: "Current album",
  value: "Value",
  source: "Source",
  status: "Status",
  length_ms: "Length",
  position: "Position",
  disc_number: "Disc",
  excluded: "Excluded",
  ended_at: "Session ended",
};

const KIND_LABELS: Record<Exclude<SyncConflict["kind"], "field">, string> = {
  deleteEdit: "Deleted on one side, changed on the other",
  collection: "Changed differently on each side",
  orphan: "Deleted on one side but still used on the other",
  duplicate: "Created on both sides",
  deletedElsewhere: "Deleted on another computer",
};

export function conflictField(c: SyncConflict): string {
  if (c.kind !== "field") return KIND_LABELS[c.kind];
  const first = c.columns.find((col) => !col.endsWith("_precision")) ?? c.columns[0] ?? "";
  if (c.table === "metadata_provenance") return "Metadata source";
  if (c.table === "artwork") return "Artwork";
  if (first.startsWith("musicbrainz_")) return "MusicBrainz ID";
  return FIELD_LABELS[first] ?? first.replace(/_/g, " ");
}

function stars(v: unknown): string {
  if (v === null || v === undefined) return "Unrated";
  if (typeof v !== "number") return JSON.stringify(v);
  const n = v / 2;
  return n === 0 ? "0 stars (rated zero)" : `${String(n)} ${n === 1 ? "star" : "stars"}`;
}

function scalar(col: string, v: unknown): string {
  if (col === "rating") return stars(v);
  if (col === "is_favourite") return v ? "Liked" : "Not liked";
  if (col === "deleted_at") return v ? "Deleted" : "Not deleted";
  if (col === "is_full") return v ? "Full listen" : "Partial listen";
  if (col === "excluded") return v ? "Excluded" : "Counted";
  if (col === "length_ms" && typeof v === "number") {
    const s = Math.round(v / 1000);
    return `${String(Math.floor(s / 60))}:${String(s % 60).padStart(2, "0")}`;
  }
  if (v === null || v === undefined || v === "") {
    if (col.endsWith("date") || col === "listened_on") return "Unknown";
    return "Empty";
  }
  if (typeof v === "string") return v.length > 280 ? `${v.slice(0, 280)}…` : v;
  if (typeof v === "number" || typeof v === "boolean") return String(v);
  return JSON.stringify(v);
}

/** One side of a conflict, in words. */
export function conflictValue(c: SyncConflict, side: "local" | "remote"): string {
  const value = c[side];
  switch (c.kind) {
    case "field": {
      if (typeof value !== "object" || value === null) return scalar("", value);
      const obj = value as Record<string, unknown>;
      if (c.table === "artwork")
        return obj.source === "removed" ? "Artwork removed" : "Custom image";
      const col = Object.keys(obj).find((k) => !k.endsWith("_precision")) ?? "";
      return scalar(col, obj[col]);
    }
    case "collection":
      return Array.isArray(value)
        ? value.length === 0
          ? "None"
          : plural(value.length, "entry", "entries")
        : "None";
    case "duplicate": {
      const rows = typeof value === "object" && value !== null ? Object.values(value) : [];
      const kept = rows.filter((r) => r !== null).length;
      return kept === 0 ? "Nothing" : plural(kept, "record");
    }
    default:
      return value === null || value === undefined ? "Deleted" : "Kept, with its changes";
  }
}
