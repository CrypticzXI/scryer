use std::time::Duration;

use crate::AppError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateLimitSignalSource {
    RetryAfterText,
    RetryAfterSecondsText,
    RetryAfterHyphenText,
    RetryAfterUnderscoreText,
    RateLimitPhrase,
    TooManyRequestsPhrase,
    Http429Phrase,
    Status429Phrase,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RateLimitSignal {
    pub retry_after: Option<Duration>,
    pub source: RateLimitSignalSource,
    pub status_code: Option<u16>,
    pub message: Option<String>,
}

impl RateLimitSignal {
    pub fn from_error(error: &AppError) -> Option<Self> {
        Self::from_text(&error.to_string())
    }

    pub fn from_text(message: &str) -> Option<Self> {
        if let Some((retry_after, source)) = retry_after_from_text(message) {
            return Some(Self {
                retry_after: Some(retry_after),
                source,
                status_code: status_code_from_text(message),
                message: Some(message.to_string()),
            });
        }

        let lower = message.to_ascii_lowercase();
        let source = if lower.contains("too many requests") {
            RateLimitSignalSource::TooManyRequestsPhrase
        } else if contains_status_marker(&lower, "http 429") {
            RateLimitSignalSource::Http429Phrase
        } else if contains_status_marker(&lower, "status 429") {
            RateLimitSignalSource::Status429Phrase
        } else if lower.contains("rate limit") || lower.contains("rate-limit") {
            RateLimitSignalSource::RateLimitPhrase
        } else {
            return None;
        };

        Some(Self {
            retry_after: None,
            source,
            status_code: status_code_from_text(message),
            message: Some(message.to_string()),
        })
    }
}

fn status_code_from_text(message: &str) -> Option<u16> {
    for marker in ["http ", "status "] {
        let lower = message.to_ascii_lowercase();
        for (index, _) in lower.match_indices(marker) {
            let digits = lower[index + marker.len()..]
                .chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect::<String>();
            if let Ok(status) = digits.parse::<u16>() {
                return Some(status);
            }
        }
    }
    None
}

fn contains_status_marker(message: &str, marker: &str) -> bool {
    message.match_indices(marker).any(|(index, _)| {
        message[index + marker.len()..]
            .chars()
            .next()
            .is_none_or(|ch| !ch.is_ascii_digit())
    })
}

fn retry_after_from_text(message: &str) -> Option<(Duration, RateLimitSignalSource)> {
    let lower = message.to_ascii_lowercase();
    for (marker, source) in [
        (
            "retry_after_seconds=",
            RateLimitSignalSource::RetryAfterSecondsText,
        ),
        ("retry after", RateLimitSignalSource::RetryAfterText),
        ("retry-after", RateLimitSignalSource::RetryAfterHyphenText),
        (
            "retry_after",
            RateLimitSignalSource::RetryAfterUnderscoreText,
        ),
    ] {
        let Some(index) = lower.find(marker) else {
            continue;
        };
        let suffix = lower[index + marker.len()..]
            .trim_start_matches(|ch: char| ch == ':' || ch == '=' || ch == ' ' || ch == '_');
        let digits = suffix
            .chars()
            .skip_while(|ch| !ch.is_ascii_digit())
            .take_while(|ch| ch.is_ascii_digit())
            .collect::<String>();
        if let Ok(seconds) = digits.parse::<u64>()
            && seconds > 0
        {
            return Some((Duration::from_secs(seconds), source));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_retry_after_seconds_from_flattened_plugin_errors() {
        let signal = RateLimitSignal::from_text("HTTP 429: rate limited; retry_after_seconds=900")
            .expect("retry after should parse");
        assert_eq!(signal.retry_after, Some(Duration::from_secs(900)));
        assert_eq!(signal.source, RateLimitSignalSource::RetryAfterSecondsText);

        let signal = RateLimitSignal::from_text("Prowlarr rate limited (retry after 120s)")
            .expect("Prowlarr retry after should parse");
        assert_eq!(signal.retry_after, Some(Duration::from_secs(120)));
        assert_eq!(signal.source, RateLimitSignalSource::RetryAfterText);
    }

    #[test]
    fn detects_rate_limits_without_retry_after() {
        let signal =
            RateLimitSignal::from_text("HTTP 429: too many requests").expect("429 should parse");
        assert_eq!(signal.retry_after, None);
        assert_eq!(signal.source, RateLimitSignalSource::TooManyRequestsPhrase);
    }

    #[test]
    fn does_not_match_bare_429_substrings() {
        assert!(RateLimitSignal::from_text("release title contains 429").is_none());
        assert!(RateLimitSignal::from_text("provider returned id 429001").is_none());
        assert!(RateLimitSignal::from_text("provider returned HTTP 429001").is_none());
    }
}
