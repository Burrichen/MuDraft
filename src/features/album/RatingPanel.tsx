import { useId, useState } from "react";
import { Button } from "../../components/Button";
import { RatingDisplay, RatingInput } from "../../components/Rating";
import type { AlbumDetail } from "../../services/album";
import { describeRating, setAlbumRating, setReview } from "../../services/ratings";

/**
 * Explicit album rating, the shared calculated rating, and review/notes for the shown
 * edition. Rating or writing notes never marks the album as listened.
 */
export function RatingPanel({
  detail,
  onError,
}: {
  detail: AlbumDetail;
  onError: (message: string | null) => void;
}) {
  const notesId = useId();
  const [draft, setDraft] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const notes = draft ?? detail.review ?? "";
  const r = detail.rating;
  const edition = detail.editions.find((e) => e.id === detail.editionId)?.name ?? "this edition";

  const fail = (err: unknown) => {
    onError(err instanceof Error ? err.message : String(err));
  };

  return (
    <section className="panel section album-section" aria-labelledby="album-rating">
      <h2 id="album-rating" className="section-title">
        Your rating and notes · {edition}
      </h2>
      <div className="rating-summary" role="status">
        <RatingDisplay value={r.effective} />
        <span>{describeRating(r)}</span>
      </div>
      <RatingInput
        label="Your album rating"
        value={r.explicit}
        onChange={(v) => {
          onError(null);
          setAlbumRating(detail.editionId, v).catch(fail);
        }}
      />
      <p className="setting-hint">
        {r.explicit === null
          ? "Leave this empty to use the rating calculated from your track ratings."
          : "Your album rating overrides the calculated one. Clearing it brings the calculated rating back; track ratings are never changed."}
      </p>
      <div className="match-field">
        <label htmlFor={notesId}>Review and notes</label>
        <textarea
          id={notesId}
          className="album-textarea"
          rows={4}
          value={notes}
          onChange={(e) => {
            setDraft(e.target.value);
            setSaved(false);
          }}
        />
      </div>
      <div className="page-actions">
        <Button
          disabledReason={
            draft === null || draft === (detail.review ?? "") ? "No changes to save." : undefined
          }
          onClick={() => {
            onError(null);
            setReview(detail.editionId, notes.trim() ? notes : null).then(() => {
              setDraft(null);
              setSaved(true);
            }, fail);
          }}
        >
          Save notes
        </Button>
        {saved && (
          <span className="setting-hint" role="status">
            Notes saved.
          </span>
        )}
      </div>
    </section>
  );
}
