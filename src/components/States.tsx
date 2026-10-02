import type { ReactNode } from "react";
import { Icon, type IconName } from "./Icon";

export function EmptyState({
  title,
  description,
  icon = "music",
  action,
}: {
  title: string;
  description?: string;
  icon?: IconName;
  action?: ReactNode;
}) {
  return (
    <div className="state state-empty">
      <span className="state-icon">
        <Icon name={icon} size={28} />
      </span>
      <h2 className="state-title">{title}</h2>
      {description && <p className="state-description">{description}</p>}
      {action && <div className="state-action">{action}</div>}
    </div>
  );
}

export function LoadingState({ label = "Loading…" }: { label?: string }) {
  return (
    <div className="state state-loading" role="status">
      <span className="spinner" aria-hidden="true" />
      <p className="state-description">{label}</p>
    </div>
  );
}

export function ErrorState({
  title,
  message,
  code,
  onRetry,
}: {
  title: string;
  message: string;
  code?: string;
  onRetry?: () => void;
}) {
  return (
    <div className="state state-error" role="alert">
      <span className="state-icon">
        <Icon name="alert" size={28} />
      </span>
      <h2 className="state-title">{title}</h2>
      <p className="state-description">{message}</p>
      {code && <p className="state-code">Error code: {code}</p>}
      {onRetry && (
        <div className="state-action">
          <button type="button" className="button button-secondary" onClick={onRetry}>
            Try again
          </button>
        </div>
      )}
    </div>
  );
}
