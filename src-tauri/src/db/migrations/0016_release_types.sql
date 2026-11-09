-- v16: library albums remember their release-group type, so Single and Live albums never
-- count toward an artist's progress; and interviews, audiobooks, and audio dramas leave the
-- fetched online catalogues (they are re-fetchable data and are no longer stored).

-- NULL = type unknown (added by hand, or before types were recorded).
ALTER TABLE album ADD COLUMN primary_type TEXT;
ALTER TABLE album ADD COLUMN secondary_types TEXT
    CHECK (secondary_types IS NULL OR json_valid(secondary_types));

-- Backfill from the artist catalogue already fetched for the same release group…
UPDATE album SET
    primary_type = (SELECT c.primary_type FROM catalogue_entry c
                    WHERE c.musicbrainz_release_group_id = album.musicbrainz_release_group_id
                    ORDER BY c.fetched_at DESC LIMIT 1),
    secondary_types = (SELECT c.secondary_types FROM catalogue_entry c
                       WHERE c.musicbrainz_release_group_id = album.musicbrainz_release_group_id
                       ORDER BY c.fetched_at DESC LIMIT 1)
WHERE musicbrainz_release_group_id IS NOT NULL
  AND EXISTS (SELECT 1 FROM catalogue_entry c
              WHERE c.musicbrainz_release_group_id = album.musicbrainz_release_group_id);

-- …or from a saved release-group lookup (the raw response the album was imported from).
UPDATE album SET
    primary_type = (SELECT json_extract(m.body, '$."primary-type"') FROM metadata_cache m
                    WHERE m.key LIKE '%/release-group/' || album.musicbrainz_release_group_id || '?%'
                      AND json_valid(m.body)
                      AND json_type(m.body, '$."secondary-types"') = 'array'
                    ORDER BY m.fetched_at DESC LIMIT 1),
    secondary_types = (SELECT json_extract(m.body, '$."secondary-types"') FROM metadata_cache m
                       WHERE m.key LIKE '%/release-group/' || album.musicbrainz_release_group_id || '?%'
                         AND json_valid(m.body)
                         AND json_type(m.body, '$."secondary-types"') = 'array'
                       ORDER BY m.fetched_at DESC LIMIT 1)
WHERE musicbrainz_release_group_id IS NOT NULL
  AND secondary_types IS NULL
  AND EXISTS (SELECT 1 FROM metadata_cache m
              WHERE m.key LIKE '%/release-group/' || album.musicbrainz_release_group_id || '?%'
                AND json_valid(m.body)
                AND json_type(m.body, '$."secondary-types"') = 'array');

-- Fetched catalogue entries for spoken-word releases go (manual entries and albums stay).
DELETE FROM catalogue_entry
WHERE source = 'musicbrainz'
  AND EXISTS (SELECT 1 FROM json_each(catalogue_entry.secondary_types)
              WHERE lower(value) IN ('interview', 'audiobook', 'audio drama'));
