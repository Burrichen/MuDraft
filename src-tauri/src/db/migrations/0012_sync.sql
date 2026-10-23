-- v12: GitHub sync bookkeeping for this computer. Never exported in profiles or synced.
-- The access token is NOT stored here; it lives in the OS secure store.
CREATE TABLE sync_state (
    slot                 INTEGER PRIMARY KEY NOT NULL CHECK (slot = 1),
    owner                TEXT NOT NULL,
    repo                 TEXT NOT NULL,
    -- The repository's default branch, learned when the remote is first read.
    branch               TEXT,
    -- Identity of the remote sync data this computer is linked to (format.json syncId).
    sync_id              TEXT,
    -- The last common state applied here and the remote commit it came from.
    base_revision        TEXT,
    base_state           TEXT,
    last_synced_at       TEXT,
    last_error           TEXT CHECK (last_error IS NULL OR json_valid(last_error)),
    -- Conflicts found by the last sync attempt and not yet resolved.
    conflicts            INTEGER NOT NULL DEFAULT 0 CHECK (conflicts >= 0),
    updated_at           TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK ((base_revision IS NULL) = (base_state IS NULL))
) STRICT;
