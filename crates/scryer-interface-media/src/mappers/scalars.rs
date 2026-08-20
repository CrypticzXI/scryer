use super::*;

pub fn parse_iso_date(value: Option<String>) -> Option<Date> {
    value.and_then(|value| Date::parse_iso(&value).ok())
}

pub(super) fn parse_date(value: Option<String>) -> Option<Date> {
    parse_iso_date(value)
}

pub(super) fn parse_datetime(value: &str, field: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| format!("invalid {field} timestamp: {error}"))
}

pub(super) fn parse_required_datetime(value: &str, field: &str) -> DateTime<Utc> {
    parse_datetime(value, field).expect("Scryer-owned timestamp should be RFC3339")
}

pub(super) fn parse_optional_datetime(value: Option<String>, field: &str) -> Option<DateTime<Utc>> {
    value.and_then(|value| parse_datetime(&value, field).ok())
}
