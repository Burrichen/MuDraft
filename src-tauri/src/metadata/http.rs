//! Native HTTP transport. A trait so provider logic (queue, cache, retry, parsing) can be
//! tested against canned responses without touching the network.

use std::time::Duration;

use super::BoxFuture;

/// Sent with every provider request. MusicBrainz requires an application name, version,
/// and a contact reference; this is the project's public repository.
pub const USER_AGENT: &str = concat!(
    "MuDraft/",
    env!("CARGO_PKG_VERSION"),
    " ( https://github.com/Burrichen/MuDraft )"
);

pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
    /// Parsed `Retry-After` (seconds form) when the server sent one.
    pub retry_after: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    Timeout,
    /// DNS, connection, or TLS failure: treated as offline.
    Unreachable(String),
}

pub trait Transport: Send + Sync {
    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<HttpResponse, TransportError>>;
}

pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    pub fn new() -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .https_only(true)
            .build()
            .map_err(|e| format!("cannot build HTTP client: {e}"))?;
        Ok(Self { client })
    }
}

impl Transport for ReqwestTransport {
    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<HttpResponse, TransportError>> {
        Box::pin(async move {
            let response = self
                .client
                .get(url)
                .header(reqwest::header::ACCEPT, "application/json")
                .send()
                .await
                .map_err(classify)?;
            let status = response.status().as_u16();
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok())
                .map(Duration::from_secs);
            let body = response.text().await.map_err(classify)?;
            Ok(HttpResponse {
                status,
                body,
                retry_after,
            })
        })
    }
}

fn classify(err: reqwest::Error) -> TransportError {
    if err.is_timeout() {
        TransportError::Timeout
    } else {
        TransportError::Unreachable(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_agent_names_app_version_and_real_project() {
        assert_eq!(
            USER_AGENT,
            format!(
                "MuDraft/{} ( https://github.com/Burrichen/MuDraft )",
                env!("CARGO_PKG_VERSION")
            )
        );
        assert!(ReqwestTransport::new().is_ok());
    }
}
