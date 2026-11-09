import { Link } from "react-router";
import { AlbumCard } from "../../components/AlbumCard";
import { Button } from "../../components/Button";
import type { NextUpSlot } from "../../services/nextUp";
import { toSummary } from "../albumSummary";
import { eligibilityMessage, explainMatch, METHOD_LABEL } from "./criteria";

const albums = (n: number) => `${String(n)} more album${n === 1 ? "" : "s"}`;

/**
 * One Next Up slot: its album, why it was picked, and what to do with it — or, once that
 * album is listened to, its picking method ready to pick again.
 */
export function NextUpResult({
  slot,
  busy,
  onRoll,
  onChangeChoices,
  onRemove,
  onMarkListened,
}: {
  slot: NextUpSlot;
  busy: boolean;
  /** Reroll a pick, or pick again in an empty slot, with the slot's method. */
  onRoll: () => void;
  onChangeChoices: () => void;
  onRemove: () => void;
  onMarkListened: () => void;
}) {
  const { current: pick, session } = slot;
  const working = busy ? "Working…" : undefined;
  const poolHint = session && (
    <p className="setting-hint">
      {pick ? "Rerolls use" : "Picks with"} {METHOD_LABEL[session.method.mode]}.{" "}
      {session.remaining > 0
        ? `${albums(session.remaining)} can come up this round.`
        : "Every matching album has come up this round."}
    </p>
  );
  const remove = (
    <Button variant="ghost" disabledReason={working} onClick={onRemove}>
      Remove
    </Button>
  );

  if (!pick) {
    return (
      <article className="next-up-slot next-up-slot-empty" aria-label="Ready for another pick">
        <p className="next-up-empty-title">
          <strong>Ready for another pick.</strong>
        </p>
        {poolHint}
        <div className="page-actions next-up-actions">
          <Button variant="primary" disabledReason={busy ? "Picking…" : undefined} onClick={onRoll}>
            Pick an Album
          </Button>
          <Button onClick={onChangeChoices}>Change Choices</Button>
          {remove}
        </div>
      </article>
    );
  }

  const warning = eligibilityMessage(pick);
  const rerollReason = busy
    ? "Picking…"
    : session
      ? undefined
      : "You picked this album yourself, so there’s no method to reroll with. Use Change Choices.";
  const summary = { ...toSummary(pick.item, "next_up"), addedAt: null };
  return (
    <article className="next-up-slot" aria-label={pick.item.title}>
      <div className="next-up-hero">
        <div className="next-up-card">
          <AlbumCard album={summary} layout="grid" />
        </div>
        <div className="next-up-details">
          <p className="next-up-why">
            <strong>Why this album: </strong>
            {explainMatch(pick.matched, pick.item.originalYear)}
          </p>
          {warning && (
            <p className="match-note match-note-error" role="note">
              {warning}
            </p>
          )}
          {poolHint}
          <div className="page-actions next-up-actions">
            <Button variant="primary" onClick={onMarkListened}>
              Mark Listened
            </Button>
            <Link
              className="button button-secondary"
              to={`/albums/${pick.albumId}`}
              state={{ from: "/next-up" }}
            >
              Open Album
            </Link>
            <Button disabledReason={rerollReason} onClick={onRoll}>
              Reroll
            </Button>
            <Button onClick={onChangeChoices}>Change Choices</Button>
            {remove}
          </div>
        </div>
      </div>
    </article>
  );
}
