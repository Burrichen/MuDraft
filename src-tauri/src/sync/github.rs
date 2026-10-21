//! GitHub REST access for sync: the Git database API (refs, commits, trees, blobs) over
//! HTTPS with a fine-grained token, plus the contents API once to give an empty repository
//! its first commit. The transport is a trait so every behaviour is tested against a mock.
//!
//! The token is passed per request and only ever placed in the `Authorization` header
//! (marked sensitive); it is never part of a URL, body, log line, or error message.

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::Deserialize;
use serde_json::{Value, json};
use sha1::{Digest, Sha1};

use crate::error::{AppError, AppResult};
use crate::metadata::BoxFuture;
use crate::metadata::http::USER_AGENT;

pub const API_BASE: &str = "https://api.github.com";
const API_VERSION: &str = "2022-11-28";
const JSON: &str = "application/vnd.github+json";
const RAW: &str = "application/vnd.github.raw+json";
const SERVICE: &str = "GitHub";
/// Retries after the first attempt for transient failures.
const MAX_RETRIES: u32 = 3;
/// Longer waits than this are reported instead of blocking the app.
const MAX_WAIT: Duration = Duration::from_secs(60);
/// Bound for ordinary JSON responses (trees of a big profile stay well under this).
const MAX_JSON_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Patch,
    Put,
}

#[derive(Clone)]
pub struct Request {
    pub method: Method,
    pub url: String,
    pub accept: &'static str,
    pub body: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
    /// Parsed `Retry-After` (seconds form).
    pub retry_after: Option<Duration>,
    /// `x-ratelimit-remaining: 0`.
    pub rate_exhausted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    Timeout,
    /// DNS, connection, TLS, or a dropped connection: treated as offline.
    Unreachable(String),
    /// The response body exceeded the caller's bound.
    TooLarge,
}

pub trait GitHubTransport: Send + Sync {
    fn send<'a>(
        &'a self,
        request: &'a Request,
        token: &'a str,
        max_body: u64,
    ) -> BoxFuture<'a, Result<Reply, TransportError>>;
}

pub struct ReqwestGitHubTransport {
    client: reqwest::Client,
}

impl ReqwestGitHubTransport {
    pub fn new() -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .https_only(true)
            // A moved repository is reported, never followed with the token attached.
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| format!("cannot build HTTP client: {e}"))?;
        Ok(Self { client })
    }
}

impl GitHubTransport for ReqwestGitHubTransport {
    fn send<'a>(
        &'a self,
        request: &'a Request,
        token: &'a str,
        max_body: u64,
    ) -> BoxFuture<'a, Result<Reply, TransportError>> {
        Box::pin(async move {
            let method = match request.method {
                Method::Get => reqwest::Method::GET,
                Method::Post => reqwest::Method::POST,
                Method::Patch => reqwest::Method::PATCH,
                Method::Put => reqwest::Method::PUT,
            };
            let mut auth = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|_| TransportError::Unreachable("the stored token is malformed".into()))?;
            auth.set_sensitive(true);
            let mut builder = self
                .client
                .request(method, &request.url)
                .header(reqwest::header::AUTHORIZATION, auth)
                .header(reqwest::header::ACCEPT, request.accept)
                .header("X-GitHub-Api-Version", API_VERSION);
            if let Some(body) = &request.body {
                builder = builder
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(body.clone());
            }
            let mut response = builder.send().await.map_err(classify)?;
            let status = response.status().as_u16();
            let header = |name: &str| {
                response
                    .headers()
                    .get(name)
                    .and_then(|v| v.to_str().ok())
                    .map(|v| v.trim().to_owned())
            };
            let retry_after = header("retry-after")
                .and_then(|v| v.parse::<u64>().ok())
                .map(Duration::from_secs);
            let rate_exhausted = header("x-ratelimit-remaining").as_deref() == Some("0");
            if response.content_length().is_some_and(|n| n > max_body) {
                return Err(TransportError::TooLarge);
            }
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(classify)? {
                if (body.len() + chunk.len()) as u64 > max_body {
                    return Err(TransportError::TooLarge);
                }
                body.extend_from_slice(&chunk);
            }
            Ok(Reply {
                status,
                body,
                retry_after,
                rate_exhausted,
            })
        })
    }
}

fn classify(err: reqwest::Error) -> TransportError {
    if err.is_timeout() {
        TransportError::Timeout
    } else {
        // Without the URL: it's ours, and keeping errors short keeps them clean.
        TransportError::Unreachable(err.without_url().to_string())
    }
}

