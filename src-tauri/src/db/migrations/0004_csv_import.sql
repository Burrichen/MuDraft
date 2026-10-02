-- v4: staged CSV import. Staging rows are working state, not library data: discarding a
-- session deletes them and never touches albums, editions, or personal data.

CREATE TABLE import_session (
    id           TEXT PRIMARY KEY NOT NULL,
    file_name    TEXT NOT NULL,
    headers      TEXT NOT NULL CHECK (json_valid(headers)),
    -- Column index per field, e.g. {"album": 0, "artist": 1}; NULL until mapped.
    mapping      TEXT CHECK (mapping IS NULL OR json_valid(mapping)),
    status       TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'committed')),
    report       TEXT CHECK (report IS NULL OR json_valid(report)),
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TABLE import_row (
    session_id    TEXT NOT NULL REFERENCES import_session (id) ON DELETE CASCADE,
    row_number    INTEGER NOT NULL CHECK (row_number >= 2), -- spreadsheet row (header is 1)
    raw           TEXT NOT NULL CHECK (json_valid(raw)),
    -- Normalized, validated fields and row-level problems (JSON), set when mapped.
    fields        TEXT CHECK (fields IS NULL OR json_valid(fields)),
    errors        TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(errors)),
    fingerprint   TEXT,
    duplicate_of  INTEGER,
    -- How the row will be committed; see csv_import::Decision.
    decision      TEXT CHECK (decision IS NULL OR json_valid(decision)),
    -- Online enrichment is optional and resumable, one row at a time.
    enrichment    TEXT NOT NULL DEFAULT 'not_started'
                  CHECK (enrichment IN ('not_started', 'resolved', 'candidates', 'no_results', 'not_needed', 'failed')),
    candidates    TEXT CHECK (candidates IS NULL OR json_valid(candidates)),
    -- Provider details fetched for a match decision, so commit needs no network.
    details       TEXT CHECK (details IS NULL OR json_valid(details)),
    PRIMARY KEY (session_id, row_number)
) STRICT;
CREATE INDEX import_row_enrichment_idx ON import_row (session_id, enrichment);

-- Rows already committed, by normalized content, so re-importing the same file maps each
-- row to the records it created the first time instead of duplicating them.
CREATE TABLE imported_row (
    fingerprint  TEXT PRIMARY KEY NOT NULL,
    album_id     TEXT NOT NULL REFERENCES album (id) ON DELETE CASCADE,
    edition_id   TEXT NOT NULL REFERENCES edition (id) ON DELETE CASCADE,
    imported_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;
CREATE INDEX imported_row_edition_idx ON imported_row (edition_id);
