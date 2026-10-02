//! MusicBrainz provider (<https://musicbrainz.org/doc/MusicBrainz_API>).
//! Release groups are canonical albums; releases are editions with tracklists.
//! All requests share one queue at ≤1 request/second, as the API's rate limit requires.

use std::sync::Arc;
use std::time::Duration;

use reqwest::Url;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tokio_util::sync::CancellationToken;

use super::cache::ResponseCache;
use super::clock::utc_now;
use super::http::{Transport, TransportError};
use super::queue::RateLimiter;
use super::{
    BoxFuture, CatalogueReleaseGroup, CreditPart, EditionCandidate, EditionList, FetchSource,
    Fetched, MetadataProvider, ProviderDate, ReleaseDetail, ReleaseGroupCandidate,
    ReleaseGroupDetail, ReleaseGroupPage, SearchPage, SearchQuery, SourceTag, TrackDetail,
};
use crate::domain::ids::parse_uuid;
use crate::error::{AppError, AppResult};

const SERVICE: &str = "MusicBrainz";
pub const BASE_URL: &str = "https://musicbrainz.org/ws/2/";
const SEARCH_LIMIT: u32 = 25;
const MAX_SEARCH_OFFSET: u32 = 10_000;
/// Browse pages are capped at 100 by the API.
const BROWSE_LIMIT: u32 = 100;
/// Stop paging editions after this many; the result is marked truncated.
const MAX_EDITIONS: usize = 500;
const SEARCH_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const LOOKUP_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// Total tries per request, including the first.
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(8),
        }
    }
}

impl RetryPolicy {
    fn backoff(&self, attempt: u32) -> Duration {
        self.base_delay
            .saturating_mul(2_u32.saturating_pow(attempt))
            .min(self.max_delay)
    }
}

pub struct MusicBrainz {
    transport: Arc<dyn Transport>,
    cache: Arc<dyn ResponseCache>,
    queue: RateLimiter,
    retry: RetryPolicy,
    base: Url,
}

impl MusicBrainz {
    pub fn new(transport: Arc<dyn Transport>, cache: Arc<dyn ResponseCache>) -> Self {
        Self {
            transport,
            cache,
            queue: RateLimiter::new(Duration::from_secs(1)),
            retry: RetryPolicy::default(),
            base: Url::parse(BASE_URL).expect("valid base URL"),
        }
    }

    fn url(&self, path: &str, params: &[(&str, String)]) -> Url {
        let mut url = self.base.join(path).expect("static path");
        url.query_pairs_mut()
            .extend_pairs(params.iter().map(|(k, v)| (*k, v.as_str())))
            .append_pair("fmt", "json");
        url
    }

