import { IS_ANDROID } from "../app/platform";
import { callNative, NativeError } from "./native";
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
  /**
   * Android: `install` only opens the APK download in the browser (resolving once it has);
   * the user opens the finished download to install it over this version.
   */
  manual?: boolean;
}

interface NativeUpdate {
  version: string;
  currentVersion: string;
  notes: string;
}

function isNativeUpdate(v: unknown): v is NativeUpdate {
  if (typeof v !== "object" || v === null) return false;
  const u = v as Record<string, unknown>;
  return (
    typeof u.version === "string" &&
    typeof u.currentVersion === "string" &&
    typeof u.notes === "string"
  );
}

/** Android has no in-place updater: Rust compares versions and opens the APK download. */
async function checkAndroidUpdate(): Promise<AvailableUpdate | null> {
  const raw = await callNative("app_update_check");
  if (raw === null) return null;
  if (!isNativeUpdate(raw)) {
    throw new NativeError("invalid_response", "Unexpected response from app_update_check");
  }
  return {
    version: raw.version,
    currentVersion: raw.currentVersion,
    notes: raw.notes,
    manual: true,
    install: async () => {
      await callNative("app_update_open", { version: raw.version });
    },
  };
}

const CHECK_TIMEOUT_MS = 15_000;

/**
 * Ask the latest GitHub release whether a newer version exists. Resolves to null when
 * this is the newest version. The download and its signature check happen in Rust.
 */
export async function checkForUpdate(): Promise<AvailableUpdate | null> {
  if (IS_ANDROID) return checkAndroidUpdate();
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
