//! Next Up picker: which Listen List editions a method allows, and a uniform choice among
//! those not yet shown in the session's pool. Pure: randomness and the current year are
//! passed in, so every outcome is reproducible in tests.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::text::norm;
use crate::error::AppResult;

/// A category filter. `Agnostic` bypasses the category and is exclusive by construction.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Filter<T> {
    #[default]
    Agnostic,
    AnyOf(Vec<T>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum YearChoice {
    /// First year of the decade, e.g. 1990 for the 1990s.
    Decade(i32),
    /// The local calendar's current year, resolved when picking.
    CurrentYear,
}

/// Guided Recommendation: OR within a category, AND between categories.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuidedCriteria {
    #[serde(default)]
    pub years: Filter<YearChoice>,
    /// Matched case- and Unicode-insensitively.
    #[serde(default)]
    pub genres: Filter<String>,
    #[serde(default)]
    pub tag_ids: Filter<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "criteria", rename_all = "snake_case")]
pub enum Method {
    CompletelyRandom,
    /// Uniform among albums tagged Listen ASAP only — never a probability weighting and
    /// never a fallback to the whole list.
    WeightedRandom,
    Guided(GuidedCriteria),
}

/// One Listen List entry with what the filters need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub album_id: String,
    pub edition_id: String,
    /// Original (canonical album) year, not the edition's.
    pub year: Option<i32>,
    /// Already in `norm` form.
    pub genres: Vec<String>,
    pub tag_ids: Vec<String>,
    pub listen_asap: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmptyReason {
    ListenListEmpty,
    NoListenAsap,
    NoGuidedMatches,
}

/// Uniform index source. Production uses the OS generator; tests script the sequence.
pub trait RandomSource {
    /// A uniformly distributed index in `0..len`; `len` is never zero.
    fn below(&mut self, len: usize) -> AppResult<usize>;
}

fn year_matches(choices: &[YearChoice], year: Option<i32>, current_year: i32) -> bool {
    let Some(y) = year else {
        return false; // unknown years qualify only when the category is agnostic
    };
    choices.iter().any(|c| match *c {
        YearChoice::Decade(d) => (d..d + 10).contains(&y),
        YearChoice::CurrentYear => y == current_year,
    })
}

fn any_of<T>(filter: &Filter<T>, matches: impl Fn(&T) -> bool) -> bool {
    match filter {
        Filter::Agnostic => true,
        Filter::AnyOf(items) => items.iter().any(matches),
    }
}

/// Whether one candidate satisfies the method. A predicate, so a candidate matching
/// several choices (the current year and its decade) is still a single candidate.
pub fn allows(method: &Method, c: &Candidate, current_year: i32) -> bool {
    match method {
        Method::CompletelyRandom => true,
        Method::WeightedRandom => c.listen_asap,
        Method::Guided(g) => {
            let years = match &g.years {
                Filter::Agnostic => true,
                Filter::AnyOf(choices) => year_matches(choices, c.year, current_year),
            };
            years
                && any_of(&g.genres, |want| c.genres.iter().any(|have| have == want))
                && any_of(&g.tag_ids, |want| c.tag_ids.contains(want))
        }
    }
}

pub fn eligible<'a>(
    method: &Method,
    all: &'a [Candidate],
    current_year: i32,
) -> Vec<&'a Candidate> {
    all.iter()
        .filter(|c| allows(method, c, current_year))
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
pub enum Pick<'a> {
    Chosen {
        candidate: &'a Candidate,
        pool_size: usize,
        /// Unseen candidates left after this one.
        remaining: usize,
    },
    Empty(EmptyReason),
    /// Every eligible album was shown or rejected this cycle; only Reset pool continues.
    Exhausted {
        pool_size: usize,
    },
}

/// Choose uniformly among eligible candidates whose album is not in `seen`. Candidates
/// must be in a stable order (the caller sorts them) for scripted randomness to replay.
pub fn pick<'a>(
    method: &Method,
    all: &'a [Candidate],
    seen: &HashSet<String>,
    current_year: i32,
    rng: &mut dyn RandomSource,
) -> AppResult<Pick<'a>> {
    let pool = eligible(method, all, current_year);
    if pool.is_empty() {
        return Ok(Pick::Empty(match method {
            _ if all.is_empty() => EmptyReason::ListenListEmpty,
            Method::WeightedRandom => EmptyReason::NoListenAsap,
            _ => EmptyReason::NoGuidedMatches,
        }));
    }
    let unseen: Vec<&Candidate> = pool
        .iter()
        .copied()
        .filter(|c| !seen.contains(&c.album_id))
        .collect();
    if unseen.is_empty() {
        return Ok(Pick::Exhausted {
            pool_size: pool.len(),
        });
    }
    let index = rng.below(unseen.len())?.min(unseen.len() - 1);
    Ok(Pick::Chosen {
        candidate: unseen[index],
        pool_size: pool.len(),
        remaining: unseen.len() - 1,
    })
}

