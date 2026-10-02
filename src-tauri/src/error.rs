use serde::ser::{Serialize, SerializeStruct, Serializer};

/// Structured error crossing the IPC boundary as `{ code, message }`.
/// Codes are stable identifiers the UI may branch on; messages are for humans.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AppError {
    #[error("Storage is unavailable: {0}")]
    StorageUnavailable(String),
    #[error("Storage appears damaged: {0}")]
    StorageCorrupt(String),
    #[error(
        "Database schema v{found} was created by a newer MuDraft (this build supports v{supported}); it has not been modified"
    )]
    SchemaTooNew { found: i64, supported: i64 },
    #[error("Invalid data directory: {0}")]
    InvalidPath(String),
    #[error("Invalid {field}: {reason}")]
    Validation { field: &'static str, reason: String },
    #[error("{entity} {id} does not exist")]
    NotFound { entity: &'static str, id: String },
    #[error("Conflicts with existing data: {0}")]
    Conflict(String),
    #[error("Backup failed: {0}")]
    Backup(String),
    #[error("Can't reach {0}. Check your connection; cached results are used when available.")]
    Offline(String),
    #[error("{0} took too long to respond")]
    Timeout(String),
    #[error("{0} is limiting requests right now; try again shortly")]
    RateLimited(String),
    #[error("Request cancelled")]
    Cancelled,
    #[error("Metadata provider error: {0}")]
    Provider(String),
    #[error("Internal error: {0}")]
    Internal(String),
}

impl AppError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::StorageUnavailable(_) => "storage_unavailable",
            Self::StorageCorrupt(_) => "storage_corrupt",
            Self::SchemaTooNew { .. } => "schema_too_new",
            Self::InvalidPath(_) => "invalid_path",
            Self::Validation { .. } => "validation",
            Self::NotFound { .. } => "not_found",
            Self::Conflict(_) => "conflict",
            Self::Backup(_) => "backup_failed",
            Self::Offline(_) => "offline",
            Self::Timeout(_) => "timeout",
            Self::RateLimited(_) => "rate_limited",
            Self::Cancelled => "cancelled",
            Self::Provider(_) => "provider_error",
            Self::Internal(_) => "internal",
        }
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("AppError", 2)?;
        s.serialize_field("code", self.code())?;
        s.serialize_field("message", &self.to_string())?;
        s.end()
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(err: rusqlite::Error) -> Self {
        use rusqlite::ErrorCode;
        match err.sqlite_error_code() {
            Some(ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt) => {
                Self::StorageCorrupt(err.to_string())
            }
            // Constraints are the last line of defence behind Rust validation; report, never retry.
            Some(ErrorCode::ConstraintViolation) => {
                match err.sqlite_error().map(|e| e.extended_code) {
                    Some(
                        rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                        | rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY,
                    ) => Self::Conflict(err.to_string()),
                    _ => Self::validation("record", err.to_string()),
                }
            }
            _ => Self::StorageUnavailable(err.to_string()),
        }
    }
}

impl AppError {
    pub fn validation(field: &'static str, reason: impl Into<String>) -> Self {
        Self::Validation {
            field,
            reason: reason.into(),
        }
    }

    pub fn not_found(entity: &'static str, id: impl Into<String>) -> Self {
        Self::NotFound {
            entity,
            id: id.into(),
        }
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_code_and_message() {
        let json = serde_json::to_value(AppError::SchemaTooNew {
            found: 9,
            supported: 1,
        })
        .unwrap();
        assert_eq!(json["code"], "schema_too_new");
        assert!(json["message"].as_str().unwrap().contains("v9"));
        assert_eq!(json.as_object().unwrap().len(), 2);
    }
}
