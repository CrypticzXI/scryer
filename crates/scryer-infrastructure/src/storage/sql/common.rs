use chrono::{DateTime, NaiveDate, Utc};
use scryer_application::{AppError, AppResult};

pub(crate) fn parse_utc_datetime(raw: &str) -> AppResult<DateTime<Utc>> {
    if let Ok(datetime) = DateTime::parse_from_rfc3339(raw) {
        return Ok(datetime.with_timezone(&Utc));
    }

    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .map(|datetime| datetime.and_utc())
        .ok_or_else(|| AppError::Repository(format!("invalid UTC datetime: {raw}")))
}
