//! Listen List reads and global tag management.

mod common;

use common::{album, artist, count, edition, open_temp};
use mudraft_lib::csv_import::commit::{self, TagDecisions};
use mudraft_lib::csv_import::{mapping, parse, staging};
use mudraft_lib::db::Database;
use mudraft_lib::domain::rating::HalfStars;
use mudraft_lib::library::catalogue::LISTEN_ASAP_TAG_ID;
use mudraft_lib::library::listing::{self, ListQuery, SortOrder, Source};
use mudraft_lib::library::personal::{self, CollectionSource, NewListen};
use mudraft_lib::library::tags;

fn add_album(
    db: &mut Database,
    title: &str,
    artist_id: &str,
    year: &str,
    genres: &[&str],
) -> (String, String) {
    let id = db
        .write(|tx| {
            mudraft_lib::library::catalogue::create_album(
                tx,
                None,
                &mudraft_lib::library::catalogue::NewAlbum {
                    title: title.into(),
                    original_date: Some(year.into()),
                    musicbrainz_release_group_id: None,
                    credits: vec![common::credit(artist_id)],
                    genres: genres.iter().map(|g| (*g).into()).collect(),
                },
            )
        })
        .unwrap();
    let (ed, _) = edition(db, &id, "Standard", 1);
    db.write(|tx| personal::add_to_listen_list(tx, &ed))
        .unwrap();
    (id, ed)
}

fn list(db: &Database, q: ListQuery) -> listing::ListResult {
    db.read(|c| listing::query(c, Source::ListenList, &q))
        .unwrap()
}

fn titles(r: &listing::ListResult) -> Vec<String> {
    r.items.iter().map(|i| i.title.clone()).collect()
}

#[test]
fn exactly_one_protected_builtin_tag_survives_restart_and_imports() {
    let (dir, db) = open_temp();
    assert_eq!(db.read(tags::builtin_count).unwrap(), 1);
    drop(db);
    let mut db = Database::open(&dir.path().join("mudraft.sqlite3")).unwrap();
    assert_eq!(db.read(tags::builtin_count).unwrap(), 1);

    // A CSV spelling the tag differently reuses the built-in instead of duplicating it.
    let parsed = parse::parse("Album,Artist,Tags\nA,B,LISTEN  asap\n".as_bytes()).unwrap();
    db.write(|tx| {
        let s = staging::create_session(tx, "a.csv", &parsed)?;
        staging::apply_mapping(tx, &s, &mapping::suggest(&parsed.headers))?;
        let unknown = staging::summary(tx, &s)?.unknown_tags;
        assert!(unknown.is_empty(), "{unknown:?}");
        commit::commit(tx, &s, &TagDecisions::default())
    })
    .unwrap();
    assert_eq!(count(&db, "tag"), 1);
    assert_eq!(count(&db, "album_tag"), 1);

    let asap = db.read(|c| tags::get(c, LISTEN_ASAP_TAG_ID)).unwrap();
    assert!(asap.builtin);
    assert_eq!(asap.name, "Listen ASAP");
    assert_eq!(
        db.write(|tx| tags::update(tx, LISTEN_ASAP_TAG_ID, Some("Soon"), None))
            .unwrap_err()
            .code(),
        "validation"
    );
    assert_eq!(
        db.write(|tx| tags::delete(tx, LISTEN_ASAP_TAG_ID))
            .unwrap_err()
            .code(),
        "validation"
    );
    // Even raw SQL can't break its identity.
    assert!(
        db.write(|tx| Ok(tx.execute("DELETE FROM tag WHERE builtin_key IS NOT NULL", [])?))
            .is_err()
    );
    assert!(
        db.write(|tx| Ok(tx.execute(
            "UPDATE tag SET name = 'x' WHERE builtin_key IS NOT NULL",
            []
        )?))
        .is_err()
    );
    // Colour stays editable.
    let recoloured = db
        .write(|tx| tags::update(tx, LISTEN_ASAP_TAG_ID, Some("Listen ASAP"), Some("#10B981")))
        .unwrap();
    assert_eq!(recoloured.color, "#10b981");
}

