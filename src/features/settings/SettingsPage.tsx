import { useEffect, useId, useState } from "react";
import { usePreferences } from "../../app/preferencesContext";
import { LayoutToggle } from "../../components/Filters";
import { PageHeader } from "../../components/PageHeader";
import { ErrorState, LoadingState } from "../../components/States";
import { checkHealth, type HealthReport } from "../../services/health";
import { ArtworkSettings } from "../artwork/ArtworkSettings";
import { ListeningSettings } from "../listening/ListeningSettings";
import { TagManager } from "../tags/TagManager";

type Storage =
  | { kind: "loading" }
  | { kind: "ok"; report: HealthReport }
  | { kind: "error"; code: string; message: string };

function StorageFacts() {
  const [state, setState] = useState<Storage>({ kind: "loading" });
  useEffect(() => {
    let cancelled = false;
    checkHealth().then(
      (report) => {
        if (!cancelled) setState({ kind: "ok", report });
      },
      (err: unknown) => {
        if (cancelled) return;
        const code = typeof err === "object" && err && "code" in err ? String(err.code) : "unknown";
        setState({
          kind: "error",
          code,
          message: err instanceof Error ? err.message : String(err),
        });
      },
    );
    return () => {
      cancelled = true;
    };
  }, []);

  if (state.kind === "loading") return <LoadingState label="Checking storage…" />;
  if (state.kind === "error") {
    return <ErrorState title="Storage problem" message={state.message} code={state.code} />;
  }
  const r = state.report;
  return (
    <dl className="facts">
      <dt>Data folder</dt>
      <dd>{r.dataDir}</dd>
      <dt>Profile</dt>
      <dd>
        {r.profile === "development"
          ? "Development (separate from your real library)"
          : "Production"}
      </dd>
      <dt>Schema</dt>
      <dd>v{r.schemaVersion}</dd>
      <dt>Version</dt>
      <dd>
        MuDraft {r.appVersion} · SQLite {r.sqliteVersion}
      </dd>
    </dl>
  );
}

export function SettingsPage() {
  const { prefs, update } = usePreferences();
  const sidebarId = useId();
  const sidebarHint = useId();
  return (
    <section>
      <PageHeader title="Settings" />
      <div className="settings-list">
        <section className="panel section" aria-labelledby="settings-interface">
          <h2 id="settings-interface" className="section-title">
            Interface
          </h2>
          <div className="setting-row">
            <div className="setting-text">
              <label className="setting-label" htmlFor={sidebarId}>
                Collapse sidebar
              </label>
              <span id={sidebarHint} className="setting-hint">
                Shows icons only. Narrow windows and high zoom always use the compact sidebar.
              </span>
            </div>
            <input
              id={sidebarId}
              type="checkbox"
              role="switch"
              className="switch"
              aria-describedby={sidebarHint}
              checked={prefs.sidebarCollapsed}
              onChange={(e) => {
                update({ sidebarCollapsed: e.target.checked });
              }}
            />
          </div>
          <div className="setting-row">
            <div className="setting-text">
              <span className="setting-label">Album layout</span>
              <span className="setting-hint">Used by Listen List and Collection.</span>
            </div>
            <LayoutToggle
              value={prefs.albumLayout}
              onChange={(albumLayout) => {
                update({ albumLayout });
              }}
            />
          </div>
        </section>
        <ListeningSettings />
        <TagManager />
        <ArtworkSettings />
        <section className="panel section" aria-labelledby="settings-storage">
          <h2 id="settings-storage" className="section-title">
            Storage
          </h2>
          <StorageFacts />
        </section>
      </div>
    </section>
  );
}
