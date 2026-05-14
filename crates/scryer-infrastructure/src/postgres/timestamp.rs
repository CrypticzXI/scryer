use chrono::{DateTime, Utc};
use scryer_application::{AppError, AppResult};

pub(crate) fn parse_rfc3339_timestamp(value: &str, field: &str) -> AppResult<DateTime<Utc>> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return Err(AppError::Repository(format!(
            "missing RFC3339 timestamp for {field}"
        )));
    }

    DateTime::parse_from_rfc3339(normalized)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| {
            AppError::Repository(format!("invalid RFC3339 timestamp for {field}: {error}"))
        })
}

pub(crate) fn parse_optional_rfc3339_timestamp(
    value: Option<&str>,
    field: &str,
) -> AppResult<Option<DateTime<Utc>>> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| parse_rfc3339_timestamp(value, field))
        .transpose()
}

pub(crate) fn parse_rfc3339_or_now(value: impl AsRef<str>) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value.as_ref().trim())
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::{parse_optional_rfc3339_timestamp, parse_rfc3339_or_now, parse_rfc3339_timestamp};
    use scryer_application::AppError;

    #[test]
    fn parses_required_rfc3339_timestamp() {
        let parsed = parse_rfc3339_timestamp(" 2026-05-14T03:19:45Z ", "field")
            .expect("timestamp should parse");

        assert_eq!(
            parsed,
            Utc.with_ymd_and_hms(2026, 5, 14, 3, 19, 45).unwrap()
        );
    }

    #[test]
    fn optional_blank_timestamp_is_treated_as_none() {
        let parsed = parse_optional_rfc3339_timestamp(Some("   "), "field")
            .expect("blank optional timestamp should be ignored");

        assert_eq!(parsed, None);
    }

    #[test]
    fn invalid_timestamp_reports_the_field_name() {
        let error = parse_rfc3339_timestamp("nope", "media_files.grabbed_at")
            .expect_err("invalid timestamp should fail");

        match error {
            AppError::Repository(message) => {
                assert!(message.contains("media_files.grabbed_at"));
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn parse_or_now_preserves_valid_values() {
        let parsed = parse_rfc3339_or_now("2026-05-14T03:19:45Z");

        assert_eq!(
            parsed,
            Utc.with_ymd_and_hms(2026, 5, 14, 3, 19, 45).unwrap()
        );
    }
}
