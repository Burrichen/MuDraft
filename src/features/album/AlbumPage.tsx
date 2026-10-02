import { useEffect, useRef, useState } from "react";
import { Link, useParams, useSearchParams } from "react-router";
import { usePreferences } from "../../app/preferencesContext";
import { useBack } from "../../app/useBack";
import { Artwork } from "../../components/Artwork";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { PageHeader } from "../../components/PageHeader";
import { useMakeNextUp } from "../next-up/useMakeNextUp";
import { Popover } from "../../components/Popover";
import { ErrorState, LoadingState } from "../../components/States";
import { TagChip } from "../../components/TagChip";
import {
  albumDetail,
  artworkPreference,
  artworkUrl,
  fetchArtwork,
  refreshAlbum,
  removeArtwork,
  renameEdition,
  replaceArtwork,
  restoreArtwork,
  type AlbumDetail,
  type EditionView,
} from "../../services/album";
import { useLibraryVersion } from "../../services/libraryEvents";
import { TagPicker } from "../tags/TagPicker";
import { EditAlbumDialog } from "./EditAlbumDialog";
import { AddEditionDialog, SwitchEditionDialog } from "./EditionDialogs";
import { setFavourite, setTrackRating } from "../../services/ratings";
import { ListeningPanel } from "../listening/ListeningPanel";
import { RatingPanel } from "./RatingPanel";
import { runtimeText } from "./runtime";
import { Tracklist } from "./Tracklist";
import "./album.css";

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

type Load =
  | { kind: "loading" }
  | { kind: "error"; code: string; message: string }
  | { kind: "ready"; data: AlbumDetail };

function errorInfo(err: unknown) {
  const code = typeof err === "object" && err && "code" in err ? String(err.code) : "unknown";
  return { code, message: err instanceof Error ? err.message : String(err) };
}

function editionFacts(e: EditionView): string {
  return [e.releaseDate.value ?? "Date unknown", `${String(e.trackCount)} tracks`].join(" · ");
}

