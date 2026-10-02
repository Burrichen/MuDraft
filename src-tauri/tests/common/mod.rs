#![allow(dead_code)] // Each test crate uses a different subset of these helpers.

use mudraft_lib::db::Database;
use mudraft_lib::library::catalogue::{
    self, CreditInput, NewAlbum, NewArtist, NewEdition, NewTrack,
};

pub fn open_temp() -> (tempfile::TempDir, Database) {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("mudraft.sqlite3")).unwrap();
    (dir, db)
}

pub fn artist(db: &mut Database, name: &str) -> String {
    db.write(|tx| {
        catalogue::create_artist(
            tx,
            None,
            &NewArtist {
                name: name.into(),
                sort_name: None,
                musicbrainz_id: None,
            },
        )
    })
    .unwrap()
}

pub fn credit(artist_id: &str) -> CreditInput {
    CreditInput {
        artist_id: artist_id.into(),
        credited_name: None,
        join_phrase: None,
    }
}

pub fn album(db: &mut Database, title: &str, artist_ids: &[&str]) -> String {
    db.write(|tx| {
        catalogue::create_album(
            tx,
            None,
            &NewAlbum {
                title: title.into(),
                original_date: Some("1997-05-21".into()),
                musicbrainz_release_group_id: None,
                credits: artist_ids.iter().map(|a| credit(a)).collect(),
                genres: vec!["Rock".into()],
            },
        )
    })
    .unwrap()
}

pub fn track(position: i64, title: &str) -> NewTrack {
    NewTrack {
        disc_number: None,
        position,
        title: title.into(),
        length_ms: Some(200_000),
        recording_id: None,
        musicbrainz_track_id: None,
        credits: vec![],
    }
}

pub fn new_edition(album_id: &str, name: &str, tracks: Vec<NewTrack>) -> NewEdition {
    NewEdition {
        album_id: album_id.into(),
        name: name.into(),
        release_date: Some("2017".into()),
        musicbrainz_release_id: None,
        tracks,
    }
}

/// Returns (edition_id, track_ids).
pub fn edition(
    db: &mut Database,
    album_id: &str,
    name: &str,
    track_count: i64,
) -> (String, Vec<String>) {
    let tracks = (1..=track_count)
        .map(|n| track(n, &format!("Track {n}")))
        .collect();
    let created = db
        .write(|tx| catalogue::create_edition(tx, None, &new_edition(album_id, name, tracks)))
        .unwrap();
    (created.edition_id, created.track_ids)
}

pub fn count(db: &Database, table: &str) -> i64 {
    db.read(|c| Ok(c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?))
        .unwrap()
}
