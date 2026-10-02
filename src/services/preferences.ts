import { callNative, NativeError } from "../transport/native";

export type AlbumLayout = "grid" | "list";

/** Mirrors `UiPreferences` in `src-tauri/src/library/preferences.rs`. */
export type StartPage = "last" | "/listen-list" | "/next-up" | "/collection" | "/stats";
export const START_PAGES: readonly StartPage[] = [
  "last",
  "/listen-list",
  "/next-up",
  "/collection",
  "/stats",
];

export interface UiPreferences {
  sidebarCollapsed: boolean;
  lastRoute: string;
  albumLayout: AlbumLayout;
  /** Where the app opens; "last" reopens the page you were on. */
  startPage: StartPage;
  /** Show cached artwork (downloads have their own consent in Settings → Artwork). */
  showArtwork: boolean;
}

export type UiPreferencesPatch = Partial<UiPreferences>;

export const DEFAULT_PREFERENCES: UiPreferences = {
  sidebarCollapsed: false,
  lastRoute: "/listen-list",
  albumLayout: "grid",
  startPage: "last",
  showArtwork: true,
};

/** The page a fresh launch opens. */
export function startRoute(p: UiPreferences): string {
  return p.startPage === "last" ? p.lastRoute : p.startPage;
}

const PRIMARY_ROUTES = new Set(["/listen-list", "/next-up", "/collection", "/stats", "/settings"]);
const NESTED_ROUTE =
  /^\/(albums|artists)\/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** Same rule Rust enforces: the five sections, or an album/artist page by UUID. */
export function isPersistableRoute(path: string): boolean {
  return PRIMARY_ROUTES.has(path) || NESTED_ROUTE.test(path);
}

function isPreferences(value: unknown): value is UiPreferences {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.sidebarCollapsed === "boolean" &&
    typeof v.lastRoute === "string" &&
    isPersistableRoute(v.lastRoute) &&
    (v.albumLayout === "grid" || v.albumLayout === "list") &&
    START_PAGES.includes(v.startPage as StartPage) &&
    typeof v.showArtwork === "boolean"
  );
}

function validated(raw: unknown): UiPreferences {
  if (!isPreferences(raw)) {
    throw new NativeError("invalid_response", "Interface preferences had an unexpected shape");
  }
  return raw;
}

export async function loadPreferences(): Promise<UiPreferences> {
  return validated(await callNative("get_ui_preferences"));
}

export async function updatePreferences(patch: UiPreferencesPatch): Promise<UiPreferences> {
  return validated(await callNative("update_ui_preferences", { patch }));
}
