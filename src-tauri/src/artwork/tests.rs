use std::sync::Mutex;

use rusqlite::params;

use super::http::ImageResponse;
use super::*;
use crate::metadata::BoxFuture;
use crate::paths::StorageProfile;

const RG: &str = "b1392450-e666-3926-a536-22c65f834433";
const REL: &str = "11111111-1111-4111-8111-111111111111";
const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, b'J', b'F', b'I', b'F'];
const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];

type Reply = Box<dyn Fn(&str) -> Result<ImageResponse, ImageError> + Send + Sync>;

struct Mock {
    reply: Reply,
    calls: Mutex<Vec<String>>,
}

impl Mock {
    fn new(
        reply: impl Fn(&str) -> Result<ImageResponse, ImageError> + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            reply: Box::new(reply),
            calls: Mutex::new(Vec::new()),
        })
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl ImageTransport for Mock {
    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<ImageResponse, ImageError>> {
        self.calls.lock().unwrap().push(url.to_owned());
        let r = (self.reply)(url);
        Box::pin(async move { r })
    }
}

fn ok(bytes: &[u8]) -> Result<ImageResponse, ImageError> {
    Ok(ImageResponse {
        status: 200,
        bytes: bytes.to_vec(),
    })
}

fn not_found() -> Result<ImageResponse, ImageError> {
    Ok(ImageResponse {
        status: 404,
        bytes: Vec::new(),
    })
}

/// One album with a MusicBrainz-linked edition. Returns (dir, state, edition id).
fn setup(consent: Option<bool>) -> (tempfile::TempDir, AppState, String) {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::open(StorageProfile::Development, dir.path().to_path_buf());
    state
        .with_db(|db| {
            db.write(|tx| {
                tx.execute("INSERT INTO album (id, title, musicbrainz_release_group_id) VALUES ('0190f5c3-0000-7000-8000-0000000000a1', 'OK Computer', ?1)", [RG])?;
                tx.execute("INSERT INTO edition (id, album_id, name, musicbrainz_release_id) VALUES ('0190f5c3-0000-7000-8000-0000000000e1', '0190f5c3-0000-7000-8000-0000000000a1', 'Standard', ?1)", [REL])?;
                if let Some(c) = consent {
                    settings::set_artwork_download(tx, c)?;
                }
                Ok(())
            })
        })
        .unwrap();
    (dir, state, "0190f5c3-0000-7000-8000-0000000000e1".into())
}

fn files(state: &AppState) -> usize {
    std::fs::read_dir(artwork_dir(state))
        .map(|d| d.count())
        .unwrap_or(0)
}

#[tokio::test(start_paused = true)]
async fn makes_no_requests_without_consent() {
    for consent in [None, Some(false)] {
        let (_d, state, ed) = setup(consent);
        let mock = Mock::new(|_| ok(JPEG));
        let svc = ArtworkService::new(mock.clone());
        let err = svc
            .fetch(&state, &ed, &CancellationToken::new())
            .await
            .unwrap_err();
        assert_eq!(err.code(), "validation");
        assert!(mock.calls().is_empty(), "{consent:?}");
    }
}

