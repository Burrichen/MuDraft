-- v3: MusicBrainz metadata support.

-- When provider data for a field was fetched (NULL for manual entries).
ALTER TABLE metadata_provenance ADD COLUMN fetched_at TEXT;

-- Distinguishes same-name artists (e.g. MusicBrainz "UK punk band"). Never used to merge.
ALTER TABLE artist ADD COLUMN disambiguation TEXT;

-- Raw provider genres/tags exactly as received, kept apart from user tags (album_tag)
-- and from normalized broad genres (album_genre). Replaced wholesale on refresh.
CREATE TABLE album_source_tag (
    album_id    TEXT NOT NULL REFERENCES album (id) ON DELETE CASCADE,
    source      TEXT NOT NULL CHECK (source IN ('musicbrainz')),
    kind        TEXT NOT NULL CHECK (kind IN ('genre', 'tag')),
    name        TEXT NOT NULL CHECK (length(name) > 0),
    votes       INTEGER NOT NULL DEFAULT 0,
    fetched_at  TEXT NOT NULL,
    PRIMARY KEY (album_id, source, kind, name)
) STRICT;

-- Provider response cache: lets repeat lookups skip the network and serves stale
-- results when offline. Keyed by the full request URL.
CREATE TABLE metadata_cache (
    key         TEXT PRIMARY KEY NOT NULL,
    body        TEXT NOT NULL,
    fetched_at  TEXT NOT NULL,
    expires_at  TEXT NOT NULL
) STRICT;
CREATE INDEX metadata_cache_expires_idx ON metadata_cache (expires_at);
