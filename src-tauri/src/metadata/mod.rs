//! Metadata provider boundary. The rest of the app sees only these normalized types and
//! the [`MetadataProvider`] trait; MusicBrainz specifics (URLs, JSON, rate limits) stay in
//! [`musicbrainz`]. Networking is native (Rust) — the renderer never talks to providers.

pub mod cache;
pub mod clock;
pub mod genres;
pub mod http;
pub mod musicbrainz;
pub mod queue;
pub mod requests;

#[cfg(test)]
pub(crate) mod testing;

use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::domain::dates::PartialDate;
use crate::error::{AppError, AppResult};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What the user typed. Title is required; artist narrows the search; year only ranks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchQuery {
    pub title: String,
    pub artist: Option<String>,
    pub year: Option<i64>,
}

impl SearchQuery {
    /// Validate and trim at the native boundary.
    pub fn validated(&self) -> AppResult<Self> {
        let title = self.title.trim();
        if title.is_empty() {
            return Err(AppError::validation(
                "album title",
                "enter an album title to search",
            ));
        }
        if title.chars().count() > 300 {
            return Err(AppError::validation(
                "album title",
                "longer than 300 characters",
            ));
        }
        let artist = self
            .artist
            .as_deref()
            .map(str::trim)
            .filter(|a| !a.is_empty());
        if artist.is_some_and(|a| a.chars().count() > 300) {
            return Err(AppError::validation("artist", "longer than 300 characters"));
        }
        if let Some(y) = self.year
            && !(1000..=9999).contains(&y)
        {
            return Err(AppError::validation(
                "year",
                format!("{y} is not a four-digit year"),
            ));
        }
        Ok(Self {
            title: title.to_owned(),
            artist: artist.map(str::to_owned),
            year: self.year,
        })
    }
}

/// Where a response came from. `StaleCache` means the network failed and an expired
/// cached copy was used instead (offline use).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchSource {
    Network,
    Cache,
    StaleCache,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Fetched<T> {
    pub value: T,
    /// UTC RFC 3339 time the provider data was originally fetched.
    pub fetched_at: String,
    pub source: FetchSource,
}

/// One part of an artist credit ("Queen" + " & " + "David Bowie").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditPart {
    /// Provider artist ID. Artists are only ever matched by this, never by name.
    pub artist_id: String,
    pub artist_name: String,
    pub sort_name: Option<String>,
    pub disambiguation: Option<String>,
    /// Name as credited on this release, when it differs from the artist name.
    pub credited_name: Option<String>,
    pub join_phrase: String,
}

pub fn credit_text(parts: &[CreditPart]) -> String {
    parts
        .iter()
        .map(|p| {
            format!(
                "{}{}",
                p.credited_name.as_deref().unwrap_or(&p.artist_name),
                p.join_phrase
            )
        })
        .collect()
}

/// A dated value with explicit precision, as received from the provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDate {
    pub value: Option<String>,
    pub precision: String,
    pub year: Option<u16>,
}

impl ProviderDate {
    /// Malformed provider dates become Unknown rather than failing the whole result.
    pub fn parse(raw: Option<&str>) -> Self {
        let date = PartialDate::parse("provider date", raw).unwrap_or(PartialDate::Unknown);
        let (value, precision) = date.to_db();
        Self {
            value,
            precision: precision.to_owned(),
            year: date.year(),
        }
    }
}

/// A canonical album (MusicBrainz release group) offered for review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseGroupCandidate {
    pub id: String,
    pub title: String,
    pub disambiguation: Option<String>,
    pub primary_type: Option<String>,
    pub secondary_types: Vec<String>,
    pub original_date: ProviderDate,
    pub artist_credit: Vec<CreditPart>,
    /// Provider relevance 0–100. Informational only: nothing is accepted automatically.
    pub score: u8,
    /// Title equal to the query ignoring case and surrounding whitespace.
    pub exact_title: bool,
    /// Whether the original year equals the requested year (None when not comparable).
    pub year_matches: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPage {
    pub candidates: Vec<ReleaseGroupCandidate>,
    pub total: u32,
    pub offset: u32,
}

/// One release (edition) inside a release group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditionCandidate {
    pub id: String,
    pub title: String,
    pub disambiguation: Option<String>,
    pub date: ProviderDate,
    pub country: Option<String>,
    pub status: Option<String>,
    pub formats: Vec<String>,
    pub track_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditionList {
    pub editions: Vec<EditionCandidate>,
    pub total: u32,
    /// True when more editions exist than were fetched (very large release groups).
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceTag {
    pub name: String,
    pub votes: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseGroupDetail {
    pub id: String,
    pub title: String,
    pub disambiguation: Option<String>,
    /// Community-written MusicBrainz annotation, used as a sourced description.
    #[serde(default)]
    pub annotation: Option<String>,
    pub original_date: ProviderDate,
    pub artist_credit: Vec<CreditPart>,
    pub genres: Vec<SourceTag>,
    pub tags: Vec<SourceTag>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackDetail {
    /// Provider track ID: stable identity of this track on this release.
    pub id: String,
    pub disc: u32,
    pub position: u32,
    pub title: String,
    pub length_ms: Option<i64>,
    pub recording_id: Option<String>,
    pub recording_title: Option<String>,
    pub artist_credit: Vec<CreditPart>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseDetail {
    pub id: String,
    pub title: String,
    pub release_group_id: Option<String>,
    pub disambiguation: Option<String>,
    pub date: ProviderDate,
    pub country: Option<String>,
    pub formats: Vec<String>,
    pub artist_credit: Vec<CreditPart>,
    pub tracks: Vec<TrackDetail>,
}

/// One page of an artist's canonical albums (release groups) from a browse request.
/// Includes release groups where the artist is any part of the credit (collaborations).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseGroupPage {
    pub release_groups: Vec<CatalogueReleaseGroup>,
    /// Provider total for the whole browse, not this page.
    pub total: u32,
    pub offset: u32,
    /// Items the provider returned on this page (before discarding malformed ones), which
    /// is what the next offset must advance by.
    pub received: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogueReleaseGroup {
    pub id: String,
    pub title: String,
    pub disambiguation: Option<String>,
    pub primary_type: Option<String>,
    pub secondary_types: Vec<String>,
    pub original_date: ProviderDate,
    pub artist_credit: Vec<CreditPart>,
}

/// A metadata source. Every call is cancellable and goes through the provider's own
/// queue, cache, timeout, and retry policy.
pub trait MetadataProvider: Send + Sync {
    /// Short name used in provenance (`metadata_provenance.source`).
    fn source(&self) -> &'static str;

    fn search<'a>(
        &'a self,
        query: &'a SearchQuery,
        offset: u32,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, AppResult<Fetched<SearchPage>>>;

    fn editions<'a>(
        &'a self,
        release_group_id: &'a str,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, AppResult<Fetched<EditionList>>>;

    fn release_group<'a>(
        &'a self,
        release_group_id: &'a str,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, AppResult<Fetched<ReleaseGroupDetail>>>;

    fn release<'a>(
        &'a self,
        release_id: &'a str,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, AppResult<Fetched<ReleaseDetail>>>;

    /// One page of the artist's official release groups, all types, starting at `offset`.
    fn artist_release_groups<'a>(
        &'a self,
        artist_id: &'a str,
        offset: u32,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, AppResult<Fetched<ReleaseGroupPage>>>;
}
