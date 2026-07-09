use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use glob::Pattern;
use reqwest::blocking::Client;
use reqwest::{Method, StatusCode};
use scryer_application::challenge_solver as solver;

pub(crate) const HTTP_ENV_NAMESPACE: &str = "extism:host/env";
const DEFAULT_MAX_HTTP_RESPONSE_BYTES: u64 = 50 * 1024 * 1024;
type HostResult<T> = Result<T, String>;

static SHARED_PLUGIN_HTTP_RUNTIME: LazyLock<PluginHttpRuntime> =
    LazyLock::new(PluginHttpRuntime::default);

#[derive(Clone, Default)]
pub struct PluginHttpRuntime {
    state: Arc<Mutex<PluginHttpRuntimeState>>,
}

#[derive(Default)]
struct PluginHttpRuntimeState {
    extra_ca_bundle_pem: String,
    cached_client: Option<Client>,
}

pub(crate) struct PluginHttpHost {
    state: Arc<Mutex<PluginHttpHostState>>,
}

struct PluginHttpHostState {
    runtime: PluginHttpRuntime,
    allowed_hosts: Option<Vec<String>>,
    indexer_proxy_policy: Option<IndexerProxyPolicy>,
    max_http_response_bytes: Option<u64>,
    last_responses: HashMap<String, PluginHttpLastResponse>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct PluginHttpRequest {
    pub(crate) url: String,
    #[serde(default)]
    pub(crate) method: Option<String>,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
}

#[derive(Clone, Default)]
struct PluginHttpLastResponse {
    status_code: u16,
    headers: BTreeMap<String, String>,
}

#[derive(Clone)]
pub(crate) struct IndexerProxyPolicy {
    pub indexer_id: String,
    pub indexer_name: String,
    pub config: scryer_domain::IndexerProxyConfig,
}

struct ProxiedHttpResponse {
    status_code: u16,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

pub fn shared_plugin_http_runtime() -> PluginHttpRuntime {
    SHARED_PLUGIN_HTTP_RUNTIME.clone()
}

impl scryer_application::PluginHttpTrustConfigRuntime for PluginHttpRuntime {
    fn set_plugin_http_ca_bundle_pem(
        &self,
        bundle_pem: String,
    ) -> scryer_application::AppResult<()> {
        self.set_extra_ca_bundle_pem(bundle_pem)
            .map_err(scryer_application::AppError::Repository)
    }
}

impl PluginHttpRuntime {
    pub fn set_extra_ca_bundle_pem(&self, bundle_pem: impl Into<String>) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|error| format!("plugin HTTP runtime lock poisoned: {error}"))?;
        state.extra_ca_bundle_pem = bundle_pem.into();
        state.cached_client = None;
        Ok(())
    }

    /// Operator-trusted client for indexer-proxy endpoints (e.g. Byparr). Those
    /// targets are operator-configured, so they are not subject to the plugin
    /// egress guard and may legitimately live on the LAN.
    fn client(&self) -> HostResult<Client> {
        let mut state = self
            .state
            .lock()
            .map_err(|error| format!("plugin HTTP runtime lock poisoned: {error}"))?;
        if let Some(client) = &state.cached_client {
            return Ok(client.clone());
        }

        let client = scryer_outbound_http::blocking_plugin_host_client(&state.extra_ca_bundle_pem)
            .map_err(|error| error.to_string())?;
        state.cached_client = Some(client.clone());
        Ok(client)
    }

    /// Builds a DNS-pinned, guarded blocking client for an untrusted
    /// plugin-controlled request URL under the plugin egress policy. A fresh
    /// client is built per request because DNS pinning is host-specific; this
    /// is what stops a plugin from reaching cloud-metadata / link-local space
    /// (even via DNS rebinding) once its `allowed_hosts` allowlist has passed.
    fn pinned_request_client(&self, url: &str) -> HostResult<Client> {
        let extra_ca_bundle_pem = {
            let state = self
                .state
                .lock()
                .map_err(|error| format!("plugin HTTP runtime lock poisoned: {error}"))?;
            state.extra_ca_bundle_pem.clone()
        };
        scryer_outbound_http::prepare_plugin_blocking_http_target(
            url,
            &extra_ca_bundle_pem,
            "plugin HTTP",
        )
        .map(scryer_outbound_http::PinnedPluginBlockingHttpTarget::into_client)
        .map_err(|error| error.to_string())
    }
}

