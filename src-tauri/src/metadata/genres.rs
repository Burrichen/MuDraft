//! Broad-genre normalization. Provider genres/tags are fine-grained and noisy; MuDraft
//! stores 2–3 *broad* genres per album for filtering and stats, and keeps the raw tags
//! separately (`album_source_tag`). Everything here is deterministic.
//!
//! ## Editing the vocabulary
//! - Add or rename a broad genre in [`BROAD_GENRES`] (order breaks score ties).
//! - Map exact source names in [`EXACT`] (checked first, lowercase).
//! - Map by keyword in [`KEYWORDS`] (checked in order; first containing keyword wins).
//!
//! A source name matching nothing is ignored — it is still kept as a raw tag.
//!
//! ## Scoring
//! Each source genre scores `2 × max(votes, 1)` for its broad genre; each free-form tag
//! scores `1 × max(votes, 1)` (genres are curated, tags are not). Broad genres are ranked
//! by score, then vocabulary order. At most [`MAX_GENRES`] are kept, and each must reach
//! [`MIN_EVIDENCE`] (one curated genre with 2+ votes, or several tag votes) so single
//! stray tags don't pad the list. There is no threshold relative to the top genre: an
//! album dominated by many rock subgenres can still show a well-supported second genre.
//! With little or no evidence, fewer (or zero) genres are returned — never invented.

use std::collections::HashMap;

use crate::error::{AppError, AppResult};

use super::SourceTag;

pub const MAX_GENRES: usize = 3;
pub const MIN_EVIDENCE: i64 = 4;

pub const BROAD_GENRES: &[&str] = &[
    "Rock",
    "Pop",
    "Hip Hop",
    "R&B & Soul",
    "Electronic",
    "Jazz",
    "Classical",
    "Folk",
    "Country",
    "Metal",
    "Punk",
    "Blues",
    "Reggae",
    "Latin",
    "World",
    "Soundtrack",
    "Experimental",
];

/// Exact source names (lowercase) whose keyword match would be wrong or ambiguous.
const EXACT: &[(&str, &str)] = &[
    ("post-punk", "Rock"),
    ("post-rock", "Rock"),
    ("experimental rock", "Rock"),
    ("art pop", "Pop"),
    ("synth-pop", "Pop"),
    ("synthpop", "Pop"),
    ("dream pop", "Rock"),
    ("pop rock", "Rock"),
    ("pop punk", "Punk"),
    ("pop rap", "Hip Hop"),
    ("country rock", "Country"),
    ("folk rock", "Folk"),
    ("jazz rap", "Hip Hop"),
    ("jazz fusion", "Jazz"),
    ("trip hop", "Electronic"),
    ("neo-soul", "R&B & Soul"),
    ("funk", "R&B & Soul"),
    ("disco", "Pop"),
    ("singer-songwriter", "Folk"),
    ("ambient", "Electronic"),
    ("noise", "Experimental"),
    ("avant-garde", "Experimental"),
    ("film score", "Soundtrack"),
    ("video game music", "Soundtrack"),
    ("musical", "Soundtrack"),
    ("bossa nova", "Latin"),
    ("afrobeat", "World"),
    ("ska", "Reggae"),
    ("dub", "Reggae"),
    ("gospel", "R&B & Soul"),
];

/// Keyword rules, checked in order against the lowercase source name.
const KEYWORDS: &[(&str, &str)] = &[
    ("metal", "Metal"),
    ("grindcore", "Metal"),
    ("hardcore punk", "Punk"),
    ("punk", "Punk"),
    ("hip hop", "Hip Hop"),
    ("hip-hop", "Hip Hop"),
    ("rap", "Hip Hop"),
    ("trap", "Hip Hop"),
    ("grime", "Hip Hop"),
    ("r&b", "R&B & Soul"),
    ("rhythm and blues", "R&B & Soul"),
    ("soul", "R&B & Soul"),
    ("blues", "Blues"),
    ("jazz", "Jazz"),
    ("bebop", "Jazz"),
    ("swing", "Jazz"),
    ("classical", "Classical"),
    ("baroque", "Classical"),
    ("opera", "Classical"),
    ("symphon", "Classical"),
    ("orchestral", "Classical"),
    ("chamber", "Classical"),
    ("electronic", "Electronic"),
    ("electronica", "Electronic"),
    ("techno", "Electronic"),
    ("house", "Electronic"),
    ("trance", "Electronic"),
    ("drum and bass", "Electronic"),
    ("dubstep", "Electronic"),
    ("idm", "Electronic"),
    ("edm", "Electronic"),
    ("synth", "Electronic"),
    ("downtempo", "Electronic"),
    ("reggae", "Reggae"),
    ("dancehall", "Reggae"),
    ("country", "Country"),
    ("bluegrass", "Country"),
    ("americana", "Country"),
    ("folk", "Folk"),
    ("latin", "Latin"),
    ("salsa", "Latin"),
    ("reggaeton", "Latin"),
    ("cumbia", "Latin"),
    ("samba", "Latin"),
    ("tango", "Latin"),
    ("world", "World"),
    ("britpop", "Rock"),
    ("k-pop", "Pop"),
    ("j-pop", "Pop"),
    ("pop", "Pop"),
    ("soundtrack", "Soundtrack"),
    ("score", "Soundtrack"),
    ("experimental", "Experimental"),
    ("avant", "Experimental"),
    ("rock", "Rock"),
    ("grunge", "Rock"),
    ("shoegaze", "Rock"),
    ("indie", "Rock"),
    ("new wave", "Rock"),
];

