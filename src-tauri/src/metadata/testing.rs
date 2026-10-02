//! Scripted transport for provider tests: no network, deterministic replies.

use std::sync::Mutex;

use super::BoxFuture;
use super::http::{HttpResponse, Transport, TransportError};

pub enum Reply {
    Json(u16, String),
    RateLimited(Option<u64>),
    Timeout,
    Offline,
    /// Never completes (to test cancellation mid-request).
    Hang,
}

type Responder = dyn Fn(&str, usize) -> Reply + Send + Sync;

pub struct MockTransport {
    responder: Box<Responder>,
    pub calls: Mutex<Vec<(String, tokio::time::Instant)>>,
}

impl MockTransport {
    /// `responder(url, call_index)` decides each reply.
    pub fn new(responder: impl Fn(&str, usize) -> Reply + Send + Sync + 'static) -> Self {
        Self {
            responder: Box::new(responder),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    pub fn urls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|(u, _)| u.clone())
            .collect()
    }
}

impl Transport for MockTransport {
    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<HttpResponse, TransportError>> {
        let index = {
            let mut calls = self.calls.lock().unwrap();
            calls.push((url.to_owned(), tokio::time::Instant::now()));
            calls.len() - 1
        };
        let reply = (self.responder)(url, index);
        Box::pin(async move {
            match reply {
                Reply::Json(status, body) => Ok(HttpResponse {
                    status,
                    body,
                    retry_after: None,
                }),
                Reply::RateLimited(secs) => Ok(HttpResponse {
                    status: 503,
                    body: String::new(),
                    retry_after: secs.map(std::time::Duration::from_secs),
                }),
                Reply::Timeout => Err(TransportError::Timeout),
                Reply::Offline => Err(TransportError::Unreachable("no route to host".into())),
                Reply::Hang => std::future::pending().await,
            }
        })
    }
}