export function AlbumPage() {
  const { albumId = "" } = useParams();
  const [params, setParams] = useSearchParams();
  const editionParam = params.get("edition");
  const { label, goBack } = useBack("/listen-list");
  const { prefs } = usePreferences();
  const makeNextUp = useMakeNextUp();
  const version = useLibraryVersion();
  const [load, setLoad] = useState<Load>({ kind: "loading" });
  const [retry, setRetry] = useState(0);
  const [status, setStatus] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);
  const [adding, setAdding] = useState(false);
  const [switching, setSwitching] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<EditionView | null>(null);
  const [newName, setNewName] = useState("");
  const [refreshing, setRefreshing] = useState(false);
  const [tagsOpen, setTagsOpen] = useState(false);
  const tagButton = useRef<HTMLButtonElement | null>(null);
  const valid = UUID.test(albumId);

  useEffect(() => {
    if (!valid) return;
    let cancelled = false;
    albumDetail(albumId, editionParam).then(
      (data) => {
        if (!cancelled) setLoad({ kind: "ready", data });
      },
      (err: unknown) => {
        if (!cancelled) setLoad({ kind: "error", ...errorInfo(err) });
      },
    );
    return () => {
      cancelled = true;
    };
  }, [albumId, editionParam, version, retry, valid]);

  const detail = load.kind === "ready" ? load.data : null;

  // Download artwork for the shown edition when missing — only with the user's consent.
  const needsArt =
    detail && !detail.artwork.current && !detail.artwork.removed ? detail.editionId : null;
  useEffect(() => {
    if (!needsArt) return;
    const controller = new AbortController();
    artworkPreference()
      .then((p) => (p.allowed ? fetchArtwork(needsArt, controller.signal) : null))
      .then((state) => {
        if (state?.current) setRetry((n) => n + 1);
      })
      .catch(() => undefined);
    return () => {
      controller.abort();
    };
  }, [needsArt]);

  const run = async (task: () => Promise<string | null>) => {
    setActionError(null);
    setStatus(null);
    try {
      const done = await task();
      if (done) setStatus(done);
    } catch (err) {
      setActionError(errorInfo(err).message);
    }
  };

  if (!valid) {
    return (
      <section>
        <PageHeader title="Album" back={{ label, onBack: goBack }} />
        <ErrorState
          title="This link doesn’t point to an album"
          message={`“${albumId}” isn’t a valid album ID.`}
        />
      </section>
    );
  }
  if (!detail) {
    return (
      <section>
        <PageHeader title="Album" back={{ label, onBack: goBack }} />
        {load.kind === "loading" && <LoadingState label="Loading album…" />}
        {load.kind === "error" && (
          <ErrorState
            title={load.code === "not_found" ? "Album not found" : "Couldn’t load this album"}
            message={load.message}
            code={load.code}
            {...(load.code === "not_found"
              ? {}
              : {
                  onRetry: () => {
                    setRetry((n) => n + 1);
                  },
                })}
          />
        )}
      </section>
    );
  }

  const edition = detail.editions.find((e) => e.id === detail.editionId);
  const listEdition = detail.editions.find((e) => e.onListenList);
  const art = detail.artwork;
  const linked = detail.musicbrainzReleaseGroupId !== null;
  const showBadge =
    edition && (detail.editions.length > 1 || edition.name.toLowerCase() !== "standard");
  const editionYear = edition?.releaseDate.year;

  return (
    <section className="album-page">
      <PageHeader
        title={detail.title}
        back={{ label, onBack: goBack }}
        actions={
          <>
            {listEdition && (
              <Button
                onClick={() => {
                  makeNextUp.start(listEdition.id, detail.title);
                }}
              >
                Make Next Up
              </Button>
            )}
            <Button
              onClick={() => {
                setEditing(true);
              }}
            >
              Edit details
            </Button>
            <Button
              disabledReason={
                !linked
                  ? "This album isn’t linked to MusicBrainz."
                  : refreshing
                    ? "Refreshing…"
                    : undefined
              }
              onClick={() =>
                void run(async () => {
                  setRefreshing(true);
                  try {
                    const out = await refreshAlbum(detail.albumId, detail.editionId);
                    const kept =
                      out.lockedFieldsKept.length > 0
                        ? ` Your edits were kept (${out.lockedFieldsKept.join(", ")}).`
                        : "";
                    const retained =
                      out.retainedTracks > 0
                        ? ` ${String(out.retainedTracks)} rated or listened tracks no longer on MusicBrainz were kept.`
                        : "";
                    return `Refreshed from MusicBrainz.${kept}${retained}`;
                  } finally {
                    setRefreshing(false);
                  }
                })
              }
            >
              Refresh from MusicBrainz
            </Button>
          </>
        }
      />
      {makeNextUp.element}
      {status && (
        <p className="banner banner-ok" role="status">
          {status}
        </p>
      )}
      {actionError && (
        <p className="match-note match-note-error library-note" role="alert">
          {actionError}
        </p>
      )}

      <div className="album-hero">
        <figure className="album-art">
          <Artwork
            src={prefs.showArtwork ? artworkUrl(art.current) : null}
            title={detail.title}
            size="hero"
          />
          <figcaption className="setting-hint">
            {art.removed
              ? "Artwork removed for this edition."
              : !art.current
                ? "No artwork."
                : art.current.source === "local"
                  ? "Your image."
                  : art.current.canonicalFallback
                    ? "Album artwork from Cover Art Archive (not specific to this edition)."
                    : "Edition artwork from Cover Art Archive."}
          </figcaption>
          <div className="page-actions">
            <Button
              variant="ghost"
              className="button-small"
              onClick={() =>
                void run(async () =>
                  (await replaceArtwork(detail.editionId)) ? "Artwork replaced." : null,
                )
              }
            >
              Replace…
            </Button>
            {art.current && (
              <Button
                variant="ghost"
                className="button-small"
                onClick={() =>
                  void run(async () => {
                    await removeArtwork(detail.editionId);
                    return "Artwork removed. It won’t be downloaded again unless you restore it.";
                  })
                }
              >
                Remove
              </Button>
            )}
            {(art.removed || art.current?.source === "local") && (
              <Button
                variant="ghost"
                className="button-small"
                onClick={() =>
                  void run(async () => {
                    await restoreArtwork(detail.editionId);
                    return "Artwork restored.";
                  })
                }
              >
                Restore default
              </Button>
            )}
          </div>
        </figure>

        <div className="album-facts">
          <p className="album-credits">
            {detail.credits.length === 0
              ? "Unknown artist"
              : detail.credits.map((c) => (
                  <span key={c.artistId}>
                    <Link to={`/artists/${c.artistId}`}>{c.creditedName ?? c.name}</Link>
                    {c.joinPhrase}
                  </span>
                ))}
          </p>
          <p className="album-badges">
            <span className="edition-badge">{detail.originalDate.year ?? "Year unknown"}</span>
            {showBadge && (
              <span className="edition-badge">
                {edition.name}
                {editionYear && editionYear !== detail.originalDate.year
                  ? ` · ${String(editionYear)}`
                  : ""}
              </span>
            )}
          </p>
          <p className="album-genres-line">
            <span className="filter-group-label">Genres </span>
            {detail.genres.length > 0 ? detail.genres.join(" · ") : "None yet"}
          </p>
          <div className="album-tag-row">
            <span className="filter-group-label">Tags </span>
            {detail.tags.length > 0 ? (
              <ul className="tag-list" aria-label={`Tags on ${detail.title}`}>
                {detail.tags.map((t) => (
                  <li key={t.id}>
                    <TagChip name={t.name} color={t.color} builtin={t.builtin} />
                  </li>
                ))}
              </ul>
            ) : (
              <span className="setting-hint">None</span>
            )}
            <button
              ref={tagButton}
              type="button"
              className="mini-button"
              aria-label={`Edit tags for ${detail.title}`}
              aria-haspopup="dialog"
              aria-expanded={tagsOpen}
              onClick={() => {
                setTagsOpen(true);
              }}
            >
              +
            </button>
          </div>
          <p className="setting-hint">{runtimeText(detail.runtime)}</p>
        </div>
      </div>

      <section className="panel section album-section" aria-labelledby="album-about">
        <h2 id="album-about" className="section-title">
          About
        </h2>
        {detail.description ? (
          <>
            <p className="album-description">{detail.description.text}</p>
            <p className="setting-hint">
              {detail.description.source === "musicbrainz"
                ? "From the MusicBrainz annotation."
                : "Written by you."}
            </p>
          </>
        ) : (
          <>
            <p>{detail.summary}</p>
            <p className="setting-hint">
              A summary of facts from your library — no description has been added.
            </p>
          </>
        )}
      </section>

      <ListeningPanel detail={detail} />

      <RatingPanel detail={detail} onError={setActionError} />

      <section className="panel section album-section" aria-labelledby="album-editions">
        <div className="toolbar">
          <h2 id="album-editions" className="section-title">
            Editions
          </h2>
          <Button
            disabledReason={
              linked ? undefined : "Editions come from MusicBrainz; this album isn’t linked."
            }
            onClick={() => {
              setAdding(true);
            }}
          >
            Add another edition…
          </Button>
        </div>
        <ul className="edition-list">
          {detail.editions.map((e) => (
            <li key={e.id} className="edition-row" data-current={e.id === detail.editionId}>
              <div className="edition-text">
                <span className="track-title">{e.name}</span>
                <span className="setting-hint">{editionFacts(e)}</span>
                <span className="setting-hint">
                  {[e.onListenList && "On your Listen List", e.inCollection && "In your Collection"]
                    .filter(Boolean)
                    .join(" · ")}
                </span>
              </div>
              <div className="page-actions">
                {e.id === detail.editionId ? (
                  <span className="edition-badge" aria-current="true">
                    Showing
                  </span>
                ) : (
                  <Button
                    variant="ghost"
                    className="button-small"
                    onClick={() => {
                      setParams({ edition: e.id }, { replace: true });
                    }}
                  >
                    Show tracks
                  </Button>
                )}
                {listEdition && !e.onListenList && (
                  <Button
                    variant="ghost"
                    className="button-small"
                    onClick={() => {
                      setSwitching(e.id);
                    }}
                  >
                    Use on Listen List…
                  </Button>
                )}
                <Button
                  variant="ghost"
                  className="button-small"
                  aria-label={`Rename edition ${e.name}`}
                  onClick={() => {
                    setRenaming(e);
                    setNewName(e.name);
                  }}
                >
                  Rename
                </Button>
              </div>
            </li>
          ))}
        </ul>
      </section>

      <section className="panel section album-section" aria-labelledby="album-tracks">
        <h2 id="album-tracks" className="section-title">
          Tracklist{edition ? ` · ${edition.name}` : ""}
        </h2>
        <Tracklist
          detail={detail}
          onRate={(trackId, v) => {
            setActionError(null);
            setTrackRating(trackId, v).catch((err: unknown) => {
              setActionError(errorInfo(err).message);
            });
          }}
          onFavourite={(trackId, favourite) => {
            setActionError(null);
            setFavourite(trackId, favourite).catch((err: unknown) => {
              setActionError(errorInfo(err).message);
            });
          }}
        />
      </section>

      <Popover
        open={tagsOpen}
        label={`Tags for ${detail.title}`}
        anchorRef={tagButton}
        onClose={() => {
          setTagsOpen(false);
        }}
      >
        {tagsOpen && (
          <TagPicker
            albumIds={[detail.albumId]}
            assigned={new Map(detail.tags.map((t) => [t.id, 1]))}
            onDone={() => {
              setTagsOpen(false);
              tagButton.current?.focus();
            }}
          />
        )}
      </Popover>

      {editing && (
        <EditAlbumDialog
          detail={detail}
          onClose={() => {
            setEditing(false);
          }}
        />
      )}
      {adding && (
        <AddEditionDialog
          albumId={detail.albumId}
          albumTitle={detail.title}
          onClose={() => {
            setAdding(false);
          }}
          onAdded={(editionId, name) => {
            setAdding(false);
            setStatus(
              `Added the “${name}” edition. Its tracks are kept separately from your other editions.`,
            );
            setParams({ edition: editionId }, { replace: true });
          }}
        />
      )}
      {switching && (
        <SwitchEditionDialog
          albumId={detail.albumId}
          toEditionId={switching}
          onClose={() => {
            setSwitching(null);
          }}
          onSwitched={(copied) => {
            setSwitching(null);
            setStatus(
              `Your Listen List now uses this edition.${copied > 0 ? ` Copied ${String(copied)} ratings to matching recordings.` : ""}`,
            );
          }}
        />
      )}
      <Dialog
        open={renaming !== null}
        title="Rename edition"
        onClose={() => {
          setRenaming(null);
        }}
        actions={
          <>
            <Button
              onClick={() => {
                setRenaming(null);
              }}
            >
              Cancel
            </Button>
            <Button
              variant="primary"
              disabledReason={newName.trim() ? undefined : "Enter a name."}
              onClick={() =>
                void run(async () => {
                  if (!renaming) return null;
                  await renameEdition(renaming.id, newName);
                  setRenaming(null);
                  return "Edition renamed.";
                })
              }
            >
              Save
            </Button>
          </>
        }
      >
        <div className="match-field">
          <label htmlFor="edition-rename">Edition name</label>
          <input
            id="edition-rename"
            value={newName}
            onChange={(e) => {
              setNewName(e.target.value);
            }}
          />
        </div>
      </Dialog>
    </section>
  );
}
