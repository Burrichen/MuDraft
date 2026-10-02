//! Image downloads from the Cover Art Archive. HTTPS only, redirects limited to the
//! Archive's own hosts, and responses capped while streaming.

use std::time::Duration;

use reqwest::Url;

use crate::metadata::BoxFuture;
use crate::metadata::http::USER_AGENT;

pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageResponse {
    pub status: u16,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageError {
    Timeout,
    Unreachable(String),
    TooLarge,
    /// A redirect left HTTPS or the Archive's hosts.
    UnsafeRedirect(String),
}

pub trait ImageTransport: Send + Sync {
    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<ImageResponse, ImageError>>;
}

/// Hosts a Cover Art Archive request may be redirected to.
pub fn allowed_host(host: &str) -> bool {
    host == "coverartarchive.org" || host == "archive.org" || host.ends_with(".archive.org")
}

pub fn redirect_allowed(url: &Url, previous_hops: usize) -> Result<(), String> {
    if previous_hops >= MAX_REDIRECTS {
        return Err("too many redirects".into());
    }
    if url.scheme() != "https" {
        return Err(format!("redirect to non-HTTPS URL {url}"));
    }
    match url.host_str() {
        Some(h) if allowed_host(h) => Ok(()),
        other => Err(format!(
            "redirect to unexpected host {}",
            other.unwrap_or("?")
        )),
    }
}

pub struct ReqwestImageTransport {
    client: reqwest::Client,
}

impl ReqwestImageTransport {
    pub fn new() -> Result<Self, String> {
        let policy = reqwest::redirect::Policy::custom(|attempt| {
            match redirect_allowed(attempt.url(), attempt.previous().len()) {
                Ok(()) => attempt.follow(),
                Err(e) => attempt.error(e),
            }
        });
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .redirect(policy)
            .https_only(true)
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| format!("cannot build image client: {e}"))?;
        Ok(Self { client })
    }
}

fn classify(err: &reqwest::Error) -> ImageError {
    if err.is_timeout() {
        ImageError::Timeout
    } else if err.is_redirect() {
        ImageError::UnsafeRedirect(err.to_string())
    } else {
        ImageError::Unreachable(err.to_string())
    }
}

impl ImageTransport for ReqwestImageTransport {
    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<ImageResponse, ImageError>> {
        Box::pin(async move {
            let mut response = self
                .client
                .get(url)
                .send()
                .await
                .map_err(|e| classify(&e))?;
            let status = response.status().as_u16();
            if status != 200 {
                return Ok(ImageResponse {
                    status,
                    bytes: Vec::new(),
                });
            }
            if response
                .content_length()
                .is_some_and(|n| n as usize > MAX_IMAGE_BYTES)
            {
                return Err(ImageError::TooLarge);
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|e| classify(&e))? {
                if bytes.len() + chunk.len() > MAX_IMAGE_BYTES {
                    return Err(ImageError::TooLarge);
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(ImageResponse { status, bytes })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirects_stay_on_https_archive_hosts() {
        let ok = |s: &str| redirect_allowed(&Url::parse(s).unwrap(), 1);
        assert!(ok("https://archive.org/download/x.jpg").is_ok());
        assert!(ok("https://dn720706.ca.archive.org/0/items/x.jpg").is_ok());
        assert!(ok("https://coverartarchive.org/release/x/front").is_ok());
        assert!(
            ok("http://archive.org/download/x.jpg").is_err(),
            "no downgrade to HTTP"
        );
        assert!(ok("https://evil.example/archive.org").is_err());
        assert!(ok("https://archive.org.evil.example/x").is_err());
        assert!(ok("https://notarchive.org/x").is_err());
        assert!(redirect_allowed(&Url::parse("https://archive.org/x").unwrap(), 5).is_err());
        assert!(ReqwestImageTransport::new().is_ok());
    }
}
