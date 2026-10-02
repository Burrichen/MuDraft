//! Native end-to-end test support, compiled only with `--features e2e` (never in release
//! or ordinary dev builds). It replaces things a WebDriver session cannot drive:
//! - native file dialogs, via `MUDRAFT_E2E_<KIND>` paths;
//! - MusicBrainz, via recorded JSON in `MUDRAFT_E2E_FIXTURES` (no network at all).

use std::path::PathBuf;

use crate::metadata::BoxFuture;
use crate::metadata::http::{HttpResponse, Transport, TransportError};

/// The path a test chose for a native dialog (`csv`, `export`, `restore`), if any.
pub fn dialog_path(kind: &str) -> Option<PathBuf> {
    std::env::var_os(format!("MUDRAFT_E2E_{}", kind.to_uppercase())).map(PathBuf::from)
}

/// Serves `release-group-<id>.json`, `release-<id>.json`, `editions-<id>.json`, and
/// `search.json` from a fixture folder; anything else is a 404.
pub struct FixtureTransport {
    dir: PathBuf,
}

impl FixtureTransport {
    pub fn from_env() -> Option<Self> {
        std::env::var_os("MUDRAFT_E2E_FIXTURES").map(|d| Self {
            dir: PathBuf::from(d),
        })
    }

    fn file_for(url: &str) -> Option<String> {
        let url = reqwest::Url::parse(url).ok()?;
        let path = url.path().trim_start_matches("/ws/2/");
        let query = |key: &str| {
            url.query_pairs()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.into_owned())
        };
        let safe = |id: &str| {
            id.chars()
                .all(|c| c.is_ascii_hexdigit() || c == '-')
                .then(|| id.to_owned())
        };
        match path.split_once('/') {
            Some(("release-group", id)) => safe(id).map(|id| format!("release-group-{id}.json")),
            Some(("release", id)) => safe(id).map(|id| format!("release-{id}.json")),
            _ if path == "release" => query("release-group")
                .and_then(|id| safe(&id))
                .map(|id| format!("editions-{id}.json")),
            _ if path == "release-group" && query("query").is_some() => Some("search.json".into()),
            _ => None,
        }
    }
}

impl Transport for FixtureTransport {
    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<HttpResponse, TransportError>> {
        let body = Self::file_for(url).and_then(|f| std::fs::read_to_string(self.dir.join(f)).ok());
        Box::pin(async move {
            Ok(match body {
                Some(body) => HttpResponse {
                    status: 200,
                    body,
                    retry_after: None,
                },
                None => HttpResponse {
                    status: 404,
                    body: String::new(),
                    retry_after: None,
                },
            })
        })
    }
}