/// Git's object ID for a blob with this content.
pub fn git_blob_sha(bytes: &[u8]) -> String {
    let mut h = Sha1::new();
    h.update(format!("blob {}\0", bytes.len()).as_bytes());
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// GitHub owner and repository names as GitHub allows them.
pub fn validate_repo(owner: &str, repo: &str) -> AppResult<(String, String)> {
    let (owner, repo) = (owner.trim(), repo.trim());
    let owner_ok = !owner.is_empty()
        && owner.len() <= 39
        && !owner.starts_with('-')
        && owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !owner_ok {
        return Err(AppError::validation(
            "owner",
            "enter a GitHub user or organisation name",
        ));
    }
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    let repo_ok = !repo.is_empty()
        && repo.len() <= 100
        && repo != "."
        && repo != ".."
        && repo
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if !repo_ok {
        return Err(AppError::validation(
            "repository",
            "enter the repository name, e.g. MuDraft-Sync",
        ));
    }
    Ok((owner.to_owned(), repo.to_owned()))
}

fn encode_path(segment: &str) -> String {
    segment
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoInfo {
    pub private: bool,
    pub default_branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TreeEntry {
    pub path: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub sha: String,
    #[serde(default)]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefUpdate {
    Updated,
    /// Someone else moved the branch first.
    NotFastForward,
}

pub struct Client<'a> {
    transport: &'a dyn GitHubTransport,
    token: &'a str,
    repo_url: String,
}

fn message(body: &[u8]) -> String {
    let text = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| v.get("message").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_default();
    text.chars().take(200).collect()
}

fn decode<T: for<'de> Deserialize<'de>>(reply: &Reply, what: &str) -> AppResult<T> {
    serde_json::from_slice(&reply.body)
        .map_err(|e| AppError::SyncRemote(format!("unexpected reply for {what} ({e})")))
}

fn is_rate_limited(reply: &Reply) -> bool {
    reply.status == 429
        || (reply.status == 403
            && (reply.rate_exhausted
                || reply.retry_after.is_some()
                || message(&reply.body)
                    .to_ascii_lowercase()
                    .contains("rate limit")))
}

/// Map a final non-success reply to a clear error. Never includes the token.
pub fn status_error(reply: &Reply, what: &str) -> AppError {
    let detail = message(&reply.body);
    match reply.status {
        401 => AppError::AuthRequired(
            "GitHub rejected the access token (expired or revoked). Add a new token.".into(),
        ),
        _ if is_rate_limited(reply) => AppError::RateLimited(SERVICE.into()),
        403 => AppError::SyncSetup(
            "GitHub refused access. The token needs Contents read and write on this repository."
                .into(),
        ),
        404 => {
            AppError::SyncSetup("the repository wasn't found, or the token can't access it".into())
        }
        301 | 302 | 307 | 308 => AppError::SyncSetup(
            "the repository has moved or been renamed; enter its new name".into(),
        ),
        s if s >= 500 => AppError::SyncRemote(format!(
            "GitHub is having problems (status {s}); try again later"
        )),
        s => AppError::SyncRemote(
            format!("{what} failed (status {s}) {detail}")
                .trim()
                .to_owned(),
        ),
    }
}

impl<'a> Client<'a> {
    pub fn new(
        transport: &'a dyn GitHubTransport,
        token: &'a str,
        api_base: &str,
        owner: &str,
        repo: &str,
    ) -> Self {
        Self {
            transport,
            token,
            repo_url: format!("{api_base}/repos/{owner}/{repo}"),
        }
    }

    /// Send with retries for transient failures (rate limits, server errors, network).
    /// `retry` is false for requests whose effect is uncertain after a lost reply.
    async fn call(
        &self,
        method: Method,
        path: &str,
        accept: &'static str,
        body: Option<Value>,
        retry: bool,
        max_body: u64,
    ) -> AppResult<Reply> {
        let request = Request {
            method,
            url: format!("{}{path}", self.repo_url),
            accept,
            body: body.map(|b| serde_json::to_vec(&b).unwrap_or_default()),
        };
        let mut attempt = 0;
        loop {
            let backoff = Duration::from_secs(1 << attempt.min(5));
            match self.transport.send(&request, self.token, max_body).await {
                Ok(reply) if (200..300).contains(&reply.status) => return Ok(reply),
                Ok(reply) => {
                    let transient =
                        is_rate_limited(&reply) || matches!(reply.status, 500 | 502 | 503 | 504);
                    if !transient || !retry || attempt >= MAX_RETRIES {
                        return Ok(reply);
                    }
                    let wait = reply.retry_after.unwrap_or(backoff);
                    if wait > MAX_WAIT {
                        return Err(AppError::RateLimited(SERVICE.into()));
                    }
                    tokio::time::sleep(wait).await;
                }
                Err(TransportError::TooLarge) => {
                    return Err(AppError::validation(
                        "sync data",
                        "GitHub sent more data than allowed",
                    ));
                }
                Err(e) if retry && attempt < MAX_RETRIES => {
                    let _ = e;
                    tokio::time::sleep(backoff).await;
                }
                Err(TransportError::Timeout) => return Err(AppError::Timeout(SERVICE.into())),
                Err(TransportError::Unreachable(_)) => {
                    return Err(AppError::SyncOffline(SERVICE.into()));
                }
            }
            attempt += 1;
        }
    }

    async fn ok(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        retry: bool,
        what: &str,
    ) -> AppResult<Reply> {
        let reply = self
            .call(method, path, JSON, body, retry, MAX_JSON_BYTES)
            .await?;
        if (200..300).contains(&reply.status) {
            Ok(reply)
        } else {
            Err(status_error(&reply, what))
        }
    }

    pub async fn repo_info(&self) -> AppResult<RepoInfo> {
        #[derive(Deserialize)]
        struct Repo {
            private: bool,
            default_branch: String,
        }
        let reply = self
            .ok(Method::Get, "", None, true, "reading the repository")
            .await?;
        let repo: Repo = decode(&reply, "the repository")?;
        Ok(RepoInfo {
            private: repo.private,
            default_branch: repo.default_branch,
        })
    }

    /// The branch's commit, or `None` for an empty repository (no commits yet).
    pub async fn head(&self, branch: &str) -> AppResult<Option<String>> {
        let path = format!("/git/ref/heads/{}", encode_path(branch));
        let reply = self
            .call(Method::Get, &path, JSON, None, true, MAX_JSON_BYTES)
            .await?;
        match reply.status {
            200 => {
                #[derive(Deserialize)]
                struct Obj {
                    sha: String,
                }
                #[derive(Deserialize)]
                struct Ref {
                    object: Obj,
                }
                Ok(Some(decode::<Ref>(&reply, "the branch")?.object.sha))
            }
            // GitHub answers 409 "Git Repository is empty" (or 404) before the first commit.
            404 | 409 => Ok(None),
            _ => Err(status_error(&reply, "reading the branch")),
        }
    }

    pub async fn commit_tree(&self, commit: &str) -> AppResult<String> {
        #[derive(Deserialize)]
        struct Tree {
            sha: String,
        }
        #[derive(Deserialize)]
        struct Commit {
            tree: Tree,
        }
        let reply = self
            .ok(
                Method::Get,
                &format!("/git/commits/{commit}"),
                None,
                true,
                "reading a commit",
            )
            .await?;
        Ok(decode::<Commit>(&reply, "a commit")?.tree.sha)
    }

    pub async fn tree(&self, tree: &str) -> AppResult<Vec<TreeEntry>> {
        #[derive(Deserialize)]
        struct Tree {
            tree: Vec<TreeEntry>,
            #[serde(default)]
            truncated: bool,
        }
        let reply = self
            .ok(
                Method::Get,
                &format!("/git/trees/{tree}?recursive=1"),
                None,
                true,
                "listing files",
            )
            .await?;
        let tree: Tree = decode(&reply, "the file list")?;
        if tree.truncated {
            return Err(AppError::SyncSetup(
                "the repository holds too many files for MuDraft to read".into(),
            ));
        }
        Ok(tree.tree)
    }

    pub async fn blob(&self, sha: &str, max: u64) -> AppResult<Vec<u8>> {
        let reply = self
            .call(
                Method::Get,
                &format!("/git/blobs/{sha}"),
                RAW,
                None,
                true,
                max,
            )
            .await?;
        if !(200..300).contains(&reply.status) {
            return Err(status_error(&reply, "downloading sync data"));
        }
        if git_blob_sha(&reply.body) != sha {
            return Err(AppError::SyncRemote(
                "downloaded data didn't match its checksum".into(),
            ));
        }
        Ok(reply.body)
    }

    /// Blobs, trees, and commits are content-addressed, so retrying them is harmless.
    pub async fn create_blob(&self, bytes: &[u8]) -> AppResult<String> {
        #[derive(Deserialize)]
        struct Created {
            sha: String,
        }
        let body = json!({ "content": BASE64.encode(bytes), "encoding": "base64" });
        let reply = self
            .ok(
                Method::Post,
                "/git/blobs",
                Some(body),
                true,
                "uploading data",
            )
            .await?;
        let sha = decode::<Created>(&reply, "an upload")?.sha;
        if sha != git_blob_sha(bytes) {
            return Err(AppError::SyncRemote(
                "GitHub stored different data than was sent".into(),
            ));
        }
        Ok(sha)
    }

    /// A tree from `base` with `changes` applied (`None` deletes the path).
    pub async fn create_tree(
        &self,
        base: &str,
        changes: &[(String, Option<String>)],
    ) -> AppResult<String> {
        #[derive(Deserialize)]
        struct Created {
            sha: String,
        }
        let entries: Vec<Value> = changes
            .iter()
            .map(
                |(path, sha)| json!({ "path": path, "mode": "100644", "type": "blob", "sha": sha }),
            )
            .collect();
        let body = json!({ "base_tree": base, "tree": entries });
        let reply = self
            .ok(
                Method::Post,
                "/git/trees",
                Some(body),
                true,
                "uploading the file list",
            )
            .await?;
        Ok(decode::<Created>(&reply, "a file list")?.sha)
    }

    pub async fn create_commit(
        &self,
        message: &str,
        tree: &str,
        parent: &str,
    ) -> AppResult<String> {
        #[derive(Deserialize)]
        struct Created {
            sha: String,
        }
        let body = json!({ "message": message, "tree": tree, "parents": [parent] });
        let reply = self
            .ok(
                Method::Post,
                "/git/commits",
                Some(body),
                true,
                "creating a commit",
            )
            .await?;
        Ok(decode::<Created>(&reply, "a commit")?.sha)
    }

    /// Move the branch to `commit` only as a fast-forward (`force: false`); history is
    /// never rewritten. Not retried: after a lost reply the caller re-reads the branch.
    pub async fn update_ref(&self, branch: &str, commit: &str) -> AppResult<RefUpdate> {
        let path = format!("/git/refs/heads/{}", encode_path(branch));
        let body = json!({ "sha": commit, "force": false });
        let reply = self
            .call(
                Method::Patch,
                &path,
                JSON,
                Some(body),
                false,
                MAX_JSON_BYTES,
            )
            .await?;
        match reply.status {
            200 => Ok(RefUpdate::Updated),
            409 | 422 => Ok(RefUpdate::NotFastForward),
            _ => Err(status_error(&reply, "publishing the sync")),
        }
    }

    /// Give an empty repository its first commit (the Git database API can't write to a
    /// repository with no commits).
    pub async fn put_first_file(&self, path: &str, bytes: &[u8], message: &str) -> AppResult<()> {
        let body = json!({ "message": message, "content": BASE64.encode(bytes) });
        self.ok(
            Method::Put,
            &format!("/contents/{}", encode_path(path)),
            Some(body),
            false,
            "preparing the repository",
        )
        .await
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_ids_match_git() {
        // `printf 'hello\n' | git hash-object --stdin`
        assert_eq!(
            git_blob_sha(b"hello\n"),
            "ce013625030ba8dba906f756967f9e9ca394464a"
        );
        assert_eq!(
            git_blob_sha(b""),
            "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"
        );
    }

    #[test]
    fn repository_names_are_validated() {
        assert_eq!(
            validate_repo(" me ", "MuDraft-Sync.git").unwrap(),
            ("me".to_owned(), "MuDraft-Sync".to_owned())
        );
        for (o, r) in [
            ("", "x"),
            ("a/b", "x"),
            ("me", ".."),
            ("me", "a/b"),
            ("me", "a b"),
            ("-me", "x"),
        ] {
            assert!(validate_repo(o, r).is_err(), "{o}/{r}");
        }
    }

    #[test]
    fn errors_never_echo_the_reply_beyond_githubs_message() {
        let reply = |status, body: &str| Reply {
            status,
            body: body.as_bytes().to_vec(),
            retry_after: None,
            rate_exhausted: false,
        };
        assert_eq!(status_error(&reply(401, "{}"), "x").code(), "auth_required");
        assert_eq!(status_error(&reply(403, "{}"), "x").code(), "sync_setup");
        assert_eq!(
            status_error(&reply(403, r#"{"message":"API rate limit exceeded"}"#), "x").code(),
            "rate_limited"
        );
        assert_eq!(status_error(&reply(404, "{}"), "x").code(), "sync_setup");
        assert_eq!(status_error(&reply(502, "{}"), "x").code(), "sync_remote");
        assert_eq!(status_error(&reply(301, "{}"), "x").code(), "sync_setup");
    }
}
