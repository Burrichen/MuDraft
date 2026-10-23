import type { SyncStatus } from "../services/sync";

/** Sync as it is before setup, for tests that render the whole Settings page. */
export const UNCONFIGURED_SYNC: SyncStatus = {
  configured: false,
  owner: null,
  repo: null,
  branch: null,
  hasToken: false,
  linked: false,
  revision: null,
  lastSyncedAt: null,
  pendingLocalChanges: null,
  pending: null,
  unresolvedConflicts: 0,
  lastError: null,
};

/** For `vi.mock` of `services/sync` in page tests (import it inside the factory). */
export async function syncServiceMock(importOriginal: <T>() => Promise<T>) {
  return {
    ...(await importOriginal<typeof import("../services/sync")>()),
    getSyncStatus: () => Promise.resolve(UNCONFIGURED_SYNC),
    onSyncProgress: () => Promise.resolve(() => undefined),
  };
}
