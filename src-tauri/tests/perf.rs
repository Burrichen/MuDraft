//! Large-library measurements (not a pass/fail benchmark). Deterministic fixture:
//! 1,200 artists, 5,000 albums × 12 tracks, 2,000 on the Listen List, 3,000 in the
//! Collection with 8,000 listens (full listens record their tracks), 18,000 track ratings.
//!
//! Run: `cargo test --release --test perf -- --ignored --nocapture`

mod common;

use std::time::{Duration, Instant};

use mudraft_lib::db::Database;
use mudraft_lib::domain::picker::{Method, RandomSource};
use mudraft_lib::error::AppResult;
use mudraft_lib::library::listing::{self, ListQuery, SortOrder, Source};
use mudraft_lib::library::{discography, next_up, stats};

const ARTISTS: usize = 1_200;
const ALBUMS: usize = 5_000;
const TRACKS: usize = 12;
const GENRES: [&str; 6] = ["Rock", "Pop", "Jazz", "Electronic", "Hip Hop", "Folk"];

fn id(kind: u32, n: usize) -> String {
    format!("0190f5c3-{kind:04x}-7000-8000-{n:012x}")
}

fn build(db: &mut Database) {
    db.write(|tx| {
        for (i, g) in GENRES.iter().enumerate() {
            tx.execute("INSERT INTO genre (id, name) VALUES (?1, ?2)", (id(1, i), g))?;
        }
        tx.execute("INSERT INTO tag (id, name) VALUES (?1, 'Road trip')", [id(2, 0)])?;
        for a in 0..ARTISTS {
            tx.execute(
                "INSERT INTO artist (id, name) VALUES (?1, ?2)",
                (id(3, a), format!("Artist {a:04}")),
            )?;
        }
        for n in 0..ALBUMS {
            let (al, ed) = (id(4, n), id(5, n));
            let year = 1950 + (n * 7) % 75;
            tx.execute(
                "INSERT INTO album (id, title, original_date, original_date_precision) VALUES (?1, ?2, ?3, 'year')",
                (&al, format!("Album {n:05} {}", ["Blue", "Night", "Glass", "Echo"][n % 4]), format!("{year:04}")),
            )?;
            tx.execute(
                "INSERT INTO album_artist_credit (album_id, position, artist_id) VALUES (?1, 0, ?2)",
                (&al, id(3, n % ARTISTS)),
            )?;
            if n % 9 == 0 {
                tx.execute(
                    "INSERT INTO album_artist_credit (album_id, position, artist_id, join_phrase) VALUES (?1, 1, ?2, '')",
                    (&al, id(3, (n + 1) % ARTISTS)),
                )?;
            }
            tx.execute(
                "INSERT INTO album_genre (album_id, genre_id, position) VALUES (?1, ?2, 0)",
                (&al, id(1, n % GENRES.len())),
            )?;
            if n % 3 == 0 {
                tx.execute("INSERT INTO album_tag (album_id, tag_id) VALUES (?1, ?2)", (&al, id(2, 0)))?;
            }
            if n % 5 == 0 {
                tx.execute(
                    "INSERT INTO album_tag (album_id, tag_id) VALUES (?1, '00000000-0000-7000-8000-000000000001')",
                    [&al],
                )?;
            }
            tx.execute(
                "INSERT INTO edition (id, album_id, name) VALUES (?1, ?2, 'Standard')",
                (&ed, &al),
            )?;
            for t in 0..TRACKS {
                tx.execute(
                    "INSERT INTO track (id, edition_id, position, title, length_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
                    (id(6, n * TRACKS + t), &ed, (t + 1) as i64, format!("Track {t}"), (180_000 + t * 1_000) as i64),
                )?;
            }
            if n < 2_000 {
                tx.execute(
                    "INSERT INTO listen_list_entry (album_id, edition_id) VALUES (?1, ?2)",
                    (&al, &ed),
                )?;
            } else {
                tx.execute(
                    "INSERT INTO collection_entry (edition_id, album_id, source) VALUES (?1, ?2, 'listen')",
                    (&ed, &al),
                )?;
                if n % 2 == 0 {
                    for t in 0..TRACKS {
                        tx.execute(
                            "INSERT INTO track_rating (track_id, rating) VALUES (?1, ?2)",
                            (id(6, n * TRACKS + t), ((n + t) % 11) as i64),
                        )?;
                    }
                }
            }
        }
        // 8,000 listens over the Collection, each full listen with its tracks.
        for l in 0..8_000usize {
            let n = 2_000 + l % 3_000;
            let le = id(7, l);
            tx.execute(
                "INSERT INTO listen_event (id, edition_id, album_id, listened_on, date_status, is_full, kind)
                 VALUES (?1, ?2, ?3, ?4, 'known', 1, ?5)",
                (
                    &le,
                    id(5, n),
                    id(4, n),
                    format!("20{:02}-{:02}-{:02}", 15 + l % 11, 1 + l % 12, 1 + l % 28),
                    if l < 3_000 { "first" } else { "relisten" },
                ),
            )?;
            for t in 0..TRACKS {
                tx.execute(
                    "INSERT INTO listen_event_track (listen_event_id, edition_id, track_id) VALUES (?1, ?2, ?3)",
                    (&le, id(5, n), id(6, n * TRACKS + t)),
                )?;
            }
        }
        Ok(())
    })
    .unwrap();
}

