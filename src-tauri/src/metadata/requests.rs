//! In-flight request registry so the renderer can cancel a search it no longer needs
//! (new keystrokes, dialog closed). IDs are renderer-chosen UUIDs.

use std::collections::HashMap;
use std::sync::Mutex;

use tokio_util::sync::CancellationToken;

use crate::domain::ids::parse_uuid;
use crate::error::{AppError, AppResult};

#[derive(Default)]
pub struct RequestRegistry {
    active: Mutex<HashMap<String, CancellationToken>>,
}

/// Removes its entry when the request finishes, however it finishes.
pub struct RequestGuard<'a> {
    registry: &'a RequestRegistry,
    id: String,
    pub token: CancellationToken,
}

impl Drop for RequestGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.registry.active.lock() {
            active.remove(&self.id);
        }
    }
}

impl RequestRegistry {
    pub fn begin(&self, request_id: &str) -> AppResult<RequestGuard<'_>> {
        let id = parse_uuid("request ID", request_id)?;
        let token = CancellationToken::new();
        let mut active = self
            .active
            .lock()
            .map_err(|_| AppError::Internal("request registry poisoned".into()))?;
        if active.contains_key(&id) {
            return Err(AppError::Conflict(format!(
                "request {id} is already running"
            )));
        }
        active.insert(id.clone(), token.clone());
        Ok(RequestGuard {
            registry: self,
            id,
            token,
        })
    }

    /// Cancel a running request. Unknown or finished IDs are a no-op (it may have just ended).
    pub fn cancel(&self, request_id: &str) -> AppResult<bool> {
        let id = parse_uuid("request ID", request_id)?;
        let active = self
            .active
            .lock()
            .map_err(|_| AppError::Internal("request registry poisoned".into()))?;
        Ok(active.get(&id).map(CancellationToken::cancel).is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::new_id;

    #[test]
    fn begin_cancel_and_cleanup() {
        let registry = RequestRegistry::default();
        let id = new_id();
        {
            let guard = registry.begin(&id).unwrap();
            assert_eq!(registry.begin(&id).err().unwrap().code(), "conflict");
            assert!(registry.cancel(&id).unwrap());
            assert!(guard.token.is_cancelled());
        }
        assert!(
            !registry.cancel(&id).unwrap(),
            "finished requests are forgotten"
        );
        assert_eq!(registry.cancel("nope").unwrap_err().code(), "validation");
    }
}