impl PluginHttpHost {
    pub(crate) fn new(
        allowed_hosts: Vec<String>,
        indexer_proxy_policy: Option<IndexerProxyPolicy>,
        max_http_response_bytes: Option<u64>,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(PluginHttpHostState {
                runtime: shared_plugin_http_runtime(),
                allowed_hosts: Some(allowed_hosts),
                indexer_proxy_policy,
                max_http_response_bytes,
                last_responses: HashMap::new(),
            })),
        }
    }

    pub(crate) fn request(
        &self,
        plugin_id: &str,
        request: PluginHttpRequest,
        body: Option<Vec<u8>>,
        timeout: Option<Duration>,
    ) -> HostResult<Vec<u8>> {
        let (runtime, allowed_hosts, indexer_proxy_policy, max_http_response_bytes) = {
            let mut host_state = self
                .state
                .lock()
                .map_err(|error| format!("plugin HTTP host state lock poisoned: {error}"))?;
            host_state
                .last_responses
                .insert(plugin_id.to_string(), PluginHttpLastResponse::default());
            (
                host_state.runtime.clone(),
                host_state.allowed_hosts.clone(),
                host_state.indexer_proxy_policy.clone(),
                host_state.max_http_response_bytes,
            )
        };

        enforce_allowed_hosts(allowed_hosts.as_deref(), &request.url)?;

        // The allowlist is the primary boundary; the guarded, DNS-pinned client
        // is the second layer that keeps a declared host from reaching
        // link-local / cloud-metadata space.
        let request_client = runtime.pinned_request_client(&request.url)?;
        let started_at = Instant::now();
        let request_is_get = request
            .method
            .as_deref()
            .unwrap_or("GET")
            .eq_ignore_ascii_case("GET");
        // Reuse a previously solved clearance session for this proxy + origin so
        // repeat requests skip the solver entirely until the session goes stale.
        let session_headers = indexer_proxy_policy
            .as_ref()
            .filter(|_| request_is_get)
            .map(|policy| {
                solver::SolvedSessionCache::shared()
                    .session_headers(&policy.config.id, &request.url)
            })
            .unwrap_or_default();
        let response = execute_request_with_extra_headers(
            &request_client,
            &request,
            body.clone(),
            timeout,
            &session_headers,
        )?;
        let status = response.status();
        let status_code = status.as_u16();
        let headers = response_headers(&response);

        if status == StatusCode::TOO_MANY_REQUESTS {
            self.store_last_response(plugin_id, status_code, headers)?;
            tracing::debug!(
                plugin_id,
                status = status_code,
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                response_bytes = 0_u64,
                "plugin HTTP request skipped indexer proxy after direct rate limit"
            );
            return Ok(Vec::new());
        }

        let should_read_body = status.is_success()
            || indexer_proxy_policy.is_some() && solver::challenge_candidate_status(status_code);
        let direct_body = if should_read_body {
            read_response_body(response, max_http_response_bytes)?
        } else {
            Vec::new()
        };

        if let Some(policy) = indexer_proxy_policy.as_ref()
            && solver::looks_like_challenge_response(status_code, &headers, &direct_body)
        {
            let method = request.method.as_deref().unwrap_or("GET");
            if !method.eq_ignore_ascii_case("GET") {
                return Err(format!(
                    "indexer proxy only supports GET challenge solving for plugin HTTP requests; got {method}"
                ));
            }
            if !session_headers.is_empty() {
                // The cached session no longer clears the challenge.
                solver::SolvedSessionCache::shared().invalidate(&policy.config.id, &request.url);
            }

            tracing::debug!(
                plugin_id,
                indexer_id = policy.indexer_id.as_str(),
                indexer_name = policy.indexer_name.as_str(),
                proxy_config_id = policy.config.id.as_str(),
                status = status_code,
                request_url = solver::sanitized_url_for_log(&request.url).as_str(),
                "plugin HTTP request detected browser challenge"
            );

            // The proxy endpoint itself is operator-configured, so it uses the
            // trusted client; the plugin URL retry inside stays on the guarded
            // pinned client.
            let proxy_client = runtime.client()?;
            let solved = match execute_byparr_request(
                &proxy_client,
                &request_client,
                policy,
                &request,
                body,
                timeout,
                max_http_response_bytes,
            ) {
                Ok(solved) => {
                    solver::SolverHealthLedger::shared().record_success(&policy.config.id);
                    solved
                }
                Err(error) => {
                    if solver::is_solver_service_error_message(&error) {
                        solver::SolverHealthLedger::shared()
                            .record_failure(&policy.config.id, &error);
                    }
                    return Err(error);
                }
            };
            let response_bytes = solved.body.len();
            self.store_last_response(plugin_id, solved.status_code, solved.headers)?;
            tracing::debug!(
                plugin_id,
                indexer_id = policy.indexer_id.as_str(),
                proxy_config_id = policy.config.id.as_str(),
                status = solved.status_code,
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                response_bytes,
                "plugin HTTP request completed through indexer proxy"
            );
            return Ok(solved.body);
        }

        let response_bytes = direct_body.len();
        self.store_last_response(plugin_id, status_code, headers)?;
        tracing::debug!(
            plugin_id,
            status = status_code,
            elapsed_ms = started_at.elapsed().as_millis() as u64,
            response_bytes,
            "plugin HTTP request completed"
        );
        if status.is_success() {
            Ok(direct_body)
        } else {
            Ok(Vec::new())
        }
    }

    pub(crate) fn status_code(&self, plugin_id: &str) -> HostResult<u16> {
        let host_state = self
            .state
            .lock()
            .map_err(|error| format!("plugin HTTP host state lock poisoned: {error}"))?;
        Ok(host_state
            .last_responses
            .get(plugin_id)
            .map(|response| response.status_code)
            .unwrap_or(0))
    }

    pub(crate) fn headers(&self, plugin_id: &str) -> HostResult<Option<BTreeMap<String, String>>> {
        let host_state = self
            .state
            .lock()
            .map_err(|error| format!("plugin HTTP host state lock poisoned: {error}"))?;
        Ok(host_state
            .last_responses
            .get(plugin_id)
            .filter(|response| !response.headers.is_empty())
            .map(|response| response.headers.clone()))
    }

    fn store_last_response(
        &self,
        plugin_id: &str,
        status_code: u16,
        headers: BTreeMap<String, String>,
    ) -> HostResult<()> {
        let mut host_state = self
            .state
            .lock()
            .map_err(|error| format!("plugin HTTP host state lock poisoned: {error}"))?;
        host_state.last_responses.insert(
            plugin_id.to_string(),
            PluginHttpLastResponse {
                status_code,
                headers,
            },
        );
        Ok(())
    }
}