#[tokio::test(start_paused = true)]
async fn prefers_edition_art_and_caches_it() {
    let (_d, state, ed) = setup(Some(true));
    let mock = Mock::new(|_| ok(JPEG));
    let svc = ArtworkService::new(mock.clone());
    let got = svc
        .fetch(&state, &ed, &CancellationToken::new())
        .await
        .unwrap()
        .current
        .unwrap();
    assert_eq!(
        (got.owner_type.as_str(), got.canonical_fallback),
        ("edition", false)
    );
    assert_eq!(
        mock.calls(),
        vec![format!(
            "https://coverartarchive.org/release/{REL}/front-500"
        )]
    );
    let (bytes, mime) = serve(&state, "edition", &ed).unwrap();
    assert_eq!((bytes.as_slice(), mime.as_str()), (JPEG, "image/jpeg"));
    // Cached: no second request, and it works offline.
    svc.fetch(&state, &ed, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(mock.calls().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn falls_back_to_labelled_canonical_art_and_remembers_misses() {
    let (_d, state, ed) = setup(Some(true));
    let mock = Mock::new(|url| {
        if url.contains("/release-group/") {
            ok(PNG)
        } else {
            not_found()
        }
    });
    let svc = ArtworkService::new(mock.clone());
    let got = svc
        .fetch(&state, &ed, &CancellationToken::new())
        .await
        .unwrap()
        .current
        .unwrap();
    assert!(got.canonical_fallback);
    assert_eq!(got.owner_type, "album");
    assert_eq!(mock.calls().len(), 2);
    svc.fetch(&state, &ed, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(mock.calls().len(), 2, "the edition miss is remembered");

    let (_d2, state2, ed2) = setup(Some(true));
    let none = Mock::new(|_| not_found());
    let svc2 = ArtworkService::new(none.clone());
    assert_eq!(
        svc2.fetch(&state2, &ed2, &CancellationToken::new())
            .await
            .unwrap()
            .current,
        None
    );
    svc2.fetch(&state2, &ed2, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(none.calls().len(), 2, "both misses remembered");
}

#[tokio::test(start_paused = true)]
async fn rejects_non_images_and_reports_offline_without_marking() {
    let (_d, state, ed) = setup(Some(true));
    let html = Mock::new(|_| ok(b"<html>not an image</html>"));
    let svc = ArtworkService::new(html.clone());
    assert_eq!(
        svc.fetch(&state, &ed, &CancellationToken::new())
            .await
            .unwrap()
            .current,
        None
    );
    assert_eq!(files(&state), 0);

    let (_d2, state2, ed2) = setup(Some(true));
    let offline = Mock::new(|_| Err(ImageError::Unreachable("dns".into())));
    let err = ArtworkService::new(offline)
        .fetch(&state2, &ed2, &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(err.code(), "offline");
    let rows: i64 = state2
        .with_db(|db| {
            db.read(|c| Ok(c.query_row("SELECT COUNT(*) FROM artwork", [], |r| r.get(0))?))
        })
        .unwrap();
    assert_eq!(rows, 0, "an outage isn't recorded as 'no artwork'");
}

#[tokio::test(start_paused = true)]
async fn local_replacement_and_removal_win_over_downloads() {
    let (_d, state, ed) = setup(Some(true));
    let mock = Mock::new(|_| ok(JPEG));
    let svc = ArtworkService::new(mock.clone());
    svc.fetch(&state, &ed, &CancellationToken::new())
        .await
        .unwrap();

    let local = replace_local(&state, &ed, PNG).unwrap().current.unwrap();
    assert_eq!(local.source, "local");
    assert_eq!(
        files(&state),
        1,
        "the downloaded file was replaced, not leaked"
    );
    assert_eq!(
        replace_local(&state, &ed, b"GIF?").unwrap_err().code(),
        "validation"
    );

    // Cache clearing keeps the user's image; fetching never overrides it.
    let cleared = clear_cache(&state).unwrap();
    assert_eq!(cleared.files, 0);
    svc.fetch(&state, &ed, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(serve(&state, "edition", &ed).unwrap().1, "image/png");

    let removed = remove(&state, &ed).unwrap();
    assert!(removed.removed && removed.current.is_none());
    assert_eq!(files(&state), 0);
    let before = mock.calls().len();
    assert_eq!(
        svc.fetch(&state, &ed, &CancellationToken::new())
            .await
            .unwrap()
            .current,
        None
    );
    assert_eq!(
        mock.calls().len(),
        before,
        "removed artwork is never re-downloaded"
    );

    restore(&state, &ed).unwrap();
    assert!(
        svc.fetch(&state, &ed, &CancellationToken::new())
            .await
            .unwrap()
            .current
            .is_some()
    );
}

#[tokio::test(start_paused = true)]
async fn clearing_the_cache_frees_downloads_and_allows_refetch() {
    let (_d, state, ed) = setup(Some(true));
    let mock = Mock::new(|_| ok(JPEG));
    let svc = ArtworkService::new(mock.clone());
    svc.fetch(&state, &ed, &CancellationToken::new())
        .await
        .unwrap();
    std::fs::write(artwork_dir(&state).join("orphan.partial"), b"x").unwrap();
    let size = state.with_db(|db| db.read(cache_size)).unwrap();
    assert_eq!((size.files, size.bytes), (1, JPEG.len() as u64));
    let cleared = clear_cache(&state).unwrap();
    assert_eq!((cleared.files, cleared.bytes), (2, JPEG.len() as u64 + 1));
    assert_eq!(files(&state), 0);
    assert!(serve(&state, "edition", &ed).is_err());
    svc.fetch(&state, &ed, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(mock.calls().len(), 2);
}

#[test]
fn serves_only_recorded_files() {
    let (_d, state, _) = setup(Some(true));
    assert_eq!(
        serve(&state, "../etc", "passwd").unwrap_err().code(),
        "not_found"
    );
    assert_eq!(
        serve(&state, "edition", "../../x").unwrap_err().code(),
        "validation"
    );
    state
        .with_db(|db| {
            db.write(|tx| {
                Ok(tx.execute(
                    "DELETE FROM edition WHERE id = ?1",
                    params!["0190f5c3-0000-7000-8000-0000000000e1"],
                )?)
            })
        })
        .unwrap();
}

#[test]
fn sniffs_image_types() {
    assert_eq!(sniff_image(JPEG), Some("image/jpeg"));
    assert_eq!(sniff_image(PNG), Some("image/png"));
    assert_eq!(sniff_image(b"GIF89a...."), Some("image/gif"));
    assert_eq!(sniff_image(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
    assert_eq!(sniff_image(b"<svg xmlns="), None);
    assert!(validate_image(&vec![0xFF; MAX_IMAGE_BYTES + 1]).is_err());
}

/// Live check through the real HTTPS client and redirect policy.
/// Never runs by default: `cargo test live_cover_art -- --ignored`.
#[tokio::test]
#[ignore = "network: contacts coverartarchive.org and archive.org"]
async fn live_cover_art_archive_download_follows_safe_redirects() {
    let transport = http::ReqwestImageTransport::new().unwrap();
    let url = format!("https://coverartarchive.org/release-group/{RG}/front-500");
    let r = transport.get(&url).await.unwrap();
    assert_eq!(r.status, 200);
    let mime = validate_image(&r.bytes).unwrap();
    println!("{} bytes, {mime}", r.bytes.len());
    let missing = transport
        .get("https://coverartarchive.org/release/00000000-0000-4000-8000-000000000000/front-500")
        .await
        .unwrap();
    assert_eq!(missing.status, 404);
}