    /// Cached, queued, retried, cancellable GET returning parsed JSON.
    async fn get<T: DeserializeOwned>(
        &self,
        url: &Url,
        ttl: Duration,
        cancel: &CancellationToken,
    ) -> AppResult<Fetched<T>> {
        let key = url.as_str();
        let cached = self.cache.get(key);
        if let Some(hit) = cached.as_ref().filter(|c| c.fresh)
            && let Ok(value) = serde_json::from_str(&hit.body)
        {
            return Ok(Fetched {
                value,
                fetched_at: hit.fetched_at.clone(),
                source: FetchSource::Cache,
            });
        }

        let mut last_error = AppError::Offline(SERVICE.into());
        for attempt in 0..self.retry.max_attempts {
            if cancel.is_cancelled() {
                return Err(AppError::Cancelled);
            }
            self.queue.acquire(cancel).await?;
            let response = tokio::select! {
                r = self.transport.get(key) => r,
                () = cancel.cancelled() => return Err(AppError::Cancelled),
            };
            let retry_delay = match response {
                Ok(r) if r.status == 200 => {
                    let value = serde_json::from_str(&r.body).map_err(|e| {
                        AppError::Provider(format!("{SERVICE} sent an unexpected response: {e}"))
                    })?;
                    let fetched_at = utc_now();
                    self.cache.put(key, &r.body, &fetched_at, ttl);
                    return Ok(Fetched {
                        value,
                        fetched_at,
                        source: FetchSource::Network,
                    });
                }
                Ok(r) if r.status == 404 => {
                    return Err(AppError::not_found("MusicBrainz entry", key));
                }
                Ok(r) if r.status == 503 || r.status == 429 => {
                    last_error = AppError::RateLimited(SERVICE.into());
                    let delay = r
                        .retry_after
                        .unwrap_or_else(|| self.retry.backoff(attempt))
                        .min(Duration::from_secs(60));
                    // Everyone in the queue waits, not just this caller.
                    self.queue.penalize(delay).await;
                    Duration::ZERO
                }
                Ok(r) if r.status >= 500 => {
                    last_error =
                        AppError::Provider(format!("{SERVICE} server error ({})", r.status));
                    self.retry.backoff(attempt)
                }
                Ok(r) => {
                    return Err(AppError::Provider(format!(
                        "{SERVICE} rejected the request ({})",
                        r.status
                    )));
                }
                Err(TransportError::Timeout) => {
                    last_error = AppError::Timeout(SERVICE.into());
                    self.retry.backoff(attempt)
                }
                Err(TransportError::Unreachable(_)) => {
                    // Offline: retrying immediately won't help; fall back to the cache.
                    last_error = AppError::Offline(SERVICE.into());
                    break;
                }
            };
            if attempt + 1 < self.retry.max_attempts && !retry_delay.is_zero() {
                tokio::select! {
                    () = tokio::time::sleep(retry_delay) => {}
                    () = cancel.cancelled() => return Err(AppError::Cancelled),
                }
            }
        }

        if let Some(stale) = cached
            && let Ok(value) = serde_json::from_str(&stale.body)
        {
            return Ok(Fetched {
                value,
                fetched_at: stale.fetched_at,
                source: FetchSource::StaleCache,
            });
        }
        Err(last_error)
    }

    async fn search_impl(
        &self,
        query: &SearchQuery,
        offset: u32,
        cancel: &CancellationToken,
    ) -> AppResult<Fetched<SearchPage>> {
        let query = query.validated()?;
        if offset > MAX_SEARCH_OFFSET {
            return Err(AppError::validation(
                "offset",
                format!("{offset} is too large"),
            ));
        }
        let url = self.url(
            "release-group",
            &[
                ("query", lucene_query(&query)),
                ("limit", SEARCH_LIMIT.to_string()),
                ("offset", offset.to_string()),
            ],
        );
        let fetched: Fetched<RawSearch> = self.get(&url, SEARCH_TTL, cancel).await?;
        let raw = fetched.value;
        let wanted = query.title.to_lowercase();
        let mut candidates: Vec<ReleaseGroupCandidate> = raw
            .release_groups
            .into_iter()
            .filter_map(|rg| {
                let id = parse_uuid("release group", &rg.id).ok()?;
                let original_date = ProviderDate::parse(rg.first_release_date.as_deref());
                let year_matches = match (query.year, original_date.year) {
                    (Some(want), Some(have)) => Some(i64::from(have) == want),
                    _ => None,
                };
                Some(ReleaseGroupCandidate {
                    id,
                    exact_title: rg.title.trim().to_lowercase() == wanted,
                    title: rg.title,
                    disambiguation: non_empty(rg.disambiguation),
                    primary_type: rg.primary_type,
                    secondary_types: rg.secondary_types,
                    original_date,
                    artist_credit: credits(rg.artist_credit),
                    score: score(&rg.score),
                    year_matches,
                })
            })
            .collect();
        // Year only re-ranks (stable): matching years first, provider order otherwise.
        if query.year.is_some() {
            candidates.sort_by_key(|c| c.year_matches != Some(true));
        }
        Ok(Fetched {
            value: SearchPage {
                candidates,
                total: raw.count,
                offset: raw.offset,
            },
            fetched_at: fetched.fetched_at,
            source: fetched.source,
        })
    }

