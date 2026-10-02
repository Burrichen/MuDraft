import { useEffect, useState } from "react";
import { checkHealth, type HealthReport } from "../services/health";
import { showErrorDialog } from "../transport/dialogs";

type State =
  | { kind: "checking" }
  | { kind: "ok"; report: HealthReport }
  | { kind: "error"; code: string; message: string };

export function HealthStatus({ compact = false }: { compact?: boolean }) {
  const [state, setState] = useState<State>({ kind: "checking" });

  useEffect(() => {
    let cancelled = false;
    checkHealth().then(
      (report) => {
        if (!cancelled) setState({ kind: "ok", report });
      },
      (err: unknown) => {
        if (cancelled) return;
        const code = typeof err === "object" && err && "code" in err ? String(err.code) : "unknown";
        const message = err instanceof Error ? err.message : String(err);
        setState({ kind: "error", code, message });
        void showErrorDialog("MuDraft storage problem", `${message} (${code})`).catch(() => {
          // Dialog failure must not hide the inline status.
        });
      },
    );
    return () => {
      cancelled = true;
    };
  }, []);

  const label =
    state.kind === "checking"
      ? "Checking storage…"
      : state.kind === "ok"
        ? `Storage OK · schema v${String(state.report.schemaVersion)}${state.report.profile === "development" ? " · dev" : ""}`
        : `Storage error: ${state.code}`;

  return (
    <div
      className="health"
      data-state={state.kind}
      role="status"
      title={state.kind === "ok" ? state.report.dataDir : undefined}
    >
      <span className="health-dot" aria-hidden="true" />
      <span className={compact ? "visually-hidden" : "health-detail"}>{label}</span>
    </div>
  );
}
