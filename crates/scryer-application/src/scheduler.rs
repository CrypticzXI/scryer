use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) fn stable_jitter_offset(
    seed: &str,
    namespace: &str,
    stream: &str,
    window: Duration,
) -> Duration {
    let window_seconds = window.as_secs().max(1);
    let hash = blake3::hash(format!("scryer:scheduler:{namespace}:{stream}:{seed}").as_bytes());
    let mut prefix = [0_u8; 8];
    prefix.copy_from_slice(&hash.as_bytes()[..8]);
    Duration::from_secs(u64::from_le_bytes(prefix) % window_seconds)
}

pub(crate) fn next_jittered_cycle_delay(
    now: SystemTime,
    cadence: Duration,
    offset: Duration,
    minimum_delay: Duration,
) -> Duration {
    let cadence_seconds = cadence.as_secs().max(1);
    let now_seconds = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let offset_seconds = offset.as_secs() % cadence_seconds;

    let mut next_slot = (now_seconds / cadence_seconds) * cadence_seconds + offset_seconds;
    if next_slot <= now_seconds {
        next_slot = next_slot.saturating_add(cadence_seconds);
    }

    let earliest = now_seconds.saturating_add(minimum_delay.as_secs());
    if next_slot < earliest {
        let delta = earliest - next_slot;
        let skips = delta.div_ceil(cadence_seconds);
        next_slot = next_slot.saturating_add(skips.saturating_mul(cadence_seconds));
    }

    Duration::from_secs(next_slot.saturating_sub(now_seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_jitter_offset_is_deterministic_and_stream_specific() {
        let first = stable_jitter_offset(
            "instance-a",
            "background_library_refresh",
            "movie",
            Duration::from_secs(7200),
        );
        let second = stable_jitter_offset(
            "instance-a",
            "background_library_refresh",
            "movie",
            Duration::from_secs(7200),
        );
        let different_stream = stable_jitter_offset(
            "instance-a",
            "background_library_refresh",
            "series",
            Duration::from_secs(7200),
        );

        assert_eq!(first, second);
        assert_ne!(first, different_stream);
        assert!(first < Duration::from_secs(7200));
    }

    #[test]
    fn next_jittered_cycle_delay_advances_to_next_offset_slot() {
        let cadence = Duration::from_secs(4 * 60 * 60);
        let offset = Duration::from_secs(2 * 60 * 60);
        let before_offset = UNIX_EPOCH + Duration::from_secs(60 * 60);
        let after_offset = UNIX_EPOCH + Duration::from_secs(3 * 60 * 60);

        assert_eq!(
            next_jittered_cycle_delay(before_offset, cadence, offset, Duration::ZERO),
            Duration::from_secs(60 * 60)
        );
        assert_eq!(
            next_jittered_cycle_delay(after_offset, cadence, offset, Duration::ZERO),
            Duration::from_secs(3 * 60 * 60)
        );
    }
}
