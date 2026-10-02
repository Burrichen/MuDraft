//! Optional online enrichment. Rows are looked up one at a time through the provider's
//! shared queue (never one request per row at once), in bounded batches the renderer asks
//! for. Each row's state is stored, so stopping — offline, rate limited, cancelled, or the
//! app closing — loses nothing and the next call resumes where it stopped.
//!
//! Lookups never accept a fuzzy match: search results become candidates for review.
//! Only IDs the user wrote in the CSV are resolved automatically (plus a release group
//! with exactly one edition, which leaves nothing to choose).

use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use super::staging::{self, Candidates, ChosenBy, Decision, MatchDetails};
use crate::error::{AppError, AppResult};
use crate::metadata::{MetadataProvider, SearchQuery};
use crate::state::AppState;

pub const MAX_BATCH: u32 = 25;
const MAX_CANDIDATES: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stopped {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrichProgress {
    pub processed: u32,
    pub remaining: u32,
    /// Why the batch ended early (offline, rate_limited, cancelled, …). Resumable.
    pub stopped: Option<Stopped>,
}

fn remaining(state: &AppState, session_id: &str) -> AppResult<u32> {
    state.with_db(|db| {
        db.read(|c| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM import_row WHERE session_id = ?1 AND enrichment = 'not_started'",
                params![session_id],
                |r| r.get(0),
            )?)
        })
    })
}

fn next_row(state: &AppState, session_id: &str) -> AppResult<Option<staging::StagedRow>> {
    state.with_db(|db| {
        db.read(|c| {
            staging::require_open(c, session_id)?;
            let n: Option<u32> = c
                .query_row(
                    "SELECT row_number FROM import_row WHERE session_id = ?1 AND enrichment = 'not_started'
                     ORDER BY row_number LIMIT 1",
                    params![session_id],
                    |r| r.get(0),
                )
                .optional()?;
            n.map(|n| staging::row(c, session_id, n)).transpose()
        })
    })
}

/// Look up at most `max_rows` pending rows.
pub async fn enrich(
    provider: &dyn MetadataProvider,
    state: &AppState,
    session_id: &str,
    max_rows: u32,
    cancel: &CancellationToken,
) -> AppResult<EnrichProgress> {
    let mut processed = 0;
    let mut stopped = None;
    while processed < max_rows.clamp(1, MAX_BATCH) {
        let Some(row) = next_row(state, session_id)? else {
            break;
        };
        match enrich_row(provider, &row, cancel).await {
            Ok(outcome) => {
                state.with_db(|db| {
                    db.write(|tx| {
                        staging::store_enrichment(
                            tx,
                            session_id,
                            row.row_number,
                            outcome.state,
                            outcome.candidates.as_ref(),
                            outcome.resolved.as_ref().map(|(d, m)| (d, m)),
                        )
                    })
                })?;
                processed += 1;
            }
            Err(AppError::NotFound { .. }) => {
                let failed = Candidates::Failed {
                    message: "MusicBrainz has no entry for the ID in this row. Check it, or keep the row as manual."
                        .into(),
                };
                state.with_db(|db| {
                    db.write(|tx| {
                        staging::store_enrichment(
                            tx,
                            session_id,
                            row.row_number,
                            "failed",
                            Some(&failed),
                            None,
                        )
                    })
                })?;
                processed += 1;
            }
            Err(AppError::Validation { reason, .. }) => {
                let failed = Candidates::Failed { message: reason };
                state.with_db(|db| {
                    db.write(|tx| {
                        staging::store_enrichment(
                            tx,
                            session_id,
                            row.row_number,
                            "failed",
                            Some(&failed),
                            None,
                        )
                    })
                })?;
                processed += 1;
            }
            // Offline, rate limited, timeout, cancelled, provider errors: stop, keep the row pending.
            Err(e) => {
                stopped = Some(Stopped {
                    code: e.code().into(),
                    message: e.to_string(),
                });
                break;
            }
        }
    }
    Ok(EnrichProgress {
        processed,
        remaining: remaining(state, session_id)?,
        stopped,
    })
}

