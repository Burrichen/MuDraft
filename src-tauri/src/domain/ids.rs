use uuid::Uuid;

use crate::error::{AppError, AppResult};

/// New stable local ID (UUIDv7: time-ordered, so inserts stay index-friendly).
pub fn new_id() -> String {
    Uuid::now_v7().hyphenated().to_string()
}

/// Validate a local or provider UUID and normalize it to lowercase hyphenated form.
pub fn parse_uuid(field: &'static str, raw: &str) -> AppResult<String> {
    let id = Uuid::try_parse(raw.trim())
        .map_err(|_| AppError::validation(field, format!("'{raw}' is not a valid UUID")))?;
    if id.is_nil() {
        return Err(AppError::validation(field, "must not be the nil UUID"));
    }
    Ok(id.hyphenated().to_string())
}

pub fn parse_optional_uuid(field: &'static str, raw: Option<&str>) -> AppResult<Option<String>> {
    raw.map(|r| parse_uuid(field, r)).transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_ids_are_unique_and_parse() {
        let a = new_id();
        assert_ne!(a, new_id());
        assert_eq!(parse_uuid("id", &a).unwrap(), a);
    }

    #[test]
    fn normalizes_and_rejects() {
        let upper = "0190F5C3-2B7A-7C3E-9A1B-3C4D5E6F7A8B";
        assert_eq!(parse_uuid("id", upper).unwrap(), upper.to_lowercase());
        assert_eq!(parse_uuid("id", "nope").unwrap_err().code(), "validation");
        assert!(parse_uuid("id", &Uuid::nil().to_string()).is_err());
    }
}
