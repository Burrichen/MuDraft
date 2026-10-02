-- v9: artist discography catalogues discovered online, kept apart from the library.
-- Nothing here adds albums to the Listen List or Collection.

-- Fetch state per artist. A pass pages through the provider from next_offset, so an
-- interrupted pass resumes where it stopped. completed_at records the last full pass.
CREATE TABLE artist_catalogue (
    artist_id              TEXT PRIMARY KEY NOT NULL REFERENCES artist (id) ON DELETE CASCADE,
    musicbrainz_artist_id  TEXT,
    provider_total         INTEGER CHECK (provider_total >= 0),
    next_offset            INTEGER NOT NULL DEFAULT 0 CHECK (next_offset >= 0),
    started_at             TEXT,
    page_fetched_at        TEXT,
    completed_at           TEXT,
    last_error             TEXT,
    -- Progress scope (types counted); NULL = the default, official studio albums.
    scope                  TEXT CHECK (scope IS NULL OR json_valid(scope))
) STRICT;

-- Canonical albums in an artist's catalogue: discovered online, or added by the user.
CREATE TABLE catalogue_entry (
    id                            TEXT PRIMARY KEY NOT NULL,
    artist_id                     TEXT NOT NULL REFERENCES artist (id) ON DELETE CASCADE,
    source                        TEXT NOT NULL CHECK (source IN ('musicbrainz', 'manual')),
    musicbrainz_release_group_id  TEXT,
    title                         TEXT NOT NULL CHECK (length(trim(title)) > 0),
    disambiguation                TEXT,
    primary_type                  TEXT,
    secondary_types               TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(secondary_types)),
    original_date                 TEXT,
    original_year                 INTEGER,
    credit                        TEXT NOT NULL DEFAULT '',
    credit_artists                INTEGER NOT NULL DEFAULT 1 CHECK (credit_artists >= 0),
    fetched_at                    TEXT,
    -- User correction: shown but never counted. Survives refetches.
    excluded                      INTEGER NOT NULL DEFAULT 0 CHECK (excluded IN (0, 1)),
    created_at                    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK ((source = 'musicbrainz') = (musicbrainz_release_group_id IS NOT NULL)),
    UNIQUE (artist_id, musicbrainz_release_group_id)
) STRICT;
CREATE INDEX catalogue_entry_group_idx ON catalogue_entry (musicbrainz_release_group_id);

-- The representative edition chosen for a release group the user has no edition of.
-- track_count stays NULL until its tracklist is loaded: unknown, never zero.
CREATE TABLE catalogue_reference (
    musicbrainz_release_group_id  TEXT PRIMARY KEY NOT NULL,
    musicbrainz_release_id        TEXT,
    title                         TEXT,
    release_date                  TEXT,
    country                       TEXT,
    formats                       TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(formats)),
    editions_considered           INTEGER NOT NULL DEFAULT 0,
    editions_fetched_at           TEXT,
    track_count                   INTEGER CHECK (track_count >= 0),
    tracks_fetched_at             TEXT,
    last_error                    TEXT
) STRICT;

-- Tracklist of a representative release (not imported as library tracks).
CREATE TABLE catalogue_track (
    musicbrainz_release_id    TEXT NOT NULL,
    disc                      INTEGER NOT NULL CHECK (disc >= 1),
    position                  INTEGER NOT NULL CHECK (position >= 1),
    title                     TEXT NOT NULL,
    length_ms                 INTEGER,
    musicbrainz_track_id      TEXT,
    musicbrainz_recording_id  TEXT,
    PRIMARY KEY (musicbrainz_release_id, disc, position)
) STRICT;
CREATE INDEX catalogue_track_recording_idx ON catalogue_track (musicbrainz_recording_id);
