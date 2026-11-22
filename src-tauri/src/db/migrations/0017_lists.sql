-- v17: Lists — curated album lists (for example a magazine's "100 best" ranking) tracked
-- against the library. An entry is the list's own record (artist, title, year as the
-- source wrote them); whether it is listened to is derived live from the Collection, so
-- no copy of library state is stored here.
CREATE TABLE list (
    id          TEXT PRIMARY KEY NOT NULL,
    name        TEXT NOT NULL CHECK (length(trim(name)) > 0),
    -- Page the list was imported from, if any.
    source_url  TEXT,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TABLE list_entry (
    id           TEXT PRIMARY KEY NOT NULL,
    list_id      TEXT NOT NULL REFERENCES list (id) ON DELETE CASCADE,
    -- Rank or order within the list (1 = first). Display order only; not unique.
    position     INTEGER NOT NULL CHECK (position >= 1),
    artist_name  TEXT NOT NULL CHECK (length(trim(artist_name)) > 0),
    title        TEXT NOT NULL CHECK (length(trim(title)) > 0),
    year         INTEGER CHECK (year IS NULL OR year BETWEEN 1000 AND 9999),
    -- Library album this entry was created for (set when it is added to the Listen List).
    -- Entries are also matched to library albums by artist and title.
    album_id     TEXT REFERENCES album (id) ON DELETE SET NULL,
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;
CREATE INDEX list_entry_list_idx ON list_entry (list_id, position);
CREATE INDEX list_entry_album_idx ON list_entry (album_id);
