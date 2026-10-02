import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { createHashRouter, RouterProvider } from "react-router";
import { PreferencesProvider } from "./app/PreferencesProvider";
import { routes } from "./app/routes";
import { installTabNavigation } from "./app/tabNavigation";
import { DEFAULT_PREFERENCES, loadPreferences, type UiPreferences } from "./services/preferences";
import "./theme/tokens.css";
import "./components/components.css";

async function bootstrap() {
  const root = document.getElementById("root");
  if (!root) throw new Error("Missing #root element");

  let initial: UiPreferences = DEFAULT_PREFERENCES;
  let loadError: string | null = null;
  try {
    initial = await loadPreferences();
  } catch (err) {
    // Keep working with defaults, but say so; nothing is written over stored settings.
    loadError = err instanceof Error ? err.message : String(err);
  }

  // A fresh launch has no hash: reopen the page the user was last on.
  if (window.location.hash === "" || window.location.hash === "#/") {
    window.history.replaceState(null, "", `#${initial.lastRoute}`);
  }

  installTabNavigation();

  // Hash routing: the packaged app serves a single index.html, so deep links must not hit the asset resolver.
  const router = createHashRouter(routes);
  createRoot(root).render(
    <StrictMode>
      <PreferencesProvider initial={initial} loadError={loadError}>
        <RouterProvider router={router} />
      </PreferencesProvider>
    </StrictMode>,
  );
}

void bootstrap();
