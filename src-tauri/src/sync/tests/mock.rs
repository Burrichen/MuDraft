//! An in-memory GitHub: the Git database endpoints MuDraft uses, with real blob IDs,
//! fast-forward checks, empty-repository behaviour, fault injection, and a hook that moves
//! the branch just before the next ref update (a remote-head race).

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Value, json};
use sha1::{Digest, Sha1};

use crate::metadata::BoxFuture;
use crate::sync::github::{GitHubTransport, Method, Reply, Request, TransportError, git_blob_sha};

pub const PREFIX: &str = "https://api.github.com/repos/me/MuDraft-Sync";

#[derive(Debug, Clone, Copy)]
pub enum FaultReply {
    Status(u16),
    /// 429 with `Retry-After`.
    RetryAfter(u64),
    /// Connection lost before the request reached GitHub.
    Transport,
    /// GitHub handled the request but the reply was lost.
    LostReply,
}

pub struct Fault {
    pub method: Method,
    pub contains: &'static str,
    pub times: usize,
    pub reply: FaultReply,
}

#[derive(Default)]
pub struct Repo {
    pub token: String,
    pub private: bool,
    pub default_branch: String,
    pub blobs: HashMap<String, Vec<u8>>,
    pub trees: HashMap<String, BTreeMap<String, String>>,
    pub commits: HashMap<String, (String, Vec<String>)>,
    pub refs: HashMap<String, String>,
    pub log: Vec<(Method, String)>,
    pub tokens_seen: Vec<String>,
    pub faults: Vec<Fault>,
    /// Commit the branch jumps to right before the next ref update.
    pub race: Option<String>,
    counter: u64,
}

pub struct MockGitHub(pub Mutex<Repo>);

fn sha1_hex(s: &str) -> String {
    Sha1::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn reply(status: u16, body: Value) -> Reply {
    Reply {
        status,
        body: serde_json::to_vec(&body).unwrap(),
        retry_after: None,
        rate_exhausted: false,
    }
}

impl MockGitHub {
    pub fn new(token: &str) -> Self {
        Self(Mutex::new(Repo {
            token: token.into(),
            private: true,
            default_branch: "main".into(),
            ..Repo::default()
        }))
    }

    pub fn repo(&self) -> std::sync::MutexGuard<'_, Repo> {
        self.0.lock().unwrap()
    }

    pub fn head(&self) -> Option<String> {
        self.repo().refs.get("main").cloned()
    }

    pub fn file(&self, path: &str) -> Option<Vec<u8>> {
        let r = self.repo();
        let head = r.refs.get("main")?;
        let tree = &r.trees[&r.commits[head].0];
        tree.get(path).map(|sha| r.blobs[sha].clone())
    }

    pub fn paths(&self) -> Vec<String> {
        let r = self.repo();
        r.refs
            .get("main")
            .map(|h| r.trees[&r.commits[h].0].keys().cloned().collect())
            .unwrap_or_default()
    }

    pub fn count(&self, method: Method, contains: &str) -> usize {
        self.repo()
            .log
            .iter()
            .filter(|(m, u)| *m == method && u.contains(contains))
            .count()
    }

    pub fn fault(&self, method: Method, contains: &'static str, times: usize, reply: FaultReply) {
        self.repo().faults.push(Fault {
            method,
            contains,
            times,
            reply,
        });
    }

    pub fn clear_faults(&self) {
        self.repo().faults.clear();
    }

    /// Commit `files` (path → content; `None` deletes) on top of the branch, as if edited
    /// on github.com.
    pub fn commit_files(&self, files: &[(&str, Option<&[u8]>)]) -> String {
        let mut r = self.repo();
        let parent = r.refs.get("main").cloned();
        let mut tree = parent
            .as_ref()
            .map(|p| r.trees[&r.commits[p].0].clone())
            .unwrap_or_default();
        for (path, content) in files {
            match content {
                Some(bytes) => {
                    let sha = git_blob_sha(bytes);
                    r.blobs.insert(sha.clone(), bytes.to_vec());
                    tree.insert((*path).to_owned(), sha);
                }
                None => {
                    tree.remove(*path);
                }
            }
        }
        let tree_sha = r.store_tree(tree);
        let commit = r.store_commit(&tree_sha, parent.into_iter().collect(), "edited by hand");
        r.refs.insert("main".into(), commit.clone());
        commit
    }

    fn handle(&self, req: &Request, token: &str, max_body: u64) -> Result<Reply, TransportError> {
        let mut r = self.repo();
        r.log.push((req.method, req.url.clone()));
        r.tokens_seen.push(token.to_owned());
        let mut lost = false;
        if let Some(i) = r
            .faults
            .iter()
            .position(|f| f.method == req.method && req.url.contains(f.contains) && f.times > 0)
        {
            r.faults[i].times -= 1;
            match r.faults[i].reply {
                FaultReply::Status(s) => return Ok(reply(s, json!({ "message": "injected" }))),
                FaultReply::RetryAfter(secs) => {
                    let mut out = reply(429, json!({ "message": "slow down" }));
                    out.retry_after = Some(Duration::from_secs(secs));
                    return Ok(out);
                }
                FaultReply::Transport => {
                    return Err(TransportError::Unreachable("connection reset".into()));
                }
                FaultReply::LostReply => lost = true,
            }
        }
        if token != r.token {
            return Ok(reply(401, json!({ "message": "Bad credentials" })));
        }
        let path = req
            .url
            .strip_prefix(PREFIX)
            .expect("request for another repository");
        let body: Value = req
            .body
            .as_ref()
            .map(|b| serde_json::from_slice(b).unwrap())
            .unwrap_or(Value::Null);
        let out = r.route(req.method, path, req.accept, &body);
        if lost {
            return Err(TransportError::Unreachable("reply lost".into()));
        }
        if out.body.len() as u64 > max_body {
            return Err(TransportError::TooLarge);
        }
        Ok(out)
    }
}