/// Normalize a genre for comparison with `Candidate::genres`.
pub fn genre_key(name: &str) -> String {
    norm(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scripted(Vec<usize>);
    impl RandomSource for Scripted {
        fn below(&mut self, len: usize) -> AppResult<usize> {
            let next = self.0.remove(0);
            assert!(next < len, "scripted index {next} out of 0..{len}");
            Ok(next)
        }
    }

    fn cand(n: u8, year: Option<i32>, genres: &[&str], tags: &[&str], asap: bool) -> Candidate {
        Candidate {
            album_id: format!("a{n}"),
            edition_id: format!("e{n}"),
            year,
            genres: genres.iter().map(|g| genre_key(g)).collect(),
            tag_ids: tags.iter().map(|t| (*t).to_owned()).collect(),
            listen_asap: asap,
        }
    }

    fn guided(years: Filter<YearChoice>, genres: Filter<String>, tags: Filter<String>) -> Method {
        Method::Guided(GuidedCriteria {
            years,
            genres,
            tag_ids: tags,
        })
    }

    fn ids(v: Vec<&Candidate>) -> Vec<&str> {
        v.into_iter().map(|c| c.album_id.as_str()).collect()
    }

    #[test]
    fn zero_one_and_many_candidates() {
        let none: Vec<Candidate> = vec![];
        let seen = HashSet::new();
        assert_eq!(
            pick(
                &Method::CompletelyRandom,
                &none,
                &seen,
                2026,
                &mut Scripted(vec![])
            )
            .unwrap(),
            Pick::Empty(EmptyReason::ListenListEmpty)
        );
        let one = vec![cand(1, None, &[], &[], false)];
        let Pick::Chosen {
            candidate,
            pool_size,
            remaining,
        } = pick(
            &Method::CompletelyRandom,
            &one,
            &seen,
            2026,
            &mut Scripted(vec![0]),
        )
        .unwrap()
        else {
            panic!("expected a pick")
        };
        assert_eq!(
            (candidate.album_id.as_str(), pool_size, remaining),
            ("a1", 1, 0)
        );

        let many: Vec<Candidate> = (1..=5).map(|n| cand(n, None, &[], &[], false)).collect();
        let Pick::Chosen { candidate, .. } = pick(
            &Method::CompletelyRandom,
            &many,
            &seen,
            2026,
            &mut Scripted(vec![3]),
        )
        .unwrap() else {
            panic!("expected a pick")
        };
        assert_eq!(candidate.album_id, "a4");
    }

    #[test]
    fn excludes_seen_albums_until_exhausted() {
        let all: Vec<Candidate> = (1..=3).map(|n| cand(n, None, &[], &[], false)).collect();
        let seen: HashSet<String> = ["a1".to_owned(), "a3".to_owned()].into();
        let Pick::Chosen {
            candidate,
            remaining,
            ..
        } = pick(
            &Method::CompletelyRandom,
            &all,
            &seen,
            2026,
            &mut Scripted(vec![0]),
        )
        .unwrap()
        else {
            panic!("expected a pick")
        };
        assert_eq!((candidate.album_id.as_str(), remaining), ("a2", 0));
        let seen: HashSet<String> = ["a1", "a2", "a3"].map(String::from).into();
        assert_eq!(
            pick(
                &Method::CompletelyRandom,
                &all,
                &seen,
                2026,
                &mut Scripted(vec![])
            )
            .unwrap(),
            Pick::Exhausted { pool_size: 3 }
        );
    }

    #[test]
    fn weighted_is_uniform_among_listen_asap_only() {
        let all = vec![
            cand(1, None, &[], &[], false),
            cand(2, None, &[], &[], true),
            cand(3, None, &[], &[], false),
            cand(4, None, &[], &[], true),
        ];
        assert_eq!(
            ids(eligible(&Method::WeightedRandom, &all, 2026)),
            ["a2", "a4"]
        );
        let none = vec![cand(1, None, &[], &[], false)];
        assert_eq!(
            pick(
                &Method::WeightedRandom,
                &none,
                &HashSet::new(),
                2026,
                &mut Scripted(vec![])
            )
            .unwrap(),
            Pick::Empty(EmptyReason::NoListenAsap)
        );
    }

    #[test]
    fn guided_or_within_and_between_categories() {
        let all = vec![
            cand(1, Some(1994), &["Rock"], &["t1"], false),
            cand(2, Some(1999), &["Jazz"], &["t2"], false),
            cand(3, Some(2005), &["rock", "Pop"], &["t1", "t2"], false),
            cand(4, None, &["Rock"], &[], false),
        ];
        let rock_or_jazz = Filter::AnyOf(vec![genre_key("ROCK"), genre_key("jazz")]);
        let m = guided(Filter::Agnostic, rock_or_jazz.clone(), Filter::Agnostic);
        assert_eq!(ids(eligible(&m, &all, 2026)), ["a1", "a2", "a3", "a4"]);

        let nineties = Filter::AnyOf(vec![YearChoice::Decade(1990)]);
        let m = guided(
            nineties.clone(),
            Filter::AnyOf(vec![genre_key("rock")]),
            Filter::Agnostic,
        );
        assert_eq!(ids(eligible(&m, &all, 2026)), ["a1"]);

        let m = guided(nineties, rock_or_jazz, Filter::AnyOf(vec!["t2".into()]));
        assert_eq!(ids(eligible(&m, &all, 2026)), ["a2"]);

        let m = guided(
            Filter::Agnostic,
            Filter::Agnostic,
            Filter::AnyOf(vec!["t9".into()]),
        );
        assert_eq!(
            pick(&m, &all, &HashSet::new(), 2026, &mut Scripted(vec![])).unwrap(),
            Pick::Empty(EmptyReason::NoGuidedMatches)
        );
    }

    #[test]
    fn unknown_years_qualify_only_when_year_agnostic() {
        let all = vec![
            cand(1, None, &[], &[], false),
            cand(2, Some(1990), &[], &[], false),
        ];
        let any = guided(Filter::Agnostic, Filter::Agnostic, Filter::Agnostic);
        assert_eq!(ids(eligible(&any, &all, 2026)), ["a1", "a2"]);
        let decade = guided(
            Filter::AnyOf(vec![YearChoice::Decade(1990)]),
            Filter::Agnostic,
            Filter::Agnostic,
        );
        assert_eq!(ids(eligible(&decade, &all, 2026)), ["a2"]);
    }

    #[test]
    fn current_year_uses_the_supplied_calendar_and_never_duplicates() {
        let all = vec![
            cand(1, Some(2026), &[], &[], false),
            cand(2, Some(2021), &[], &[], false),
            cand(3, Some(2031), &[], &[], false),
        ];
        let current = |year| {
            ids(eligible(
                &guided(
                    Filter::AnyOf(vec![YearChoice::CurrentYear]),
                    Filter::Agnostic,
                    Filter::Agnostic,
                ),
                &all,
                year,
            ))
        };
        assert_eq!(current(2026), ["a1"]);
        assert_eq!(current(2031), ["a3"]);

        let both = guided(
            Filter::AnyOf(vec![YearChoice::Decade(2020), YearChoice::CurrentYear]),
            Filter::Agnostic,
            Filter::Agnostic,
        );
        assert_eq!(ids(eligible(&both, &all, 2026)), ["a1", "a2"]);
        // Overlap: after a1 and a2 are seen the pool is exhausted, not offering a1 twice.
        let seen: HashSet<String> = ["a1", "a2"].map(String::from).into();
        assert_eq!(
            pick(&both, &all, &seen, 2026, &mut Scripted(vec![])).unwrap(),
            Pick::Exhausted { pool_size: 2 }
        );
    }

    #[test]
    fn method_json_shape() {
        let m: Method = serde_json::from_value(serde_json::json!({
            "mode": "guided",
            "criteria": { "years": { "any_of": [{ "decade": 1990 }, "current_year"] }, "genres": "agnostic" }
        }))
        .unwrap();
        assert_eq!(
            m,
            guided(
                Filter::AnyOf(vec![YearChoice::Decade(1990), YearChoice::CurrentYear]),
                Filter::Agnostic,
                Filter::Agnostic
            )
        );
        let m: Method =
            serde_json::from_value(serde_json::json!({ "mode": "weighted_random" })).unwrap();
        assert_eq!(m, Method::WeightedRandom);
        assert!(
            serde_json::from_value::<Method>(
                serde_json::json!({ "mode": "guided", "criteria": { "mood": "x" } })
            )
            .is_err()
        );
    }
}
