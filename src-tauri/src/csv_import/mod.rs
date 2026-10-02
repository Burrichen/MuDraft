//! Staged Custom CSV import into the Listen List.
//!
//! 1. **Parse** (local, offline): decode, read records, keep row-level problems.
//! 2. **Map** columns to fields; validate each row; detect duplicates in the file and in
//!    the library; choose a safe default decision per row. Nothing in the library changes.
//! 3. **Enrich** (optional, online): look rows up on MusicBrainz a few at a time through
//!    the shared provider queue. Progress is stored per row, so it can pause and resume.
//! 4. **Review**: the user changes matches, skips rows, or keeps rows as manual entries.
//! 5. **Commit**: one transaction, after unknown tags are confirmed. Discarding before
//!    commit deletes only the staging rows.
//!
//! ## CSV format
//! UTF-8 (with or without BOM) or UTF-16 with BOM, comma-separated, RFC 4180 quoting
//! (quoted fields may contain commas, quotes as `""`, and newlines). The first row is a
//! header. Required columns: album, artist. Optional: year, edition, tags, MusicBrainz
//! release group ID, MusicBrainz release ID; missing optional columns are fine.
//! - **Tags** are separated by semicolons (`Listen ASAP; road trip`), so tags may contain
//!   commas. A tag cannot contain `;`.
//! - **Artist** is taken exactly as written. It is never split on `&`, `,`, `feat.`, or
//!   anything else to guess collaborations.

pub mod commit;
pub mod enrich;
pub mod mapping;
pub mod parse;
pub mod staging;

pub const MAX_FILE_BYTES: usize = 10 * 1024 * 1024;
pub const MAX_ROWS: usize = 20_000;
pub const TAG_DELIMITER: char = ';';

pub const TEMPLATE_HEADERS: [&str; 7] = [
    "Album",
    "Artist",
    "Year",
    "Edition",
    "Tags",
    "MusicBrainz Release Group ID",
    "MusicBrainz Release ID",
];

/// Header-only template (no sample albums that could be imported by accident).
pub fn template_csv() -> String {
    // BOM so spreadsheet apps open it as UTF-8; CRLF per RFC 4180.
    format!("\u{feff}{}\r\n", TEMPLATE_HEADERS.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_round_trips_through_the_parser_and_mapping() {
        let parsed = parse::parse(template_csv().as_bytes()).unwrap();
        assert_eq!(parsed.headers, TEMPLATE_HEADERS);
        assert!(parsed.records.is_empty());
        let m = mapping::suggest(&parsed.headers);
        assert_eq!(m.album, Some(0));
        assert_eq!(m.artist, Some(1));
        assert_eq!(m.release_group_id, Some(5));
        assert_eq!(m.release_id, Some(6));
    }
}
