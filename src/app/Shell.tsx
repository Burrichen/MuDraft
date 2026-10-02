import { useEffect, useRef } from "react";
import { NavLink, Outlet, useLocation } from "react-router";
import { Icon } from "../components/Icon";
import { ArtworkConsent } from "../features/artwork/ArtworkConsent";
import { Tooltip } from "../components/Tooltip";
import { isPersistableRoute } from "../services/preferences";
import { HealthStatus } from "./HealthStatus";
import { NAV_ITEMS } from "./navigation";
import { usePreferences } from "./preferencesContext";
import { useMediaQuery } from "./useMediaQuery";
import "./shell.css";

/** Below this width (including high zoom) the sidebar is always the compact rail. */
export const NARROW_QUERY = "(max-width: 45rem)";

export function Shell() {
  const { prefs, update, problem, dismissProblem } = usePreferences();
  const { pathname } = useLocation();
  const narrow = useMediaQuery(NARROW_QUERY);
  const compact = narrow || prefs.sidebarCollapsed;
  const mainRef = useRef<HTMLElement>(null);
  const firstRender = useRef(true);

  // Remember where the user is so a restart reopens the same page. Each path is requested
  // once per visit, so a failed save (which reverts lastRoute) cannot retry in a loop.
  const requestedRoute = useRef<string | null>(null);
  useEffect(() => {
    if (
      isPersistableRoute(pathname) &&
      pathname !== prefs.lastRoute &&
      pathname !== requestedRoute.current
    ) {
      requestedRoute.current = pathname;
      update({ lastRoute: pathname });
    }
  }, [pathname, prefs.lastRoute, update]);

  // After in-app navigation, move focus to the new page's heading for keyboard and
  // screen-reader users. The initial load keeps the browser's default focus.
  useEffect(() => {
    if (firstRender.current) {
      firstRender.current = false;
      return;
    }
    const heading = mainRef.current?.querySelector<HTMLElement>("h1");
    heading?.focus({ preventScroll: false });
  }, [pathname]);

  return (
    <div className="shell" data-compact={compact}>
      {/* Moves focus directly: with hash routing, following "#main-content" would navigate. */}
      <a
        className="skip-link"
        href="#main-content"
        onClick={(e) => {
          e.preventDefault();
          mainRef.current?.querySelector<HTMLElement>("h1")?.focus();
        }}
      >
        Skip to content
      </a>
      <aside className="sidebar" aria-label="Sidebar">
        <div className="sidebar-header">
          <span className="brand" aria-hidden={compact}>
            <span className="brand-mark">
              <Icon name="music" size={18} />
            </span>
            <span className="brand-name">MuDraft</span>
          </span>
          {!narrow && (
            <Tooltip content={compact ? "Expand sidebar" : "Collapse sidebar"} placement="right">
              <button
                type="button"
                className="icon-button sidebar-toggle"
                aria-label={compact ? "Expand sidebar" : "Collapse sidebar"}
                aria-expanded={!compact}
                aria-controls="primary-nav"
                onClick={() => {
                  update({ sidebarCollapsed: !prefs.sidebarCollapsed });
                }}
              >
                <Icon name={compact ? "expand" : "collapse"} size={18} />
              </button>
            </Tooltip>
          )}
        </div>
        <nav id="primary-nav" aria-label="Main">
          <ul className="nav-list">
            {NAV_ITEMS.map((item) => (
              <li key={item.path}>
                <Tooltip content={item.label} placement="right" enabled={compact}>
                  <NavLink to={item.path} className="nav-link">
                    <Icon name={item.icon} />
                    <span className={compact ? "visually-hidden" : "nav-label"}>{item.label}</span>
                  </NavLink>
                </Tooltip>
              </li>
            ))}
          </ul>
        </nav>
        <HealthStatus compact={compact} />
      </aside>
      <main id="main-content" ref={mainRef} className="content" tabIndex={-1}>
        {problem && (
          <div className="banner" role="alert">
            <Icon name="alert" size={18} />
            <p className="banner-text">{problem}</p>
            <button
              type="button"
              className="icon-button"
              aria-label="Dismiss message"
              onClick={dismissProblem}
            >
              <Icon name="close" size={16} />
            </button>
          </div>
        )}
        <div className="content-inner">
          <Outlet />
        </div>
        <ArtworkConsent />
      </main>
    </div>
  );
}
