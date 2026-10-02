use rusqlite::{OptionalExtension, Transaction, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::domain::ids::parse_uuid;
use crate::error::{AppError, AppResult};

/// Apply `f` at most once per caller-supplied `key`. A retry with the same key and an
/// identical request returns the recorded result; the same key with a different request
/// is a conflict. Without a key, `f` simply runs. Must be called inside the transaction
/// that performs the mutation, so the record and the effect commit together.
pub fn once<Req, Res>(
    tx: &Transaction<'_>,
    key: Option<&str>,
    kind: &'static str,
    request: &Req,
    f: impl FnOnce(&Transaction<'_>) -> AppResult<Res>,
) -> AppResult<Res>
where
    Req: Serialize,
    Res: Serialize + DeserializeOwned,
{
    let Some(key) = key else {
        return f(tx);
    };
    let key = parse_uuid("mutation key", key)?;
    let request_json = serde_json::to_string(request)
        .map_err(|e| AppError::Internal(format!("cannot encode request: {e}")))?;

    let prior: Option<(String, String, String)> = tx
        .query_row(
            "SELECT kind, request, result FROM applied_mutation WHERE key = ?1",
            params![key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    if let Some((prior_kind, prior_request, prior_result)) = prior {
        if prior_kind != kind || prior_request != request_json {
            return Err(AppError::Conflict(format!(
                "mutation key {key} was already used for a different {prior_kind} request"
            )));
        }
        return serde_json::from_str(&prior_result).map_err(|e| {
            AppError::StorageCorrupt(format!("recorded result for {key} is unreadable: {e}"))
        });
    }

    let result = f(tx)?;
    let result_json = serde_json::to_string(&result)
        .map_err(|e| AppError::Internal(format!("cannot encode result: {e}")))?;
    tx.execute(
        "INSERT INTO applied_mutation (key, kind, request, result) VALUES (?1, ?2, ?3, ?4)",
        params![key, kind, request_json, result_json],
    )?;
    Ok(result)
}