#[test]
fn tag_names_are_trimmed_and_unique_ignoring_case_and_unicode_form() {
    let (_d, mut db) = open_temp();
    let t = db
        .write(|tx| tags::create(tx, "  Road   trip ", Some("#abc")))
        .unwrap();
    assert_eq!(
        (t.name.as_str(), t.color.as_str(), t.album_count),
        ("Road trip", "#aabbcc", 0)
    );
    assert_eq!(
        db.write(|tx| tags::create(tx, "ROAD TRIP", None))
            .unwrap_err()
            .code(),
        "conflict"
    );
    db.write(|tx| tags::create(tx, "Café", None)).unwrap();
    assert_eq!(
        db.write(|tx| tags::create(tx, "CAFE\u{301}", None))
            .unwrap_err()
            .code(),
        "conflict"
    );
    assert_eq!(
        db.write(|tx| tags::create(tx, "listen asap", None))
            .unwrap_err()
            .code(),
        "conflict"
    );
    assert_eq!(
        db.write(|tx| tags::create(tx, "a;b", None))
            .unwrap_err()
            .code(),
        "validation"
    );

    // Renaming to itself in another case is allowed; onto another tag is not.
    let renamed = db
        .write(|tx| tags::update(tx, &t.id, Some("road Trip"), None))
        .unwrap();
    assert_eq!(renamed.name, "road Trip");
    assert_eq!(
        db.write(|tx| tags::update(tx, &t.id, Some("café"), None))
            .unwrap_err()
            .code(),
        "conflict"
    );
    assert_eq!(
        db.write(|tx| tags::update(tx, &t.id, None, Some("blue")))
            .unwrap_err()
            .code(),
        "validation"
    );
}

#[test]
fn deleting_a_tag_reports_impact_and_keeps_albums_and_unused_tags_stay_listed() {
    let (_d, mut db) = open_temp();
    let a = artist(&mut db, "Björk");
    let (al1, _) = add_album(&mut db, "Homogenic", &a, "1997", &[]);
    let (al2, _) = add_album(&mut db, "Vespertine", &a, "2001", &[]);
    let t = db.write(|tx| tags::create(tx, "Icelandic", None)).unwrap();
    let unused = db.write(|tx| tags::create(tx, "Someday", None)).unwrap();
    let r = db
        .write(|tx| {
            tags::set_album_tags(
                tx,
                &[al1.clone(), al2.clone()],
                std::slice::from_ref(&t.id),
                &[],
            )
        })
        .unwrap();
    assert_eq!((r.added, r.removed), (2, 0));
    let again = db
        .write(|tx| {
            tags::set_album_tags(
                tx,
                std::slice::from_ref(&al1),
                std::slice::from_ref(&t.id),
                &[],
            )
        })
        .unwrap();
    assert_eq!(again.added, 0, "repeat is a no-op");

    let listed = db.read(tags::list).unwrap();
    assert_eq!(listed[0].id, LISTEN_ASAP_TAG_ID, "built-in first");
    assert_eq!(
        listed
            .iter()
            .find(|x| x.id == unused.id)
            .unwrap()
            .album_count,
        0
    );
    assert_eq!(listed.iter().find(|x| x.id == t.id).unwrap().album_count, 2);

    assert_eq!(db.write(|tx| tags::delete(tx, &t.id)).unwrap(), 2);
    assert_eq!(count(&db, "album"), 2);
    assert_eq!(count(&db, "album_tag"), 0);
    assert!(
        db.read(tags::list)
            .unwrap()
            .iter()
            .any(|x| x.id == unused.id)
    );
}

#[test]
fn bulk_tagging_is_atomic() {
    let (_d, mut db) = open_temp();
    let a = artist(&mut db, "A");
    let (al1, _) = add_album(&mut db, "One", &a, "2000", &[]);
    let t = db.write(|tx| tags::create(tx, "x", None)).unwrap();
    let missing = mudraft_lib::domain::ids::new_id();
    let err = db
        .write(|tx| {
            tags::set_album_tags(
                tx,
                &[al1.clone(), missing],
                std::slice::from_ref(&t.id),
                &[],
            )
        })
        .unwrap_err();
    assert_eq!(err.code(), "not_found");
    assert_eq!(count(&db, "album_tag"), 0);
    let both = db.write(|tx| {
        tags::set_album_tags(
            tx,
            &[al1],
            std::slice::from_ref(&t.id),
            std::slice::from_ref(&t.id),
        )
    });
    assert_eq!(both.unwrap_err().code(), "validation");
}

