import { relaunch } from "@tauri-apps/plugin-process";
import { check } from "@tauri-apps/plugin-updater";

/** A newer, signature-checked MuDraft release, ready to install. */
export interface AvailableUpdate {
  version: string;
  currentVersion: string;
  /** Short patch notes from the release; may be empty. */
  notes: string;
  /**
   * Download, verify, install, and restart into the new version. `onProgress` gets the
   * downloaded fraction (0–1), or null while the size is unknown. On Windows the installer
   * closes MuDraft itself; elsewhere MuDraft relaunches once installed.
   */
  install: (onProgress: (fraction: number | null) => void) => Promise<void>;
}

const CHECK_TIMEOUT_MS = 15_000;

/**
 * Ask the latest GitHub release whether a newer version exists. Resolves to null when
 * this is the newest version. The download and its signature check happen in Rust.
 */
export async function checkForUpdate(): Promise<AvailableUpdate | null> {
  const update = await check({ timeout: CHECK_TIMEOUT_MS });
  if (!update) return null;
  return {
    version: update.version,
    currentVersion: update.currentVersion,
    notes: update.body?.trim() ?? "",
    install: async (onProgress) => {
      let total: number | null = null;
      let done = 0;
      onProgress(null);
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") {
          total = event.data.contentLength ?? null;
        } else if (event.event === "Progress") {
          done += event.data.chunkLength;
          onProgress(total ? Math.min(done / total, 1) : null);
        } else {
          onProgress(1);
        }
      });
      await relaunch();
    },
  };
}