fn enforce_allowed_hosts(allowed_hosts: Option<&[String]>, request_url: &str) -> HostResult<()> {
    let url = url::Url::parse(request_url).map_err(|error| format!("Invalid URL: {error:?}"))?;
    let host = url.host_str().unwrap_or_default();
    let matches = allowed_hosts.is_some_and(|patterns| {
        patterns.iter().any(|pattern| {
            Pattern::new(pattern)
                .map(|compiled| compiled.matches(host))
                .unwrap_or_else(|_| pattern == host)
        })
    });

    if matches {
        return Ok(());
    }

    // Never interpolate the raw request URL here: for indexer requests it
    // carries `?apikey=`/`passkey=` credentials that would otherwise leak into
    // WARN logs and the user-facing error. Log the query-stripped URL instead.
    Err(format!(
        "HTTP request to {} is not allowed",
        solver::sanitized_url_for_log(request_url)
    ))
}

fn execute_request_with_extra_headers(
    client: &Client,
    request: &PluginHttpRequest,
    body: Option<Vec<u8>>,
    timeout: Option<Duration>,
    extra_headers: &[(String, String)],
) -> HostResult<reqwest::blocking::Response> {
    let method = Method::from_bytes(
        request
            .method
            .as_deref()
            .unwrap_or("GET")
            .to_uppercase()
            .as_bytes(),
    )
    .map_err(|error| format!("Invalid HTTP method: {error}"))?;

    let mut builder = client.request(method, &request.url);
    for (name, value) in &request.headers {
        // Solver-session headers must replace plugin-supplied ones: clearance
        // cookies are only honoured together with the solver's user agent.
        if extra_headers
            .iter()
            .any(|(extra, _)| extra.eq_ignore_ascii_case(name))
        {
            continue;
        }
        builder = builder.header(name, value);
    }
    for (name, value) in extra_headers {
        builder = builder.header(name, value);
    }
    if let Some(timeout) = timeout {
        builder = builder.timeout(timeout);
    }
    if let Some(body) = body {
        builder = builder.body(body);
    }

    scryer_outbound_http::send_blocking_reqwest_request(builder).map_err(|error| {
        if error.is_timeout() {
            "timeout".to_string()
        } else {
            error.to_string()
        }
    })
}

