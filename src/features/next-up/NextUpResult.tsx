import { Link } from "react-router";
import { AlbumCard } from "../../components/AlbumCard";
import { Button } from "../../components/Button";
import type { CurrentPick, NextUpState } from "../../services/nextUp";
import { toSummary } from "../albumSummary";
import { eligibilityMessage, explainMatch, METHOD_LABEL } from "./criteria";

/** The one current album, why it was picked, and what to do with it. */
export function NextUpResult({
  pick,
  session,
  busy,
  onReroll,
  onChangeChoices,
  onClear,
  onMarkListened,
}: {
  pick: CurrentPick;
  session: NextUpState["session"];
  busy: boolean;
  onReroll: () => void;
  onChangeChoices: () => void;
  onClear: () => void;
  onMarkListened: () => void;
}) {
  const warning = eligibilityMessage(pick);
  const rerollReason = busy
    ? "Picking…"
    : session
      ? undefined
      : "There’s no picking method to reroll with. Use Change Choices.";
  const summary = { ...toSummary(pick.item, "next_up"), addedAt: null };
  return (
    <section className="next-up-result" aria-labelledby="next-up-current">
      <h2 id="next-up-current" className="section-title">
        Your Next Up
      </h2>
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
          {session && (
            <p className="setting-hint">
              Rerolls use {METHOD_LABEL[session.method.mode]}.{" "}
              {session.remaining > 0
                ? `${String(session.remaining)} more album${session.remaining === 1 ? "" : "s"} can come up this round.`
                : "Every matching album has come up this round."}
            </p>
          )}
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
            <Button disabledReason={rerollReason} onClick={onReroll}>
              Reroll
            </Button>
            <Button onClick={onChangeChoices}>Change Choices</Button>
            <Button
              variant="ghost"
              disabledReason={busy ? "Working…" : undefined}
              onClick={onClear}
            >
              Clear Selection
            </Button>
          </div>
        </div>
      </div>
    </section>
  );
}
