import type { RemoteCheck, SyncConflict, SyncStatus } from "../../services/sync";
import { conflictField, conflictValue, deriveStatus, recovery, summaryText } from "./syncText";

const BASE: SyncStatus = {
  configured: true,
  owner: "me",
  repo: "MuDraft-Sync",
  branch: "main",
  hasToken: true,
  linked: true,
  revision: "r1",
  lastSyncedAt: "2026-10-20T10:00:00Z",
  pendingLocalChanges: false,
  pending: null,
  unresolvedConflicts: 0,
  lastError: null,
};

const EMPTY = { albums: 0, listens: 0, ratings: 0, reviews: 0, tags: 0, artwork: 0, other: 0 };

function conflict(overrides: Partial<SyncConflict>): SyncConflict {
  return {
    id: "c",
    kind: "field",
    record: "Rating of “Airbag”",
    table: "track_rating",
    key: ["t"],
    columns: ["rating"],
    base: { rating: 8 },
    local: { rating: 0 },
    remote: { rating: null },
    ...overrides,
  };
}

describe("sync status", () => {
  it("picks the headline in priority order", () => {
    expect(deriveStatus(null, false, null).label).toBe("Not Connected");
    expect(deriveStatus({ ...BASE, hasToken: false }, false, null).label).toBe("Not Connected");
    expect(deriveStatus({ ...BASE, linked: false, revision: null }, false, null).label).toBe(
      "Not Connected",
    );
    expect(deriveStatus(BASE, true, null).label).toBe("Syncing");
    expect(deriveStatus({ ...BASE, unresolvedConflicts: 2 }, false, null)).toMatchObject({
      label: "Conflicts",
      detail: expect.stringContaining("2 conflicts") as string,
    });
    const offline = { code: "sync_offline", message: "x" };
    expect(deriveStatus({ ...BASE, lastError: offline }, false, null).label).toBe("Offline");
    const auth = { code: "auth_required", message: "x" };
    expect(deriveStatus({ ...BASE, lastError: auth }, false, null).label).toBe("Error");
    expect(deriveStatus({ ...BASE, pendingLocalChanges: true }, false, null).label).toBe(
      "Changes Pending",
    );
    expect(deriveStatus(BASE, false, null).label).toBe("Up to Date");
  });

  it("counts remote changes only when the check is newer than the last sync", () => {
    const check: RemoteCheck = {
      branch: "main",
      kind: "ready",
      sameSync: true,
      remote: null,
      local: {
        artists: 0,
        albums: 0,
        editions: 0,
        listens: 0,
        ratedTracks: 0,
        reviews: 0,
        tags: 0,
        listenList: 0,
        collection: 0,
        artwork: 0,
      },
      remoteChanges: { ...EMPTY, listens: 1 },
      revision: "r2",
    };
    expect(deriveStatus(BASE, false, check).label).toBe("Changes Pending");
    expect(deriveStatus({ ...BASE, revision: "r2" }, false, check).label).toBe("Up to Date");
  });

  it("summarises changes in words", () => {
    expect(summaryText({ ...EMPTY, albums: 1, listens: 2, ratings: 1, artwork: 3 })).toBe(
      "1 album, 2 listens, 1 rating, 3 artwork changes",
    );
    expect(summaryText(EMPTY)).toBe("");
  });
});

describe("sync conflicts", () => {
  it("keeps zero and unrated distinct", () => {
    const c = conflict({});
    expect(conflictField(c)).toBe("Rating");
    expect(conflictValue(c, "local")).toBe("0 stars (rated zero)");
    expect(conflictValue(c, "remote")).toBe("Unrated");
    expect(conflictValue(conflict({ local: { rating: 7 } }), "local")).toBe("3.5 stars");
  });

  it("describes dates, deletions, lists, and duplicates", () => {
    const date = conflict({
      table: "album",
      columns: ["original_date", "original_date_precision"],
      local: { original_date: "1997", original_date_precision: "year" },
      remote: { original_date: null, original_date_precision: "unknown" },
    });
    expect(conflictField(date)).toBe("Original release date");
    expect(conflictValue(date, "local")).toBe("1997");
    expect(conflictValue(date, "remote")).toBe("Unknown");
    const gone = conflict({ kind: "deleteEdit", local: null, remote: { name: "X" } });
    expect(conflictField(gone)).toMatch(/Deleted on one side/);
    expect(conflictValue(gone, "local")).toBe("Deleted");
    expect(conflictValue(gone, "remote")).toBe("Kept, with its changes");
    const list = conflict({ kind: "collection", local: [{}, {}], remote: [] });
    expect(conflictValue(list, "local")).toBe("2 entries");
    expect(conflictValue(list, "remote")).toBe("None");
  });
});

describe("sync recovery", () => {
  it("offers a safe next step for each failure", () => {
    expect(recovery("auth_required", "").action).toBe("token");
    expect(recovery("sync_offline", "").action).toBe("retry");
    expect(recovery("rate_limited", "").title).toMatch(/limiting/);
    expect(recovery("sync_setup", "the repository is public. …").title).toMatch(/public/);
    expect(recovery("sync_setup", "comes from a newer MuDraft").action).toBeNull();
    expect(recovery("sync_setup", "the repository wasn't found, or …").action).toBe("repository");
    expect(recovery("sync_setup", "now holds different sync data").action).toBe("disconnect");
    expect(recovery("disk_full", "Needs 80 MB.").advice).toMatch(/^Needs 80 MB\. Free some space/);
    expect(recovery("backup_failed", "").advice).toMatch(/nothing was changed/);
  });
});
