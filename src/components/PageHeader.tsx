import type { ReactNode } from "react";
import { Icon } from "./Icon";

export function PageHeader({
  title,
  description,
  actions,
  back,
}: {
  title: string;
  description?: string;
  actions?: ReactNode;
  back?: { label: string; onBack: () => void };
}) {
  return (
    <header className="page-header">
      {back && (
        <button type="button" className="back-button" onClick={back.onBack}>
          <Icon name="back" size={18} />
          {back.label}
        </button>
      )}
      <div className="page-header-row">
        <div className="page-header-text">
          <h1 className="page-title" tabIndex={-1}>
            {title}
          </h1>
          {description && <p className="page-description">{description}</p>}
        </div>
        {actions && <div className="page-actions">{actions}</div>}
      </div>
    </header>
  );
}