/// Broad genre for one source name, if the vocabulary covers it.
pub fn broad_genre_for(source_name: &str) -> Option<&'static str> {
    let name = source_name.trim().to_lowercase();
    if name.is_empty() {
        return None;
    }
    EXACT
        .iter()
        .find(|(k, _)| *k == name)
        .or_else(|| KEYWORDS.iter().find(|(k, _)| name.contains(k)))
        .map(|(_, g)| *g)
}

/// Normalize provider genres and tags to at most [`MAX_GENRES`] broad genres.
pub fn normalize(genres: &[SourceTag], tags: &[SourceTag]) -> Vec<&'static str> {
    let mut scores: HashMap<&'static str, i64> = HashMap::new();
    for (list, weight) in [(genres, 2), (tags, 1)] {
        for t in list {
            if let Some(g) = broad_genre_for(&t.name) {
                *scores.entry(g).or_default() += weight * t.votes.max(1);
            }
        }
    }
    let mut ranked: Vec<(&'static str, i64)> = scores.into_iter().collect();
    let order = |g: &str| {
        BROAD_GENRES
            .iter()
            .position(|b| *b == g)
            .unwrap_or(usize::MAX)
    };
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| order(a.0).cmp(&order(b.0))));
    ranked
        .into_iter()
        .filter(|(_, score)| *score >= MIN_EVIDENCE)
        .take(MAX_GENRES)
        .map(|(g, _)| g)
        .collect()
}

/// Validate a manual genre correction against the vocabulary (canonical spelling out).
pub fn validate_manual(names: &[String]) -> AppResult<Vec<&'static str>> {
    let mut out: Vec<&'static str> = Vec::new();
    for name in names {
        let canonical = BROAD_GENRES
            .iter()
            .find(|g| g.eq_ignore_ascii_case(name.trim()))
            .ok_or_else(|| {
                AppError::validation(
                    "genres",
                    format!("'{name}' is not one of MuDraft's broad genres"),
                )
            })?;
        if !out.contains(canonical) {
            out.push(canonical);
        }
    }
    if out.len() > MAX_GENRES {
        return Err(AppError::validation(
            "genres",
            format!("choose at most {MAX_GENRES}"),
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(name: &str, votes: i64) -> SourceTag {
        SourceTag {
            name: name.into(),
            votes,
        }
    }

    #[test]
    fn every_mapping_targets_the_vocabulary() {
        for (_, g) in EXACT.iter().chain(KEYWORDS) {
            assert!(BROAD_GENRES.contains(g), "{g} missing from BROAD_GENRES");
        }
    }

    #[test]
    fn maps_specific_names_deterministically() {
        assert_eq!(broad_genre_for("Post-Punk"), Some("Rock"));
        assert_eq!(broad_genre_for("pop punk"), Some("Punk"));
        assert_eq!(broad_genre_for("black metal"), Some("Metal"));
        assert_eq!(broad_genre_for("conscious hip hop"), Some("Hip Hop"));
        assert_eq!(broad_genre_for("alternative rock"), Some("Rock"));
        assert_eq!(
            broad_genre_for("post-britpop"),
            Some("Rock"),
            "britpop is checked before pop"
        );
        assert_eq!(broad_genre_for("seen live"), None);
        assert_eq!(broad_genre_for("1997"), None);
    }

    #[test]
    fn targets_three_but_keeps_fewer_without_evidence() {
        let genres = [
            t("alternative rock", 12),
            t("art rock", 8),
            t("electronic", 5),
            t("ambient", 2),
        ];
        let tags = [t("british", 9), t("jazz", 1)];
        assert_eq!(normalize(&genres, &tags), vec!["Rock", "Electronic"]);

        let many = [t("rock", 5), t("electronic", 5), t("jazz", 5), t("folk", 5)];
        assert_eq!(
            normalize(&many, &[]),
            vec!["Rock", "Electronic", "Jazz"],
            "ties follow vocabulary order"
        );

        assert!(normalize(&[], &[t("seen live", 30), t("favourites", 4)]).is_empty());
        // One zero-vote tag is not enough evidence; several votes are.
        assert!(normalize(&[], &[t("shoegaze", 0)]).is_empty());
        assert_eq!(normalize(&[], &[t("shoegaze", 4)]), vec!["Rock"]);
    }

    #[test]
    fn secondary_genres_survive_a_dominant_one() {
        // Shape of real MusicBrainz data: many rock subgenres, a little of everything else.
        let genres = [
            t("alternative rock", 26),
            t("art rock", 14),
            t("rock", 13),
            t("experimental", 3),
            t("experimental rock", 2),
            t("electronic", 2),
            t("britpop", 1),
        ];
        assert_eq!(
            normalize(&genres, &[t("alienation", 1)]),
            vec!["Rock", "Experimental", "Electronic"]
        );
    }

    #[test]
    fn manual_corrections_use_the_vocabulary() {
        assert_eq!(
            validate_manual(&["jazz".into(), "Jazz".into(), "hip hop".into()]).unwrap(),
            vec!["Jazz", "Hip Hop"]
        );
        assert_eq!(
            validate_manual(&["Chillwave".into()]).unwrap_err().code(),
            "validation"
        );
        let four: Vec<String> = ["Rock", "Pop", "Jazz", "Folk"].map(String::from).into();
        assert!(validate_manual(&four).is_err());
        assert!(validate_manual(&[]).unwrap().is_empty());
    }
}