    async fn editions_impl(
        &self,
        release_group_id: &str,
        cancel: &CancellationToken,
    ) -> AppResult<Fetched<EditionList>> {
        let rgid = parse_uuid("release group", release_group_id)?;
        let mut editions = Vec::new();
        let mut offset = 0_u32;
        let mut total: u32;
        let mut fetched_at = None;
        let mut source = FetchSource::Network;
        loop {
            let url = self.url(
                "release",
                &[
                    ("release-group", rgid.clone()),
                    ("inc", "media".into()),
                    ("limit", BROWSE_LIMIT.to_string()),
                    ("offset", offset.to_string()),
                ],
            );
            let page: Fetched<RawBrowse> = self.get(&url, LOOKUP_TTL, cancel).await?;
            fetched_at.get_or_insert(page.fetched_at);
            source = worse(source, page.source);
            total = page.value.release_count;
            let received = page.value.releases.len() as u32;
            editions.extend(page.value.releases.into_iter().filter_map(edition));
            // Advance by what was actually returned, as the API documentation requires.
            offset += received;
            if received == 0 || offset >= total || editions.len() >= MAX_EDITIONS {
                break;
            }
        }
        editions.truncate(MAX_EDITIONS);
        Ok(Fetched {
            value: EditionList {
                truncated: (offset as usize) < total as usize,
                total,
                editions,
            },
            fetched_at: fetched_at.unwrap_or_else(utc_now),
            source,
        })
    }

    async fn release_group_impl(
        &self,
        release_group_id: &str,
        cancel: &CancellationToken,
    ) -> AppResult<Fetched<ReleaseGroupDetail>> {
        let rgid = parse_uuid("release group", release_group_id)?;
        let url = self.url(
            &format!("release-group/{rgid}"),
            &[("inc", "artist-credits+genres+tags+annotation".into())],
        );
        let f: Fetched<RawReleaseGroup> = self.get(&url, LOOKUP_TTL, cancel).await?;
        let rg = f.value;
        let detail = ReleaseGroupDetail {
            id: parse_uuid("release group", &rg.id)
                .map_err(|_| AppError::Provider("release group without an ID".into()))?,
            title: rg.title,
            disambiguation: non_empty(rg.disambiguation),
            annotation: non_empty(rg.annotation),
            original_date: ProviderDate::parse(rg.first_release_date.as_deref()),
            artist_credit: credits(rg.artist_credit),
            genres: tags(rg.genres),
            tags: tags(rg.tags),
        };
        Ok(Fetched {
            value: detail,
            fetched_at: f.fetched_at,
            source: f.source,
        })
    }

    async fn release_impl(
        &self,
        release_id: &str,
        cancel: &CancellationToken,
    ) -> AppResult<Fetched<ReleaseDetail>> {
        let rid = parse_uuid("release", release_id)?;
        let url = self.url(
            &format!("release/{rid}"),
            &[(
                "inc",
                "recordings+artist-credits+release-groups+media".into(),
            )],
        );
        let f: Fetched<RawRelease> = self.get(&url, LOOKUP_TTL, cancel).await?;
        let r = f.value;
        let mut tracks = Vec::new();
        for (mi, medium) in r.media.iter().enumerate() {
            let disc = medium.position.unwrap_or(mi as u32 + 1).max(1);
            for (ti, t) in medium.tracks.iter().enumerate() {
                let id = parse_uuid("track", &t.id)
                    .map_err(|_| AppError::Provider("release has a track without an ID".into()))?;
                let recording = t.recording.as_ref();
                let title = non_empty(Some(t.title.clone()))
                    .or_else(|| recording.and_then(|rec| non_empty(Some(rec.title.clone()))))
                    .unwrap_or_else(|| "Untitled track".into());
                tracks.push(TrackDetail {
                    id,
                    disc,
                    position: t.position.unwrap_or(ti as u32 + 1).max(1),
                    title,
                    length_ms: t
                        .length
                        .or(recording.and_then(|rec| rec.length))
                        .filter(|l| *l > 0),
                    recording_id: recording.and_then(|rec| parse_uuid("recording", &rec.id).ok()),
                    recording_title: recording.map(|rec| rec.title.clone()),
                    artist_credit: credits(t.artist_credit.clone()),
                });
            }
        }
        let detail = ReleaseDetail {
            id: parse_uuid("release", &r.id)
                .map_err(|_| AppError::Provider("release without an ID".into()))?,
            title: r.title,
            release_group_id: r
                .release_group
                .and_then(|g| parse_uuid("release group", &g.id).ok()),
            disambiguation: non_empty(r.disambiguation),
            date: ProviderDate::parse(r.date.as_deref()),
            country: non_empty(r.country),
            formats: formats(&r.media),
            artist_credit: credits(r.artist_credit),
            tracks,
        };
        Ok(Fetched {
            value: detail,
            fetched_at: f.fetched_at,
            source: f.source,
        })
    }
}

