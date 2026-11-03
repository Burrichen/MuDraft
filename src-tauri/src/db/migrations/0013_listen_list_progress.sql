-- v13: Listen List progress. Tracks the user has ticked off while working through an
-- album on the Listen List. Not listen evidence (that is listen_event): it goes away with
-- the List entry, and is cleared when the album is logged as listened.
CREATE TABLE listen_list_track (
    album_id   TEXT NOT NULL REFERENCES listen_list_entry (album_id) ON DELETE CASCADE,
    track_id   TEXT NOT NULL REFERENCES track (id) ON DELETE CASCADE,
    marked_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (album_id, track_id)
) STRICT;
CREATE INDEX listen_list_track_track_idx ON listen_list_track (track_id);