impl Repo {
    fn store_tree(&mut self, tree: BTreeMap<String, String>) -> String {
        let sha = sha1_hex(&format!("tree {}", serde_json::to_string(&tree).unwrap()));
        self.trees.insert(sha.clone(), tree);
        sha
    }

    fn store_commit(&mut self, tree: &str, parents: Vec<String>, message: &str) -> String {
        self.counter += 1;
        let sha = sha1_hex(&format!(
            "commit {tree} {parents:?} {message} {}",
            self.counter
        ));
        self.commits.insert(sha.clone(), (tree.to_owned(), parents));
        sha
    }

    fn descends(&self, commit: &str, ancestor: &str) -> bool {
        let mut stack = vec![commit.to_owned()];
        while let Some(c) = stack.pop() {
            if c == ancestor {
                return true;
            }
            if let Some((_, parents)) = self.commits.get(&c) {
                stack.extend(parents.iter().cloned());
            }
        }
        false
    }

    fn route(&mut self, method: Method, path: &str, accept: &str, body: &Value) -> Reply {
        let empty = self.refs.is_empty();
        match (method, path) {
            (Method::Get, "") => reply(
                200,
                json!({ "private": self.private, "default_branch": self.default_branch, "full_name": "me/MuDraft-Sync" }),
            ),
            (Method::Get, p) if p.starts_with("/git/ref/heads/") => {
                if empty {
                    return reply(409, json!({ "message": "Git Repository is empty." }));
                }
                match self.refs.get(&p["/git/ref/heads/".len()..]) {
                    Some(sha) => reply(200, json!({ "object": { "sha": sha, "type": "commit" } })),
                    None => reply(404, json!({ "message": "Not Found" })),
                }
            }
            (Method::Get, p) if p.starts_with("/git/commits/") => {
                match self.commits.get(&p["/git/commits/".len()..]) {
                    Some((tree, parents)) => reply(
                        200,
                        json!({ "tree": { "sha": tree }, "parents": parents.iter().map(|s| json!({ "sha": s })).collect::<Vec<_>>() }),
                    ),
                    None => reply(404, json!({ "message": "Not Found" })),
                }
            }
            (Method::Get, p) if p.starts_with("/git/trees/") => {
                let sha = p["/git/trees/".len()..].trim_end_matches("?recursive=1");
                let Some(tree) = self.trees.get(sha) else {
                    return reply(404, json!({ "message": "Not Found" }));
                };
                let mut entries = Vec::new();
                let mut dirs = std::collections::BTreeSet::new();
                for (path, blob) in tree {
                    let mut prefix = String::new();
                    for part in path.split('/').collect::<Vec<_>>().split_last().unwrap().1 {
                        prefix = if prefix.is_empty() {
                            (*part).to_owned()
                        } else {
                            format!("{prefix}/{part}")
                        };
                        dirs.insert(prefix.clone());
                    }
                    entries.push(json!({ "path": path, "mode": "100644", "type": "blob", "sha": blob, "size": self.blobs[blob].len() }));
                }
                for d in dirs {
                    entries.push(
                        json!({ "path": d, "mode": "040000", "type": "tree", "sha": sha1_hex(&d) }),
                    );
                }
                reply(
                    200,
                    json!({ "sha": sha, "tree": entries, "truncated": false }),
                )
            }
            (Method::Get, p) if p.starts_with("/git/blobs/") => {
                assert_eq!(accept, "application/vnd.github.raw+json");
                match self.blobs.get(&p["/git/blobs/".len()..]) {
                    Some(bytes) => Reply {
                        status: 200,
                        body: bytes.clone(),
                        retry_after: None,
                        rate_exhausted: false,
                    },
                    None => reply(404, json!({ "message": "Not Found" })),
                }
            }
            (Method::Post, "/git/blobs") => {
                if empty {
                    return reply(409, json!({ "message": "Git Repository is empty." }));
                }
                assert_eq!(body["encoding"], "base64");
                let bytes = BASE64.decode(body["content"].as_str().unwrap()).unwrap();
                let sha = git_blob_sha(&bytes);
                self.blobs.insert(sha.clone(), bytes);
                reply(201, json!({ "sha": sha }))
            }
            (Method::Post, "/git/trees") => {
                let Some(mut tree) = body["base_tree"]
                    .as_str()
                    .and_then(|b| self.trees.get(b))
                    .cloned()
                else {
                    return reply(422, json!({ "message": "base_tree missing" }));
                };
                for e in body["tree"].as_array().unwrap() {
                    let path = e["path"].as_str().unwrap().to_owned();
                    match e["sha"].as_str() {
                        Some(sha) if self.blobs.contains_key(sha) => {
                            tree.insert(path, sha.to_owned());
                        }
                        Some(_) => return reply(422, json!({ "message": "blob not found" })),
                        None => {
                            if tree.remove(&path).is_none() {
                                return reply(422, json!({ "message": "path doesn't exist" }));
                            }
                        }
                    }
                }
                let sha = self.store_tree(tree);
                reply(201, json!({ "sha": sha }))
            }
            (Method::Post, "/git/commits") => {
                let tree = body["tree"].as_str().unwrap();
                let parents: Vec<String> = body["parents"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| p.as_str().unwrap().to_owned())
                    .collect();
                if !self.trees.contains_key(tree)
                    || parents.iter().any(|p| !self.commits.contains_key(p))
                {
                    return reply(422, json!({ "message": "bad commit" }));
                }
                let sha = self.store_commit(tree, parents, body["message"].as_str().unwrap());
                reply(201, json!({ "sha": sha }))
            }
            (Method::Patch, p) if p.starts_with("/git/refs/heads/") => {
                assert_eq!(body["force"], false, "MuDraft must never force-push");
                let branch = p["/git/refs/heads/".len()..].to_owned();
                if let Some(raced) = self.race.take() {
                    self.refs.insert(branch.clone(), raced);
                }
                let new = body["sha"].as_str().unwrap().to_owned();
                let Some(current) = self.refs.get(&branch).cloned() else {
                    return reply(422, json!({ "message": "Reference does not exist" }));
                };
                if !self.commits.contains_key(&new) || !self.descends(&new, &current) {
                    return reply(422, json!({ "message": "Update is not a fast forward" }));
                }
                self.refs.insert(branch, new.clone());
                reply(200, json!({ "object": { "sha": new } }))
            }
            (Method::Put, p) if p.starts_with("/contents/") => {
                let file = p["/contents/".len()..].to_owned();
                let bytes = BASE64.decode(body["content"].as_str().unwrap()).unwrap();
                let sha = git_blob_sha(&bytes);
                self.blobs.insert(sha.clone(), bytes);
                let parent = self.refs.get(&self.default_branch).cloned();
                let mut tree = parent
                    .as_ref()
                    .map(|c| self.trees[&self.commits[c].0].clone())
                    .unwrap_or_default();
                if tree.insert(file, sha).is_some() {
                    return reply(422, json!({ "message": "sha wasn't supplied" }));
                }
                let tree_sha = self.store_tree(tree);
                let commit = self.store_commit(
                    &tree_sha,
                    parent.into_iter().collect(),
                    body["message"].as_str().unwrap(),
                );
                let branch = self.default_branch.clone();
                self.refs.insert(branch, commit);
                reply(201, json!({ "content": {} }))
            }
            _ => reply(404, json!({ "message": format!("no route for {path}") })),
        }
    }
}

impl GitHubTransport for MockGitHub {
    fn send<'a>(
        &'a self,
        request: &'a Request,
        token: &'a str,
        max_body: u64,
    ) -> BoxFuture<'a, Result<Reply, TransportError>> {
        Box::pin(async move { self.handle(request, token, max_body) })
    }
}
