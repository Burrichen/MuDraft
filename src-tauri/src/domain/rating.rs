use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// A rating in half-stars: 0..=10 maps to 0.0..=5.0 stars. Zero is a real rating;
/// "unrated" is `Option::None`, never a sentinel value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "i64", into = "u8")]
pub struct HalfStars(u8);

impl HalfStars {
    pub const MAX: u8 = 10;

    pub fn new(value: i64) -> AppResult<Self> {
        u8::try_from(value)
            .ok()
            .filter(|v| *v <= Self::MAX)
            .map(Self)
            .ok_or_else(|| {
                AppError::validation("rating", format!("{value} is outside 0–10 half-stars"))
            })
    }

    pub fn get(self) -> u8 {
        self.0
    }
}

impl TryFrom<i64> for HalfStars {
    type Error = AppError;
    fn try_from(value: i64) -> AppResult<Self> {
        Self::new(value)
    }
}

impl From<HalfStars> for u8 {
    fn from(value: HalfStars) -> Self {
        value.0
    }
}

/// Where an album's effective rating comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RatingSource {
    /// The user rated the album directly (zero included).
    Explicit,
    /// Derived from the rated tracks.
    Calculated,
    Unrated,
}

/// The one shared album rating calculation, used by album pages, cards, sorting, artist
/// pages, and stats. Never stored: computed on read from explicit and track ratings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RatingSummary {
    pub effective: Option<HalfStars>,
    pub source: RatingSource,
    pub explicit: Option<HalfStars>,
    /// Track-derived value, shown even when an explicit rating overrides it.
    pub calculated: Option<HalfStars>,
    /// "Calculated from X/Y rated tracks".
    pub rated_tracks: u32,
    pub total_tracks: u32,
}

/// Mean of rated tracks rounded to the nearest half-star, exact halves rounding up.
/// Done in integer half-star units, so no precision is lost before the final rounding.
fn rounded_mean(sum: u64, count: u64) -> Option<HalfStars> {
    if count == 0 {
        return None;
    }
    // floor(sum / count + 1/2) == floor((2·sum + count) / (2·count)), exactly.
    let rounded = (2 * sum + count) / (2 * count);
    Some(HalfStars(
        u8::try_from(rounded).expect("mean of 0..=10 stays in 0..=10"),
    ))
}

/// Summarize an album edition: explicit rating if present (zero is a rating), otherwise
/// the rounded mean of rated tracks; unrated tracks are ignored, and with neither the
/// album stays unrated. An explicit rating never changes the track ratings.
pub fn summarize(
    explicit: Option<HalfStars>,
    tracks: impl IntoIterator<Item = Option<HalfStars>>,
) -> RatingSummary {
    let (mut sum, mut rated, mut total) = (0_u64, 0_u32, 0_u32);
    for t in tracks {
        total += 1;
        if let Some(r) = t {
            sum += u64::from(r.get());
            rated += 1;
        }
    }
    let calculated = rounded_mean(sum, u64::from(rated));
    let (effective, source) = match (explicit, calculated) {
        (Some(e), _) => (Some(e), RatingSource::Explicit),
        (None, Some(c)) => (Some(c), RatingSource::Calculated),
        (None, None) => (None, RatingSource::Unrated),
    };
    RatingSummary {
        effective,
        source,
        explicit,
        calculated,
        rated_tracks: rated,
        total_tracks: total,
    }
}

/// Effective album rating only (see [`summarize`]).
pub fn effective_album_rating(
    explicit: Option<HalfStars>,
    tracks: impl IntoIterator<Item = Option<HalfStars>>,
) -> Option<HalfStars> {
    summarize(explicit, tracks).effective
}

