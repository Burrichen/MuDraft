import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import type { Side, SyncConflict } from "../../services/sync";
import { conflictField, conflictValue, plural } from "./syncText";

interface Props {
  conflicts: SyncConflict[];
  choices: Record<string, Side>;
  busy: boolean;
  onChoose: (id: string, side: Side) => void;
  onApply: () => void;
  onCancel: () => void;
}

/**
 * One choice per conflict. Nothing changes on either side until every conflict has a
 * choice and the user applies them; cancelling keeps all changes pending.
 */
export function ConflictDialog({ conflicts, choices, busy, onChoose, onApply, onCancel }: Props) {
  const left = conflicts.filter((c) => !choices[c.id]).length;
  const chooseAll = (side: Side) => {
    for (const c of conflicts) onChoose(c.id, side);
  };
  return (
    <Dialog
      open={conflicts.length > 0}
      size="wide"
      title={`Resolve ${plural(conflicts.length, "conflict")}`}
      onClose={onCancel}
      actions={
        <>
          <Button onClick={onCancel}>Cancel</Button>
          <Button
            variant="primary"
            disabledReason={
              busy
                ? "Syncing…"
                : left > 0
                  ? `Choose a version for ${plural(left, "conflict")}`
                  : undefined
            }
            onClick={onApply}
          >
            Apply choices and sync
          </Button>
        </>
      }
    >
      <div className="sync-conflicts">
        <p>
          The same records were changed differently on this computer and on GitHub. Choose which
          version to keep for each. Nothing has changed yet: until you apply, your other changes
          stay pending on both sides, and cancelling leaves everything as it is.
        </p>
        <div className="page-actions">
          <Button
            variant="ghost"
            onClick={() => {
              chooseAll("local");
            }}
          >
            Keep all from this computer
          </Button>
          <Button
            variant="ghost"
            onClick={() => {
              chooseAll("remote");
            }}
          >
            Keep all from GitHub
          </Button>
        </div>
        {conflicts.map((c) => (
          <fieldset key={c.id} className="sync-conflict">
            <legend className="sync-conflict-title">
              <span className="sync-conflict-record">{c.record || c.table}</span>
              <span className="sync-conflict-field">{conflictField(c)}</span>
            </legend>
            <div className="sync-choices">
              {(["local", "remote"] as const).map((side) => (
                <label key={side} className="sync-choice" data-selected={choices[c.id] === side}>
                  <input
                    type="radio"
                    name={c.id}
                    value={side}
                    checked={choices[c.id] === side}
                    onChange={() => {
                      onChoose(c.id, side);
                    }}
                  />
                  <span className="sync-choice-text">
                    <span className="sync-choice-label">
                      {side === "local" ? "Keep this computer’s version" : "Keep GitHub’s version"}
                    </span>
                    <span className="sync-choice-value">{conflictValue(c, side)}</span>
                  </span>
                </label>
              ))}
            </div>
          </fieldset>
        ))}
      </div>
    </Dialog>
  );
}