struct First;
impl RandomSource for First {
    fn below(&mut self, _len: usize) -> AppResult<usize> {
        Ok(0)
    }
}

fn time<T>(label: &str, runs: u32, mut f: impl FnMut() -> T) -> T {
    let mut best = Duration::MAX;
    let mut out = None;
    for _ in 0..runs {
        let start = Instant::now();
        out = Some(f());
        best = best.min(start.elapsed());
    }
    println!(
        "{label:<48} {:>8.1} ms (best of {runs})",
        best.as_secs_f64() * 1000.0
    );
    out.unwrap()
}

fn rss_mb() -> f64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output();
    out.ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<f64>().ok())
        .map_or(f64::NAN, |kb| kb / 1024.0)
}

#[test]
#[ignore = "measurement; run explicitly in release mode"]
fn large_library_measurements() {
    let dir = tempfile::tempdir().unwrap();
    // Optionally keep the generated library for UI measurements (MUDRAFT_PERF_KEEP=<dir>).
    let keep = std::env::var_os("MUDRAFT_PERF_KEEP").map(std::path::PathBuf::from);
    let base = keep.clone().unwrap_or_else(|| dir.path().to_path_buf());
    std::fs::create_dir_all(&base).unwrap();
    let path = base.join("mudraft.sqlite3");
    {
        let mut db = Database::open(&path).unwrap();
        time("build fixture (one transaction)", 1, || build(&mut db));
    }
    let size = std::fs::metadata(&path).unwrap().len() as f64 / 1_048_576.0;
    println!("{:<48} {size:>8.1} MB", "database file");
    let db = time("open existing database (startup)", 5, || {
        Database::open(&path).unwrap()
    });

    let q = |sort, search: Option<&str>, genres: Vec<String>| ListQuery {
        search: search.map(Into::into),
        sort,
        genres,
        ..ListQuery::default()
    };
    let r = time("Listen List, default sort (2,000)", 5, || {
        db.read(|c| {
            listing::query(
                c,
                Source::ListenList,
                &q(SortOrder::AddedNewest, None, vec![]),
            )
        })
        .unwrap()
    });
    assert_eq!(r.items.len(), 2_000);
    time("Collection, rating sort (3,000)", 5, || {
        db.read(|c| {
            listing::query(
                c,
                Source::Collection,
                &q(SortOrder::RatingHighest, None, vec![]),
            )
        })
        .unwrap()
    });
    time("Collection, search + genre filter", 5, || {
        db.read(|c| {
            listing::query(
                c,
                Source::Collection,
                &q(SortOrder::Title, Some("glass"), vec!["Jazz".into()]),
            )
        })
        .unwrap()
    });
    time("Stats overview", 5, || {
        db.read(|c| stats::overview(c, "2026-10-02T00:00:00.000Z"))
            .unwrap()
    });
    time("Next Up state", 5, || {
        db.read(|c| next_up::state(c, 2026)).unwrap()
    });
    let mut writable = Database::open(&path).unwrap();
    time("Next Up roll (Completely Random)", 5, || {
        writable
            .write(|tx| next_up::roll(tx, None, Method::CompletelyRandom, 2026, &mut First))
            .unwrap()
    });
    time("Artist page (artist with ~5 albums)", 5, || {
        db.read(|c| discography::catalogue(c, &id(3, 7))).unwrap()
    });
    let out = dir.path().join("export.mudraft");
    let exported = time("profile export (no artwork)", 1, || {
        db.read(|c| mudraft_lib::profile::export::export(c, dir.path(), &out, true))
            .unwrap()
    });
    println!(
        "{:<48} {:>8.1} MB",
        "export file",
        exported.bytes as f64 / 1_048_576.0
    );
    println!(
        "{:<48} {:>8.1} MB",
        "process RSS after all of the above",
        rss_mb()
    );
}
