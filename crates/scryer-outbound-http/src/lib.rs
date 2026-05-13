use std::borrow::Cow;
use std::collections::HashMap;
use std::convert::Infallible;
use std::fmt;
use std::future::Future;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use metrics::{counter, histogram};
use reqwest::header::{HeaderMap, RETRY_AFTER};
use reqwest::{Certificate, Client, Identity, RequestBuilder, Response, StatusCode};
use thiserror::Error;
use tokio::sync::Mutex;
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
pub enum RetryAfterSource {
    HttpDate,
    Seconds,
    FallbackBackoff,
    ExistingCooldown,
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

    fn retry_allowed(&self, attempt: u32) -> bool {
        !matches!(self.retry_mode, RetryMode::NoRetry) && attempt <= self.max_retries
    }

    fn backoff_for_retry(&self, retry_index: u32) -> Duration {
        bounded_exponential_backoff(self.base_backoff, self.max_backoff, retry_index)
    }
}

#[derive(Clone, Default)]
pub struct RateLimitRegistry {
    deadlines: Arc<Mutex<HashMap<RateLimitScopeKey, Instant>>>,
}

impl RateLimitRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn wait_if_needed(&self, scope: &RateLimitScopeKey) -> Option<Duration> {
        let mut total_wait = Duration::ZERO;

        loop {
            let wait_duration = {
                let mut deadlines = self.deadlines.lock().await;
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
        let mut deadlines = self.deadlines.lock().await;

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
}

fn reqwest_client_builder() -> reqwest::ClientBuilder {
    install_default_rustls_provider();
    Client::builder()
}

pub fn install_default_rustls_provider() {
    static INSTALL_RUSTLS_PROVIDER: OnceLock<()> = OnceLock::new();

    INSTALL_RUSTLS_PROVIDER.get_or_init(|| {
        // The provider is process-global; parallel workspace tests may install it first.
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

pub fn default_reqwest_client() -> Client {
    reqwest_client_builder()
        .build()
        .expect("default reqwest client should build")
}

pub fn timeout_reqwest_client(timeout: Option<Duration>) -> Result<Client, reqwest::Error> {
    let mut builder = reqwest_client_builder();
    if let Some(timeout) = timeout {
        builder = builder.timeout(timeout);
    }
    builder.build()
}

pub fn no_redirect_timeout_reqwest_client(
    timeout: Option<Duration>,
) -> Result<Client, reqwest::Error> {
    let mut builder = reqwest_client_builder().redirect(reqwest::redirect::Policy::none());
    if let Some(timeout) = timeout {
        builder = builder.timeout(timeout);
    }
    builder.build()
}

pub fn user_agent_reqwest_client(user_agent: &str) -> Result<Client, reqwest::Error> {
    reqwest_client_builder().user_agent(user_agent).build()
}

pub fn title_image_reqwest_client(
    user_agent: &str,
    connect_timeout: Duration,
    request_timeout: Duration,
) -> Result<Client, reqwest::Error> {
    reqwest_client_builder()
        .user_agent(user_agent)
        .connect_timeout(connect_timeout)
        .timeout(request_timeout)
        .build()
}

pub fn external_arr_reqwest_client() -> Client {
    reqwest_client_builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap_or_else(|_| default_reqwest_client())
}

pub fn metadata_gateway_reqwest_client(
    accept_invalid_certs: bool,
    user_agent: &str,
) -> Result<Client, reqwest::Error> {
    reqwest_client_builder()
        .timeout(Duration::from_secs(100))
        .user_agent(user_agent)
        .danger_accept_invalid_certs(accept_invalid_certs)
        .build()
}

pub fn metadata_gateway_mtls_reqwest_client(
    identity: Identity,
    ca_cert: Certificate,
) -> Result<Client, reqwest::Error> {
    reqwest_client_builder()
        .timeout(Duration::from_secs(100))
        .identity(identity)
        .add_root_certificate(ca_cert)
        .build()
}

pub fn enrollment_reqwest_client(
    ca_cert_override: Option<Certificate>,
    user_agent: &str,
) -> Result<Client, reqwest::Error> {
    let mut builder = reqwest_client_builder()
        .timeout(Duration::from_secs(30))
        .user_agent(user_agent);
    if let Some(ca_cert_override) = ca_cert_override {
        builder = builder.add_root_certificate(ca_cert_override);
    }
    builder.build()
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

            match builder.send().await {
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
    error.is_timeout() || error.is_connect()
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

    #[tokio::test]
    async fn cooldowns_are_isolated_per_scope() {
        let registry = RateLimitRegistry::new();
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

        let client = OutboundHttpClient::new(default_reqwest_client(), RateLimitRegistry::new());
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

    #[tokio::test]
    async fn no_retry_returns_rate_limited_error_immediately() {
        let (url, hits) =
            spawn_http_server(vec![http_response(429, &[("Retry-After", "bogus")], "")]).await;

        let client = OutboundHttpClient::new(default_reqwest_client(), RateLimitRegistry::new());
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
        let registry = RateLimitRegistry::new();
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
