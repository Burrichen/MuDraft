import type { IconName } from "../components/Icon";

export interface NavItem {
  path: string;
  label: string;
  icon: IconName;
}

/** The primary sections. Album, Artist, and Liked (opened from Stats) are not listed here. */
export const NAV_ITEMS: readonly NavItem[] = [
  { path: "/listen-list", label: "Listen List", icon: "listenList" },
  { path: "/next-up", label: "Next Up", icon: "nextUp" },
  { path: "/collection", label: "Collection", icon: "collection" },
  { path: "/stats", label: "Stats", icon: "stats" },
  { path: "/settings", label: "Settings", icon: "settings" },
];

/** Human label for an in-app path, used by back buttons. */
export function labelForPath(path: string): string {
  const item = NAV_ITEMS.find((n) => n.path === path);
  if (item) return item.label;
  if (path === "/liked") return "Liked";
  if (path.startsWith("/albums/")) return "album";
  if (path.startsWith("/artists/")) return "artist";
  return "previous page";
}
