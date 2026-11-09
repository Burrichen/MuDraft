import { useEffect, useState } from "react";
import { useUpdates } from "../../app/updateContext";
import { Button } from "../../components/Button";
import { checkHealth } from "../../services/health";

/** Settings → Version: which MuDraft this is, and a manual update check. */
export function VersionSettings() {
  const { status, checkNow, unavailable } = useUpdates();
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    checkHealth().then(
      (r) => {
        if (!cancelled) setVersion(r.appVersion);
      },
      () => {
        // The Storage section reports health problems.
      },
    );
    return () => {
      cancelled = true;
    };
  }, []);

  const result =
    status.kind === "checking"
      ? "Checking for updates…"
      : status.kind === "current"
        ? "You have the latest version."
        : status.kind === "available"
          ? `MuDraft ${status.update.version} is available.`
          : status.kind === "error"
            ? `Couldn’t check for updates: ${status.message}`
            : null;

  return (
    <section className="panel section" aria-labelledby="settings-version">
      <h2 id="settings-version" className="section-title">
        Version
      </h2>
      <div className="setting-row">
        <div className="setting-text">
          <span className="setting-label">MuDraft {version ?? "…"}</span>
          <span className="setting-hint">
            {unavailable ??
              "MuDraft checks for a new version when it starts and asks before updating."}
          </span>
          {result && (
            <span className="setting-hint" role={status.kind === "error" ? "alert" : "status"}>
              {result}
            </span>
          )}
        </div>
        <Button
          disabledReason={
            unavailable ?? (status.kind === "checking" ? "Checking for updates…" : undefined)
          }
          onClick={checkNow}
        >
          Check for Updates
        </Button>
      </div>
    </section>
  );
}