struct RowOutcome {
    state: &'static str,
    candidates: Option<Candidates>,
    resolved: Option<(Decision, MatchDetails)>,
}

async fn resolve_ids(
    provider: &dyn MetadataProvider,
    group_hint: Option<&str>,
    release_id: &str,
    edition_name: Option<String>,
    cancel: &CancellationToken,
) -> AppResult<RowOutcome> {
    let release = provider.release(release_id, cancel).await?;
    let group_id = release.value.release_group_id.clone().ok_or_else(|| {
        AppError::validation(
            "release ID",
            "MusicBrainz did not say which album this release belongs to",
        )
    })?;
    if let Some(hint) = group_hint
        && hint != group_id
    {
        return Err(AppError::validation(
            "release ID",
            "the release ID belongs to a different album than the release group ID in this row",
        ));
    }
    let group = provider.release_group(&group_id, cancel).await?;
    let decision = Decision::Match {
        release_group_id: Some(group_id),
        release_id: Some(release_id.to_owned()),
        edition_name,
        chosen_by: ChosenBy::CsvIds,
    };
    let details = MatchDetails {
        group_fetched_at: group.fetched_at,
        release_fetched_at: release.fetched_at,
        release_group: group.value,
        release: release.value,
    };
    Ok(RowOutcome {
        state: "resolved",
        candidates: None,
        resolved: Some((decision, details)),
    })
}

async fn enrich_row(
    provider: &dyn MetadataProvider,
    row: &staging::StagedRow,
    cancel: &CancellationToken,
) -> AppResult<RowOutcome> {
    let Some(fields) = &row.fields else {
        return Ok(RowOutcome {
            state: "not_needed",
            candidates: None,
            resolved: None,
        });
    };
    match &row.decision {
        Decision::Match {
            release_group_id,
            release_id: Some(rid),
            edition_name,
            ..
        } => {
            resolve_ids(
                provider,
                release_group_id.as_deref(),
                rid,
                edition_name.clone(),
                cancel,
            )
            .await
        }
        Decision::Match {
            release_group_id: Some(gid),
            release_id: None,
            edition_name,
            ..
        } => {
            let editions = provider.editions(gid, cancel).await?.value.editions;
            match editions.as_slice() {
                [only] => {
                    resolve_ids(provider, Some(gid), &only.id, edition_name.clone(), cancel).await
                }
                [] => Ok(RowOutcome {
                    state: "failed",
                    candidates: Some(Candidates::Failed {
                        message: "MusicBrainz lists no editions for this album.".into(),
                    }),
                    resolved: None,
                }),
                _ => Ok(RowOutcome {
                    state: "candidates",
                    candidates: Some(Candidates::Editions { editions }),
                    resolved: None,
                }),
            }
        }
        Decision::Manual => {
            let query = SearchQuery {
                title: fields.album.clone(),
                artist: Some(fields.artist.clone()),
                year: fields.year,
            };
            let mut found = provider.search(&query, 0, cancel).await?.value.candidates;
            found.truncate(MAX_CANDIDATES);
            if found.is_empty() {
                Ok(RowOutcome {
                    state: "no_results",
                    candidates: None,
                    resolved: None,
                })
            } else {
                Ok(RowOutcome {
                    state: "candidates",
                    candidates: Some(Candidates::Groups { candidates: found }),
                    resolved: None,
                })
            }
        }
        _ => Ok(RowOutcome {
            state: "not_needed",
            candidates: None,
            resolved: None,
        }),
    }
}

