use std::borrow::Cow;
use std::collections::HashMap;
use std::convert::Infallible;
use std::fmt;
use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use metrics::{counter, histogram};
use reqwest::header::{HeaderMap, LOCATION, RETRY_AFTER};
use reqwest::{
    Certificate, Client, RequestBuilder, Response, StatusCode, blocking::Client as BlockingClient,
};
use thiserror::Error;
use tokio::time::{Instant, sleep};
use tracing::debug;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RateLimitScopeKey(Arc<str>);

impl RateLimitScopeKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RateLimitScopeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for RateLimitScopeKey {
    fn from(value: &str) -> Self {
        Self(Arc::<str>::from(value))
    }
}

impl From<String> for RateLimitScopeKey {
    fn from(value: String) -> Self {
        Self(Arc::<str>::from(value))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryMode {
    SafeRead,
    ExplicitMutationRetry,
    NoRetry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RedirectMode {
    NoFollow,
    TrustedFollow { max_hops: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryAfterSource {
    HttpDate,
    Seconds,
    FallbackBackoff,
    ExistingCooldown,
}

impl RetryAfterSource {
    pub fn as_persistent_str(self) -> &'static str {
        match self {
            Self::HttpDate => "http_date",
            Self::Seconds => "seconds",
            Self::FallbackBackoff => "fallback_backoff",
            Self::ExistingCooldown => "existing_cooldown",
        }
    }

    pub fn from_persistent_str(value: &str) -> Option<Self> {
        match value {
            "http_date" => Some(Self::HttpDate),
            "seconds" => Some(Self::Seconds),
            "fallback_backoff" => Some(Self::FallbackBackoff),
            "existing_cooldown" => Some(Self::ExistingCooldown),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RateLimitRegistrySnapshot {
    pub host_rps: Vec<HostRpsSnapshotEntry>,
    pub destination_cooldowns: Vec<DestinationCooldownSnapshotEntry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HostRpsSnapshotEntry {
    pub host_key: HostKey,
    pub available_in: Duration,
    pub profile: HostRpsProfile,
    pub profile_source: HostRpsProfileSource,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DestinationCooldownSnapshotEntry {
    pub destination_key: DestinationKey,
    pub available_in: Duration,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PersistedDestinationCooldown {
    pub destination_key: DestinationKey,
    pub cooldown_until: DateTime<Utc>,
    pub retry_after: Option<Duration>,
    pub source: RetryAfterSource,
    pub status_code: Option<u16>,
    pub message: Option<String>,
    pub observed_at: DateTime<Utc>,
}

fn destination_cooldown_is_newer_or_equal(
    candidate: &PersistedDestinationCooldown,
    existing: &PersistedDestinationCooldown,
) -> bool {
    candidate.observed_at > existing.observed_at
        || (candidate.observed_at == existing.observed_at
            && candidate.cooldown_until >= existing.cooldown_until)
}

#[derive(Clone, Debug)]
pub struct RequestPolicy {
    pub scope: RateLimitScopeKey,
    pub request_label: Cow<'static, str>,
    pub retry_mode: RetryMode,
    pub max_retries: u32,
    pub base_backoff: Duration,
    pub max_backoff: Duration,
    pub max_retry_after: Duration,
    pub redirect_mode: RedirectMode,
}

impl RequestPolicy {
    pub fn new(
        scope: impl Into<RateLimitScopeKey>,
        request_label: impl Into<Cow<'static, str>>,
        retry_mode: RetryMode,
    ) -> Self {
        let max_retries = match retry_mode {
            RetryMode::SafeRead => 2,
            RetryMode::ExplicitMutationRetry => 1,
            RetryMode::NoRetry => 0,
        };

        Self {
            scope: scope.into(),
            request_label: request_label.into(),
            retry_mode,
            max_retries,
            base_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(30),
            max_retry_after: default_max_retry_after(),
            redirect_mode: RedirectMode::NoFollow,
        }
    }

    pub fn safe_read(
        scope: impl Into<RateLimitScopeKey>,
        request_label: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self::new(scope, request_label, RetryMode::SafeRead)
    }

    pub fn explicit_mutation_retry(
        scope: impl Into<RateLimitScopeKey>,
        request_label: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self::new(scope, request_label, RetryMode::ExplicitMutationRetry)
    }

    pub fn no_retry(
        scope: impl Into<RateLimitScopeKey>,
        request_label: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self::new(scope, request_label, RetryMode::NoRetry)
    }

    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    pub fn with_backoff(mut self, base_backoff: Duration, max_backoff: Duration) -> Self {
        self.base_backoff = base_backoff;
        self.max_backoff = max_backoff;
        self
    }

    pub fn with_max_retry_after(mut self, max_retry_after: Duration) -> Self {
        self.max_retry_after = max_retry_after;
        self
    }

    pub fn without_redirects(mut self) -> Self {
        self.redirect_mode = RedirectMode::NoFollow;
        self
    }

    pub fn with_trusted_redirects(mut self, max_hops: usize) -> Self {
        self.redirect_mode = RedirectMode::TrustedFollow { max_hops };
        self
    }

    fn retry_allowed(&self, attempt: u32) -> bool {
        !matches!(self.retry_mode, RetryMode::NoRetry) && attempt <= self.max_retries
    }

    fn backoff_for_retry(&self, retry_index: u32) -> Duration {
        bounded_exponential_backoff(self.base_backoff, self.max_backoff, retry_index)
    }
}

pub const DEFAULT_HOST_RPS: f64 = 1.0;
pub const DEFAULT_HOST_RPS_BURST: u32 = 2;
pub const LOCAL_MANAGED_HOST_RPS: f64 = 10.0;
pub const LOCAL_MANAGED_HOST_RPS_BURST: u32 = 20;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HostRpsProfile {
    pub requests_per_second: f64,
    pub burst: u32,
}

impl HostRpsProfile {
    pub const fn limited(requests_per_second: f64, burst: u32) -> Self {
        Self {
            requests_per_second,
            burst,
        }
    }

    pub const fn unthrottled() -> Self {
        Self {
            requests_per_second: f64::INFINITY,
            burst: u32::MAX,
        }
    }

    fn interval(self) -> Option<Duration> {
        (self.requests_per_second.is_finite() && self.requests_per_second > 0.0)
            .then(|| Duration::from_secs_f64(1.0 / self.requests_per_second))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostRpsProfileSource {
    UnknownPublicDefault,
    LocalOrManagedDefault,
    Loopback,
    ExplicitRegistration,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HostRpsProfileAssignment {
    pub profile: HostRpsProfile,
    pub source: HostRpsProfileSource,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HostKey(Arc<str>);

impl HostKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for HostKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for HostKey {
    fn from(value: &str) -> Self {
        Self(Arc::<str>::from(normalize_host_key(value)))
    }
}

impl From<String> for HostKey {
    fn from(value: String) -> Self {
        Self::from(value.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DestinationKey(Arc<str>);

impl DestinationKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DestinationKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for DestinationKey {
    fn from(value: &str) -> Self {
        Self(Arc::<str>::from(normalize_host_key(value)))
    }
}

impl From<String> for DestinationKey {
    fn from(value: String) -> Self {
        Self::from(value.as_str())
    }
}

#[derive(Default)]
struct RateLimitRegistryState {
    deadlines: Mutex<HashMap<RateLimitScopeKey, Instant>>,
    host_deadlines: Mutex<HashMap<HostKey, Instant>>,
    host_profiles: Mutex<HashMap<HostKey, HostRpsProfileAssignment>>,
    destination_deadlines: Mutex<HashMap<DestinationKey, Instant>>,
    destination_cooldowns: Mutex<HashMap<DestinationKey, PersistedDestinationCooldown>>,
    dirty_destination_cooldowns: Mutex<HashMap<DestinationKey, PersistedDestinationCooldown>>,
}

#[derive(Clone)]
pub struct RateLimitRegistry {
    state: Arc<RateLimitRegistryState>,
}

impl RateLimitRegistry {
    pub fn new() -> Self {
        static SHARED: LazyLock<RateLimitRegistry> = LazyLock::new(RateLimitRegistry::isolated);
        SHARED.clone()
    }

    pub fn isolated() -> Self {
        Self {
            state: Arc::new(RateLimitRegistryState::default()),
        }
    }

    pub fn snapshot(&self) -> RateLimitRegistrySnapshot {
        let now = Instant::now();
        let mut host_rps = self
            .state
            .host_deadlines
            .lock()
            .expect("host RPS lock poisoned")
            .iter()
            .map(|(host_key, deadline)| {
                let assignment = self.profile_for_host(host_key);
                HostRpsSnapshotEntry {
                    host_key: host_key.clone(),
                    available_in: deadline.saturating_duration_since(now),
                    profile: assignment.profile,
                    profile_source: assignment.source,
                }
            })
            .collect::<Vec<_>>();
        host_rps.sort_by(|left, right| left.host_key.as_str().cmp(right.host_key.as_str()));

        let mut destination_cooldowns = self
            .state
            .destination_deadlines
            .lock()
            .expect("destination deadline lock poisoned")
            .iter()
            .filter_map(|(destination_key, deadline)| {
                let available_in = deadline.saturating_duration_since(now);
                (!available_in.is_zero()).then(|| DestinationCooldownSnapshotEntry {
                    destination_key: destination_key.clone(),
                    available_in,
                })
            })
            .collect::<Vec<_>>();
        destination_cooldowns.sort_by(|left, right| {
            left.destination_key
                .as_str()
                .cmp(right.destination_key.as_str())
        });

        RateLimitRegistrySnapshot {
            host_rps,
            destination_cooldowns,
        }
    }

    pub async fn wait_if_needed(&self, scope: &RateLimitScopeKey) -> Option<Duration> {
        let mut total_wait = Duration::ZERO;

        loop {
            let wait_duration = {
                let mut deadlines = self
                    .state
                    .deadlines
                    .lock()
                    .expect("rate limit deadline lock poisoned");
                let Some(deadline) = deadlines.get(scope).copied() else {
                    break;
                };
                let now = Instant::now();
                let remaining = deadline.saturating_duration_since(now);
                if remaining.is_zero() {
                    deadlines.remove(scope);
                    break;
                } else {
                    remaining
                }
            };

            total_wait += wait_duration;
            sleep(wait_duration).await;
        }

        (!total_wait.is_zero()).then_some(total_wait)
    }

    pub async fn wait_for_destination_if_needed(
        &self,
        destination: &DestinationKey,
    ) -> Option<Duration> {
        let mut total_wait = Duration::ZERO;

        loop {
            let wait_duration = {
                let mut deadlines = self
                    .state
                    .destination_deadlines
                    .lock()
                    .expect("destination deadline lock poisoned");
                let Some(deadline) = deadlines.get(destination).copied() else {
                    break;
                };
                let now = Instant::now();
                let remaining = deadline.saturating_duration_since(now);
                if remaining.is_zero() {
                    deadlines.remove(destination);
                    break;
                } else {
                    remaining
                }
            };

            total_wait += wait_duration;
            sleep(wait_duration).await;
        }

        (!total_wait.is_zero()).then_some(total_wait)
    }

    pub fn active_destination_cooldown(&self, destination: &DestinationKey) -> Option<Duration> {
        let mut deadlines = self
            .state
            .destination_deadlines
            .lock()
            .expect("destination deadline lock poisoned");
        let deadline = deadlines.get(destination).copied()?;
        let now = Instant::now();
        let remaining = deadline.saturating_duration_since(now);
        if remaining.is_zero() {
            deadlines.remove(destination);
            None
        } else {
            Some(remaining)
        }
    }

    pub fn hydrate_destination_cooldowns<I>(&self, cooldowns: I)
    where
        I: IntoIterator<Item = PersistedDestinationCooldown>,
    {
        let now_wall = Utc::now();
        let now_instant = Instant::now();
        let mut deadlines = self
            .state
            .destination_deadlines
            .lock()
            .expect("destination deadline lock poisoned");
        let mut metadata = self
            .state
            .destination_cooldowns
            .lock()
            .expect("destination cooldown metadata lock poisoned");

        for cooldown in cooldowns {
            let Ok(delay) = (cooldown.cooldown_until - now_wall).to_std() else {
                continue;
            };
            if delay.is_zero() {
                continue;
            }
            deadlines.insert(cooldown.destination_key.clone(), now_instant + delay);
            metadata.insert(cooldown.destination_key.clone(), cooldown);
        }
    }

    pub fn drain_dirty_destination_cooldowns(&self) -> Vec<PersistedDestinationCooldown> {
        self.state
            .dirty_destination_cooldowns
            .lock()
            .expect("dirty destination cooldown lock poisoned")
            .drain()
            .map(|(_, cooldown)| cooldown)
            .collect()
    }

    pub fn requeue_dirty_destination_cooldowns<I>(&self, cooldowns: I)
    where
        I: IntoIterator<Item = PersistedDestinationCooldown>,
    {
        let mut dirty = self
            .state
            .dirty_destination_cooldowns
            .lock()
            .expect("dirty destination cooldown lock poisoned");
        for cooldown in cooldowns {
            match dirty.get(&cooldown.destination_key) {
                Some(existing) if !destination_cooldown_is_newer_or_equal(&cooldown, existing) => {}
                _ => {
                    dirty.insert(cooldown.destination_key.clone(), cooldown);
                }
            }
        }
    }

    pub fn wait_for_destination_if_needed_blocking(
        &self,
        destination: &DestinationKey,
    ) -> Option<Duration> {
        let mut total_wait = Duration::ZERO;

        loop {
            let wait_duration = {
                let mut deadlines = self
                    .state
                    .destination_deadlines
                    .lock()
                    .expect("destination deadline lock poisoned");
                let Some(deadline) = deadlines.get(destination).copied() else {
                    break;
                };
                let now = Instant::now();
                let remaining = deadline.saturating_duration_since(now);
                if remaining.is_zero() {
                    deadlines.remove(destination);
                    break;
                } else {
                    remaining
                }
            };

            total_wait += wait_duration;
            std::thread::sleep(wait_duration);
        }

        (!total_wait.is_zero()).then_some(total_wait)
    }

    pub async fn acquire_host_rps(&self, host: &HostKey) -> Option<Duration> {
        let wait_duration = self.reserve_host_rps(host);

        if wait_duration.is_zero() {
            None
        } else {
            sleep(wait_duration).await;
            Some(wait_duration)
        }
    }

    pub fn acquire_host_rps_blocking(&self, host: &HostKey) -> Option<Duration> {
        let wait_duration = self.reserve_host_rps(host);

        if wait_duration.is_zero() {
            None
        } else {
            std::thread::sleep(wait_duration);
            Some(wait_duration)
        }
    }

    pub fn register_host_profile(
        &self,
        host: HostKey,
        profile: HostRpsProfile,
        source: HostRpsProfileSource,
    ) {
        self.state
            .host_profiles
            .lock()
            .expect("host profile lock poisoned")
            .insert(host, HostRpsProfileAssignment { profile, source });
    }

    pub fn profile_for_host(&self, host: &HostKey) -> HostRpsProfileAssignment {
        if let Some(assignment) = self
            .state
            .host_profiles
            .lock()
            .expect("host profile lock poisoned")
            .get(host)
            .copied()
        {
            return assignment;
        }

        classify_host_rps_profile(host.as_str())
    }

    pub fn preview_host_rps_wait(&self, host: &HostKey) -> Option<Duration> {
        let assignment = self.profile_for_host(host);
        let interval = assignment.profile.interval()?;
        let host_deadlines = self
            .state
            .host_deadlines
            .lock()
            .expect("host RPS lock poisoned");
        let (wait_duration, _) = next_host_rps_reservation(
            host_deadlines.get(host).copied(),
            Instant::now(),
            interval,
            assignment.profile.burst,
        );
        (!wait_duration.is_zero()).then_some(wait_duration)
    }

    pub async fn record_cooldown(
        &self,
        scope: &RateLimitScopeKey,
        delay: Duration,
        source: RetryAfterSource,
    ) -> (Duration, RetryAfterSource) {
        if delay.is_zero() {
            return (Duration::ZERO, source);
        }

        let now = Instant::now();
        let new_deadline = now + delay;
        let mut deadlines = self
            .state
            .deadlines
            .lock()
            .expect("rate limit deadline lock poisoned");

        let existing_deadline = deadlines
            .get(scope)
            .copied()
            .filter(|deadline| *deadline > now);

        let effective_deadline = match existing_deadline {
            Some(existing) if existing > new_deadline => existing,
            _ => new_deadline,
        };

        deadlines.insert(scope.clone(), effective_deadline);

        let effective_delay = effective_deadline.saturating_duration_since(now);
        let effective_source = match existing_deadline {
            Some(existing) if existing > new_deadline => RetryAfterSource::ExistingCooldown,
            _ => source,
        };

        (effective_delay, effective_source)
    }

    pub async fn record_destination_cooldown(
        &self,
        destination: &DestinationKey,
        delay: Duration,
        source: RetryAfterSource,
    ) -> (Duration, RetryAfterSource) {
        self.record_destination_cooldown_inner(destination, delay, source)
    }

    pub fn record_destination_cooldown_blocking(
        &self,
        destination: &DestinationKey,
        delay: Duration,
        source: RetryAfterSource,
    ) -> (Duration, RetryAfterSource) {
        self.record_destination_cooldown_inner(destination, delay, source)
    }

    fn record_destination_cooldown_inner(
        &self,
        destination: &DestinationKey,
        delay: Duration,
        source: RetryAfterSource,
    ) -> (Duration, RetryAfterSource) {
        if delay.is_zero() {
            return (Duration::ZERO, source);
        }

        let now_instant = Instant::now();
        let observed_at = Utc::now();
        let new_deadline = now_instant + delay;
        let mut deadlines = self
            .state
            .destination_deadlines
            .lock()
            .expect("destination deadline lock poisoned");

        let existing_deadline = deadlines
            .get(destination)
            .copied()
            .filter(|deadline| *deadline > now_instant);

        let effective_deadline = match existing_deadline {
            Some(existing) if existing > new_deadline => existing,
            _ => new_deadline,
        };

        deadlines.insert(destination.clone(), effective_deadline);

        let effective_delay = effective_deadline.saturating_duration_since(now_instant);
        let effective_source = match existing_deadline {
            Some(existing) if existing > new_deadline => RetryAfterSource::ExistingCooldown,
            _ => source,
        };

        if effective_source != RetryAfterSource::ExistingCooldown
            && let Ok(effective_chrono_delay) = chrono::Duration::from_std(effective_delay)
        {
            let cooldown = PersistedDestinationCooldown {
                destination_key: destination.clone(),
                cooldown_until: observed_at + effective_chrono_delay,
                retry_after: Some(delay),
                source,
                status_code: None,
                message: None,
                observed_at,
            };
            self.state
                .destination_cooldowns
                .lock()
                .expect("destination cooldown metadata lock poisoned")
                .insert(destination.clone(), cooldown.clone());
            self.state
                .dirty_destination_cooldowns
                .lock()
                .expect("dirty destination cooldown lock poisoned")
                .insert(destination.clone(), cooldown);
        }

        (effective_delay, effective_source)
    }

    fn reserve_host_rps(&self, host: &HostKey) -> Duration {
        let assignment = self.profile_for_host(host);
        let Some(interval) = assignment.profile.interval() else {
            return Duration::ZERO;
        };

        let mut host_deadlines = self
            .state
            .host_deadlines
            .lock()
            .expect("host RPS lock poisoned");
        let now = Instant::now();
        let (wait_duration, next_deadline) = next_host_rps_reservation(
            host_deadlines.get(host).copied(),
            now,
            interval,
            assignment.profile.burst,
        );
        host_deadlines.insert(host.clone(), next_deadline);
        wait_duration
    }
}

fn next_host_rps_reservation(
    current_deadline: Option<Instant>,
    now: Instant,
    interval: Duration,
    burst: u32,
) -> (Duration, Instant) {
    let burst_credit = interval.saturating_mul(burst);
    let earliest_base = now.checked_sub(burst_credit).unwrap_or(now);
    let base = current_deadline
        .filter(|deadline| *deadline > earliest_base)
        .unwrap_or(earliest_base);
    (base.saturating_duration_since(now), base + interval)
}

impl Default for RateLimitRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub const DEFAULT_USER_AGENT: &str = concat!("Scryer/", env!("CARGO_PKG_VERSION"));
pub const STANDARD_HTTP_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Error)]
pub enum OutboundDestinationError {
    #[error("{label} URL is invalid: {message}")]
    InvalidUrl {
        label: &'static str,
        message: String,
    },
    #[error("{label} URL must use http or https")]
    UnsupportedScheme { label: &'static str },
    #[error("{label} URL must include a host")]
    MissingHost { label: &'static str },
    #[error("{label} URL must not include embedded credentials")]
    EmbeddedCredentials { label: &'static str },
    #[error("{label} host failed to resolve: {host}: {source}")]
    ResolveFailed {
        label: &'static str,
        host: String,
        source: std::io::Error,
    },
    #[error("{label} host did not resolve: {host}")]
    NoResolvedAddresses { label: &'static str, host: String },
    #[error("{label} host resolves to a private or local address: {host}")]
    ForbiddenAddress { label: &'static str, host: String },
    #[error("failed to build pinned {label} client for {host}: {source}")]
    ClientBuild {
        label: &'static str,
        host: String,
        source: reqwest::Error,
    },
}

pub fn validate_operator_http_url(
    raw: &str,
    label: &'static str,
) -> Result<reqwest::Url, OutboundDestinationError> {
    let url = reqwest::Url::parse(raw).map_err(|source| OutboundDestinationError::InvalidUrl {
        label,
        message: source.to_string(),
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(OutboundDestinationError::UnsupportedScheme { label });
    }
    if url.host_str().is_none() {
        return Err(OutboundDestinationError::MissingHost { label });
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(OutboundDestinationError::EmbeddedCredentials { label });
    }
    Ok(url)
}

pub fn validate_public_http_url(
    raw: &str,
    label: &'static str,
) -> Result<reqwest::Url, OutboundDestinationError> {
    validate_operator_http_url(raw, label)
}

pub fn validate_untrusted_public_http_url(
    raw: &str,
    label: &'static str,
) -> Result<reqwest::Url, OutboundDestinationError> {
    validate_operator_http_url(raw, label)
}

#[derive(Clone)]
pub struct PinnedPublicHttpTarget {
    url: reqwest::Url,
    host: String,
    resolved_addrs: Vec<SocketAddr>,
    client: Client,
}

impl PinnedPublicHttpTarget {
    pub fn url(&self) -> &reqwest::Url {
        &self.url
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn resolved_addrs(&self) -> &[SocketAddr] {
        &self.resolved_addrs
    }

    pub fn client(&self) -> &Client {
        &self.client
    }
}

pub async fn prepare_untrusted_public_http_target(
    raw: &str,
    label: &'static str,
) -> Result<PinnedPublicHttpTarget, OutboundDestinationError> {
    let url = validate_untrusted_public_http_url(raw, label)?;
    prepare_untrusted_public_http_target_from_url(url, label).await
}

pub async fn prepare_untrusted_public_http_target_from_url(
    url: reqwest::Url,
    label: &'static str,
) -> Result<PinnedPublicHttpTarget, OutboundDestinationError> {
    let resolved_addrs = resolve_public_http_destination(&url, label).await?;
    let host = url
        .host_str()
        .ok_or(OutboundDestinationError::MissingHost { label })?
        .to_string();
    let client = reqwest_client_builder()
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(&host, &resolved_addrs)
        .build()
        .map_err(|source| OutboundDestinationError::ClientBuild {
            label,
            host: host.clone(),
            source,
        })?;

    Ok(PinnedPublicHttpTarget {
        url,
        host,
        resolved_addrs,
        client,
    })
}

pub async fn validate_public_http_destination(
    url: &reqwest::Url,
    label: &'static str,
) -> Result<(), OutboundDestinationError> {
    resolve_public_http_destination(url, label)
        .await
        .map(|_| ())
}

async fn resolve_public_http_destination(
    url: &reqwest::Url,
    label: &'static str,
) -> Result<Vec<SocketAddr>, OutboundDestinationError> {
    let host = url
        .host_str()
        .ok_or(OutboundDestinationError::MissingHost { label })?;
    let port = url
        .port_or_known_default()
        .ok_or(OutboundDestinationError::MissingHost { label })?;
    if let Some(ip) = parse_host_ip_literal(host) {
        validate_public_ip(ip, host, label)?;
        return Ok(vec![SocketAddr::new(ip, port)]);
    }

    let mut resolved = tokio::net::lookup_host((host, port))
        .await
        .map_err(|source| OutboundDestinationError::ResolveFailed {
            label,
            host: host.to_string(),
            source,
        })?;
    let mut resolved_addrs = Vec::new();
    for addr in &mut resolved {
        resolved_addrs.push(addr);
        validate_public_ip(addr.ip(), host, label)?;
    }
    if resolved_addrs.is_empty() {
        return Err(OutboundDestinationError::NoResolvedAddresses {
            label,
            host: host.to_string(),
        });
    }
    Ok(resolved_addrs)
}

fn validate_public_ip(
    ip: IpAddr,
    host: &str,
    label: &'static str,
) -> Result<(), OutboundDestinationError> {
    if public_http_ip_is_forbidden(ip) {
        return Err(OutboundDestinationError::ForbiddenAddress {
            label,
            host: host.to_string(),
        });
    }
    Ok(())
}

fn parse_host_ip_literal(host: &str) -> Option<IpAddr> {
    host.parse::<IpAddr>().ok().or_else(|| {
        host.strip_prefix('[')?
            .strip_suffix(']')?
            .parse::<IpAddr>()
            .ok()
    })
}

fn public_http_ip_is_forbidden(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_multicast()
                || ip.is_unspecified()
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || ip.is_multicast()
        }
    }
}

fn reqwest_client_builder() -> reqwest::ClientBuilder {
    install_default_rustls_provider();
    Client::builder()
        .min_tls_version(reqwest::tls::Version::TLS_1_2)
        .timeout(STANDARD_HTTP_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(DEFAULT_USER_AGENT)
        .gzip(true)
        .brotli(true)
        .deflate(true)
        .zstd(true)
}

fn blocking_reqwest_client_builder() -> reqwest::blocking::ClientBuilder {
    install_default_rustls_provider();
    BlockingClient::builder()
        .min_tls_version(reqwest::tls::Version::TLS_1_2)
        .timeout(STANDARD_HTTP_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(DEFAULT_USER_AGENT)
        .gzip(true)
        .brotli(true)
        .deflate(true)
        .zstd(true)
}

pub fn install_default_rustls_provider() {
    static INSTALL_RUSTLS_PROVIDER: OnceLock<()> = OnceLock::new();

    INSTALL_RUSTLS_PROVIDER.get_or_init(|| {
        // The provider is process-global; parallel workspace tests may install it first.
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

pub fn generic_reqwest_client() -> Client {
    static CLIENT: LazyLock<Client> = LazyLock::new(|| {
        reqwest_client_builder()
            .build()
            .expect("generic reqwest client should build")
    });
    CLIENT.clone()
}

pub fn external_arr_reqwest_client() -> Client {
    let mut builder = reqwest_client_builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none());
    if let Ok(proxy_url) = std::env::var("SCRYER_EXTERNAL_ARR_PROXY_URL")
        && !proxy_url.trim().is_empty()
        && let Ok(proxy) = reqwest::Proxy::all(proxy_url.trim())
    {
        builder = builder.proxy(proxy);
    }
    builder
        .build()
        .unwrap_or_else(|_| no_redirect_reqwest_client())
}

pub fn plugin_reqwest_client() -> Client {
    static CLIENT: LazyLock<Client> = LazyLock::new(|| {
        reqwest_client_builder()
            .build()
            .expect("plugin reqwest client should build")
    });
    CLIENT.clone()
}

pub fn no_redirect_reqwest_client() -> Client {
    static CLIENT: LazyLock<Client> = LazyLock::new(|| {
        reqwest_client_builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("no-redirect reqwest client should build")
    });
    CLIENT.clone()
}

pub fn smg_reqwest_client() -> Client {
    static CLIENT: LazyLock<Client> = LazyLock::new(|| {
        reqwest_client_builder()
            .build()
            .expect("SMG reqwest client should build")
    });
    CLIENT.clone()
}

pub fn blocking_plugin_host_client(extra_ca_bundle_pem: &str) -> Result<BlockingClient, String> {
    let mut builder = blocking_reqwest_client_builder().redirect(reqwest::redirect::Policy::none());
    if !extra_ca_bundle_pem.trim().is_empty() {
        builder = builder.tls_certs_merge(uploaded_root_certificates(extra_ca_bundle_pem)?);
    }
    builder
        .build()
        .map_err(|error| format!("failed to build plugin HTTP client: {error}"))
}

pub fn blocking_reqwest_client() -> Result<BlockingClient, reqwest::Error> {
    blocking_reqwest_client_builder().build()
}

pub async fn send_reqwest_request(request: RequestBuilder) -> Result<Response, reqwest::Error> {
    let registry = RateLimitRegistry::new();
    let destination = request
        .try_clone()
        .and_then(|clone| clone.build().ok())
        .and_then(|request| destination_key_from_url(request.url()));
    let host = request
        .try_clone()
        .and_then(|clone| clone.build().ok())
        .and_then(|request| host_key_from_url(request.url()));

    if let Some(destination) = destination.as_ref() {
        let _ = registry.wait_for_destination_if_needed(destination).await;
    }
    if let Some(host) = host.as_ref() {
        let _ = registry.acquire_host_rps(host).await;
    }

    let response = request.send().await?;
    if response.status() == StatusCode::TOO_MANY_REQUESTS
        && let Some(destination) = destination_key_from_url(response.url()).or(destination)
    {
        let (delay, source) = retry_after_delay(response.headers(), Duration::from_secs(1));
        let _ = registry
            .record_destination_cooldown(&destination, delay, source)
            .await;
    }
    Ok(response)
}

pub fn send_blocking_reqwest_request(
    request: reqwest::blocking::RequestBuilder,
) -> Result<reqwest::blocking::Response, reqwest::Error> {
    let registry = RateLimitRegistry::new();
    let destination = request
        .try_clone()
        .and_then(|clone| clone.build().ok())
        .and_then(|request| destination_key_from_url(request.url()));
    let host = request
        .try_clone()
        .and_then(|clone| clone.build().ok())
        .and_then(|request| host_key_from_url(request.url()));

    if let Some(destination) = destination.as_ref() {
        let _ = registry.wait_for_destination_if_needed_blocking(destination);
    }
    if let Some(host) = host.as_ref() {
        let _ = registry.acquire_host_rps_blocking(host);
    }

    let response = request.send()?;
    if response.status() == StatusCode::TOO_MANY_REQUESTS
        && let Some(destination) = destination_key_from_url(response.url()).or(destination)
    {
        let (delay, source) = retry_after_delay(response.headers(), Duration::from_secs(1));
        let _ = registry.record_destination_cooldown_blocking(&destination, delay, source);
    }
    Ok(response)
}

fn uploaded_root_certificates(bundle_pem: &str) -> Result<Vec<Certificate>, String> {
    if bundle_pem.trim().is_empty() {
        return Ok(Vec::new());
    }

    let certificates = Certificate::from_pem_bundle(bundle_pem.as_bytes())
        .map_err(|error| format!("failed to parse uploaded trusted certificate bundle: {error}"))?;
    if certificates.is_empty() {
        return Err(
            "uploaded trusted certificate bundle did not contain any X.509 certificates"
                .to_string(),
        );
    }
    Ok(certificates)
}

#[derive(Clone)]
pub struct OutboundHttpClient {
    client: Client,
    registry: RateLimitRegistry,
}

impl OutboundHttpClient {
    pub fn new(client: Client, registry: RateLimitRegistry) -> Self {
        Self { client, registry }
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn registry(&self) -> &RateLimitRegistry {
        &self.registry
    }

    async fn send_builder_with_trusted_redirects(
        &self,
        builder: RequestBuilder,
        request_label: &str,
        max_hops: usize,
    ) -> Result<Response, reqwest::Error> {
        let mut response = builder.send().await?;
        for _ in 0..max_hops {
            let Some(next_url) = redirect_target_url(&response) else {
                return Ok(response);
            };
            if let Some(destination) = destination_key_from_url(&next_url) {
                let _ = self
                    .registry
                    .wait_for_destination_if_needed(&destination)
                    .await;
            }
            if let Some(host) = host_key_from_url(&next_url)
                && let Some(wait_duration) = self.registry.acquire_host_rps(&host).await
            {
                debug!(
                    host = %host,
                    request_label,
                    wait_ms = wait_duration.as_millis(),
                    "outbound HTTP redirect host RPS wait"
                );
            }
            response = self.client.get(next_url).send().await?;
        }
        Ok(response)
    }

    pub async fn send<F>(
        &self,
        policy: RequestPolicy,
        build_request: F,
    ) -> Result<Response, OutboundHttpError>
    where
        F: Fn() -> RequestBuilder,
    {
        match self
            .send_async(policy, || async {
                Ok::<RequestBuilder, Infallible>(build_request())
            })
            .await
        {
            Ok(response) => Ok(response),
            Err(OutboundRequestError::Build(_)) => unreachable!("infallible builder"),
            Err(OutboundRequestError::Http(error)) => Err(error),
        }
    }

    pub async fn send_async<F, Fut, E>(
        &self,
        policy: RequestPolicy,
        build_request: F,
    ) -> Result<Response, OutboundRequestError<E>>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = Result<RequestBuilder, E>>,
    {
        let mut attempt = 0u32;

        loop {
            if let Some(wait_duration) = self.registry.wait_if_needed(&policy.scope).await {
                counter!(
                    "scryer_outbound_http_cooldown_wait_total",
                    "scope" => policy.scope.to_string(),
                    "request_label" => policy.request_label.to_string()
                )
                .increment(1);
                histogram!(
                    "scryer_outbound_http_cooldown_wait_seconds",
                    "scope" => policy.scope.to_string(),
                    "request_label" => policy.request_label.to_string()
                )
                .record(wait_duration.as_secs_f64());
                debug!(
                    scope = %policy.scope,
                    request_label = policy.request_label.as_ref(),
                    wait_ms = wait_duration.as_millis(),
                    "outbound HTTP cooldown wait"
                );
            }

            attempt += 1;
            let builder = build_request().await.map_err(OutboundRequestError::Build)?;
            let request_destination = builder
                .try_clone()
                .and_then(|clone| clone.build().ok())
                .and_then(|request| destination_key_from_url(request.url()));

            if let Some(destination) = request_destination.as_ref()
                && let Some(wait_duration) = self
                    .registry
                    .wait_for_destination_if_needed(destination)
                    .await
            {
                counter!(
                    "scryer_outbound_http_destination_cooldown_wait_total",
                    "destination" => destination.to_string(),
                    "request_label" => policy.request_label.to_string()
                )
                .increment(1);
                histogram!(
                    "scryer_outbound_http_destination_cooldown_wait_seconds",
                    "destination" => destination.to_string(),
                    "request_label" => policy.request_label.to_string()
                )
                .record(wait_duration.as_secs_f64());
                debug!(
                    destination = %destination,
                    request_label = policy.request_label.as_ref(),
                    wait_ms = wait_duration.as_millis(),
                    "outbound HTTP destination cooldown wait"
                );
            }

            if let Some(host) = builder
                .try_clone()
                .and_then(|clone| clone.build().ok())
                .and_then(|request| host_key_from_url(request.url()))
                && let Some(wait_duration) = self.registry.acquire_host_rps(&host).await
            {
                counter!(
                    "scryer_outbound_http_host_rps_wait_total",
                    "host" => host.to_string(),
                    "request_label" => policy.request_label.to_string()
                )
                .increment(1);
                histogram!(
                    "scryer_outbound_http_host_rps_wait_seconds",
                    "host" => host.to_string(),
                    "request_label" => policy.request_label.to_string()
                )
                .record(wait_duration.as_secs_f64());
                debug!(
                    host = %host,
                    request_label = policy.request_label.as_ref(),
                    wait_ms = wait_duration.as_millis(),
                    "outbound HTTP host RPS wait"
                );
            }

            let send_result = match policy.redirect_mode {
                RedirectMode::NoFollow => builder.send().await,
                RedirectMode::TrustedFollow { max_hops } => {
                    self.send_builder_with_trusted_redirects(
                        builder,
                        policy.request_label.as_ref(),
                        max_hops,
                    )
                    .await
                }
            };

            match send_result {
                Ok(response) if response.status() != StatusCode::TOO_MANY_REQUESTS => {
                    return Ok(response);
                }
                Ok(response) => {
                    let retry_index = attempt.saturating_sub(1);
                    let fallback_backoff = policy.backoff_for_retry(retry_index);
                    let (candidate_delay, candidate_source) =
                        retry_after_delay(response.headers(), fallback_backoff);
                    let candidate_delay = candidate_delay.min(policy.max_retry_after);
                    let (effective_delay, effective_source) = self
                        .registry
                        .record_cooldown(&policy.scope, candidate_delay, candidate_source)
                        .await;
                    let response_destination =
                        destination_key_from_url(response.url()).or(request_destination);
                    if let Some(destination) = response_destination.as_ref() {
                        let _ = self
                            .registry
                            .record_destination_cooldown(
                                destination,
                                candidate_delay,
                                candidate_source,
                            )
                            .await;
                    }

                    counter!(
                        "scryer_outbound_http_429_total",
                        "scope" => policy.scope.to_string(),
                        "request_label" => policy.request_label.to_string(),
                        "source" => retry_after_source_label(effective_source).to_string()
                    )
                    .increment(1);

                    debug!(
                        scope = %policy.scope,
                        request_label = policy.request_label.as_ref(),
                        attempt,
                        retry_after_source = retry_after_source_label(effective_source),
                        retry_after_ms = effective_delay.as_millis(),
                        "outbound HTTP received 429"
                    );

                    if policy.retry_allowed(attempt) {
                        continue;
                    }

                    counter!(
                        "scryer_outbound_http_rate_limited_total",
                        "scope" => policy.scope.to_string(),
                        "request_label" => policy.request_label.to_string(),
                        "source" => retry_after_source_label(effective_source).to_string()
                    )
                    .increment(1);

                    return Err(OutboundRequestError::Http(OutboundHttpError::RateLimited(
                        RateLimitedError {
                            scope: policy.scope.clone(),
                            retry_after: Some(effective_delay),
                            attempts: attempt,
                            retry_after_source: effective_source,
                            request_label: policy.request_label.clone(),
                        },
                    )));
                }
                Err(source) => {
                    if is_retryable_transport_error(&source) && policy.retry_allowed(attempt) {
                        let backoff = policy.backoff_for_retry(attempt.saturating_sub(1));
                        counter!(
                            "scryer_outbound_http_transport_retry_total",
                            "scope" => policy.scope.to_string(),
                            "request_label" => policy.request_label.to_string()
                        )
                        .increment(1);
                        histogram!(
                            "scryer_outbound_http_transport_backoff_seconds",
                            "scope" => policy.scope.to_string(),
                            "request_label" => policy.request_label.to_string()
                        )
                        .record(backoff.as_secs_f64());
                        debug!(
                            scope = %policy.scope,
                            request_label = policy.request_label.as_ref(),
                            attempt,
                            backoff_ms = backoff.as_millis(),
                            error = %source,
                            "outbound HTTP transport retry"
                        );
                        sleep(backoff).await;
                        continue;
                    }

                    return Err(OutboundRequestError::Http(OutboundHttpError::Transport {
                        scope: policy.scope.clone(),
                        request_label: policy.request_label.clone(),
                        attempts: attempt,
                        source,
                    }));
                }
            }
        }
    }
}

#[derive(Debug, Error)]
#[error("request '{request_label}' was rate limited for scope '{scope}' after {attempts} attempts")]
pub struct RateLimitedError {
    pub scope: RateLimitScopeKey,
    pub retry_after: Option<Duration>,
    pub attempts: u32,
    pub retry_after_source: RetryAfterSource,
    pub request_label: Cow<'static, str>,
}

#[derive(Debug, Error)]
pub enum OutboundHttpError {
    #[error(transparent)]
    RateLimited(#[from] RateLimitedError),
    #[error(
        "request '{request_label}' transport failed for scope '{scope}' after {attempts} attempts: {source}"
    )]
    Transport {
        scope: RateLimitScopeKey,
        request_label: Cow<'static, str>,
        attempts: u32,
        #[source]
        source: reqwest::Error,
    },
}

#[derive(Debug)]
pub enum OutboundRequestError<E> {
    Build(E),
    Http(OutboundHttpError),
}

fn retry_after_delay(
    headers: &HeaderMap,
    fallback_delay: Duration,
) -> (Duration, RetryAfterSource) {
    let Some(raw_header) = headers.get(RETRY_AFTER) else {
        return (fallback_delay, RetryAfterSource::FallbackBackoff);
    };
    let Ok(raw_value) = raw_header.to_str() else {
        return (fallback_delay, RetryAfterSource::FallbackBackoff);
    };
    parse_retry_after(raw_value).unwrap_or((fallback_delay, RetryAfterSource::FallbackBackoff))
}

fn default_max_retry_after() -> Duration {
    const DEFAULT_MAX_RETRY_AFTER_SECS: u64 = 5 * 60;
    std::env::var("SCRYER_OUTBOUND_RETRY_AFTER_MAX_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(DEFAULT_MAX_RETRY_AFTER_SECS))
}

pub fn parse_retry_after(raw_value: &str) -> Option<(Duration, RetryAfterSource)> {
    let trimmed = raw_value.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(retry_at) = DateTime::parse_from_rfc2822(trimmed) {
        let retry_at = retry_at.with_timezone(&Utc);
        let now = Utc::now();
        if retry_at > now
            && let Ok(delay) = (retry_at - now).to_std()
            && !delay.is_zero()
        {
            return Some((delay, RetryAfterSource::HttpDate));
        }
    }

    if let Ok(seconds) = trimmed.parse::<u64>()
        && seconds > 0
    {
        return Some((Duration::from_secs(seconds), RetryAfterSource::Seconds));
    }

    None
}

pub fn host_key_from_url(url: &reqwest::Url) -> Option<HostKey> {
    url.host_str().map(HostKey::from)
}

pub fn destination_key_from_url(url: &reqwest::Url) -> Option<DestinationKey> {
    url.host_str().map(DestinationKey::from)
}

fn redirect_target_url(response: &Response) -> Option<reqwest::Url> {
    if !response.status().is_redirection() {
        return None;
    }
    let location = response.headers().get(LOCATION)?.to_str().ok()?;
    response.url().join(location).ok()
}

fn normalize_host_key(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

fn host_is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "::1")
        || host
            .parse::<IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
}

fn classify_host_rps_profile(host: &str) -> HostRpsProfileAssignment {
    if host_is_loopback(host) {
        return HostRpsProfileAssignment {
            profile: HostRpsProfile::unthrottled(),
            source: HostRpsProfileSource::Loopback,
        };
    }

    if host_is_local_or_managed(host) {
        return HostRpsProfileAssignment {
            profile: HostRpsProfile::limited(LOCAL_MANAGED_HOST_RPS, LOCAL_MANAGED_HOST_RPS_BURST),
            source: HostRpsProfileSource::LocalOrManagedDefault,
        };
    }

    HostRpsProfileAssignment {
        profile: default_public_host_rps_profile(),
        source: HostRpsProfileSource::UnknownPublicDefault,
    }
}

fn host_is_local_or_managed(host: &str) -> bool {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(ip) => ip.is_private() || ip.is_link_local(),
            IpAddr::V6(ip) => ip.is_unique_local() || ip.is_unicast_link_local(),
        };
    }

    !host.contains('.')
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".home.arpa")
}

fn default_public_host_rps_profile() -> HostRpsProfile {
    let rps = std::env::var("SCRYER_OUTBOUND_HOST_RPS")
        .ok()
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(DEFAULT_HOST_RPS);
    HostRpsProfile::limited(rps, DEFAULT_HOST_RPS_BURST)
}

fn bounded_exponential_backoff(base: Duration, max: Duration, retry_index: u32) -> Duration {
    if base.is_zero() || max.is_zero() {
        return Duration::ZERO;
    }

    let shift = retry_index.min(31);
    let factor = 1u128 << shift;
    let base_millis = base.as_millis();
    let max_millis = max.as_millis();
    let scaled = base_millis.saturating_mul(factor).min(max_millis);
    Duration::from_millis(scaled.min(u64::MAX as u128) as u64)
}

fn is_retryable_transport_error(error: &reqwest::Error) -> bool {
    error.is_timeout()
        || error.is_connect()
        || retryable_transport_error_text(&transport_error_chain_text(error))
}

fn transport_error_chain_text(error: &reqwest::Error) -> String {
    let mut messages = vec![error.to_string()];
    let mut source = std::error::Error::source(error);
    while let Some(error) = source {
        messages.push(error.to_string());
        source = std::error::Error::source(error);
    }
    messages.join(": ")
}

fn retryable_transport_error_text(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    if normalized.contains("certificate") || normalized.contains("invalid url") {
        return false;
    }

    [
        "peer closed connection without sending tls close_notify",
        "connection closed before message completed",
        "connection reset",
        "unexpected eof",
        "end of file",
        "broken pipe",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn retry_after_source_label(source: RetryAfterSource) -> &'static str {
    match source {
        RetryAfterSource::HttpDate => "http_date",
        RetryAfterSource::Seconds => "seconds",
        RetryAfterSource::FallbackBackoff => "fallback_backoff",
        RetryAfterSource::ExistingCooldown => "existing_cooldown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX_MANUAL_REDIRECTS: usize = 10;
    use std::io;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::SystemTime;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn parses_http_date_retry_after_first() {
        let retry_at = DateTime::<Utc>::from(SystemTime::now() + Duration::from_secs(60));
        let header = retry_at.to_rfc2822();
        let (delay, source) = parse_retry_after(&header).expect("expected parsed Retry-After");

        assert_eq!(source, RetryAfterSource::HttpDate);
        assert!(delay.as_secs() >= 59);
    }

    #[test]
    fn falls_back_to_seconds_when_date_parse_fails() {
        let (delay, source) = parse_retry_after("120").expect("expected parsed Retry-After");

        assert_eq!(source, RetryAfterSource::Seconds);
        assert_eq!(delay, Duration::from_secs(120));
    }

    #[test]
    fn falls_back_to_bounded_backoff_when_header_is_invalid() {
        let mut headers = HeaderMap::new();
        headers.insert(
            RETRY_AFTER,
            reqwest::header::HeaderValue::from_static("not-a-date"),
        );
        let (delay, source) = retry_after_delay(&headers, Duration::from_secs(7));

        assert_eq!(source, RetryAfterSource::FallbackBackoff);
        assert_eq!(delay, Duration::from_secs(7));
    }

    #[test]
    fn past_or_zero_retry_after_uses_fallback_backoff() {
        let past = DateTime::<Utc>::from(SystemTime::now() - Duration::from_secs(5)).to_rfc2822();
        let mut past_headers = HeaderMap::new();
        past_headers.insert(
            RETRY_AFTER,
            reqwest::header::HeaderValue::from_str(&past).expect("valid Retry-After header"),
        );
        let mut zero_headers = HeaderMap::new();
        zero_headers.insert(RETRY_AFTER, reqwest::header::HeaderValue::from_static("0"));
        let (past_delay, past_source) = retry_after_delay(&past_headers, Duration::from_secs(9));
        let (zero_delay, zero_source) = retry_after_delay(&zero_headers, Duration::from_secs(9));

        assert_eq!(past_source, RetryAfterSource::FallbackBackoff);
        assert_eq!(past_delay, Duration::from_secs(9));
        assert_eq!(zero_source, RetryAfterSource::FallbackBackoff);
        assert_eq!(zero_delay, Duration::from_secs(9));
    }

    #[test]
    fn operator_http_urls_allow_homelab_destinations() {
        for raw in [
            "http://localhost:9696",
            "http://127.0.0.1:8080",
            "http://192.168.1.50:9696",
            "http://10.42.0.12:8080",
            "http://prowlarr:9696",
        ] {
            validate_operator_http_url(raw, "operator integration")
                .unwrap_or_else(|error| panic!("{raw} should be operator-valid: {error}"));
        }
    }

    #[test]
    fn operator_http_urls_reject_bad_syntax_and_credentials() {
        assert!(matches!(
            validate_operator_http_url("ftp://example.test", "operator integration"),
            Err(OutboundDestinationError::UnsupportedScheme { .. })
        ));
        assert!(matches!(
            validate_operator_http_url("https://user:secret@example.test", "operator integration"),
            Err(OutboundDestinationError::EmbeddedCredentials { .. })
        ));
    }

    #[tokio::test]
    async fn untrusted_public_http_targets_reject_private_and_local_addresses() {
        for raw in [
            "http://127.0.0.1/",
            "http://10.0.0.1/",
            "http://172.16.0.1/",
            "http://192.168.1.1/",
            "http://169.254.1.1/",
            "http://0.0.0.0/",
            "http://255.255.255.255/",
            "http://224.0.0.1/",
            "http://[::1]/",
            "http://[fc00::1]/",
            "http://[fe80::1]/",
            "http://[ff02::1]/",
        ] {
            assert!(
                matches!(
                    prepare_untrusted_public_http_target(raw, "untrusted fetch").await,
                    Err(OutboundDestinationError::ForbiddenAddress { .. })
                ),
                "{raw} should be blocked for untrusted fetches"
            );
        }
    }

    #[tokio::test]
    async fn untrusted_public_http_target_records_validated_socket_addresses() {
        let target = prepare_untrusted_public_http_target(
            "http://93.184.216.34:8080/artifact.wasm",
            "untrusted fetch",
        )
        .await
        .expect("literal public IP target should prepare");

        assert_eq!(target.host(), "93.184.216.34");
        assert_eq!(
            target.resolved_addrs(),
            &[SocketAddr::from(([93, 184, 216, 34], 8080))]
        );
    }

    #[tokio::test]
    async fn cooldowns_are_isolated_per_scope() {
        let registry = RateLimitRegistry::isolated();
        let alpha: RateLimitScopeKey = "alpha".into();
        let beta: RateLimitScopeKey = "beta".into();

        let _ = registry
            .record_cooldown(
                &alpha,
                Duration::from_millis(25),
                RetryAfterSource::FallbackBackoff,
            )
            .await;

        let alpha_wait = registry.wait_if_needed(&alpha).await;
        let beta_wait = registry.wait_if_needed(&beta).await;

        assert!(alpha_wait.is_some());
        assert_eq!(beta_wait, None);
    }

    #[tokio::test]
    async fn destination_cooldown_records_dirty_persisted_state() {
        let registry = RateLimitRegistry::isolated();
        let destination: DestinationKey = "example.test".into();

        let _ = registry
            .record_destination_cooldown(
                &destination,
                Duration::from_secs(30),
                RetryAfterSource::Seconds,
            )
            .await;

        let dirty = registry.drain_dirty_destination_cooldowns();
        assert_eq!(dirty.len(), 1);
        assert_eq!(dirty[0].destination_key, destination);
        assert_eq!(dirty[0].retry_after, Some(Duration::from_secs(30)));
        assert_eq!(dirty[0].source, RetryAfterSource::Seconds);
        assert!(dirty[0].cooldown_until > Utc::now());
        assert!(registry.drain_dirty_destination_cooldowns().is_empty());

        registry.requeue_dirty_destination_cooldowns(dirty.clone());
        assert_eq!(registry.drain_dirty_destination_cooldowns(), dirty);
    }

    #[tokio::test]
    async fn requeue_dirty_destination_cooldowns_keeps_newer_dirty_state() {
        let registry = RateLimitRegistry::isolated();
        let destination: DestinationKey = "example.test".into();

        let _ = registry
            .record_destination_cooldown(
                &destination,
                Duration::from_secs(30),
                RetryAfterSource::Seconds,
            )
            .await;
        let older = registry.drain_dirty_destination_cooldowns();

        let _ = registry
            .record_destination_cooldown(
                &destination,
                Duration::from_secs(120),
                RetryAfterSource::FallbackBackoff,
            )
            .await;
        registry.requeue_dirty_destination_cooldowns(older);

        let dirty = registry.drain_dirty_destination_cooldowns();
        assert_eq!(dirty.len(), 1);
        assert_eq!(dirty[0].destination_key, destination);
        assert_eq!(dirty[0].retry_after, Some(Duration::from_secs(120)));
        assert_eq!(dirty[0].source, RetryAfterSource::FallbackBackoff);
    }

    #[test]
    fn hydrate_destination_cooldowns_restores_active_deadline_without_dirty_state() {
        let registry = RateLimitRegistry::isolated();
        let destination: DestinationKey = "example.test".into();

        registry.hydrate_destination_cooldowns([PersistedDestinationCooldown {
            destination_key: destination.clone(),
            cooldown_until: Utc::now() + chrono::Duration::seconds(30),
            retry_after: Some(Duration::from_secs(30)),
            source: RetryAfterSource::ExistingCooldown,
            status_code: Some(429),
            message: Some("rate limited".to_string()),
            observed_at: Utc::now(),
        }]);

        assert!(registry.active_destination_cooldown(&destination).is_some());
        assert!(registry.drain_dirty_destination_cooldowns().is_empty());
    }

    #[tokio::test]
    async fn safe_read_retries_429_and_eventually_succeeds() {
        let (url, hits) = spawn_http_server(vec![
            http_response(429, &[("Retry-After", "bogus")], ""),
            http_response(
                200,
                &[("Content-Type", "application/json")],
                "{\"ok\":true}",
            ),
        ])
        .await;

        let client =
            OutboundHttpClient::new(generic_reqwest_client(), RateLimitRegistry::isolated());
        let policy = RequestPolicy::safe_read("test-server", "retry-test")
            .with_max_retries(1)
            .with_backoff(Duration::from_millis(5), Duration::from_millis(5));

        let response = client
            .send(policy, || client.client().get(&url))
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn retryable_transport_error_text_matches_transient_disconnects() {
        assert!(retryable_transport_error_text(
            "error sending request for url: client error (SendRequest): connection error: peer closed connection without sending TLS close_notify"
        ));
        assert!(retryable_transport_error_text(
            "connection closed before message completed"
        ));
        assert!(retryable_transport_error_text(
            "io error: unexpected EOF while reading response"
        ));
        assert!(retryable_transport_error_text("os error: broken pipe"));
    }

    #[test]
    fn retryable_transport_error_text_rejects_non_transient_failures() {
        assert!(!retryable_transport_error_text(
            "certificate verify failed: self signed certificate"
        ));
        assert!(!retryable_transport_error_text(
            "invalid url: relative URL without a base"
        ));
        assert!(!retryable_transport_error_text(
            "metadata gateway returned GraphQL validation error"
        ));
    }

    #[tokio::test]
    async fn safe_read_retries_dropped_transport_response() {
        let (url, hits) = spawn_http_server_with_dropped_first_response(http_response(
            200,
            &[("Content-Type", "application/json")],
            "{\"ok\":true}",
        ))
        .await;

        let client =
            OutboundHttpClient::new(generic_reqwest_client(), RateLimitRegistry::isolated());
        let policy = RequestPolicy::safe_read("test-server", "transport-retry-test")
            .with_max_retries(1)
            .with_backoff(Duration::from_millis(5), Duration::from_millis(5));

        let response = client
            .send(policy, || client.client().get(&url))
            .await
            .expect("dropped first response should be retried");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn no_retry_returns_rate_limited_error_immediately() {
        let (url, hits) =
            spawn_http_server(vec![http_response(429, &[("Retry-After", "bogus")], "")]).await;

        let client =
            OutboundHttpClient::new(generic_reqwest_client(), RateLimitRegistry::isolated());
        let policy = RequestPolicy::no_retry("test-server", "no-retry")
            .with_backoff(Duration::from_millis(5), Duration::from_millis(5));

        let error = client
            .send(policy, || client.client().get(&url))
            .await
            .expect_err("request should fail");

        match error {
            OutboundHttpError::RateLimited(rate_limited) => {
                assert_eq!(rate_limited.attempts, 1);
                assert_eq!(
                    rate_limited.retry_after_source,
                    RetryAfterSource::FallbackBackoff
                );
                assert_eq!(rate_limited.retry_after, Some(Duration::from_millis(5)));
            }
            other => panic!("expected rate limited error, got {other:?}"),
        }

        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn existing_cooldown_source_wins_when_longer() {
        let registry = RateLimitRegistry::isolated();
        let scope: RateLimitScopeKey = "scope".into();

        let _ = registry
            .record_cooldown(&scope, Duration::from_millis(50), RetryAfterSource::Seconds)
            .await;
        let (_, source) = registry
            .record_cooldown(
                &scope,
                Duration::from_millis(5),
                RetryAfterSource::FallbackBackoff,
            )
            .await;

        assert_eq!(source, RetryAfterSource::ExistingCooldown);
    }

    #[test]
    fn host_keys_are_normalized() {
        assert_eq!(HostKey::from("Example.COM.").as_str(), "example.com");
        assert_eq!(HostKey::from("[2001:db8::1]").as_str(), "2001:db8::1");
    }

    #[test]
    fn registry_new_returns_shared_state() {
        let first = RateLimitRegistry::new();
        let second = RateLimitRegistry::new();

        assert!(Arc::ptr_eq(&first.state, &second.state));
    }

    #[test]
    fn host_rps_profiles_classify_public_local_and_loopback_hosts() {
        let registry = RateLimitRegistry::isolated();

        let public = registry.profile_for_host(&HostKey::from("feed.animetosho.xyz"));
        assert_eq!(public.source, HostRpsProfileSource::UnknownPublicDefault);
        assert_eq!(
            public.profile,
            HostRpsProfile::limited(DEFAULT_HOST_RPS, DEFAULT_HOST_RPS_BURST)
        );

        let private_ip = registry.profile_for_host(&HostKey::from("192.168.1.20"));
        assert_eq!(
            private_ip.source,
            HostRpsProfileSource::LocalOrManagedDefault
        );
        assert_eq!(
            private_ip.profile,
            HostRpsProfile::limited(LOCAL_MANAGED_HOST_RPS, LOCAL_MANAGED_HOST_RPS_BURST)
        );

        let docker_service = registry.profile_for_host(&HostKey::from("prowlarr"));
        assert_eq!(
            docker_service.source,
            HostRpsProfileSource::LocalOrManagedDefault
        );

        let local_domain = registry.profile_for_host(&HostKey::from("indexer.home.arpa"));
        assert_eq!(
            local_domain.source,
            HostRpsProfileSource::LocalOrManagedDefault
        );

        let loopback = registry.profile_for_host(&HostKey::from("127.0.0.1"));
        assert_eq!(loopback.source, HostRpsProfileSource::Loopback);
        assert_eq!(loopback.profile, HostRpsProfile::unthrottled());
    }

    #[test]
    fn explicit_host_rps_profile_registration_overrides_classification() {
        let registry = RateLimitRegistry::isolated();
        let host = HostKey::from("proxy.example.com");
        let profile = HostRpsProfile::limited(LOCAL_MANAGED_HOST_RPS, LOCAL_MANAGED_HOST_RPS_BURST);

        registry.register_host_profile(
            host.clone(),
            profile,
            HostRpsProfileSource::ExplicitRegistration,
        );

        let assignment = registry.profile_for_host(&host);
        assert_eq!(assignment.profile, profile);
        assert_eq!(
            assignment.source,
            HostRpsProfileSource::ExplicitRegistration
        );
    }

    #[tokio::test]
    async fn host_rps_is_shared_per_host() {
        let registry = RateLimitRegistry::isolated();
        let host: HostKey = "rps.example.test".into();

        let first_wait = registry.acquire_host_rps(&host).await;
        let second_wait = registry.acquire_host_rps(&host).await;
        let third_wait = registry.acquire_host_rps(&host).await;
        let fourth_wait = registry.acquire_host_rps(&host).await;

        assert_eq!(first_wait, None);
        assert_eq!(second_wait, None);
        assert_eq!(third_wait, None);
        assert!(fourth_wait.is_some());
    }

    #[tokio::test]
    async fn host_rps_preview_does_not_reserve_capacity() {
        let registry = RateLimitRegistry::isolated();
        let host: HostKey = "preview.example.test".into();

        assert_eq!(registry.preview_host_rps_wait(&host), None);
        assert_eq!(registry.preview_host_rps_wait(&host), None);

        assert_eq!(registry.acquire_host_rps(&host).await, None);
        assert_eq!(registry.acquire_host_rps(&host).await, None);
        assert_eq!(registry.acquire_host_rps(&host).await, None);
        assert!(registry.preview_host_rps_wait(&host).is_some());
    }

    #[tokio::test]
    async fn loopback_hosts_bypass_default_rps() {
        let registry = RateLimitRegistry::isolated();
        let host: HostKey = "127.0.0.1".into();

        for _ in 0..8 {
            assert_eq!(registry.acquire_host_rps(&host).await, None);
        }
    }

    #[tokio::test]
    async fn snapshot_reports_host_rps_and_destination_cooldowns() {
        let registry = RateLimitRegistry::isolated();
        let host: HostKey = "snapshot.example.test".into();
        let destination: DestinationKey = "snapshot.example.test".into();

        let _ = registry.acquire_host_rps(&host).await;
        let _ = registry
            .record_destination_cooldown(
                &destination,
                Duration::from_secs(1),
                RetryAfterSource::Seconds,
            )
            .await;

        let snapshot = registry.snapshot();

        assert!(snapshot.host_rps.iter().any(|entry| {
            entry.host_key == host
                && entry.profile_source == HostRpsProfileSource::UnknownPublicDefault
        }));
        assert!(
            snapshot
                .destination_cooldowns
                .iter()
                .any(|entry| entry.destination_key == destination && !entry.available_in.is_zero())
        );
    }

    #[tokio::test]
    async fn outbound_client_paces_redirect_target_host() {
        let (target_bound_url, target_hits) = spawn_http_server(vec![http_response(
            200,
            &[("Content-Type", "text/plain")],
            "ok",
        )])
        .await;
        let target_addr = bound_url_socket_addr(&target_bound_url);
        let (origin_bound_url, origin_hits) = spawn_http_server(vec![http_response(
            302,
            &[("Location", "http://target.test/test")],
            "",
        )])
        .await;
        let origin_addr = bound_url_socket_addr(&origin_bound_url);
        let registry = RateLimitRegistry::isolated();
        let client = reqwest_client_builder()
            .resolve_to_addrs("origin.test", &[origin_addr])
            .resolve_to_addrs("target.test", &[target_addr])
            .build()
            .expect("client should build");
        let outbound = OutboundHttpClient::new(client.clone(), registry.clone());

        let response = outbound
            .send(
                RequestPolicy::safe_read("redirect-test", "redirect-test")
                    .with_trusted_redirects(MAX_MANUAL_REDIRECTS),
                || client.get("http://origin.test/test"),
            )
            .await
            .expect("redirected request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(origin_hits.load(Ordering::SeqCst), 1);
        assert_eq!(target_hits.load(Ordering::SeqCst), 1);
        let snapshot = registry.snapshot();
        assert!(
            snapshot
                .host_rps
                .iter()
                .any(|entry| entry.host_key == HostKey::from("origin.test"))
        );
        assert!(
            snapshot
                .host_rps
                .iter()
                .any(|entry| entry.host_key == HostKey::from("target.test"))
        );
    }

    #[tokio::test]
    async fn outbound_client_does_not_follow_redirects_by_default() {
        let (target_bound_url, target_hits) = spawn_http_server(vec![http_response(
            200,
            &[("Content-Type", "text/plain")],
            "ok",
        )])
        .await;
        let (origin_bound_url, origin_hits) = spawn_http_server(vec![http_response(
            302,
            &[("Location", target_bound_url.as_str())],
            "",
        )])
        .await;
        let registry = RateLimitRegistry::isolated();
        let client = reqwest_client_builder()
            .build()
            .expect("client should build");
        let outbound = OutboundHttpClient::new(client.clone(), registry);

        let response = outbound
            .send(
                RequestPolicy::safe_read("redirect-test", "redirect-test"),
                || client.get(origin_bound_url.clone()),
            )
            .await
            .expect("redirect response should be returned");

        assert_eq!(response.status(), StatusCode::FOUND);
        assert_eq!(origin_hits.load(Ordering::SeqCst), 1);
        assert_eq!(target_hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn destination_cooldowns_are_isolated_from_legacy_scopes() {
        let registry = RateLimitRegistry::isolated();
        let scope: RateLimitScopeKey = "legacy-scope".into();
        let destination: DestinationKey = "cooldown.example.test".into();

        let _ = registry
            .record_destination_cooldown(
                &destination,
                Duration::from_millis(10),
                RetryAfterSource::Seconds,
            )
            .await;

        assert!(
            registry
                .wait_for_destination_if_needed(&destination)
                .await
                .is_some()
        );
        assert_eq!(registry.wait_if_needed(&scope).await, None);
    }

    fn http_response(status: u16, headers: &[(&str, &str)], body: &str) -> String {
        let mut response = format!(
            "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\n",
            body.len()
        );
        for (name, value) in headers {
            response.push_str(name);
            response.push_str(": ");
            response.push_str(value);
            response.push_str("\r\n");
        }
        response.push_str("\r\n");
        response.push_str(body);
        response
    }

    async fn spawn_http_server(responses: Vec<String>) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let address = listener.local_addr().expect("listener should have addr");
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_for_task = hits.clone();

        tokio::spawn(async move {
            for response in responses {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                hits_for_task.fetch_add(1, Ordering::SeqCst);
                if read_request(&mut stream).await.is_err() {
                    break;
                }
                if stream.write_all(response.as_bytes()).await.is_err() {
                    break;
                }
                let _ = stream.shutdown().await;
            }
        });

        (format!("http://{address}/test"), hits)
    }

    async fn spawn_http_server_with_dropped_first_response(
        success_response: String,
    ) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let address = listener.local_addr().expect("listener should have addr");
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_for_task = hits.clone();

        tokio::spawn(async move {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            hits_for_task.fetch_add(1, Ordering::SeqCst);
            let _ = read_request(&mut stream).await;
            let _ = stream.shutdown().await;

            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            hits_for_task.fetch_add(1, Ordering::SeqCst);
            if read_request(&mut stream).await.is_err() {
                return;
            }
            let _ = stream.write_all(success_response.as_bytes()).await;
            let _ = stream.shutdown().await;
        });

        (format!("http://{address}/test"), hits)
    }

    fn bound_url_socket_addr(url: &str) -> SocketAddr {
        let url = reqwest::Url::parse(url).expect("bound URL should parse");
        SocketAddr::new(
            url.host_str()
                .expect("bound URL should include host")
                .parse()
                .expect("bound URL host should parse"),
            url.port().expect("bound URL should include port"),
        )
    }

    async fn read_request(stream: &mut tokio::net::TcpStream) -> io::Result<()> {
        let mut buffer = vec![0u8; 4096];
        let mut received = Vec::new();
        loop {
            let read = stream.read(&mut buffer).await?;
            if read == 0 {
                break;
            }
            received.extend_from_slice(&buffer[..read]);
            if received.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        Ok(())
    }
}