impl MusicBrainz {
    /// Browse (not search) so the catalogue is complete and stably ordered by the provider.
    /// `release-group-status=website-default` keeps the official release groups MusicBrainz
    /// shows on its own artist pages; without it bootlegs and unofficial groups appear.
    /// All types are fetched so EPs, singles, live albums, etc. can be included on request.
    async fn artist_release_groups_impl(
        &self,
        artist_id: &str,
        offset: u32,
        cancel: &CancellationToken,
    ) -> AppResult<Fetched<ReleaseGroupPage>> {
        let aid = parse_uuid("artist", artist_id)?;
        let url = self.url(
            "release-group",
            &[
                ("artist", aid),
                ("inc", "artist-credits".into()),
                ("release-group-status", "website-default".into()),
                ("limit", BROWSE_LIMIT.to_string()),
                ("offset", offset.to_string()),
            ],
        );
        let f: Fetched<RawGroupBrowse> = self.get(&url, LOOKUP_TTL, cancel).await?;
        let raw = f.value;
        let received = raw.release_groups.len() as u32;
        let release_groups = raw
            .release_groups
            .into_iter()
            .filter_map(|rg| {
                Some(CatalogueReleaseGroup {
                    id: parse_uuid("release group", &rg.id).ok()?,
                    title: non_empty(Some(rg.title))?,
                    disambiguation: non_empty(rg.disambiguation),
                    primary_type: non_empty(rg.primary_type),
                    secondary_types: rg.secondary_types,
                    original_date: ProviderDate::parse(rg.first_release_date.as_deref()),
                    artist_credit: credits(rg.artist_credit),
                })
            })
            .collect();
        Ok(Fetched {
            value: ReleaseGroupPage {
                release_groups,
                total: raw.release_group_count,
                offset: raw.release_group_offset.unwrap_or(offset),
                received,
            },
            fetched_at: f.fetched_at,
            source: f.source,
        })
    }
}

impl MetadataProvider for MusicBrainz {
    fn source(&self) -> &'static str {
        "musicbrainz"
    }

    fn search<'a>(
        &'a self,
        query: &'a SearchQuery,
        offset: u32,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, AppResult<Fetched<SearchPage>>> {
        Box::pin(self.search_impl(query, offset, cancel))
    }

    fn editions<'a>(
        &'a self,
        release_group_id: &'a str,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, AppResult<Fetched<EditionList>>> {
        Box::pin(self.editions_impl(release_group_id, cancel))
    }

    fn release_group<'a>(
        &'a self,
        release_group_id: &'a str,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, AppResult<Fetched<ReleaseGroupDetail>>> {
        Box::pin(self.release_group_impl(release_group_id, cancel))
    }

    fn release<'a>(
        &'a self,
        release_id: &'a str,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, AppResult<Fetched<ReleaseDetail>>> {
        Box::pin(self.release_impl(release_id, cancel))
    }

    fn artist_release_groups<'a>(
        &'a self,
        artist_id: &'a str,
        offset: u32,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, AppResult<Fetched<ReleaseGroupPage>>> {
        Box::pin(self.artist_release_groups_impl(artist_id, offset, cancel))
    }
}

/// `releasegroup:"…" AND artist:"…"` with phrase escaping. Year is not a filter: it only
/// re-ranks results, so albums with unknown dates are not hidden.
fn lucene_query(q: &SearchQuery) -> String {
    let phrase = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let mut out = format!("releasegroup:{}", phrase(&q.title));
    if let Some(artist) = &q.artist {
        out.push_str(&format!(" AND artist:{}", phrase(artist)));
    }
    out
}

