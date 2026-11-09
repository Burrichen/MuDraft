import { useEffect, useState } from "react";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { FilterBubble } from "../../components/Filters";
import { LoadingState } from "../../components/States";
import { genreVocabulary, setAlbumGenres } from "../../services/metadata";
import type { AlbumDetail } from "../../services/album";

const MAX_GENRES = 3;

function message(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/** Choose an album's broad genres by hand. Saved genres are kept when it is refreshed. */
export function EditGenresDialog({
  detail,
  onClose,
}: {
  detail: AlbumDetail;
  onClose: () => void;
}) {
  const [vocabulary, setVocabulary] = useState<string[] | null>(null);
  const [selected, setSelected] = useState<string[]>(detail.genres);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    genreVocabulary().then(
      (v) => {
        if (!cancelled) setVocabulary(v);
      },
      (err: unknown) => {
        if (!cancelled) setError(message(err));
      },
    );
    return () => {
      cancelled = true;
    };
  }, []);

  const unchanged =
    selected.length === detail.genres.length && selected.every((g, i) => g === detail.genres[i]);

  const save = async () => {
    if (unchanged) {
      onClose();
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await setAlbumGenres(detail.albumId, selected);
      onClose();
    } catch (err) {
      setError(message(err));
    } finally {
      setBusy(false);
    }
  };

  const toggle = (genre: string) => {
    setSelected((s) => (s.includes(genre) ? s.filter((g) => g !== genre) : [...s, genre]));
  };

  return (
    <Dialog
      open
      title="Edit genres"
      onClose={onClose}
      actions={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            disabledReason={
              busy
                ? "Saving…"
                : !vocabulary
                  ? "Loading genres…"
                  : selected.length > MAX_GENRES
                    ? `Choose at most ${String(MAX_GENRES)} genres.`
                    : undefined
            }
            onClick={() => void save()}
          >
            Save
          </Button>
        </>
      }
    >
      <div className="match">
        <p className="match-note">
          Choose up to {MAX_GENRES}, most relevant first. Your choice is kept when the album is
          refreshed; use <strong>Reset to default</strong> on the album page to undo it.
        </p>
        {vocabulary ? (
          <div className="filter-bubbles" role="group" aria-label="Genres">
            {vocabulary.map((g) => (
              <FilterBubble
                key={g}
                label={g}
                pressed={selected.includes(g)}
                onToggle={() => {
                  toggle(g);
                }}
              />
            ))}
          </div>
        ) : (
          !error && <LoadingState label="Loading genres…" />
        )}
        {error && (
          <p className="match-note match-note-error" role="alert">
            {error}
          </p>
        )}
      </div>
    </Dialog>
  );
}
