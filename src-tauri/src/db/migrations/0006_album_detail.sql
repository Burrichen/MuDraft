-- v6: album descriptions and cached artwork.

-- Sourced (MusicBrainz annotation) or user-written text. Provenance records which; a
-- manual edit is a locked override that refreshes never replace.
ALTER TABLE album ADD COLUMN description TEXT;

-- One artwork record per owner. Edition art is preferred; album (release group) art is
-- the labelled canonical fallback. Files live in <data dir>/artwork; only names are stored.
--   cover_art_archive  downloaded and cached
--   local              user-supplied replacement (never replaced by downloads)
--   removed            user removed artwork for this edition (show placeholder, never refetch)
--   none               Cover Art Archive had nothing (checked_at limits re-checks)
CREATE TABLE artwork (
    owner_type  TEXT NOT NULL CHECK (owner_type IN ('album', 'edition')),
    owner_id    TEXT NOT NULL,
    source      TEXT NOT NULL CHECK (source IN ('cover_art_archive', 'local', 'removed', 'none')),
    file_name   TEXT,
    mime        TEXT CHECK (mime IS NULL OR mime IN ('image/jpeg', 'image/png', 'image/gif', 'image/webp')),
    bytes       INTEGER CHECK (bytes IS NULL OR bytes > 0),
    source_url  TEXT,
    checked_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (owner_type, owner_id),
    CHECK ((source IN ('cover_art_archive', 'local')) = (file_name IS NOT NULL AND mime IS NOT NULL AND bytes IS NOT NULL))
) STRICT;

CREATE TRIGGER album_artwork_cleanup AFTER DELETE ON album BEGIN
    DELETE FROM artwork WHERE owner_type = 'album' AND owner_id = OLD.id;
END;
CREATE TRIGGER edition_artwork_cleanup AFTER DELETE ON edition BEGIN
    DELETE FROM artwork WHERE owner_type = 'edition' AND owner_id = OLD.id;
END;
