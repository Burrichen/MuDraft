//! Calendar dates without time zones. A listening date is what the user's local calendar
//! said; it is stored as `YYYY-MM-DD` and never converted through UTC midnight.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CalendarDate {
    year: u16,
    month: u8,
    day: u8,
}

impl CalendarDate {
    pub fn new(field: &'static str, year: i64, month: i64, day: i64) -> AppResult<Self> {
        let year = check_year(field, year)?;
        let month = check_month(field, month)?;
        let max = days_in_month(year, month);
        let day = u8::try_from(day)
            .ok()
            .filter(|d| (1..=max).contains(d))
            .ok_or_else(|| {
                AppError::validation(
                    field,
                    format!("day {day} is not valid for {year:04}-{month:02}"),
                )
            })?;
        Ok(Self { year, month, day })
    }

    /// Parse strict `YYYY-MM-DD`.
    pub fn parse(field: &'static str, raw: &str) -> AppResult<Self> {
        let parts = split_numeric(field, raw, &[4, 2, 2])?;
        Self::new(field, parts[0], parts[1], parts[2])
    }
}

impl fmt::Display for CalendarDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DatePrecision {
    Unknown,
    Year,
    Month,
    Day,
}

impl DatePrecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Year => "year",
            Self::Month => "month",
            Self::Day => "day",
        }
    }
}

/// A release date with explicit precision. `Unknown` is a real value, not a missing field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartialDate {
    Unknown,
    Year(u16),
    Month(u16, u8),
    Day(CalendarDate),
}

impl PartialDate {
    /// Parse `None`/empty → Unknown, `YYYY`, `YYYY-MM`, or `YYYY-MM-DD`.
    pub fn parse(field: &'static str, raw: Option<&str>) -> AppResult<Self> {
        let Some(raw) = raw.map(str::trim).filter(|r| !r.is_empty()) else {
            return Ok(Self::Unknown);
        };
        match raw.len() {
            4 => {
                let p = split_numeric(field, raw, &[4])?;
                Ok(Self::Year(check_year(field, p[0])?))
            }
            7 => {
                let p = split_numeric(field, raw, &[4, 2])?;
                Ok(Self::Month(
                    check_year(field, p[0])?,
                    check_month(field, p[1])?,
                ))
            }
            _ => CalendarDate::parse(field, raw).map(Self::Day),
        }
    }

    pub fn precision(&self) -> DatePrecision {
        match self {
            Self::Unknown => DatePrecision::Unknown,
            Self::Year(_) => DatePrecision::Year,
            Self::Month(..) => DatePrecision::Month,
            Self::Day(_) => DatePrecision::Day,
        }
    }

    /// Storage form: (text value or NULL, precision).
    pub fn to_db(&self) -> (Option<String>, &'static str) {
        let value = match self {
            Self::Unknown => None,
            Self::Year(y) => Some(format!("{y:04}")),
            Self::Month(y, m) => Some(format!("{y:04}-{m:02}")),
            Self::Day(d) => Some(d.to_string()),
        };
        (value, self.precision().as_str())
    }

    pub fn year(&self) -> Option<u16> {
        match self {
            Self::Unknown => None,
            Self::Year(y) | Self::Month(y, _) => Some(*y),
            Self::Day(d) => Some(d.year),
        }
    }
}

/// When a listen happened: a known local calendar date, or listened previously without a date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListenDate {
    Known(CalendarDate),
    Undated,
}

impl ListenDate {
    pub fn to_db(&self) -> (Option<String>, &'static str) {
        match self {
            Self::Known(d) => (Some(d.to_string()), "known"),
            Self::Undated => (None, "undated"),
        }
    }
}

fn check_year(field: &'static str, year: i64) -> AppResult<u16> {
    u16::try_from(year)
        .ok()
        .filter(|y| (1..=9999).contains(y))
        .ok_or_else(|| AppError::validation(field, format!("year {year} is outside 0001–9999")))
}

fn check_month(field: &'static str, month: i64) -> AppResult<u8> {
    u8::try_from(month)
        .ok()
        .filter(|m| (1..=12).contains(m))
        .ok_or_else(|| AppError::validation(field, format!("month {month} is outside 1–12")))
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        2 if (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400) => {
            29
        }
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Split `raw` on '-' into all-digit parts of exactly the given widths.
fn split_numeric(field: &'static str, raw: &str, widths: &[usize]) -> AppResult<Vec<i64>> {
    let parts: Vec<&str> = raw.split('-').collect();
    let well_formed = parts.len() == widths.len()
        && parts
            .iter()
            .zip(widths)
            .all(|(p, w)| p.len() == *w && p.bytes().all(|b| b.is_ascii_digit()));
    if !well_formed {
        let expected = ["YYYY", "MM", "DD"][..widths.len()].join("-");
        return Err(AppError::validation(
            field,
            format!("'{raw}' is not in {expected} form"),
        ));
    }
    Ok(parts.iter().map(|p| p.parse().expect("digits")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_dates_are_strict() {
        assert_eq!(
            CalendarDate::parse("d", "2024-02-29").unwrap().to_string(),
            "2024-02-29"
        );
        assert!(CalendarDate::parse("d", "2023-02-29").is_err());
        assert!(CalendarDate::parse("d", "1900-02-29").is_err());
        assert!(CalendarDate::parse("d", "2000-02-29").is_ok());
        assert!(CalendarDate::parse("d", "2024-13-01").is_err());
        assert!(CalendarDate::parse("d", "2024-1-01").is_err());
        assert!(CalendarDate::parse("d", "2024-01-01T00:00:00Z").is_err());
    }

    #[test]
    fn partial_dates_keep_precision() {
        assert_eq!(PartialDate::parse("d", None).unwrap(), PartialDate::Unknown);
        assert_eq!(
            PartialDate::parse("d", Some("  ")).unwrap(),
            PartialDate::Unknown
        );
        assert_eq!(
            PartialDate::parse("d", Some("1997")).unwrap().to_db(),
            (Some("1997".into()), "year")
        );
        assert_eq!(
            PartialDate::parse("d", Some("1997-05")).unwrap().to_db(),
            (Some("1997-05".into()), "month")
        );
        let day = PartialDate::parse("d", Some("1997-05-21")).unwrap();
        assert_eq!(day.precision(), DatePrecision::Day);
        assert_eq!(day.year(), Some(1997));
        assert!(PartialDate::parse("d", Some("1997-5")).is_err());
        assert!(PartialDate::parse("d", Some("0000")).is_err());
    }
}
