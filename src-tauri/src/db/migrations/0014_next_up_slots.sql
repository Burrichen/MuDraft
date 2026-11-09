-- v14: up to five Next Up slots, each with its own picking method, reroll pool, and
-- current album. Every session records its slot; existing sessions belong to slot 1,
-- so the open session and current pick carry over as the first slot.

ALTER TABLE selection_session
    ADD COLUMN slot INTEGER NOT NULL DEFAULT 1 CHECK (slot BETWEEN 1 AND 5);

-- At most one open reroll session per slot (manual picks are single-attempt sessions).
DROP INDEX selection_session_open_uq;
CREATE UNIQUE INDEX selection_session_open_uq ON selection_session (slot)
    WHERE ended_at IS NULL AND mode != 'manual';

-- One current album per slot (was a singleton). Changing a CHECK needs a table rebuild;
-- nothing references this table.
CREATE TABLE current_selection_v14 (
    slot        INTEGER PRIMARY KEY NOT NULL CHECK (slot BETWEEN 1 AND 5),
    attempt_id  TEXT NOT NULL UNIQUE REFERENCES selection_attempt (id) ON DELETE CASCADE,
    set_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

INSERT INTO current_selection_v14 (slot, attempt_id, set_at)
SELECT slot, attempt_id, set_at FROM current_selection;

DROP TABLE current_selection;
ALTER TABLE current_selection_v14 RENAME TO current_selection;
