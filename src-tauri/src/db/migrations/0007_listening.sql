-- v7: listen logging details.

-- What the user said about this listen. 'prior_history' marks "listened before, dates
-- unknown" (one per edition; counted as history, not as a number of listens).
ALTER TABLE listen_event ADD COLUMN kind TEXT NOT NULL DEFAULT 'unspecified'
    CHECK (kind IN ('first', 'relisten', 'unspecified', 'prior_history'));

-- 'tracks': listen_event_track rows say exactly which tracks were covered.
-- 'unknown': the edition had no tracklist when logged; coverage is only inferred later
-- if the user confirms it.
ALTER TABLE listen_event ADD COLUMN coverage TEXT NOT NULL DEFAULT 'tracks'
    CHECK (coverage IN ('tracks', 'unknown'));

-- A listen completed from a Next Up pick.
ALTER TABLE listen_event ADD COLUMN selection_attempt_id TEXT
    REFERENCES selection_attempt (id) ON DELETE SET NULL;

-- Soft delete so a deleted listen can be undone. Deleted listens count for nothing.
ALTER TABLE listen_event ADD COLUMN deleted_at TEXT;
ALTER TABLE listen_event ADD COLUMN updated_at TEXT;

CREATE INDEX listen_event_live_idx ON listen_event (album_id, deleted_at, listened_on);
