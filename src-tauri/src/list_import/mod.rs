//! Import a ranked album list from a web page. Only addresses on a short allow-list of
//! sites are fetched (a user-supplied address must never make the app reach arbitrary
//! hosts), and nothing is saved here: the renderer shows the result for review first.
//! Networking is native (Rust), like every other provider call.

pub mod pitchfork;

use std::sync::Arc;
use std::time::Duration;

use reqwest::Url;
use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::library::lists::MAX_ENTRIES;
use crate::metadata::BoxFuture;

const ALLOWED_HOSTS: &[&str] = &["pitchfork.com", "www.pitchfork.com"];
const MAX_PAGE_BYTES: usize = 16 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_NAME_CHARS: usize = 300;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedEntry {
    pub artist_name: String,
    pub title: String,
    pub year: Option<i64>,
    pub position: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListImport {
    pub name: String,
    pub source_url: String,
    pub entries: Vec<ImportedEntry>,
}

fn allowed_host(url: &Url) -> bool {
    url.host_str()
        .is_some_and(|h| ALLOWED_HOSTS.contains(&h.to_ascii_lowercase().as_str()))
}

/// Accept only an https address on a supported site, and drop its query and fragment.
pub fn supported_url(raw: &str) -> AppResult<Url> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(AppError::validation("address", "enter a web address"));
    }
    let mut url = Url::parse(raw).map_err(|_| {
        AppError::validation("address", "that isn’t a web address (start with https://)")
    })?;
    if url.scheme() != "https"
        || !allowed_host(&url)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.path().len() <= 1
    {
        return Err(AppError::validation(
            "address",
            "MuDraft can import lists from Pitchfork story pages (pitchfork.com). That address isn’t supported.",
        ));
    }
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

/// Downloads one page as text.
pub trait PageFetcher: Send + Sync {
    fn get<'a>(&'a self, url: &'a Url) -> BoxFuture<'a, AppResult<String>>;
}

pub struct ReqwestPages {
    client: reqwest::Client,
}

impl ReqwestPages {
    pub fn new() -> Result<Self, String> {
        // Redirects stay on the allow-list, over https.
        let redirects = reqwest::redirect::Policy::custom(|attempt| {
            let ok = attempt.previous().len() < 5
                && attempt.url().scheme() == "https"
                && allowed_host(attempt.url());
            if ok { attempt.follow() } else { attempt.stop() }
        });
        let client = crate::net::client_builder()
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(Duration::from_secs(10))
            .https_only(true)
            .redirect(redirects)
            .build()
            .map_err(|e| format!("cannot build HTTP client: {e}"))?;
        Ok(Self { client })
    }
}

fn site_error(err: &reqwest::Error) -> AppError {
    if err.is_timeout() {
        AppError::Timeout("The list’s website".into())
    } else {
        AppError::Offline("the list’s website".into())
    }
}

impl PageFetcher for ReqwestPages {
    fn get<'a>(&'a self, url: &'a Url) -> BoxFuture<'a, AppResult<String>> {
        Box::pin(async move {
            let mut response = self
                .client
                .get(url.clone())
                .header(reqwest::header::ACCEPT, "text/html")
                .send()
                .await
                .map_err(|e| site_error(&e))?;
            let status = response.status();
            if !status.is_success() {
                return Err(AppError::Provider(format!(
                    "the website answered {status} for that page"
                )));
            }
            let too_big = || AppError::Provider("that page is too large to read".into());
            if response
                .content_length()
                .is_some_and(|n| n > MAX_PAGE_BYTES as u64)
            {
                return Err(too_big());
            }
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|e| site_error(&e))? {
                if body.len() + chunk.len() > MAX_PAGE_BYTES {
                    return Err(too_big());
                }
                body.extend_from_slice(&chunk);
            }
            Ok(String::from_utf8_lossy(&body).into_owned())
        })
    }
}

pub struct ListImporter {
    pages: Arc<dyn PageFetcher>,
}

impl ListImporter {
    pub fn new(pages: Arc<dyn PageFetcher>) -> Self {
        Self { pages }
    }

    pub fn reqwest() -> Result<Self, String> {
        Ok(Self::new(Arc::new(ReqwestPages::new()?)))
    }

    /// Fetch and read the list at `raw_url`. Writes nothing.
    pub async fn preview(&self, raw_url: &str) -> AppResult<ListImport> {
        let url = supported_url(raw_url)?;
        let html = self.pages.get(&url).await?;
        let mut list = pitchfork::parse(&html, url.as_str())?;
        list.name = list.name.chars().take(MAX_NAME_CHARS).collect();
        list.entries.truncate(MAX_ENTRIES);
        Ok(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Canned(AppResult<String>);

    impl PageFetcher for Canned {
        fn get<'a>(&'a self, _: &'a Url) -> BoxFuture<'a, AppResult<String>> {
            Box::pin(async move { self.0.clone() })
        }
    }

    const PAGE: &str = r#"<title>Best Albums | Pitchfork</title>
<div role="heading" aria-level="3">2.</div><h2>B: <em>Two</em></h2><p>L ✦ 2001</p>
<div role="heading" aria-level="3">1.</div><h2>A: <em>One</em></h2><p>L ✦ 2000</p>"#;

    #[test]
    fn only_https_pitchfork_pages_are_fetched() {
        let ok = supported_url(" https://www.Pitchfork.com/story/best/?utm=1#top ").unwrap();
        assert_eq!(ok.as_str(), "https://www.pitchfork.com/story/best/");
        for bad in [
            "",
            "pitchfork.com/story/x",
            "http://pitchfork.com/story/x",
            "https://pitchfork.com",
            "https://pitchfork.com.evil.example/story/x",
            "https://evil.example/pitchfork.com/story/x",
            "https://user:pw@pitchfork.com/story/x",
            "https://pitchfork.com:8443/story/x",
            "https://127.0.0.1/story/x",
            "file:///etc/passwd",
        ] {
            assert_eq!(
                supported_url(bad).unwrap_err().code(),
                "validation",
                "{bad}"
            );
        }
    }

    #[tokio::test]
    async fn previews_a_page_without_saving_anything() {
        let importer = ListImporter::new(Arc::new(Canned(Ok(PAGE.into()))));
        let list = importer
            .preview("https://pitchfork.com/story/best/")
            .await
            .unwrap();
        assert_eq!(list.name, "Best Albums");
        assert_eq!(list.source_url, "https://pitchfork.com/story/best/");
        assert_eq!(list.entries.len(), 2);
        assert_eq!(list.entries[0].title, "One");
        assert_eq!(list.entries[0].year, Some(2000));
    }

    #[tokio::test]
    async fn fetch_errors_and_bad_addresses_are_reported() {
        let offline =
            ListImporter::new(Arc::new(Canned(Err(AppError::Offline("the site".into())))));
        let code = |r: AppResult<ListImport>| r.unwrap_err().code();
        assert_eq!(
            code(offline.preview("https://pitchfork.com/story/best/").await),
            "offline"
        );
        assert_eq!(
            code(offline.preview("https://example.com/list").await),
            "validation"
        );
    }
}