fn response_headers(response: &reqwest::blocking::Response) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    for (name, value) in response.headers() {
        if let Ok(value) = value.to_str() {
            headers.insert(name.as_str().to_string(), value.to_string());
        }
    }
    headers
}

fn read_response_body(
    response: reqwest::blocking::Response,
    max_http_response_bytes: Option<u64>,
) -> HostResult<Vec<u8>> {
    let mut body = Vec::new();
    let max = max_http_response_bytes.unwrap_or(DEFAULT_MAX_HTTP_RESPONSE_BYTES);
    response
        .take(max + 1)
        .read_to_end(&mut body)
        .map_err(|error| error.to_string())?;
    if body.len() > max as usize {
        return Err(format!(
            "HTTP response exceeds the configured maximum number of bytes: {max}"
        ));
    }
    Ok(body)
}

fn execute_byparr_request(
    proxy_client: &Client,
    request_client: &Client,
    policy: &IndexerProxyPolicy,
    request: &PluginHttpRequest,
    original_body: Option<Vec<u8>>,
    original_timeout: Option<Duration>,
    max_http_response_bytes: Option<u64>,
) -> HostResult<ProxiedHttpResponse> {
    if policy.config.provider_type != scryer_domain::IndexerProxyProviderType::Byparr {
        return Err("unsupported indexer proxy provider".to_string());
    }
    if !policy.config.is_enabled {
        return Err("Indexer proxy is disabled for this indexer.".to_string());
    }

    let endpoint = solver::byparr_solve_endpoint(&policy.config.base_url);
    let byparr_timeout = Duration::from_secs(policy.config.request_timeout_seconds as u64 + 5);
    tracing::debug!(
        indexer_id = policy.indexer_id.as_str(),
        indexer_name = policy.indexer_name.as_str(),
        proxy_config_id = policy.config.id.as_str(),
        proxy_provider = policy.config.provider_type.as_str(),
        request_url = solver::sanitized_url_for_log(&request.url).as_str(),
        "Byparr request started"
    );

    let response = proxy_client
        .post(&endpoint)
        .timeout(byparr_timeout)
        .json(&solver::byparr_solve_request(
            &request.url,
            policy.config.request_timeout_seconds,
        ))
        .send()
        .map_err(|error| {
            if error.is_timeout() {
                solver::BYPARR_TIMEOUT_MESSAGE.to_string()
            } else {
                solver::BYPARR_UNREACHABLE_MESSAGE.to_string()
            }
        })?;

    let byparr_status = response.status();
    if byparr_status == StatusCode::TOO_MANY_REQUESTS {
        tracing::warn!(
            indexer_id = policy.indexer_id.as_str(),
            proxy_config_id = policy.config.id.as_str(),
            status = byparr_status.as_u16(),
            "Byparr service rate-limited indexer proxy request"
        );
        return Err(solver::BYPARR_UNAVAILABLE_MESSAGE.to_string());
    }

    let response_body = read_response_body(response, max_http_response_bytes)?;
    let solution =
        solver::parse_byparr_solution(&response_body).map_err(|error| error.message())?;

    let solution_status = solution.status.unwrap_or_else(|| byparr_status.as_u16());
    let solved_final_url = solution.url.as_deref().map(solver::sanitized_url_for_log);
    if solution_status == StatusCode::TOO_MANY_REQUESTS.as_u16() {
        tracing::warn!(
            indexer_id = policy.indexer_id.as_str(),
            proxy_config_id = policy.config.id.as_str(),
            status = solution_status,
            "Byparr reported target indexer rate limit"
        );
        return Err(solver::target_rate_limit_message(&solution));
    }

    let solved_body = solution.response.clone().unwrap_or_default().into_bytes();
    if solver::solved_body_looks_rate_limited(&solved_body) {
        return Err(solver::target_rate_limit_message(&solution));
    }
    if !(200..300).contains(&solution_status) {
        return Err(format!(
            "Byparr target request returned HTTP {solution_status}."
        ));
    }

    // Cache the clearance session so follow-up requests to this origin skip
    // the solver until the session expires or stops clearing challenges.
    solver::SolvedSessionCache::shared().store_solution(&policy.config.id, &request.url, &solution);

    let solved_headers = solver::safe_solution_response_headers(solution.headers.as_ref());
    if !solved_body.is_empty() {
        tracing::debug!(
            indexer_id = policy.indexer_id.as_str(),
            proxy_config_id = policy.config.id.as_str(),
            status = solution_status,
            response_bytes = solved_body.len(),
            final_url = solved_final_url.as_deref(),
            "Byparr solved response used"
        );
        return Ok(ProxiedHttpResponse {
            status_code: solution_status,
            headers: solved_headers,
            body: solved_body,
        });
    }

    let retry_headers = solver::solution_retry_headers(&solution);
    if !retry_headers.is_empty() {
        tracing::debug!(
            indexer_id = policy.indexer_id.as_str(),
            proxy_config_id = policy.config.id.as_str(),
            "retrying original request with Byparr solver headers"
        );
        let retry = execute_request_with_extra_headers(
            request_client,
            request,
            original_body,
            original_timeout,
            &retry_headers,
        )?;
        let status = retry.status();
        let headers = response_headers(&retry);
        if status == StatusCode::TOO_MANY_REQUESTS {
            let retry_after = solver::header_value(&headers, "retry-after").and_then(|value| {
                scryer_outbound_http::parse_retry_after(value).map(|(delay, _)| delay)
            });
            return Err(solver::rate_limit_message_with_retry_after(retry_after));
        }
        if !status.is_success() {
            return Err(solver::BYPARR_NO_SOLUTION_MESSAGE.to_string());
        }
        let body = read_response_body(retry, max_http_response_bytes)?;
        return Ok(ProxiedHttpResponse {
            status_code: status.as_u16(),
            headers,
            body,
        });
    }

    Err(solver::BYPARR_NO_SOLUTION_MESSAGE.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_PLUGIN_HTTP_CA_CERT_PEM: &str = concat!(
        "-----BEGIN CERTIFICATE-----\n",
        "MIIDITCCAgmgAwIBAgIUY40m7DS0vG3xUR0EXxPLYFVq/WkwDQYJKoZIhvcNAQEL\n",
        "BQAwGDEWMBQGA1UEAwwNZTJlLWppbWFrdS1jYTAeFw0yNjA1MjExNzE4NTNaFw0z\n",
        "NjA1MTgxNzE4NTNaMBgxFjAUBgNVBAMMDWUyZS1qaW1ha3UtY2EwggEiMA0GCSqG\n",
        "SIb3DQEBAQUAA4IBDwAwggEKAoIBAQCygxcuiabmKSdpOdnE2Vg9x8AxDtsv3apm\n",
        "qaAeDTaG2uPeSjQsxKJfYDkRmOS9eqEV+yYQeiRwAdq3vadUd/eVlfvvrCtCswkx\n",
        "vHhDvKpgc8KW239IdygK8JFHJz1FTfZRfgWgiKGnlqef6R1w8BjewD6/byv+VJxR\n",
        "cQaVmrBfc7ZzXL41C/WCpdZLMyzRn1EeoEvTYqn1+Yqhhx8WlIQlT2Ha3gOIvAAX\n",
        "Xh1CyfosZbFGfuVk4njM01K00N8GaMk0CWwMvgKADPKNh29S1Pv4PnL5k03Qb4gS\n",
        "bAMRWJi+xMYmtAdINPnJscPKj++vOMdJxGQunpgkXKoHELZWLOANAgMBAAGjYzBh\n",
        "MB8GA1UdIwQYMBaAFMJFcy1sAajZvY0Amv6QuPe4iqPUMA8GA1UdEwEB/wQFMAMB\n",
        "Af8wDgYDVR0PAQH/BAQDAgEGMB0GA1UdDgQWBBTCRXMtbAGo2b2NAJr+kLj3uIqj\n",
        "1DANBgkqhkiG9w0BAQsFAAOCAQEAIZkWiXfdJSLtHUlqUfT5R9ko8acIt1uQt2kI\n",
        "3SiDqyFrHWTT+cyfFyqBIEASPLX9fgPHkz42K4P1Kc9W4JR8o/QWRK7A0hvbCzuB\n",
        "Z/5+agQ15hA1priLKk/oqoILFhT3LHR3/6mzk6vJ3EmIyDITUZ6tQiQS0zyXCxpR\n",
        "8aCN5dsNaBwN42hxBrm/7TjiNCdX54zjLg6cPbtrsHnAI7NBi3O/WNEYISiUcC5O\n",
        "FnEYx13QF8BQo/cY55EZDrEnF4+R6Q3DPQJHhd6tIoEYvxp8wVnUjQb3nWib1wvW\n",
        "dlYNMnHca3kyT/MHY4oX5MmPsHY8ANxBBz0XSKw5ysN4cNpK/Q==\n",
        "-----END CERTIFICATE-----\n",
    );

    #[test]
    fn build_plugin_http_client_accepts_empty_trust_bundle() {
        scryer_outbound_http::blocking_plugin_host_client("")
            .expect("default trust bundle should build");
    }

    #[test]
    fn build_plugin_http_client_accepts_uploaded_certificates() {
        scryer_outbound_http::blocking_plugin_host_client(TEST_PLUGIN_HTTP_CA_CERT_PEM)
            .expect("uploaded trust bundle should build");
    }

    #[test]
    fn add_uploaded_certificates_rejects_non_certificate_pem_items() {
        let error = scryer_outbound_http::blocking_plugin_host_client(
            "-----BEGIN PRIVATE KEY-----\nZmFrZQ==\n-----END PRIVATE KEY-----\n",
        )
        .expect_err("non-certificate bundle should be rejected");

        assert!(error.contains("uploaded trusted certificate bundle"));
    }

    #[test]
    fn enforce_allowed_hosts_rejects_disallowed_hosts() {
        let error = enforce_allowed_hosts(
            Some(&["example.com".to_string()]),
            "https://jimaku.example.test/search",
        )
        .expect_err("disallowed host should fail");

        assert!(error.to_string().contains("is not allowed"));
    }

    #[test]
    fn enforce_allowed_hosts_error_omits_query_credentials() {
        let error = enforce_allowed_hosts(
            Some(&["allowed.example".to_string()]),
            "https://tracker.example.test/download?apikey=SECRETKEY&passkey=TOPSECRET",
        )
        .expect_err("disallowed host should fail");

        assert!(error.contains("is not allowed"));
        assert!(!error.contains('?'), "error must not carry a query string: {error}");
        assert!(!error.contains("apikey"), "error must not leak apikey: {error}");
        assert!(!error.contains("passkey"), "error must not leak passkey: {error}");
        assert!(!error.contains("SECRETKEY"), "error must not leak secrets: {error}");
        assert!(!error.contains("TOPSECRET"), "error must not leak secrets: {error}");
    }

    #[test]
    fn plugin_request_egress_blocks_cloud_metadata() {
        let result = scryer_outbound_http::prepare_plugin_blocking_http_target(
            "http://169.254.169.254/latest/meta-data/",
            "",
            "plugin HTTP",
        );

        assert!(
            matches!(
                result,
                Err(scryer_outbound_http::OutboundDestinationError::BlockedLinkLocalOrMetadata { .. })
            ),
            "cloud metadata address must be rejected on the plugin HTTP host path"
        );
    }

    #[test]
    fn plugin_request_egress_allows_loopback_companion() {
        scryer_outbound_http::prepare_plugin_blocking_http_target(
            "http://127.0.0.1:9117/api",
            "",
            "plugin HTTP",
        )
        .expect("loopback companion must be allowed for self-hosted plugins");
    }
}
