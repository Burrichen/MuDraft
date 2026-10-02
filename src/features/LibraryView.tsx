import { useEffect, useId, useRef, useState } from "react";
import { Link, useLocation, useSearchParams } from "react-router";
import { usePreferences } from "../app/preferencesContext";
import { useDebounced } from "../app/useDebounced";
import { AlbumCollection, type AlbumSummary } from "../components/AlbumCard";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { FilterBubble, LayoutToggle } from "../components/Filters";
import { PageHeader } from "../components/PageHeader";
import { Popover } from "../components/Popover";
import { EmptyState, ErrorState, LoadingState } from "../components/States";
import {
  listLibrary,
  removeFromListenList,
  type LibrarySource,
  type ListItem,
  type ListResult,
  type SortOrder,
} from "../services/library";
import { useLibraryVersion } from "../services/libraryEvents";
import { artworkPreference, fetchArtworkMany } from "../services/album";
import { addManualAlbum, importRelease } from "../services/metadata";
import { type Logged, undoLog } from "../services/listening";
import { toSummary } from "./albumSummary";
import { UndoBanner } from "../components/UndoBanner";
import { ListenDialog } from "./listening/ListenDialog";
import { loggedMessage } from "./listening/loggedMessage";
import { MatchDialog, type MatchChoice } from "./match/MatchDialog";
import { useMakeNextUp } from "./next-up/useMakeNextUp";
import { TagPicker } from "./tags/TagPicker";

/** Cards rendered at a time; large libraries grow on request instead of all at once. */
const PAGE = 120;

const SORTS: { value: SortOrder; label: string }[] = [
  { value: "added_newest", label: "Recently added" },
  { value: "added_oldest", label: "Oldest added" },
  { value: "recently_listened", label: "Recently listened" },
  { value: "title", label: "Album title" },
  { value: "artist", label: "Artist" },
  { value: "year_newest", label: "Year (newest)" },
  { value: "year_oldest", label: "Year (oldest)" },
  { value: "rating_highest", label: "Rating (highest)" },
  { value: "rating_lowest", label: "Rating (lowest)" },
];

type Load =
  | { kind: "loading" }
  | { kind: "error"; code: string; message: string }
  | { kind: "ready"; data: ListResult };

function errorInfo(err: unknown) {
  const code = typeof err === "object" && err && "code" in err ? String(err.code) : "unknown";
  return { code, message: err instanceof Error ? err.message : String(err) };
}

async function addChoice(choice: MatchChoice, source: LibrarySource): Promise<string> {
  const toList = source === "listen_list";
  if (choice.kind === "manual") {
    await addManualAlbum(choice.album, toList, !toList);
    return choice.album.title;
  }
  await importRelease({
    releaseGroupId: choice.releaseGroupId,
    releaseId: choice.releaseId,
    addToListenList: toList,
    addToCollection: !toList,
    ...(choice.editionName ? { editionName: choice.editionName } : {}),
  });
  return choice.candidate.title;
}

function toggle<T>(list: readonly T[], value: T): T[] {
  return list.includes(value) ? list.filter((v) => v !== value) : [...list, value];
}

/**
 * Listen List and Collection. Both read the same album-level data, so tags edited here
 * appear on every page. Removing from the Listen List never touches reviews, ratings,
 * listens, or tags.
 */