#[test]
fn listen_list_shows_album_level_data_and_supports_search_filters_and_sorting() {
    let (_d, mut db) = open_temp();
    let bjork = artist(&mut db, "Björk");
    let radiohead = artist(&mut db, "Radiohead");
    let (homogenic, _) = add_album(&mut db, "Homogenic", &bjork, "1997", &["Electronic", "Pop"]);
    let (_ok, _) = add_album(
        &mut db,
        "OK Computer",
        &radiohead,
        "1997",
        &["Rock", "Electronic"],
    );
    let (kid_a, _) = add_album(&mut db, "Kid A", &radiohead, "2000", &["Electronic"]);
    let (unknown, _) = add_album(&mut db, "Demo", &radiohead, "", &[]);
    db.write(|tx| {
        tags::set_album_tags(
            tx,
            &[homogenic.clone(), kid_a.clone()],
            &[LISTEN_ASAP_TAG_ID.into()],
            &[],
        )
    })
    .unwrap();

    let all = list(&db, ListQuery::default());
    assert_eq!(all.total, 4);
    let h = all.items.iter().find(|i| i.album_id == homogenic).unwrap();
    assert_eq!(h.credit, "Björk");
    assert_eq!(h.artists[0].id, bjork);
    assert_eq!(h.original_year, Some(1997));
    assert_eq!(h.genres, vec!["Electronic", "Pop"]);
    assert_eq!(h.tags[0].name, "Listen ASAP");
    assert_eq!((h.edition_name.as_str(), h.edition_count), ("Standard", 1));
    assert!(h.added_at.ends_with('Z'));
    assert_eq!(all.genres[0].name, "Electronic");
    assert_eq!(all.genres[0].count, 3);
    assert_eq!(all.tags[0].count, 2);

    let q = |search: &str| ListQuery {
        search: Some(search.into()),
        ..Default::default()
    };
    assert_eq!(
        titles(&list(&db, q("bjo\u{308}rk"))),
        vec!["Homogenic"],
        "artist search, Unicode-insensitive"
    );
    assert_eq!(list(&db, q("  kid  ")).items.len(), 1);

    let genre = ListQuery {
        genres: vec!["pop".into(), "Rock".into()],
        ..Default::default()
    };
    let mut g = titles(&list(&db, genre.clone()));
    g.sort();
    assert_eq!(g, vec!["Homogenic", "OK Computer"], "OR within genres");
    let genre_and_tag = ListQuery {
        tag_ids: vec![LISTEN_ASAP_TAG_ID.into()],
        ..genre
    };
    assert_eq!(
        titles(&list(&db, genre_and_tag)),
        vec!["Homogenic"],
        "AND across genres and tags"
    );

    let none = list(&db, q("nothing like this"));
    assert!(none.items.is_empty());
    assert_eq!(
        none.total, 4,
        "zero matches is distinguishable from an empty list"
    );
    assert_eq!(none.genres.len(), 3, "facets stay available");

    let sort = |s| {
        titles(&list(
            &db,
            ListQuery {
                sort: s,
                ..Default::default()
            },
        ))
        .iter()
        .map(|t| t.to_string())
        .collect::<Vec<_>>()
    };
    assert_eq!(
        sort(SortOrder::Title),
        vec!["Demo", "Homogenic", "Kid A", "OK Computer"]
    );
    assert_eq!(sort(SortOrder::YearNewest)[0], "Kid A");
    assert_eq!(
        sort(SortOrder::YearOldest).last().unwrap(),
        "Demo",
        "unknown years last"
    );
    assert_eq!(sort(SortOrder::Artist)[0], "Homogenic");
    let _ = unknown;
}

#[test]
fn removal_keeps_history_and_tags_follow_the_album_into_collection() {
    let (_d, mut db) = open_temp();
    let a = artist(&mut db, "Radiohead");
    let al = album(&mut db, "OK Computer", &[&a]);
    let (standard, _) = edition(&mut db, &al, "Standard", 1);
    let (deluxe, _) = edition(&mut db, &al, "Deluxe", 1);
    db.write(|tx| {
        personal::add_to_listen_list(tx, &deluxe)?;
        personal::add_to_collection(tx, &standard, CollectionSource::Manual)?;
        personal::set_album_rating(tx, &deluxe, Some(HalfStars::new(9)?))?;
        personal::set_album_review(tx, &deluxe, Some("Great."))?;
        Ok(())
    })
    .unwrap();
    let listed = list(&db, ListQuery::default());
    assert_eq!(
        (
            listed.items[0].edition_name.as_str(),
            listed.items[0].edition_count
        ),
        ("Deluxe", 2)
    );

    let t = db.write(|tx| tags::create(tx, "Headphones", None)).unwrap();
    db.write(|tx| {
        tags::set_album_tags(
            tx,
            std::slice::from_ref(&al),
            std::slice::from_ref(&t.id),
            &[],
        )
    })
    .unwrap();
    let collection = db
        .read(|c| listing::query(c, Source::Collection, &ListQuery::default()))
        .unwrap();
    assert_eq!(collection.items[0].edition_name, "Standard");
    assert_eq!(
        collection.items[0].tags[0].name, "Headphones",
        "tags belong to the album, not the list"
    );

    db.write(|tx| {
        personal::record_listen(
            tx,
            None,
            &NewListen {
                edition_id: standard.clone(),
                listened_on: None,
                is_full: false,
                track_ids: vec![],
            },
        )
    })
    .unwrap();
    assert_eq!(
        db.write(|tx| listing::remove_from_listen_list(tx, std::slice::from_ref(&al)))
            .unwrap(),
        1
    );
    assert_eq!(
        db.write(|tx| listing::remove_from_listen_list(tx, std::slice::from_ref(&al)))
            .unwrap(),
        0
    );
    assert!(list(&db, ListQuery::default()).items.is_empty());
    let review = db.read(|c| personal::album_review(c, &deluxe)).unwrap();
    assert_eq!(review.review.as_deref(), Some("Great."));
    assert_eq!(count(&db, "listen_event"), 1);
    assert_eq!(count(&db, "album_tag"), 1);
    assert_eq!(count(&db, "collection_entry"), 1);
}