/// Fetch details for a match the user picked, so it can be committed offline later.
pub async fn fetch_match(
    provider: &dyn MetadataProvider,
    release_group_id: &str,
    release_id: &str,
    cancel: &CancellationToken,
) -> AppResult<MatchDetails> {
    let group = provider.release_group(release_group_id, cancel).await?;
    let release = provider.release(release_id, cancel).await?;
    Ok(MatchDetails {
        group_fetched_at: group.fetched_at,
        release_fetched_at: release.fetched_at,
        release_group: group.value,
        release: release.value,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use super::*;
    use crate::csv_import::commit::{self, TagDecisions};
    use crate::csv_import::{mapping, parse};
    use crate::metadata::cache::MemoryCache;
    use crate::metadata::musicbrainz::MusicBrainz;
    use crate::metadata::testing::{MockTransport, Reply};
    use crate::paths::StorageProfile;

    const RG: &str = "b1392450-e666-3926-a536-22c65f834433";
    const REL: &str = "11111111-1111-4111-8111-111111111111";
    const REL2: &str = "11111111-1111-4111-8111-222222222222";
    const ARTIST: &str = "a74b1b7f-71a5-4011-9441-d0b5e4122711";

    fn reply(url: &str) -> Reply {
        let credit = format!(
            r#"[{{"name": "Radiohead", "artist": {{"id": "{ARTIST}", "name": "Radiohead"}}}}]"#
        );
        if url.contains("/release-group/") {
            Reply::Json(
                200,
                format!(
                    r#"{{"id": "{RG}", "title": "OK Computer", "first-release-date": "1997-05-21", "artist-credit": {credit}}}"#
                ),
            )
        } else if url.contains("/release/") {
            let id = if url.contains(REL2) { REL2 } else { REL };
            Reply::Json(
                200,
                format!(
                    r#"{{"id": "{id}", "title": "OK Computer", "release-group": {{"id": "{RG}"}}, "artist-credit": {credit},
                "media": [{{"position": 1, "tracks": [{{"id": "22222222-2222-4222-8222-22222222222{}", "position": 1, "title": "Airbag"}}]}}]}}"#,
                    if id == REL { 1 } else { 2 }
                ),
            )
        } else if url.contains("release-group=") {
            let many = url.contains("many");
            let editions = if many {
                format!(
                    r#"{{"id": "{REL}", "title": "OK Computer"}}, {{"id": "{REL2}", "title": "OK Computer"}}"#
                )
            } else {
                format!(r#"{{"id": "{REL}", "title": "OK Computer"}}"#)
            };
            Reply::Json(
                200,
                format!(
                    r#"{{"release-count": {}, "releases": [{editions}]}}"#,
                    if many { 2 } else { 1 }
                ),
            )
        } else {
            // Search: two plausible candidates — ambiguous by design.
            Reply::Json(
                200,
                format!(
                    r#"{{"count": 2, "offset": 0, "release-groups": [
                {{"id": "{RG}", "score": 100, "title": "OK Computer", "artist-credit": {credit}}},
                {{"id": "0a1b2c3d-0000-4000-8000-000000000002", "score": 90, "title": "OK Computer (tribute)"}}]}}"#
                ),
            )
        }
    }

    fn setup(
        csv: &str,
        transport: Arc<MockTransport>,
    ) -> (tempfile::TempDir, AppState, MusicBrainz, String) {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::open(StorageProfile::Development, dir.path().to_path_buf());
        let parsed = parse::parse(csv.as_bytes()).unwrap();
        let session = state
            .with_db(|db| {
                db.write(|tx| {
                    let id = staging::create_session(tx, "a.csv", &parsed)?;
                    staging::apply_mapping(tx, &id, &mapping::suggest(&parsed.headers))?;
                    Ok(id)
                })
            })
            .unwrap();
        let mb = MusicBrainz::new(transport, Arc::new(MemoryCache::default()));
        (dir, state, mb, session)
    }

    fn row(state: &AppState, session: &str, n: u32) -> staging::StagedRow {
        state
            .with_db(|db| db.read(|c| staging::row(c, session, n)))
            .unwrap()
    }

    const HEADER: &str =
        "Album,Artist,Year,Edition,MusicBrainz Release Group ID,MusicBrainz Release ID\n";

    #[tokio::test(start_paused = true)]
    async fn batches_are_bounded_sequential_and_resumable() {
        let t = Arc::new(MockTransport::new(|url, _| reply(url)));
        let csv = format!("{HEADER}A,X,,,,\nB,X,,,,\nC,X,,,,\n");
        let (_d, state, mb, s) = setup(&csv, t.clone());
        let cancel = CancellationToken::new();
        let first = enrich(&mb, &state, &s, 2, &cancel).await.unwrap();
        assert_eq!(
            (first.processed, first.remaining, first.stopped.clone()),
            (2, 1, None)
        );
        let second = enrich(&mb, &state, &s, 10, &cancel).await.unwrap();
        assert_eq!((second.processed, second.remaining), (1, 0));
        let times: Vec<_> = t.calls.lock().unwrap().iter().map(|(_, at)| *at).collect();
        assert_eq!(times.len(), 3);
        assert!(
            times
                .windows(2)
                .all(|w| w[1] - w[0] >= Duration::from_secs(1)),
            "one shared queue, never concurrent"
        );
        assert_eq!(
            enrich(&mb, &state, &s, 10, &cancel)
                .await
                .unwrap()
                .processed,
            0
        );
    }

    #[tokio::test(start_paused = true)]
    async fn ambiguous_results_become_candidates_never_matches() {
        let t = Arc::new(MockTransport::new(|url, _| reply(url)));
        let (_d, state, mb, s) = setup(&format!("{HEADER}OK Computer,Radiohead,1997,,,\n"), t);
        enrich(&mb, &state, &s, 5, &CancellationToken::new())
            .await
            .unwrap();
        let r = row(&state, &s, 2);
        assert_eq!(
            r.decision,
            Decision::Manual,
            "even a 100 score is not accepted silently"
        );
        match r.candidates {
            Some(Candidates::Groups { candidates }) => assert_eq!(candidates.len(), 2),
            c => panic!("{c:?}"),
        }
        assert_eq!(
            r.readiness,
            staging::Readiness::Ready,
            "still importable as manual"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn offline_stops_without_losing_progress_then_resumes() {
        let online = Arc::new(AtomicBool::new(false));
        let flag = online.clone();
        let t = Arc::new(MockTransport::new(move |url, _| {
            if flag.load(Ordering::SeqCst) {
                reply(url)
            } else {
                Reply::Offline
            }
        }));
        let (_d, state, mb, s) = setup(&format!("{HEADER}A,X,,,,\nB,X,,,,\n"), t);
        let cancel = CancellationToken::new();
        let p = enrich(&mb, &state, &s, 5, &cancel).await.unwrap();
        assert_eq!(p.processed, 0);
        assert_eq!(p.remaining, 2);
        assert_eq!(p.stopped.unwrap().code, "offline");
        assert_eq!(row(&state, &s, 2).enrichment, "not_started");

        online.store(true, Ordering::SeqCst);
        let p = enrich(&mb, &state, &s, 5, &cancel).await.unwrap();
        assert_eq!((p.processed, p.remaining), (2, 0));
    }

    #[tokio::test(start_paused = true)]
    async fn csv_ids_resolve_and_commit_offline_afterwards() {
        let online = Arc::new(AtomicBool::new(true));
        let flag = online.clone();
        let t = Arc::new(MockTransport::new(move |url, _| {
            if flag.load(Ordering::SeqCst) {
                reply(url)
            } else {
                Reply::Offline
            }
        }));
        let csv = format!("{HEADER}OK Computer,Radiohead,,UK CD,,{REL}\n");
        let (_d, state, mb, s) = setup(&csv, t);
        assert_eq!(
            row(&state, &s, 2).readiness,
            staging::Readiness::NeedsLookup
        );
        enrich(&mb, &state, &s, 5, &CancellationToken::new())
            .await
            .unwrap();
        let r = row(&state, &s, 2);
        assert_eq!(r.readiness, staging::Readiness::Ready);
        assert!(r.has_details);

        online.store(false, Ordering::SeqCst);
        let report = state
            .with_db(|db| db.write(|tx| commit::commit(tx, &s, &TagDecisions::default())))
            .unwrap();
        assert_eq!(report.albums_created, 1);
        let (name, rel): (String, String) = state
            .with_db(|db| {
                db.read(|c| {
                    Ok(c.query_row(
                        "SELECT name, musicbrainz_release_id FROM edition",
                        [],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )?)
                })
            })
            .unwrap();
        assert_eq!(
            (name.as_str(), rel.as_str()),
            ("UK CD", REL),
            "CSV edition name is kept"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn release_group_ids_resolve_single_editions_and_ask_for_many() {
        let t = Arc::new(MockTransport::new(|url, _| {
            // Pretend the second release group has many editions.
            if url.contains("release-group=0a1b2c3d") {
                reply(&format!("{url}&many"))
            } else {
                reply(url)
            }
        }));
        let csv = format!(
            "{HEADER}OK Computer,Radiohead,,,{RG},\nOther,Radiohead,,,0a1b2c3d-0000-4000-8000-000000000002,\n"
        );
        let (_d, state, mb, s) = setup(&csv, t);
        enrich(&mb, &state, &s, 5, &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(row(&state, &s, 2).readiness, staging::Readiness::Ready);
        let many = row(&state, &s, 3);
        assert_eq!(many.readiness, staging::Readiness::NeedsEdition);
        assert!(
            matches!(many.candidates, Some(Candidates::Editions { ref editions }) if editions.len() == 2)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn mismatched_ids_fail_the_row_not_the_batch() {
        let t = Arc::new(MockTransport::new(|url, _| reply(url)));
        let csv = format!("{HEADER}X,Y,,,0a1b2c3d-0000-4000-8000-000000000009,{REL}\nA,B,,,,\n");
        let (_d, state, mb, s) = setup(&csv, t);
        let p = enrich(&mb, &state, &s, 5, &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!((p.processed, p.stopped), (2, None));
        let r = row(&state, &s, 2);
        assert_eq!(r.enrichment, "failed");
        assert!(
            matches!(r.candidates, Some(Candidates::Failed { ref message }) if message.contains("different album"))
        );
        assert_eq!(
            r.readiness,
            staging::Readiness::NeedsLookup,
            "blocks commit until the user decides"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn cancellation_stops_the_batch_and_keeps_rows_pending() {
        let t = Arc::new(MockTransport::new(|_, _| Reply::Hang));
        let (_d, state, mb, s) = setup(&format!("{HEADER}A,X,,,,\n"), t);
        let cancel = CancellationToken::new();
        let stopper = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            stopper.cancel();
        });
        let p = enrich(&mb, &state, &s, 5, &cancel).await.unwrap();
        assert_eq!(p.stopped.unwrap().code, "cancelled");
        assert_eq!(row(&state, &s, 2).enrichment, "not_started");
    }

    #[tokio::test(start_paused = true)]
    async fn change_match_stores_details_for_commit() {
        let t = Arc::new(MockTransport::new(|url, _| reply(url)));
        let (_d, state, mb, s) = setup(&format!("{HEADER}OK Computer,Radiohead,,Deluxe,,\n"), t);
        let details = fetch_match(&mb, RG, REL2, &CancellationToken::new())
            .await
            .unwrap();
        let input = staging::DecisionInput::Match {
            release_group_id: RG.into(),
            release_id: REL2.into(),
            edition_name: None,
        };
        let r = state
            .with_db(|db| db.write(|tx| staging::set_decision(tx, &s, 2, &input, Some(&details))))
            .unwrap();
        assert_eq!(r.readiness, staging::Readiness::Ready);
        assert!(
            matches!(r.decision, Decision::Match { ref edition_name, chosen_by: ChosenBy::User, .. } if edition_name.as_deref() == Some("Deluxe"))
        );
        let wrong = staging::DecisionInput::Match {
            release_group_id: "0a1b2c3d-0000-4000-8000-000000000002".into(),
            release_id: REL2.into(),
            edition_name: None,
        };
        assert!(
            state
                .with_db(|db| db.write(|tx| staging::set_decision(
                    tx,
                    &s,
                    2,
                    &wrong,
                    Some(&details)
                )))
                .is_err()
        );
    }
}