/// Aggregate statistic in stars with decimals (e.g. an artist's average album rating).
/// Individual ratings stay half-stars; only aggregates may show decimals.
pub fn mean_stars(ratings: impl IntoIterator<Item = HalfStars>) -> Option<f64> {
    let (sum, n) = ratings
        .into_iter()
        .fold((0_u64, 0_u64), |(s, n), r| (s + u64::from(r.get()), n + 1));
    (n > 0).then(|| sum as f64 / n as f64 / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hs(v: i64) -> Option<HalfStars> {
        Some(HalfStars::new(v).unwrap())
    }

    #[test]
    fn accepts_zero_and_bounds() {
        assert_eq!(HalfStars::new(0).unwrap().get(), 0);
        assert_eq!(HalfStars::new(10).unwrap().get(), 10);
        assert_eq!(HalfStars::new(11).unwrap_err().code(), "validation");
        assert!(HalfStars::new(-1).is_err());
    }

    #[test]
    fn explicit_rating_wins_even_when_zero() {
        assert_eq!(effective_album_rating(hs(0), [hs(10), hs(10)]), hs(0));
    }

    #[test]
    fn falls_back_to_rounded_track_mean_ignoring_unrated() {
        assert_eq!(effective_album_rating(None, [hs(7), None, hs(8)]), hs(8)); // 7.5 → 8
        assert_eq!(effective_album_rating(None, [hs(7), hs(7), hs(8)]), hs(7)); // 7.33 → 7
        assert_eq!(effective_album_rating(None, [hs(0), hs(0)]), hs(0));
        assert_eq!(effective_album_rating(None, [None, None]), None);
        assert_eq!(effective_album_rating(None, []), None);
    }

    #[test]
    fn all_zero_and_mixed_zero_unrated_tracks() {
        let s = summarize(None, [hs(0), hs(0), hs(0)]);
        assert_eq!(
            (s.effective, s.source, s.rated_tracks, s.total_tracks),
            (hs(0), RatingSource::Calculated, 3, 3)
        );
        let mixed = summarize(None, [hs(0), None, hs(0), None]);
        assert_eq!(
            (mixed.effective, mixed.rated_tracks, mixed.total_tracks),
            (hs(0), 2, 4),
            "zero is rated, None isn't"
        );
        let none = summarize(None, [None, None]);
        assert_eq!(
            (none.effective, none.source, none.rated_tracks),
            (None, RatingSource::Unrated, 0)
        );
        assert_eq!(summarize(None, []).source, RatingSource::Unrated);
    }

    #[test]
    fn partial_ratings_and_rounding_boundaries() {
        let e = |t: &[Option<HalfStars>]| summarize(None, t.iter().copied()).effective;
        assert_eq!(
            e(&[hs(7), hs(8), None]),
            hs(8),
            "7.5 half-stars → tie rounds up"
        );
        assert_eq!(e(&[hs(0), hs(1)]), hs(1), "0.25 stars → 0.5");
        assert_eq!(e(&[hs(9), hs(10)]), hs(10));
        assert_eq!(e(&[hs(7), hs(7), hs(8)]), hs(7), "7.333 rounds down");
        assert_eq!(e(&[hs(7), hs(8), hs(8)]), hs(8), "7.667 rounds up");
        // Just below a tie must not be rounded up by early rounding.
        let many: Vec<_> = std::iter::repeat_n(hs(8), 999)
            .chain([hs(7), hs(7)])
            .collect();
        assert_eq!(e(&many), hs(8));
        let below: Vec<_> = std::iter::repeat_n(hs(7), 1001)
            .chain(std::iter::repeat_n(hs(8), 1000))
            .collect();
        assert_eq!(e(&below), hs(7), "7.4997 stays 7");
        let tie: Vec<_> = std::iter::repeat_n(hs(7), 1000)
            .chain(std::iter::repeat_n(hs(8), 1000))
            .collect();
        assert_eq!(e(&tie), hs(8), "exact 7.5 rounds up");
    }

    #[test]
    fn explicit_rating_overrides_without_hiding_the_calculation() {
        let s = summarize(hs(0), [hs(10), hs(9)]);
        assert_eq!(
            (s.effective, s.source),
            (hs(0), RatingSource::Explicit),
            "explicit zero wins"
        );
        assert_eq!(s.calculated, hs(10), "derived value kept for display");
        let cleared = summarize(None, [hs(10), hs(9)]);
        assert_eq!(
            (cleared.effective, cleared.source),
            (hs(10), RatingSource::Calculated)
        );
    }

    #[test]
    fn aggregates_keep_decimals() {
        assert_eq!(
            mean_stars([
                HalfStars::new(7).unwrap(),
                HalfStars::new(8).unwrap(),
                HalfStars::new(8).unwrap()
            ]),
            Some(23.0 / 6.0)
        );
        assert_eq!(mean_stars([]), None);
    }

    #[test]
    fn deserialization_is_validated() {
        assert!(serde_json::from_str::<HalfStars>("10").is_ok());
        assert!(serde_json::from_str::<HalfStars>("12").is_err());
    }
}
