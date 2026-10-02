import { useState } from "react";

/** Success message with an immediate Undo. */
export function UndoBanner({
  message,
  onUndo,
  onDismiss,
}: {
  message: string;
  onUndo: () => Promise<void>;
  onDismiss: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <div className="banner banner-ok undo-banner" role="status">
      <span className="banner-text">{error ?? message}</span>
      <button
        type="button"
        className="button button-secondary button-small"
        disabled={busy}
        onClick={() => {
          setBusy(true);
          onUndo().then(onDismiss, (err: unknown) => {
            setError(`Couldn’t undo: ${err instanceof Error ? err.message : String(err)}`);
            setBusy(false);
          });
        }}
      >
        Undo
      </button>
      <button type="button" className="icon-button" aria-label="Dismiss" onClick={onDismiss}>
        ×
      </button>
    </div>
  );
}
