import { render } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { PreferencesProvider } from "../app/PreferencesProvider";
import { routes } from "../app/routes";
import { DEFAULT_PREFERENCES, type UiPreferences } from "../services/preferences";

/** Render the real route tree at `path` with the given preferences. */
export function renderApp(
  path: string,
  prefs: Partial<UiPreferences> = {},
  loadError: string | null = null,
) {
  const router = createMemoryRouter(routes, { initialEntries: [path] });
  const utils = render(
    <PreferencesProvider initial={{ ...DEFAULT_PREFERENCES, ...prefs }} loadError={loadError}>
      <RouterProvider router={router} />
    </PreferencesProvider>,
  );
  return { router, ...utils };
}
