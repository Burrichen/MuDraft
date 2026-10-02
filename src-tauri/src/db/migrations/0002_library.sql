-- v2: music library, personal data, and Next Up selection history.
-- IDs are stable local UUIDv7 text; provider IDs are nullable and unique when present.
-- Audit timestamps (created_at/updated_at) are UTC RFC 3339. Listening dates are local
-- calendar dates ('YYYY-MM-DD'), never UTC-midnight conversions.
-- Partial dates carry an explicit precision: unknown | year | month | day.

-- ---------------------------------------------------------------- catalogue

CREATE TABLE artist (
    id              TEXT PRIMARY KEY NOT NULL,
    name            TEXT NOT NULL CHECK (length(trim(name)) > 0),
    sort_name       TEXT,
    musicbrainz_id  TEXT UNIQUE,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;
CREATE INDEX artist_name_idx ON artist (name COLLATE NOCASE);

-- Broad genres (2–3 per album after normalization).
CREATE TABLE genre (
    id          TEXT PRIMARY KEY NOT NULL,
    name        TEXT NOT NULL CHECK (length(trim(name)) > 0),
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;
CREATE UNIQUE INDEX genre_name_uq ON genre (name COLLATE NOCASE);

-- Canonical album (release group). Original date is independent of any edition's date.
CREATE TABLE album (
    id                        TEXT PRIMARY KEY NOT NULL,
    title                     TEXT NOT NULL CHECK (length(trim(title)) > 0),
    original_date             TEXT,
    original_date_precision   TEXT NOT NULL DEFAULT 'unknown',
    original_year             INTEGER GENERATED ALWAYS AS (CAST(substr(original_date, 1, 4) AS INTEGER)) VIRTUAL,
    musicbrainz_release_group_id TEXT UNIQUE,
    created_at                TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at                TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (
        (original_date_precision = 'unknown' AND original_date IS NULL)
        OR (original_date_precision = 'year' AND original_date GLOB '[0-9][0-9][0-9][0-9]')
        OR (original_date_precision = 'month' AND original_date GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]')
        OR (original_date_precision = 'day' AND date(original_date) IS original_date)
    )
) STRICT;
CREATE INDEX album_title_idx ON album (title COLLATE NOCASE);
CREATE INDEX album_original_year_idx ON album (original_year);

-- Ordered artist credits; collaborations are multiple positions.
CREATE TABLE album_artist_credit (
    album_id       TEXT NOT NULL REFERENCES album (id) ON DELETE CASCADE,
    position       INTEGER NOT NULL CHECK (position >= 0),
    artist_id      TEXT NOT NULL REFERENCES artist (id) ON DELETE RESTRICT,
    credited_name  TEXT,
    join_phrase    TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (album_id, position),
    UNIQUE (album_id, artist_id)
) STRICT;
CREATE INDEX album_artist_credit_artist_idx ON album_artist_credit (artist_id);

CREATE TABLE album_genre (
    album_id  TEXT NOT NULL REFERENCES album (id) ON DELETE CASCADE,
    genre_id  TEXT NOT NULL REFERENCES genre (id) ON DELETE RESTRICT,
    position  INTEGER NOT NULL CHECK (position BETWEEN 0 AND 2),
    PRIMARY KEY (album_id, genre_id),
    UNIQUE (album_id, position)
) STRICT;
CREATE INDEX album_genre_genre_idx ON album_genre (genre_id);

-- User-distinguished editions (Standard, Deluxe, …). Ordinary pressings share one edition:
-- names are unique per album and a provider release maps to at most one edition.
CREATE TABLE edition (
    id                      TEXT PRIMARY KEY NOT NULL,
    album_id                TEXT NOT NULL REFERENCES album (id) ON DELETE RESTRICT,
    name                    TEXT NOT NULL CHECK (length(trim(name)) > 0),
    release_date            TEXT,
    release_date_precision  TEXT NOT NULL DEFAULT 'unknown',
    release_year            INTEGER GENERATED ALWAYS AS (CAST(substr(release_date, 1, 4) AS INTEGER)) VIRTUAL,
    musicbrainz_release_id  TEXT UNIQUE,
    created_at              TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at              TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (id, album_id),
    CHECK (
        (release_date_precision = 'unknown' AND release_date IS NULL)
        OR (release_date_precision = 'year' AND release_date GLOB '[0-9][0-9][0-9][0-9]')
        OR (release_date_precision = 'month' AND release_date GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]')
        OR (release_date_precision = 'day' AND date(release_date) IS release_date)
    )
) STRICT;
CREATE UNIQUE INDEX edition_album_name_uq ON edition (album_id, name COLLATE NOCASE);

-- Optional identity shared by the same recording across editions.
CREATE TABLE recording (
    id                        TEXT PRIMARY KEY NOT NULL,
    title                     TEXT NOT NULL CHECK (length(trim(title)) > 0),
    length_ms                 INTEGER CHECK (length_ms > 0),
    musicbrainz_recording_id  TEXT UNIQUE,
    created_at                TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TABLE track (
    id                     TEXT PRIMARY KEY NOT NULL,
    edition_id             TEXT NOT NULL REFERENCES edition (id) ON DELETE CASCADE,
    disc_number            INTEGER NOT NULL DEFAULT 1 CHECK (disc_number >= 1),
    position               INTEGER NOT NULL CHECK (position >= 1),
    title                  TEXT NOT NULL CHECK (length(trim(title)) > 0),
    length_ms              INTEGER CHECK (length_ms > 0),
    recording_id           TEXT REFERENCES recording (id) ON DELETE SET NULL,
    musicbrainz_track_id   TEXT UNIQUE,
    UNIQUE (edition_id, disc_number, position),
    UNIQUE (id, edition_id)
) STRICT;
CREATE INDEX track_recording_idx ON track (recording_id);

CREATE TABLE track_artist_credit (
    track_id       TEXT NOT NULL REFERENCES track (id) ON DELETE CASCADE,
    position       INTEGER NOT NULL CHECK (position >= 0),
    artist_id      TEXT NOT NULL REFERENCES artist (id) ON DELETE RESTRICT,
    credited_name  TEXT,
    join_phrase    TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (track_id, position),
    UNIQUE (track_id, artist_id)
) STRICT;
CREATE INDEX track_artist_credit_artist_idx ON track_artist_credit (artist_id);

-- Tags belong to canonical albums, so they follow the album across editions and lists.
CREATE TABLE tag (
    id           TEXT PRIMARY KEY NOT NULL,
    name         TEXT NOT NULL CHECK (length(trim(name)) > 0),
    builtin_key  TEXT UNIQUE,
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;
CREATE UNIQUE INDEX tag_name_uq ON tag (name COLLATE NOCASE);

INSERT INTO tag (id, name, builtin_key)
VALUES ('00000000-0000-7000-8000-000000000001', 'Listen ASAP', 'listen_asap');

CREATE TABLE album_tag (
    album_id    TEXT NOT NULL REFERENCES album (id) ON DELETE CASCADE,
    tag_id      TEXT NOT NULL REFERENCES tag (id) ON DELETE CASCADE,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (album_id, tag_id)
) STRICT;
CREATE INDEX album_tag_tag_idx ON album_tag (tag_id);

-- Field-level provenance and user overrides. The owning row is checked in Rust and
-- cleaned up by triggers, because the reference is polymorphic.
CREATE TABLE metadata_provenance (
    entity_type     TEXT NOT NULL CHECK (entity_type IN ('artist', 'album', 'edition', 'track')),
    entity_id       TEXT NOT NULL,
    field           TEXT NOT NULL CHECK (length(field) > 0),
    source          TEXT NOT NULL CHECK (source IN ('manual', 'musicbrainz', 'csv', 'cover_art_archive')),
    source_ref      TEXT,
    provider_value  TEXT,
    is_override     INTEGER NOT NULL DEFAULT 0 CHECK (is_override IN (0, 1)),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (entity_type, entity_id, field)
) STRICT;

CREATE TRIGGER artist_provenance_cleanup AFTER DELETE ON artist BEGIN
    DELETE FROM metadata_provenance WHERE entity_type = 'artist' AND entity_id = OLD.id;
END;
CREATE TRIGGER album_provenance_cleanup AFTER DELETE ON album BEGIN
    DELETE FROM metadata_provenance WHERE entity_type = 'album' AND entity_id = OLD.id;
END;
CREATE TRIGGER edition_provenance_cleanup AFTER DELETE ON edition BEGIN
    DELETE FROM metadata_provenance WHERE entity_type = 'edition' AND entity_id = OLD.id;
END;
CREATE TRIGGER track_provenance_cleanup AFTER DELETE ON track BEGIN
    DELETE FROM metadata_provenance WHERE entity_type = 'track' AND entity_id = OLD.id;
END;

-- ---------------------------------------------------------------- personal data
-- Composite (edition_id, album_id) foreign keys guarantee the edition belongs to the album.

-- One Listen List entry per canonical album, pointing at the chosen edition.
CREATE TABLE listen_list_entry (
    album_id    TEXT PRIMARY KEY NOT NULL,
    edition_id  TEXT NOT NULL,
    added_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY (edition_id, album_id) REFERENCES edition (id, album_id) ON DELETE RESTRICT
) STRICT;
CREATE INDEX listen_list_entry_edition_idx ON listen_list_entry (edition_id, album_id);

-- Collection membership per edition; repeated listens never add duplicates.
CREATE TABLE collection_entry (
    edition_id  TEXT PRIMARY KEY NOT NULL,
    album_id    TEXT NOT NULL,
    source      TEXT NOT NULL CHECK (source IN ('manual', 'listen')),
    added_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY (edition_id, album_id) REFERENCES edition (id, album_id) ON DELETE RESTRICT
) STRICT;
CREATE INDEX collection_entry_album_idx ON collection_entry (album_id);

-- A listen: known local date, or 'undated' prior history.
CREATE TABLE listen_event (
    id           TEXT PRIMARY KEY NOT NULL,
    edition_id   TEXT NOT NULL,
    album_id     TEXT NOT NULL,
    listened_on  TEXT,
    date_status  TEXT NOT NULL CHECK (date_status IN ('known', 'undated')),
    is_full      INTEGER NOT NULL CHECK (is_full IN (0, 1)),
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY (edition_id, album_id) REFERENCES edition (id, album_id) ON DELETE RESTRICT,
    UNIQUE (id, edition_id),
    CHECK (
        (date_status = 'known' AND date(listened_on) IS listened_on)
        OR (date_status = 'undated' AND listened_on IS NULL)
    )
) STRICT;
CREATE INDEX listen_event_edition_idx ON listen_event (edition_id, album_id);
CREATE INDEX listen_event_album_date_idx ON listen_event (album_id, listened_on);

-- Tracks heard in a listen; must belong to the event's edition.
CREATE TABLE listen_event_track (
    listen_event_id  TEXT NOT NULL,
    edition_id       TEXT NOT NULL,
    track_id         TEXT NOT NULL,
    PRIMARY KEY (listen_event_id, track_id),
    FOREIGN KEY (listen_event_id, edition_id) REFERENCES listen_event (id, edition_id) ON DELETE CASCADE,
    FOREIGN KEY (track_id, edition_id) REFERENCES track (id, edition_id) ON DELETE RESTRICT
) STRICT;
CREATE INDEX listen_event_track_track_idx ON listen_event_track (track_id, edition_id);

-- Ratings are half-stars 0..10; NULL is unrated and 0 is a real rating.
-- Personal rows RESTRICT catalogue deletes so user data is never cascaded away.
CREATE TABLE track_rating (
    track_id      TEXT PRIMARY KEY NOT NULL REFERENCES track (id) ON DELETE RESTRICT,
    rating        INTEGER CHECK (rating BETWEEN 0 AND 10),
    is_favourite  INTEGER NOT NULL DEFAULT 0 CHECK (is_favourite IN (0, 1)),
    updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- Explicit album rating and review for the selected edition. Calculated ratings are never stored.
CREATE TABLE album_review (
    edition_id  TEXT PRIMARY KEY NOT NULL,
    album_id    TEXT NOT NULL,
    rating      INTEGER CHECK (rating BETWEEN 0 AND 10),
    review      TEXT,
    updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY (edition_id, album_id) REFERENCES edition (id, album_id) ON DELETE RESTRICT
) STRICT;
CREATE INDEX album_review_album_idx ON album_review (album_id);

-- Typed in Rust; value is JSON.
CREATE TABLE setting (
    key         TEXT PRIMARY KEY NOT NULL,
    value       TEXT NOT NULL CHECK (json_valid(value)),
    updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- ---------------------------------------------------------------- Next Up

CREATE TABLE selection_session (
    id          TEXT PRIMARY KEY NOT NULL,
    mode        TEXT NOT NULL CHECK (mode IN ('completely_random', 'weighted_random', 'guided')),
    criteria    TEXT CHECK (criteria IS NULL OR json_valid(criteria)),
    started_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- Each result shown in a session. pool_cycle increments once the pool is exhausted,
-- so an album repeats only in a later cycle.
CREATE TABLE selection_attempt (
    id           TEXT PRIMARY KEY NOT NULL,
    session_id   TEXT NOT NULL REFERENCES selection_session (id) ON DELETE CASCADE,
    sequence     INTEGER NOT NULL CHECK (sequence >= 1),
    pool_cycle   INTEGER NOT NULL DEFAULT 1 CHECK (pool_cycle >= 1),
    album_id     TEXT NOT NULL,
    edition_id   TEXT NOT NULL,
    source       TEXT NOT NULL CHECK (source IN ('completely_random', 'weighted_random', 'guided', 'manual')),
    status       TEXT NOT NULL DEFAULT 'shown' CHECK (status IN ('shown', 'skipped', 'completed')),
    shown_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    resolved_at  TEXT,
    FOREIGN KEY (edition_id, album_id) REFERENCES edition (id, album_id) ON DELETE RESTRICT,
    UNIQUE (session_id, sequence),
    UNIQUE (session_id, pool_cycle, album_id),
    CHECK ((status = 'shown') = (resolved_at IS NULL))
) STRICT;
CREATE INDEX selection_attempt_edition_idx ON selection_attempt (edition_id, album_id);

-- Singleton: at most one current album.
CREATE TABLE current_selection (
    slot        INTEGER PRIMARY KEY NOT NULL CHECK (slot = 1),
    attempt_id  TEXT NOT NULL UNIQUE REFERENCES selection_attempt (id) ON DELETE CASCADE,
    set_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- ---------------------------------------------------------------- idempotency

-- Caller-supplied mutation keys: a retried request with the same key and payload returns
-- the original result instead of applying twice; a different payload is a conflict.
CREATE TABLE applied_mutation (
    key         TEXT PRIMARY KEY NOT NULL,
    kind        TEXT NOT NULL,
    request     TEXT NOT NULL CHECK (json_valid(request)),
    result      TEXT NOT NULL CHECK (json_valid(result)),
    applied_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;
