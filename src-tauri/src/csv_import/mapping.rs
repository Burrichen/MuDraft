//! Column mapping and row validation. Pure and deterministic.

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use super::TAG_DELIMITER;
use crate::domain::ids::parse_uuid;
use crate::error::{AppError, AppResult};

/// Column index for each field; `None` means the file has no such column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColumnMapping {
    pub album: Option<usize>,
    pub artist: Option<usize>,
    pub year: Option<usize>,
    pub edition: Option<usize>,
    pub tags: Option<usize>,
    pub release_group_id: Option<usize>,
    pub release_id: Option<usize>,
}

impl ColumnMapping {
    fn entries(&self) -> [(&'static str, Option<usize>); 7] {
        [
            ("album", self.album),
            ("artist", self.artist),
            ("year", self.year),
            ("edition", self.edition),
            ("tags", self.tags),
            ("release group ID", self.release_group_id),
            ("release ID", self.release_id),
        ]
    }

    pub fn validate(&self, column_count: usize) -> AppResult<()> {
        if self.album.is_none() || self.artist.is_none() {
            return Err(AppError::validation(
                "column mapping",
                "choose the Album and Artist columns",
            ));
        }
        let mut used = Vec::new();
        for (field, index) in self.entries() {
            let Some(i) = index else { continue };
            if i >= column_count {
                return Err(AppError::validation(
                    "column mapping",
                    format!("{field} points past the last column"),
                ));
            }
            if used.contains(&i) {
                return Err(AppError::validation(
                    "column mapping",
                    format!("column {} is used for two fields", i + 1),
                ));
            }
            used.push(i);
        }
        Ok(())
    }
}

const ALIASES: &[(&str, &[&str])] = &[
    (
        "album",
        &[
            "album",
            "albumtitle",
            "albumname",
            "title",
            "record",
            "releasetitle",
        ],
    ),
    (
        "artist",
        &["artist", "albumartist", "artists", "artistname", "band"],
    ),
    (
        "year",
        &["year", "releaseyear", "originalyear", "originalreleaseyear"],
    ),
    ("edition", &["edition", "editionname", "version"]),
    ("tags", &["tags", "tag"]),
    (
        "rgid",
        &[
            "musicbrainzreleasegroupid",
            "mbreleasegroupid",
            "releasegroupid",
            "rgid",
            "mbrgid",
        ],
    ),
    (
        "relid",
        &["musicbrainzreleaseid", "mbreleaseid", "releaseid", "mbid"],
    ),
];

fn header_key(h: &str) -> String {
    h.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// Suggest a mapping from header names. The user reviews it before rows are staged.
pub fn suggest(headers: &[String]) -> ColumnMapping {
    let keys: Vec<String> = headers.iter().map(|h| header_key(h)).collect();
    let find = |field: &str| {
        let aliases = ALIASES
            .iter()
            .find(|(f, _)| *f == field)
            .map(|(_, a)| *a)
            .unwrap_or(&[]);
        aliases
            .iter()
            .find_map(|a| keys.iter().position(|k| k == a))
    };
    ColumnMapping {
        album: find("album"),
        artist: find("artist"),
        year: find("year"),
        edition: find("edition"),
        tags: find("tags"),
        release_group_id: find("rgid"),
        release_id: find("relid"),
    }
}

/// Validated values for one row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowFields {
    pub album: String,
    /// Exactly as written: never split into collaborators.
    pub artist: String,
    pub year: Option<i64>,
    pub edition: Option<String>,
    pub tags: Vec<String>,
    pub release_group_id: Option<String>,
    pub release_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowIssue {
    pub field: String,
    pub message: String,
}

fn issue(field: &str, message: impl Into<String>) -> RowIssue {
    RowIssue {
        field: field.into(),
        message: message.into(),
    }
}

const MAX_TEXT: usize = 300;
const MAX_TAG: usize = 100;

/// Map and validate one record. Returns the fields when the row is importable, and every
/// problem found (so the report lists them all at once).
pub fn normalize_row(
    cells: &[String],
    mapping: &ColumnMapping,
) -> (Option<RowFields>, Vec<RowIssue>) {
    let cell = |i: Option<usize>| {
        i.and_then(|i| cells.get(i))
            .map(|c| c.trim())
            .filter(|c| !c.is_empty())
    };
    let mut issues = Vec::new();

    let mut text = |name: &str, i: Option<usize>, required: bool| -> Option<String> {
        match cell(i) {
            None if required => {
                issues.push(issue(name, format!("{name} is required")));
                None
            }
            None => None,
            Some(v) if v.chars().count() > MAX_TEXT => {
                issues.push(issue(
                    name,
                    format!("{name} is longer than {MAX_TEXT} characters"),
                ));
                None
            }
            Some(v) => Some(v.nfc().collect()),
        }
    };
    let album = text("album", mapping.album, true);
    let artist = text("artist", mapping.artist, true);
    let edition = text("edition", mapping.edition, false);

    let year = cell(mapping.year).and_then(|raw| match parse_year(raw) {
        Some(y) => Some(y),
        None => {
            issues.push(issue(
                "year",
                format!("“{raw}” isn’t a year — use four digits, like 1997"),
            ));
            None
        }
    });

    let mut tags: Vec<String> = Vec::new();
    if let Some(raw) = cell(mapping.tags) {
        for t in raw
            .split(TAG_DELIMITER)
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            let t: String = t.nfc().collect();
            if t.chars().count() > MAX_TAG {
                issues.push(issue(
                    "tags",
                    format!("tag “{t}” is longer than {MAX_TAG} characters"),
                ));
            } else if !tags.iter().any(|x| x.to_lowercase() == t.to_lowercase()) {
                tags.push(t);
            }
        }
    }

    let mut id = |name: &str, i: Option<usize>| {
        cell(i).and_then(|raw| match parse_uuid("id", raw) {
            Ok(id) => Some(id),
            Err(_) => {
                issues.push(issue(name, format!("“{raw}” isn’t a MusicBrainz ID")));
                None
            }
        })
    };
    let release_group_id = id("release group ID", mapping.release_group_id);
    let release_id = id("release ID", mapping.release_id);

    let fields = match (album, artist) {
        (Some(album), Some(artist)) if issues.is_empty() => Some(RowFields {
            album,
            artist,
            year,
            edition,
            tags,
            release_group_id,
            release_id,
        }),
        _ => None,
    };
    (fields, issues)
}

/// `1997`, or a date starting with a year (`1997-05-21`, `1997-05`).
fn parse_year(raw: &str) -> Option<i64> {
    let head = raw.split('-').next()?;
    let rest_ok =
        raw.len() == 4 || crate::domain::dates::PartialDate::parse("year", Some(raw)).is_ok();
    if head.len() == 4 && head.bytes().all(|b| b.is_ascii_digit()) && rest_ok {
        let y: i64 = head.parse().ok()?;
        (1000..=9999).contains(&y).then_some(y)
    } else {
        None
    }
}

pub use crate::domain::text::norm;

/// Stable identity of a row's content across re-imports (tags excluded: re-tagging a row
/// is not a new album).
pub fn fingerprint(f: &RowFields) -> String {
    [
        "v1".to_owned(),
        norm(&f.album),
        norm(&f.artist),
        f.year.map(|y| y.to_string()).unwrap_or_default(),
        f.edition.as_deref().map(norm).unwrap_or_default(),
        f.release_group_id.clone().unwrap_or_default(),
        f.release_id.clone().unwrap_or_default(),
    ]
    .join("\u{1f}")
}

/// Key for spotting the same album *edition* twice in one file. Different editions of one
/// album (Standard vs Deluxe) have different keys and are both kept.
pub fn duplicate_key(f: &RowFields) -> String {
    if let Some(r) = &f.release_id {
        return format!("release:{r}");
    }
    let edition = f.edition.as_deref().map(norm).unwrap_or_default();
    match &f.release_group_id {
        Some(g) => format!("group:{g}\u{1f}{edition}"),
        None => format!(
            "name:{}\u{1f}{}\u{1f}{edition}",
            norm(&f.album),
            norm(&f.artist)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    fn full() -> ColumnMapping {
        ColumnMapping {
            album: Some(0),
            artist: Some(1),
            year: Some(2),
            edition: Some(3),
            tags: Some(4),
            release_group_id: Some(5),
            release_id: Some(6),
        }
    }

    #[test]
    fn suggests_mapping_from_common_headers() {
        let headers = cells(&[
            "Title",
            "Album Artist",
            "Release Year",
            "Tags",
            "MB Release Group ID",
        ]);
        let m = suggest(&headers);
        assert_eq!(
            (m.album, m.artist, m.year, m.tags, m.release_group_id),
            (Some(0), Some(1), Some(2), Some(3), Some(4))
        );
        assert_eq!(m.edition, None);
        assert!(m.validate(5).is_ok());
    }

    #[test]
    fn mapping_requires_album_and_artist_and_unique_columns() {
        assert!(
            ColumnMapping {
                album: Some(0),
                ..Default::default()
            }
            .validate(2)
            .is_err()
        );
        let twice = ColumnMapping {
            album: Some(0),
            artist: Some(0),
            ..Default::default()
        };
        assert!(
            twice
                .validate(2)
                .unwrap_err()
                .to_string()
                .contains("two fields")
        );
        let past = ColumnMapping {
            album: Some(0),
            artist: Some(5),
            ..Default::default()
        };
        assert!(past.validate(2).is_err());
    }

    #[test]
    fn validates_rows_and_never_splits_artists() {
        let (f, issues) = normalize_row(
            &cells(&[
                "Bookends",
                "Simon & Garfunkel, with friends",
                "1968-04-03",
                " Deluxe ",
                "folk; 60s ;Folk;; road trip, summer",
                "",
                "",
            ]),
            &full(),
        );
        assert!(issues.is_empty(), "{issues:?}");
        let f = f.unwrap();
        assert_eq!(f.artist, "Simon & Garfunkel, with friends");
        assert_eq!(f.year, Some(1968));
        assert_eq!(f.edition.as_deref(), Some("Deluxe"));
        assert_eq!(f.tags, vec!["folk", "60s", "road trip, summer"]);
    }

    #[test]
    fn reports_every_problem_on_a_row() {
        let (f, issues) = normalize_row(
            &cells(&["", "Artist", "97", "", "", "not-an-id", "1234"]),
            &full(),
        );
        assert!(f.is_none());
        let fields: Vec<&str> = issues.iter().map(|i| i.field.as_str()).collect();
        assert_eq!(
            fields,
            vec!["album", "year", "release group ID", "release ID"]
        );
        for bad in ["97", "19977", "abcd", "0999", "1997-13-01", "circa 1997"] {
            let (_, issues) = normalize_row(&cells(&["A", "B", bad]), &full());
            assert!(issues.iter().any(|i| i.field == "year"), "{bad}");
        }
    }

    #[test]
    fn fingerprints_and_duplicates_ignore_case_spacing_and_unicode_form() {
        let composed = normalize_row(&cells(&["Homogenic", "Björk"]), &full())
            .0
            .unwrap();
        let decomposed = normalize_row(&cells(&["  homogenic ", "Bjo\u{308}rk"]), &full())
            .0
            .unwrap();
        assert_eq!(fingerprint(&composed), fingerprint(&decomposed));
        assert_eq!(duplicate_key(&composed), duplicate_key(&decomposed));

        let deluxe = normalize_row(&cells(&["Homogenic", "Björk", "", "Deluxe"]), &full())
            .0
            .unwrap();
        assert_ne!(
            duplicate_key(&composed),
            duplicate_key(&deluxe),
            "deliberate editions stay distinct"
        );
        let tagged = normalize_row(&cells(&["Homogenic", "Björk", "", "", "new tag"]), &full())
            .0
            .unwrap();
        assert_eq!(
            fingerprint(&composed),
            fingerprint(&tagged),
            "tags don't change identity"
        );
    }
}
