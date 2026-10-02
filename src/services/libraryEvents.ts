import { useSyncExternalStore } from "react";

/**
 * Library change notifications. Every mutation (tagging, removal, import, tag edits)
 * bumps the version, and every view reading library data re-fetches — so a tag change on
 * the Listen List shows up in Collection, Settings, and open pickers immediately.
 */
let version = 0;
const listeners = new Set<() => void>();

export function notifyLibraryChanged(): void {
  version += 1;
  for (const l of listeners) l();
}

export function useLibraryVersion(): number {
  return useSyncExternalStore(
    (onChange) => {
      listeners.add(onChange);
      return () => {
        listeners.delete(onChange);
      };
    },
    () => version,
  );
}
