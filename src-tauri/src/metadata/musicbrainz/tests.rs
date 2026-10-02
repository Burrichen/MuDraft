//! Provider tests against scripted responses. No network access.

use std::sync::Arc;
use std::time::Duration;

use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::*;
use crate::metadata::cache::MemoryCache;
use crate::metadata::testing::{MockTransport, Reply};

const RG: &str = "b1392450-e666-3926-a536-22c65f834433";
const RG2: &str = "0a1b2c3d-0000-4000-8000-000000000002";
const ARTIST: &str = "a74b1b7f-71a5-4011-9441-d0b5e4122711";

fn search_json() -> String {
    format!(
        r#"{{"count": 2, "offset": 0, "release-groups": [
          {{"id": "{RG2}", "score": 100, "title": "OK Computer OKNOTOK 1997 2017",
            "primary-type": "Album", "secondary-types": ["Compilation"],
            "first-release-date": "2017-06-23",
            "artist-credit": [{{"name": "Radiohead", "artist": {{"id": "{ARTIST}", "name": "Radiohead", "sort-name": "Radiohead"}}}}]}},
          {{"id": "{RG}", "score": "97", "title": "OK Computer",
            "first-release-date": "1997",
            "artist-credit": [{{"name": "Radiohead", "artist": {{"id": "{ARTIST}", "name": "Radiohead"}}}}]}},
          {{"id": "not-a-uuid", "title": "Broken"}},
          {{"id": "0a1b2c3d-0000-4000-8000-000000000003", "title": "Bare"}}
        ]}}"#
    )
}

fn provider(transport: Arc<MockTransport>, cache: Arc<MemoryCache>) -> MusicBrainz {
    MusicBrainz::new(transport, cache)
}

fn query(title: &str, year: Option<i64>) -> SearchQuery {
    SearchQuery {
        title: title.into(),
        artist: Some("Radiohead".into()),
        year,
    }
}

#[tokio::test(start_paused = true)]
async fn search_builds_escaped_query_and_parses_missing_data() {
    let t = Arc::new(MockTransport::new(|_, _| Reply::Json(200, search_json())));
    let mb = provider(t.clone(), Arc::default());
    let page = mb
        .search(
            &SearchQuery {
                title: " OK \"Computer\" ".into(),
                artist: Some("  ".into()),
                year: None,
            },
            0,
            &CancellationToken::new(),
        )
        .await
        .unwrap();

    let url = Url::parse(&t.urls()[0]).unwrap();
    let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(url.path(), "/ws/2/release-group");
    assert_eq!(
        q["query"], r#"releasegroup:"OK \"Computer\"""#,
        "blank artist is dropped"
    );
    assert_eq!(q["fmt"], "json");
    assert_eq!(q["limit"], "25");

    let c = &page.value.candidates;
    assert_eq!(c.len(), 3, "entries without a valid ID are skipped");
    assert_eq!(page.source, FetchSource::Network);
    assert_eq!(c[1].score, 97, "string scores are accepted");
    assert_eq!(c[1].original_date.precision, "year");
    assert_eq!(c[1].original_date.year, Some(1997));
    assert_eq!(c[0].secondary_types, vec!["Compilation"]);
    let bare = &c[2];
    assert!(bare.artist_credit.is_empty());
    assert_eq!(bare.original_date.precision, "unknown");
    assert_eq!(bare.primary_type, None);
    assert_eq!(bare.score, 0);
}

