-- v10: keep each catalogue entry's credited artists (with MusicBrainz IDs) so
-- collaborators can be linked to their own artist pages.
ALTER TABLE catalogue_entry ADD COLUMN credit_parts TEXT NOT NULL DEFAULT '[]'
    CHECK (json_valid(credit_parts));
