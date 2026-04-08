use crate::{AppError, WantedItem};
use chrono::{DateTime, NaiveDate, Utc};

pub(crate) const FAILED_GRAB_OLD_TITLE_DAYS: i64 = 14;
pub(crate) const FAILED_GRAB_RESEARCH_COOLDOWN_MINUTES: i64 = 20;

pub(crate) fn extract_grabbed_release_title(raw: Option<&str>) -> Option<String> {
    raw.and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .and_then(|value| {
            value
                .get("title")
                .and_then(|title| title.as_str())
                .map(str::to_string)
        })
}

/// Returns true if the error indicates all prioritized download clients failed.
pub(crate) fn is_all_clients_failed_error(err: &AppError) -> bool {
    matches!(err, AppError::Repository(msg) if msg.contains("all prioritized download clients failed"))
}

pub(crate) fn should_research_failed_grab(item: &WantedItem, now: &DateTime<Utc>) -> bool {
    !is_old_failed_grab_title(item, now)
        && is_last_search_stale(item.last_search_at.as_deref(), now)
}

pub(crate) fn is_old_failed_grab_title(item: &WantedItem, now: &DateTime<Utc>) -> bool {
    let Some(baseline_date) = item.baseline_date.as_deref() else {
        return false;
    };
    let Some(parsed_date) = parse_failed_grab_baseline_date(baseline_date) else {
        return false;
    };
    now.date_naive()
        .signed_duration_since(parsed_date)
        .num_days()
        > FAILED_GRAB_OLD_TITLE_DAYS
}

fn is_last_search_stale(last_search_at: Option<&str>, now: &DateTime<Utc>) -> bool {
    let Some(last_search_at) = last_search_at else {
        return true;
    };
    let Some(last_search_at) = crate::quality_profile::parse_published_at(last_search_at) else {
        return true;
    };
    (*now - last_search_at).num_minutes() > FAILED_GRAB_RESEARCH_COOLDOWN_MINUTES
}

fn parse_failed_grab_baseline_date(raw: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d")
        .ok()
        .or_else(|| {
            chrono::DateTime::parse_from_rfc3339(raw)
                .ok()
                .map(|value| value.date_naive())
        })
        .or_else(|| {
            chrono::DateTime::parse_from_rfc2822(raw)
                .ok()
                .map(|value| value.date_naive())
        })
}