#[tokio::test(start_paused = true)]
async fn year_reranks_without_filtering_and_nothing_is_auto_accepted() {
    let t = Arc::new(MockTransport::new(|_, _| Reply::Json(200, search_json())));
    let mb = provider(t.clone(), Arc::default());
    let page = mb
        .search(
            &query("OK Computer", Some(1997)),
            0,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    let c = &page.value.candidates;
    assert_eq!(c[0].id, RG, "year match moves up");
    assert_eq!(c[0].year_matches, Some(true));
    assert!(c[0].exact_title);
    assert_eq!(c[1].year_matches, Some(false));
    assert_eq!(c[2].year_matches, None, "unknown dates are kept");
    let q = Url::parse(&t.urls()[0])
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "query")
        .unwrap()
        .1
        .into_owned();
    assert_eq!(q, r#"releasegroup:"OK Computer" AND artist:"Radiohead""#);
}

#[tokio::test(start_paused = true)]
async fn repeated_requests_use_the_cache() {
    let t = Arc::new(MockTransport::new(|_, _| Reply::Json(200, search_json())));
    let mb = provider(t.clone(), Arc::default());
    let cancel = CancellationToken::new();
    mb.search(&query("OK Computer", None), 0, &cancel)
        .await
        .unwrap();
    let again = mb
        .search(&query("OK Computer", None), 0, &cancel)
        .await
        .unwrap();
    assert_eq!(again.source, FetchSource::Cache);
    assert_eq!(t.call_count(), 1);
}

#[tokio::test(start_paused = true)]
async fn requests_share_one_queue_at_one_per_second() {
    let t = Arc::new(MockTransport::new(|_, _| Reply::Json(200, search_json())));
    let mb = Arc::new(provider(t.clone(), Arc::default()));
    let cancel = CancellationToken::new();
    let titles = ["A", "B", "C", "D"];
    let tasks: Vec<_> = titles
        .iter()
        .map(|title| {
            let mb = mb.clone();
            let cancel = cancel.clone();
            let q = query(title, None);
            tokio::spawn(async move { mb.search(&q, 0, &cancel).await.map(|_| ()) })
        })
        .collect();
    for task in tasks {
        task.await.unwrap().unwrap();
    }
    let times: Vec<Instant> = t.calls.lock().unwrap().iter().map(|(_, at)| *at).collect();
    assert_eq!(times.len(), 4);
    for pair in times.windows(2) {
        assert!(
            pair[1] - pair[0] >= Duration::from_secs(1),
            "{:?}",
            pair[1] - pair[0]
        );
    }
}

#[tokio::test(start_paused = true)]
async fn rate_limit_responses_back_off_honour_retry_after_and_are_bounded() {
    // 503 with Retry-After: 4 → then success.
    let t = Arc::new(MockTransport::new(|_, n| {
        if n == 0 {
            Reply::RateLimited(Some(4))
        } else {
            Reply::Json(200, search_json())
        }
    }));
    let mb = provider(t.clone(), Arc::default());
    let start = Instant::now();
    mb.search(&query("OK Computer", None), 0, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(t.call_count(), 2);
    assert!(start.elapsed() >= Duration::from_secs(4));

    // Persistent 503: exactly max_attempts tries, then a rate_limited error.
    let t = Arc::new(MockTransport::new(|_, _| Reply::RateLimited(None)));
    let mb = provider(t.clone(), Arc::default());
    let err = mb
        .search(&query("OK Computer", None), 0, &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(err.code(), "rate_limited");
    assert_eq!(t.call_count(), RetryPolicy::default().max_attempts as usize);
}

#[tokio::test(start_paused = true)]
async fn timeouts_retry_then_fail_with_timeout() {
    let t = Arc::new(MockTransport::new(|_, n| {
        if n < 1 {
            Reply::Timeout
        } else {
            Reply::Json(200, search_json())
        }
    }));
    let mb = provider(t.clone(), Arc::default());
    mb.search(&query("OK Computer", None), 0, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(t.call_count(), 2);

    let t = Arc::new(MockTransport::new(|_, _| Reply::Timeout));
    let mb = provider(t.clone(), Arc::default());
    let err = mb
        .search(&query("OK Computer", None), 0, &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(err.code(), "timeout");
    assert_eq!(t.call_count(), 3);
}

#[tokio::test(start_paused = true)]
async fn offline_serves_stale_cache_or_reports_offline() {
    let online = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let flag = online.clone();
    let t = Arc::new(MockTransport::new(move |_, _| {
        if flag.load(std::sync::atomic::Ordering::SeqCst) {
            Reply::Json(200, search_json())
        } else {
            Reply::Offline
        }
    }));
    let cache = Arc::new(MemoryCache::default());
    let mb = provider(t.clone(), cache.clone());
    let cancel = CancellationToken::new();
    let first = mb
        .search(&query("OK Computer", None), 0, &cancel)
        .await
        .unwrap();

    online.store(false, std::sync::atomic::Ordering::SeqCst);
    cache.expire_all();
    let stale = mb
        .search(&query("OK Computer", None), 0, &cancel)
        .await
        .unwrap();
    assert_eq!(stale.source, FetchSource::StaleCache);
    assert_eq!(
        stale.fetched_at, first.fetched_at,
        "original fetch time is preserved"
    );
    assert_eq!(stale.value, first.value);
    let calls_before = t.call_count();

    let err = mb
        .search(&query("Kid A", None), 0, &cancel)
        .await
        .unwrap_err();
    assert_eq!(err.code(), "offline");
    assert_eq!(
        t.call_count(),
        calls_before + 1,
        "offline is not hammered with retries"
    );
}

#[tokio::test(start_paused = true)]
async fn cancellation_while_queued_or_in_flight() {
    let t = Arc::new(MockTransport::new(|_, n| {
        if n == 0 {
            Reply::Json(200, search_json())
        } else {
            Reply::Hang
        }
    }));
    let mb = Arc::new(provider(t.clone(), Arc::default()));
    mb.search(&query("A", None), 0, &CancellationToken::new())
        .await
        .unwrap();

    // In flight: the second request hangs until cancelled.
    let cancel = CancellationToken::new();
    let task = {
        let mb = mb.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move { mb.search(&query("B", None), 0, &cancel).await })
    };
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(t.call_count(), 2);
    cancel.cancel();
    assert_eq!(task.await.unwrap().unwrap_err().code(), "cancelled");

    // Already cancelled: never reaches the transport.
    let done = CancellationToken::new();
    done.cancel();
    assert_eq!(
        mb.search(&query("C", None), 0, &done)
            .await
            .unwrap_err()
            .code(),
        "cancelled"
    );
    assert_eq!(t.call_count(), 2);
}

#[tokio::test(start_paused = true)]
async fn editions_follow_pagination_by_returned_count() {
    let release = |i: usize| {
        format!(
            r#"{{"id": "00000000-0000-4000-8000-{i:012}", "title": "OK Computer", "status": "Official",
                "date": "1997-05-21", "country": "GB", "media": [{{"format": "CD", "track-count": 12}}]}}"#
        )
    };
    let t = Arc::new(MockTransport::new(move |url, _| {
        let offset: usize = Url::parse(url)
            .unwrap()
            .query_pairs()
            .find(|(k, _)| k == "offset")
            .unwrap()
            .1
            .parse()
            .unwrap();
        // Server returns fewer than asked on the first page (allowed by the API).
        let size = if offset == 0 { 90 } else { 140 - offset };
        let items: Vec<String> = (offset..offset + size.min(100)).map(release).collect();
        Reply::Json(
            200,
            format!(
                r#"{{"release-count": 140, "release-offset": {offset}, "releases": [{}]}}"#,
                items.join(",")
            ),
        )
    }));
    let mb = provider(t.clone(), Arc::default());
    let list = mb
        .editions(RG, &CancellationToken::new())
        .await
        .unwrap()
        .value;
    assert_eq!(list.total, 140);
    assert_eq!(list.editions.len(), 140);
    assert!(!list.truncated);
    let offsets: Vec<String> = t
        .urls()
        .iter()
        .map(|u| {
            Url::parse(u)
                .unwrap()
                .query_pairs()
                .find(|(k, _)| k == "offset")
                .unwrap()
                .1
                .into_owned()
        })
        .collect();
    assert_eq!(offsets, vec!["0", "90"]);
    assert_eq!(list.editions[0].formats, vec!["CD"]);
    assert_eq!(list.editions[0].track_count, 12);
    assert_eq!(list.editions[0].date.precision, "day");
}

#[tokio::test(start_paused = true)]
async fn release_detail_maps_media_tracks_and_ids() {
    let body = format!(
        r#"{{"id": "11111111-1111-4111-8111-111111111111", "title": "OK Computer",
            "release-group": {{"id": "{RG}"}}, "date": "1997-06", "country": "",
            "artist-credit": [{{"name": "Radiohead", "artist": {{"id": "{ARTIST}", "name": "Radiohead"}}}}],
            "media": [
              {{"position": 1, "format": "CD", "tracks": [
                {{"id": "22222222-2222-4222-8222-222222222221", "position": 1, "title": "Airbag", "length": 284000,
                  "recording": {{"id": "33333333-3333-4333-8333-333333333331", "title": "Airbag", "length": 284000}}}},
                {{"id": "22222222-2222-4222-8222-222222222222", "position": 2, "title": "",
                  "recording": {{"id": "33333333-3333-4333-8333-333333333332", "title": "Paranoid Android", "length": 383000}}}}
              ]}},
              {{"format": "CD", "tracks": [{{"id": "22222222-2222-4222-8222-222222222223", "title": "Bonus"}}]}}
            ]}}"#
    );
    let t = Arc::new(MockTransport::new(move |_, _| {
        Reply::Json(200, body.clone())
    }));
    let mb = provider(t.clone(), Arc::default());
    let r = mb
        .release(
            "11111111-1111-4111-8111-111111111111",
            &CancellationToken::new(),
        )
        .await
        .unwrap()
        .value;
    assert_eq!(r.release_group_id.as_deref(), Some(RG));
    assert_eq!(r.date.precision, "month");
    assert_eq!(r.country, None);
    assert_eq!(r.tracks.len(), 3);
    assert_eq!(
        r.tracks[1].title, "Paranoid Android",
        "falls back to the recording title"
    );
    assert_eq!(r.tracks[1].length_ms, Some(383_000));
    assert_eq!((r.tracks[2].disc, r.tracks[2].position), (2, 1));
    assert_eq!(r.tracks[2].recording_id, None);
    assert!(t.urls()[0].contains("inc=recordings%2Bartist-credits%2Brelease-groups%2Bmedia"));
}