fn worse(a: FetchSource, b: FetchSource) -> FetchSource {
    let rank = |s| match s {
        FetchSource::Network => 0,
        FetchSource::Cache => 1,
        FetchSource::StaleCache => 2,
    };
    if rank(b) > rank(a) { b } else { a }
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

fn score(raw: &serde_json::Value) -> u8 {
    let n = match raw {
        serde_json::Value::Number(n) => n.as_u64().unwrap_or(0),
        serde_json::Value::String(s) => s.parse().unwrap_or(0),
        _ => 0,
    };
    n.min(100) as u8
}

fn credits(raw: Vec<RawCredit>) -> Vec<CreditPart> {
    raw.into_iter()
        .filter_map(|c| {
            let artist = c.artist?;
            let artist_id = parse_uuid("artist", &artist.id).ok()?;
            let credited = non_empty(c.name).filter(|n| *n != artist.name);
            Some(CreditPart {
                artist_id,
                artist_name: artist.name,
                sort_name: non_empty(artist.sort_name),
                disambiguation: non_empty(artist.disambiguation),
                credited_name: credited,
                join_phrase: c.joinphrase.unwrap_or_default(),
            })
        })
        .collect()
}

fn tags(raw: Vec<RawTag>) -> Vec<SourceTag> {
    raw.into_iter()
        .filter_map(|t| {
            non_empty(Some(t.name)).map(|name| SourceTag {
                name,
                votes: t.count,
            })
        })
        .collect()
}

fn formats(media: &[RawMedium]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for m in media {
        let f = m.format.clone().unwrap_or_else(|| "Unknown format".into());
        if !out.contains(&f) {
            out.push(f);
        }
    }
    out
}

fn edition(r: RawRelease) -> Option<EditionCandidate> {
    Some(EditionCandidate {
        id: parse_uuid("release", &r.id).ok()?,
        formats: formats(&r.media),
        track_count: r.media.iter().map(|m| m.track_count.unwrap_or(0)).sum(),
        title: r.title,
        disambiguation: non_empty(r.disambiguation),
        date: ProviderDate::parse(r.date.as_deref()),
        country: non_empty(r.country),
        status: r.status,
    })
}

// ---------------------------------------------------------------- wire format
// Every field is optional/defaulted: missing provider data must degrade, not fail.

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawSearch {
    count: u32,
    offset: u32,
    #[serde(rename = "release-groups")]
    release_groups: Vec<RawReleaseGroup>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawReleaseGroup {
    id: String,
    title: String,
    score: serde_json::Value,
    disambiguation: Option<String>,
    #[serde(rename = "primary-type")]
    primary_type: Option<String>,
    #[serde(rename = "secondary-types")]
    secondary_types: Vec<String>,
    #[serde(rename = "first-release-date")]
    first_release_date: Option<String>,
    #[serde(rename = "artist-credit")]
    artist_credit: Vec<RawCredit>,
    genres: Vec<RawTag>,
    tags: Vec<RawTag>,
    annotation: Option<String>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
struct RawCredit {
    name: Option<String>,
    joinphrase: Option<String>,
    artist: Option<RawArtist>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
struct RawArtist {
    id: String,
    name: String,
    #[serde(rename = "sort-name")]
    sort_name: Option<String>,
    disambiguation: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawTag {
    name: String,
    count: i64,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawGroupBrowse {
    #[serde(rename = "release-group-count")]
    release_group_count: u32,
    #[serde(rename = "release-group-offset")]
    release_group_offset: Option<u32>,
    #[serde(rename = "release-groups")]
    release_groups: Vec<RawReleaseGroup>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawBrowse {
    #[serde(rename = "release-count")]
    release_count: u32,
    releases: Vec<RawRelease>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawRelease {
    id: String,
    title: String,
    status: Option<String>,
    date: Option<String>,
    country: Option<String>,
    disambiguation: Option<String>,
    #[serde(rename = "release-group")]
    release_group: Option<RawId>,
    #[serde(rename = "artist-credit")]
    artist_credit: Vec<RawCredit>,
    media: Vec<RawMedium>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawId {
    id: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawMedium {
    position: Option<u32>,
    format: Option<String>,
    #[serde(rename = "track-count")]
    track_count: Option<u32>,
    tracks: Vec<RawTrack>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawTrack {
    id: String,
    position: Option<u32>,
    title: String,
    length: Option<i64>,
    recording: Option<RawRecording>,
    #[serde(rename = "artist-credit")]
    artist_credit: Vec<RawCredit>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawRecording {
    id: String,
    title: String,
    length: Option<i64>,
}

#[cfg(test)]
mod tests;
