-- v8: Next Up sessions gain a stored pool cycle, an end marker, and a 'manual' mode.
-- Changing a CHECK needs SQLite's table-rebuild procedure; the runner disables foreign
-- keys around migrations and verifies them before commit, so attempts are kept intact.
-- Sessions from before v8 are closed; the next roll starts a fresh pool.

CREATE TABLE selection_session_v8 (
    id          TEXT PRIMARY KEY NOT NULL,
    mode        TEXT NOT NULL CHECK (mode IN ('completely_random', 'weighted_random', 'guided', 'manual')),
    criteria    TEXT CHECK (criteria IS NULL OR json_valid(criteria)),
    pool_cycle  INTEGER NOT NULL DEFAULT 1 CHECK (pool_cycle >= 1),
    started_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ended_at    TEXT
) STRICT;

INSERT INTO selection_session_v8 (id, mode, criteria, pool_cycle, started_at, ended_at)
SELECT s.id, s.mode, s.criteria,
       COALESCE((SELECT MAX(a.pool_cycle) FROM selection_attempt a WHERE a.session_id = s.id), 1),
       s.started_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
FROM selection_session s;

DROP TABLE selection_session;
ALTER TABLE selection_session_v8 RENAME TO selection_session;

-- At most one open reroll session (manual picks are single-attempt sessions).
CREATE UNIQUE INDEX selection_session_open_uq ON selection_session ((ended_at IS NULL))
    WHERE ended_at IS NULL AND mode != 'manual';
