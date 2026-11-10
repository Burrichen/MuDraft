import type { IconName } from "../components/Icon";
import { STATS_ENABLED } from "./platform";

export interface NavItem {
  path: string;
  label: string;
  icon: IconName;
}

/**
 * The primary sections. Album and Artist are not listed here. Liked is opened from Stats;
 * without Stats (Android) it takes Stats's place so it stays reachable.
 */
export function navItemsFor(statsEnabled: boolean): readonly NavItem[] {
  return [
    { path: "/listen-list", label: "Listen List", icon: "listenList" },
    { path: "/next-up", label: "Next Up", icon: "nextUp" },
    { path: "/collection", label: "Collection", icon: "collection" },
    statsEnabled
      ? { path: "/stats", label: "Stats", icon: "stats" }
      : { path: "/liked", label: "Liked", icon: "liked" },
    { path: "/settings", label: "Settings", icon: "settings" },
  ];
}

export const NAV_ITEMS: readonly NavItem[] = navItemsFor(STATS_ENABLED);

/** Human label for an in-app path, used by back buttons. */
export function labelForPath(path: string): string {
  const item = NAV_ITEMS.find((n) => n.path === path);
  if (item) return item.label;
  if (path === "/liked") return "Liked";
  if (path.startsWith("/albums/")) return "album";
  if (path.startsWith("/artists/")) return "artist";
  return "previous page";
}