export function LibraryView({
  source,
  title,
  description,
  emptyTitle,
  emptyDescription,
}: {
  source: LibrarySource;
  title: string;
  description: string;
  emptyTitle: string;
  emptyDescription: string;
}) {
  const isListen = source === "listen_list";
  const { prefs, update } = usePreferences();
  const location = useLocation();
  const version = useLibraryVersion();
  const ids = { search: useId(), sort: useId(), genres: useId(), tags: useId() };

  const [searchInput, setSearchInput] = useState(
    () => new URLSearchParams(location.search).get("q") ?? "",
  );
  const typed = useDebounced(searchInput.trim(), 200);
  // Typing is debounced; clearing applies at once.
  const search = searchInput.trim() === "" ? "" : typed;
  // The view (search, sort, filters, drill-downs, how much is shown) lives in the URL,
  // so returning from an album page restores it. Every change is one URL write, which
  // keeps concurrent updates from overwriting each other.
  const [params, setParams] = useSearchParams();
  const updateParams = (edit: (p: URLSearchParams) => void) => {
    setParams(
      (prev) => {
        const next = new URLSearchParams(prev);
        edit(next);
        return next;
      },
      { replace: true },
    );
  };
  const putList = (p: URLSearchParams, key: string, values: readonly string[]) => {
    p.delete(key);
    for (const v of values) p.append(key, v);
  };
  const wantedSort = params.get("sort");
  const sort: SortOrder = SORTS.some((o) => o.value === wantedSort)
    ? (wantedSort as SortOrder)
    : "added_newest";
  const genres = params.getAll("genre");
  const tagIds = params.getAll("tag");
  const setSort = (next: SortOrder) => {
    updateParams((p) => {
      if (next === "added_newest") p.delete("sort");
      else p.set("sort", next);
    });
  };
  const setGenres = (next: readonly string[]) => {
    updateParams((p) => {
      putList(p, "genre", next);
    });
  };
  const setTagIds = (next: readonly string[]) => {
    updateParams((p) => {
      putList(p, "tag", next);
    });
  };
  const numberParam = (key: string) => {
    const v = Number(params.get(key));
    return params.has(key) && Number.isInteger(v) ? v : undefined;
  };
  const drill = {
    year: numberParam("year"),
    decade: numberParam("decade"),
    yearUnknown: params.get("yearUnknown") === "1",
    artistId: params.get("artist") ?? undefined,
  };
  const shown = numberParam("show");
  const limit = shown !== undefined && shown > PAGE ? shown : PAGE;
  const setLimit = (n: number) => {
    updateParams((p) => {
      p.set("show", String(n));
    });
  };
  const queryKey = JSON.stringify({ search, sort, genres, tagIds, drill });

  // Write typed (debounced) searches to the URL; clearing writes it directly.
  const writtenSearch = useRef(search);
  useEffect(() => {
    if (writtenSearch.current === search) return;
    writtenSearch.current = search;
    updateParams((p) => {
      if (search) p.set("q", search);
      else p.delete("q");
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [search]);
  const [load, setLoad] = useState<Load>({ kind: "loading" });
  const [retry, setRetry] = useState(0);

  const [selecting, setSelecting] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [picker, setPicker] = useState<{ albumIds: string[]; label: string } | null>(null);
  const pickerAnchor = useRef<HTMLElement | null>(null);
  const [removing, setRemoving] = useState<{ albumIds: string[]; label: string } | null>(null);
  const [addOpen, setAddOpen] = useState(false);
  const [status, setStatus] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [listening, setListening] = useState<ListItem | null>(null);
  const [undo, setUndo] = useState<Logged | null>(null);
  const makeNextUp = useMakeNextUp();

  useEffect(() => {
    let cancelled = false;
    const q = JSON.parse(queryKey) as {
      search: string;
      sort: SortOrder;
      genres: string[];
      tagIds: string[];
      drill: typeof drill;
    };
    const d = q.drill;
    listLibrary(source, {
      ...(q.search ? { search: q.search } : {}),
      sort: q.sort,
      genres: q.genres,
      tagIds: q.tagIds,
      ...(d.year !== undefined ? { year: d.year } : {}),
      ...(d.decade !== undefined ? { decade: d.decade } : {}),
      ...(d.yearUnknown ? { yearUnknown: true } : {}),
      ...(d.artistId ? { artistId: d.artistId } : {}),
    }).then(
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
    // The query is read through `queryKey` so the effect reruns only when it changes.
  }, [source, queryKey, version, retry]);

  const data = load.kind === "ready" ? load.data : null;

  // Download missing artwork for what's shown, a bounded batch at a time, only with consent.
  const missingKey = (data?.items ?? [])
    .filter((i) => !i.artwork)
    .map((i) => i.editionId)
    .join(",");
  useEffect(() => {
    if (!missingKey) return;
    const controller = new AbortController();
    artworkPreference()
      .then((p) => (p.allowed ? fetchArtworkMany(missingKey.split(","), controller.signal) : 0))
      .catch(() => 0);
    return () => {
      controller.abort();
    };
  }, [missingKey]);
  const items = data?.items ?? [];
  const byAlbum = new Map(items.map((i) => [i.albumId, i]));
  const drillChips = [
    drill.year !== undefined && { key: "year", label: `Released in ${String(drill.year)}` },
    drill.decade !== undefined && {
      key: "decade",
      label: `Released in the ${String(drill.decade)}s`,
    },
    drill.yearUnknown && { key: "yearUnknown", label: "Release year unknown" },
    drill.artistId && {
      key: "artist",
      label: `By ${
        items.flatMap((i) => i.artists).find((a) => a.id === drill.artistId)?.name ??
        "the chosen artist"
      }`,
    },
  ].filter(Boolean) as { key: string; label: string }[];
  const filtering =
    search !== "" || genres.length > 0 || tagIds.length > 0 || drillChips.length > 0;
  const selectedIds = [...selected].filter((id) => byAlbum.has(id));

  const assignedFor = (albumIds: readonly string[]) => {
    const counts = new Map<string, number>();
    for (const id of albumIds)
      for (const t of byAlbum.get(id)?.tags ?? []) counts.set(t.id, (counts.get(t.id) ?? 0) + 1);
    return counts;
  };

  const clearDrill = (key: string) => {
    updateParams((p) => {
      p.delete(key);
    });
  };

  const clearFilters = () => {
    writtenSearch.current = "";
    setSearchInput("");
    updateParams((p) => {
      for (const k of ["q", "genre", "tag", "year", "decade", "yearUnknown", "artist", "show"])
        p.delete(k);
    });
  };

  const confirmRemove = async () => {
    if (!removing) return;
    setActionError(null);
    try {
      const n = await removeFromListenList(removing.albumIds);
      setStatus(
        `Removed ${removing.label} from your Listen List. Reviews, listens and tags are kept.`,
      );
      setSelected(new Set());
      if (n === 0) setStatus("Those albums were already off your Listen List.");
    } catch (err) {
      setActionError(errorInfo(err).message);
    } finally {
      setRemoving(null);
    }
  };

  const cardExtras = (album: AlbumSummary) => ({
    genreAction: (
      <button
        type="button"
        className="mini-button"
        aria-label={`Edit tags for ${album.title}`}
        aria-haspopup="dialog"
        aria-expanded={picker?.albumIds[0] === album.albumId && picker.albumIds.length === 1}
        onClick={(e) => {
          pickerAnchor.current = e.currentTarget;
          setPicker({ albumIds: [album.albumId], label: `Tags for ${album.title}` });
        }}
      >
        +
      </button>
    ),
    ...(selecting
      ? {
          selection: {
            checked: selected.has(album.albumId),
            onChange: (checked: boolean) => {
              const next = new Set(selected);
              if (checked) next.add(album.albumId);
              else next.delete(album.albumId);
              setSelected(next);
            },
          },
        }
      : {}),
    ...(!selecting
      ? {
          actions: (
            <>
              <Button
                variant="ghost"
                className="button-small"
                aria-label={`Mark ${album.title} listened`}
                onClick={() => {
                  const item = items.find(
                    (i) => i.albumId === album.albumId && i.editionId === album.editionId,
                  );
                  if (item) setListening(item);
                }}
              >
                Mark listened
              </Button>
              {isListen && album.editionId && (
                <Button
                  variant="ghost"
                  className="button-small"
                  aria-label={`Make ${album.title} Next Up`}
                  onClick={() => {
                    if (album.editionId) makeNextUp.start(album.editionId, album.title);
                  }}
                >
                  Make Next Up
                </Button>
              )}
              {isListen && (
                <Button
                  variant="ghost"
                  className="button-small"
                  aria-label={`Remove ${album.title} from Listen List`}
                  onClick={() => {
                    setRemoving({ albumIds: [album.albumId], label: `“${album.title}”` });
                  }}
                >
                  Remove
                </Button>
              )}
            </>
          ),
        }
      : {}),
  });

  const emptyList = (
    <EmptyState
      title={emptyTitle}
      description={emptyDescription}
      {...(isListen
        ? {
            action: (
              <div className="page-actions">
                <Button
                  variant="primary"
                  onClick={() => {
                    setAddOpen(true);
                  }}
                >
                  Add album
                </Button>
                <Link
                  className="button button-secondary"
                  to="/listen-list/import"
                  state={{ from: location.pathname }}
                >
                  Import CSV
                </Link>
              </div>
            ),
          }
        : {})}
    />
  );

  const noMatches = (
    <EmptyState
      icon="info"
      title="No albums match"
      description={[
        search && `Nothing matches “${search}”`,
        genres.length > 0 && `genre: ${genres.join(" or ")}`,
        tagIds.length > 0 &&
          `tag: ${tagIds.map((id) => data?.tags.find((t) => t.id === id)?.name ?? "removed tag").join(" or ")}`,
      ]
        .filter(Boolean)
        .join(" · ")}
      action={<Button onClick={clearFilters}>Clear search and filters</Button>}
    />
  );

  return (
    <section>
      <PageHeader
        title={title}
        description={description}
        actions={
          <>
            {isListen && (
              <Link
                className="button button-secondary"
                to="/listen-list/import"
                state={{ from: location.pathname }}
              >
                Import CSV
              </Link>
            )}
            <Button
              variant="primary"
              onClick={() => {
                setStatus(null);
                setAddOpen(true);
              }}
            >
              Add album
            </Button>
          </>
        }
      />
      {status && (
        <p className="banner banner-ok" role="status">
          {status}
        </p>
      )}
      {makeNextUp.element}
      {undo && (
        <UndoBanner
          message={loggedMessage(undo)}
          onUndo={() => undoLog(undo.undo)}
          onDismiss={() => {
            setUndo(null);
          }}
        />
      )}
      {actionError && (
        <p className="match-note match-note-error library-note" role="alert">
          {actionError}
        </p>
      )}

      {data && data.total > 0 && (
        <div className="library-controls">
          <div className="toolbar">
            <div className="library-search">
              <label htmlFor={ids.search} className="visually-hidden">
                Search {title}
              </label>
              <input
                id={ids.search}
                type="search"
                className="tag-picker-input"
                placeholder={`Search ${title} by album or artist`}
                value={searchInput}
                onChange={(e) => {
                  setSearchInput(e.target.value);
                }}
              />
            </div>
            <div className="library-sort">
              <label htmlFor={ids.sort} className="filter-group-label">
                Sort
              </label>
              <select
                id={ids.sort}
                className="import-select"
                value={sort}
                onChange={(e) => {
                  setSort(e.target.value as SortOrder);
                }}
              >
                {SORTS.map((s) => (
                  <option key={s.value} value={s.value}>
                    {s.label}
                  </option>
                ))}
              </select>
            </div>
            <LayoutToggle
              value={prefs.albumLayout}
              onChange={(albumLayout) => {
                update({ albumLayout });
              }}
            />
            <FilterBubble
              label={selecting ? "Done selecting" : "Select"}
              pressed={selecting}
              onToggle={() => {
                setSelecting(!selecting);
                setSelected(new Set());
              }}
            />
          </div>
          {data.genres.length > 0 && (
            <div className="filter-group" role="group" aria-labelledby={ids.genres}>
              <span id={ids.genres} className="filter-group-label">
                Genres
              </span>
              <div className="filter-bubbles">
                {data.genres.map((g) => (
                  <FilterBubble
                    key={g.name}
                    label={g.name}
                    count={g.count}
                    pressed={genres.includes(g.name)}
                    onToggle={() => {
                      setGenres(toggle(genres, g.name));
                    }}
                  />
                ))}
              </div>
            </div>
          )}
          {data.tags.length > 0 && (
            <div className="filter-group" role="group" aria-labelledby={ids.tags}>
              <span id={ids.tags} className="filter-group-label">
                Tags
              </span>
              <div className="filter-bubbles">
                {data.tags.map((t) => (
                  <FilterBubble
                    key={t.id}
                    label={t.name}
                    count={t.count}
                    pressed={tagIds.includes(t.id)}
                    onToggle={() => {
                      setTagIds(toggle(tagIds, t.id));
                    }}
                  />
                ))}
              </div>
            </div>
          )}
          {drillChips.length > 0 && (
            <div className="filter-group" role="group" aria-label="Showing only">
              <span className="filter-group-label">Showing only</span>
              <div className="filter-bubbles">
                {drillChips.map((c) => (
                  <FilterBubble
                    key={c.key}
                    label={`${c.label} ×`}
                    pressed
                    onToggle={() => {
                      clearDrill(c.key);
                    }}
                  />
                ))}
              </div>
            </div>
          )}
          <p className="setting-hint" role="status">
            {filtering
              ? `${String(items.length)} of ${String(data.total)} albums`
              : `${String(data.total)} album${data.total === 1 ? "" : "s"}`}
            {filtering && (
              <>
                {" · "}
                <button type="button" className="link-button" onClick={clearFilters}>
                  Clear filters
                </button>
              </>
            )}
          </p>
          {selecting && (
            <div className="bulk-bar" role="region" aria-label="Selected albums">
              <span>{selectedIds.length} selected</span>
              <Button
                variant="ghost"
                className="button-small"
                disabledReason={items.length === 0 ? "Nothing to select." : undefined}
                onClick={() => {
                  setSelected(new Set(items.map((i) => i.albumId)));
                }}
              >
                Select all shown
              </Button>
              <Button
                disabledReason={selectedIds.length === 0 ? "Select albums first." : undefined}
                onClick={(e) => {
                  pickerAnchor.current = e.currentTarget;
                  setPicker({
                    albumIds: selectedIds,
                    label: `Tags for ${String(selectedIds.length)} albums`,
                  });
                }}
              >
                Tag…
              </Button>
              {isListen && (
                <Button
                  variant="danger"
                  disabledReason={selectedIds.length === 0 ? "Select albums first." : undefined}
                  onClick={() => {
                    setRemoving({
                      albumIds: selectedIds,
                      label: `${String(selectedIds.length)} albums`,
                    });
                  }}
                >
                  Remove from Listen List
                </Button>
              )}
            </div>
          )}
        </div>
      )}

      {load.kind === "loading" && <LoadingState label={`Loading ${title}…`} />}
      {load.kind === "error" && (
        <ErrorState
          title={`Couldn’t load ${title}`}
          message={load.message}
          code={load.code}
          onRetry={() => {
            setLoad({ kind: "loading" });
            setRetry((n) => n + 1);
          }}
        />
      )}
      {data && (
        <AlbumCollection
          albums={items.slice(0, limit).map((i) => toSummary(i, source))}
          layout={prefs.albumLayout}
          label={title}
          empty={data.total === 0 ? emptyList : noMatches}
          cardExtras={cardExtras}
        />
      )}
      {items.length > limit && (
        <div className="show-more">
          <span className="setting-hint">
            Showing {limit} of {items.length}
          </span>
          <Button
            onClick={() => {
              setLimit(limit + PAGE);
            }}
          >
            Show {Math.min(PAGE, items.length - limit)} more
          </Button>
        </div>
      )}

      <Popover
        open={picker !== null}
        label={picker?.label ?? "Tags"}
        anchorRef={pickerAnchor}
        onClose={() => {
          setPicker(null);
        }}
      >
        {picker && (
          <TagPicker
            key={picker.albumIds.join(",")}
            albumIds={picker.albumIds}
            assigned={assignedFor(picker.albumIds)}
            onDone={() => {
              setPicker(null);
              pickerAnchor.current?.focus();
            }}
          />
        )}
      </Popover>

      <Dialog
        open={removing !== null}
        title="Remove from Listen List?"
        onClose={() => {
          setRemoving(null);
        }}
        actions={
          <>
            <Button
              onClick={() => {
                setRemoving(null);
              }}
            >
              Keep
            </Button>
            <Button variant="danger" onClick={() => void confirmRemove()}>
              Remove
            </Button>
          </>
        }
      >
        {removing?.label} will leave your Listen List. The album stays in your library with its
        tags, ratings, reviews, and listening history.
      </Dialog>

      <MatchDialog
        open={addOpen}
        title={isListen ? "Add to Listen List" : "Add to Collection"}
        confirmLabel={isListen ? "Add to Listen List" : "Add to Collection"}
        onClose={() => {
          setAddOpen(false);
        }}
        onConfirm={async (choice) => {
          const name = await addChoice(choice, source);
          setStatus(
            isListen
              ? `Added “${name}” to your Listen List.`
              : `Added “${name}” to your Collection. Log a listen from its card or album page when you like.`,
          );
          setAddOpen(false);
        }}
      />
      {listening && (
        <ListenDialog
          albumTitle={listening.title}
          editionId={listening.editionId}
          onClose={() => {
            setListening(null);
          }}
          onLogged={(logged) => {
            setListening(null);
            setStatus(null);
            setUndo(logged);
          }}
        />
      )}
    </section>
  );
}