#[tokio::test(start_paused = true)]
async fn invalid_ids_are_rejected_before_any_request() {
    let t = Arc::new(MockTransport::new(|_, _| Reply::Json(200, "{}".into())));
    let mb = provider(t.clone(), Arc::default());
    let err = mb
        .release("../../admin", &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(err.code(), "validation");
    let err = mb
        .search(&query("   ", None), 0, &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(err.code(), "validation");
    assert_eq!(t.call_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn not_found_and_bad_json_are_reported() {
    let t = Arc::new(MockTransport::new(|_, n| {
        if n == 0 {
            Reply::Json(404, "{}".into())
        } else {
            Reply::Json(200, "<html>".into())
        }
    }));
    let mb = provider(t.clone(), Arc::default());
    assert_eq!(
        mb.release_group(RG, &CancellationToken::new())
            .await
            .unwrap_err()
            .code(),
        "not_found"
    );
    assert_eq!(
        mb.release_group(RG, &CancellationToken::new())
            .await
            .unwrap_err()
            .code(),
        "provider_error"
    );
}

/// Live check against musicbrainz.org through the real transport and User-Agent.
/// Never runs by default: `cargo test live_musicbrainz -- --ignored`.
#[tokio::test]
#[ignore = "network: contacts musicbrainz.org"]
async fn live_musicbrainz_search_editions_and_release() {
    let transport = Arc::new(crate::metadata::http::ReqwestTransport::new().unwrap());
    let mb = MusicBrainz::new(transport, Arc::new(MemoryCache::default()));
    let cancel = CancellationToken::new();
    let q = SearchQuery {
        title: "OK Computer".into(),
        artist: Some("Radiohead".into()),
        year: Some(1997),
    };
    let page = mb.search(&q, 0, &cancel).await.unwrap();
    let top = page
        .value
        .candidates
        .iter()
        .find(|c| c.exact_title)
        .expect("an exact-title candidate");
    println!(
        "top: {} ({:?}) by {:?} score {}",
        top.title,
        top.original_date.value,
        top.artist_credit.first().map(|a| &a.artist_name),
        top.score
    );
    let editions = mb.editions(&top.id, &cancel).await.unwrap().value;
    println!(
        "editions: {} of {} (truncated {})",
        editions.editions.len(),
        editions.total,
        editions.truncated
    );
    let first = editions.editions.first().expect("at least one edition");
    let release = mb.release(&first.id, &cancel).await.unwrap().value;
    println!(
        "release {} tracks {} rg {:?}",
        release.title,
        release.tracks.len(),
        release.release_group_id
    );
    assert_eq!(release.release_group_id.as_deref(), Some(top.id.as_str()));
    let group = mb.release_group(&top.id, &cancel).await.unwrap().value;
    println!(
        "genres {:?} -> {:?}",
        group
            .genres
            .iter()
            .take(5)
            .map(|g| &g.name)
            .collect::<Vec<_>>(),
        crate::metadata::genres::normalize(&group.genres, &group.tags)
    );
}
