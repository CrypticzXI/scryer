use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use extism::{CurrentPlugin, Error, Function, Manifest, UserData, Val, ValType};
use extism_manifest::HttpRequest;
use glob::Pattern;
use reqwest::blocking::Client;
use reqwest::{Method, StatusCode};
use scryer_application::challenge_solver as solver;

const HTTP_ENV_NAMESPACE: &str = "extism:host/env";
const DEFAULT_MAX_HTTP_RESPONSE_BYTES: u64 = 50 * 1024 * 1024;

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

struct PluginHttpHostState {
    runtime: PluginHttpRuntime,
    allowed_hosts: Option<Vec<String>>,
    indexer_proxy_policy: Option<IndexerProxyPolicy>,
    max_http_response_bytes: Option<u64>,
    last_responses: HashMap<String, PluginHttpLastResponse>,
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

    fn client(&self) -> Result<Client, Error> {
        let mut state = self
            .state
            .lock()
            .map_err(|error| Error::msg(format!("plugin HTTP runtime lock poisoned: {error}")))?;
        if let Some(client) = &state.cached_client {
            return Ok(client.clone());
        }

        let client = scryer_outbound_http::blocking_plugin_host_client(&state.extra_ca_bundle_pem)
            .map_err(Error::msg)?;
        state.cached_client = Some(client.clone());
        Ok(client)
    }
}

pub(crate) fn host_functions_with_indexer_proxy(
    manifest: &Manifest,
    indexer_proxy_policy: Option<IndexerProxyPolicy>,
) -> Vec<Function> {
    let state = UserData::new(PluginHttpHostState {
        runtime: shared_plugin_http_runtime(),
        allowed_hosts: manifest.allowed_hosts.clone(),
        indexer_proxy_policy,
        max_http_response_bytes: manifest.memory.max_http_response_bytes,
        last_responses: HashMap::new(),
    });

    vec![
        Function::new(
            "http_request",
            [ValType::I64, ValType::I64],
            [ValType::I64],
            state.clone(),
            plugin_http_request,
        )
        .with_namespace(HTTP_ENV_NAMESPACE),
        Function::new(
            "http_status_code",
            [],
            [ValType::I32],
            state.clone(),
            plugin_http_status_code,
        )
        .with_namespace(HTTP_ENV_NAMESPACE),
        Function::new(
            "http_headers",
            [],
            [ValType::I64],
            state,
            plugin_http_headers,
        )
        .with_namespace(HTTP_ENV_NAMESPACE),
    ]
}

fn plugin_http_request(
    current: &mut CurrentPlugin,
    input: &[Val],
    output: &mut [Val],
    state: UserData<PluginHttpHostState>,
) -> Result<(), Error> {
    output[0] = Val::I64(0);

    let plugin_id = current.id().to_string();
    let request_offset = input.first().and_then(Val::i64).unwrap_or(0) as u64;
    let body_offset = input.get(1).and_then(Val::i64).unwrap_or(0) as u64;

    let request_handle = current.memory_handle(request_offset).ok_or_else(|| {
        Error::msg(format!(
            "invalid handle offset for http request: {request_offset}"
        ))
    })?;
    let request: HttpRequest = serde_json::from_slice(current.memory_bytes(request_handle)?)?;
    current.memory_free(request_handle)?;

    let body = if body_offset > 0 {
        let body_handle = current.memory_handle(body_offset).ok_or_else(|| {
            Error::msg(format!(
                "invalid handle offset for http request body: {request_offset}"
            ))
        })?;
        let bytes = current.memory_bytes(body_handle)?.to_vec();
        current.memory_free(body_handle)?;
        Some(bytes)
    } else {
        None
    };

    let host_state = state
        .get()
        .map_err(|error| Error::msg(format!("plugin HTTP host state unavailable: {error}")))?;
    let (runtime, allowed_hosts, indexer_proxy_policy, max_http_response_bytes) = {
        let mut host_state = host_state.lock().map_err(|error| {
            Error::msg(format!("plugin HTTP host state lock poisoned: {error}"))
        })?;
        host_state
            .last_responses
            .insert(plugin_id.clone(), PluginHttpLastResponse::default());
        (
            host_state.runtime.clone(),
            host_state.allowed_hosts.clone(),
            host_state.indexer_proxy_policy.clone(),
            host_state.max_http_response_bytes,
        )
    };

    enforce_allowed_hosts(allowed_hosts.as_deref(), &request.url)?;

    let client = runtime.client()?;
    let timeout = current.time_remaining();
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
            solver::SolvedSessionCache::shared().session_headers(&policy.config.id, &request.url)
        })
        .unwrap_or_default();
    let response = execute_request_with_extra_headers(
        &client,
        &request,
        body.clone(),
        timeout,
        &session_headers,
    )?;
    let status = response.status();
    let status_code = status.as_u16();
    let headers = response_headers(&response);

    if status == StatusCode::TOO_MANY_REQUESTS {
        store_last_response(&host_state, &plugin_id, status_code, headers)?;
        tracing::debug!(
            plugin_id = plugin_id.as_str(),
            status = status_code,
            elapsed_ms = started_at.elapsed().as_millis() as u64,
            response_bytes = 0_u64,
            "plugin HTTP request skipped indexer proxy after direct rate limit"
        );
        return Ok(());
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
            return Err(Error::msg(format!(
                "indexer proxy only supports GET challenge solving for plugin HTTP requests; got {method}"
            )));
        }
        if !session_headers.is_empty() {
            // The cached session no longer clears the challenge.
            solver::SolvedSessionCache::shared().invalidate(&policy.config.id, &request.url);
        }

        tracing::debug!(
            plugin_id = plugin_id.as_str(),
            indexer_id = policy.indexer_id.as_str(),
            indexer_name = policy.indexer_name.as_str(),
            proxy_config_id = policy.config.id.as_str(),
            status = status_code,
            request_url = solver::sanitized_url_for_log(&request.url).as_str(),
            "plugin HTTP request detected browser challenge"
        );

        let solved = match execute_byparr_request(
            &client,
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
                let message = error.to_string();
                if solver::is_solver_service_error_message(&message) {
                    solver::SolverHealthLedger::shared()
                        .record_failure(&policy.config.id, &message);
                }
                return Err(error);
            }
        };
        let response_bytes = solved.body.len();
        store_last_response(&host_state, &plugin_id, solved.status_code, solved.headers)?;
        tracing::debug!(
            plugin_id = plugin_id.as_str(),
            indexer_id = policy.indexer_id.as_str(),
            proxy_config_id = policy.config.id.as_str(),
            status = solved.status_code,
            elapsed_ms = started_at.elapsed().as_millis() as u64,
            response_bytes,
            "plugin HTTP request completed through indexer proxy"
        );
        current.memory_set_val(&mut output[0], solved.body)?;
        return Ok(());
    }

    let response_bytes = direct_body.len();
    store_last_response(&host_state, &plugin_id, status_code, headers)?;
    tracing::debug!(
        plugin_id = plugin_id.as_str(),
        status = status_code,
        elapsed_ms = started_at.elapsed().as_millis() as u64,
        response_bytes,
        "plugin HTTP request completed"
    );
    if status.is_success() {
        current.memory_set_val(&mut output[0], direct_body)?;
    }
    Ok(())
}

fn plugin_http_status_code(
    current: &mut CurrentPlugin,
    _input: &[Val],
    output: &mut [Val],
    state: UserData<PluginHttpHostState>,
) -> Result<(), Error> {
    let host_state = state
        .get()
        .map_err(|error| Error::msg(format!("plugin HTTP host state unavailable: {error}")))?;
    let host_state = host_state
        .lock()
        .map_err(|error| Error::msg(format!("plugin HTTP host state lock poisoned: {error}")))?;
    let status_code = host_state
        .last_responses
        .get(&current.id().to_string())
        .map(|response| response.status_code)
        .unwrap_or(0);
    output[0] = Val::I32(status_code as i32);
    Ok(())
}

fn plugin_http_headers(
    current: &mut CurrentPlugin,
    _input: &[Val],
    output: &mut [Val],
    state: UserData<PluginHttpHostState>,
) -> Result<(), Error> {
    let host_state = state
        .get()
        .map_err(|error| Error::msg(format!("plugin HTTP host state unavailable: {error}")))?;
    let host_state = host_state
        .lock()
        .map_err(|error| Error::msg(format!("plugin HTTP host state lock poisoned: {error}")))?;

    if let Some(headers) = host_state.last_responses.get(&current.id().to_string()) {
        if headers.headers.is_empty() {
            output[0] = Val::I64(0);
        } else {
            current.memory_set_val(&mut output[0], serde_json::to_string(&headers.headers)?)?;
        }
        return Ok(());
    }

    output[0] = Val::I64(0);
    Ok(())
}

fn enforce_allowed_hosts(allowed_hosts: Option<&[String]>, request_url: &str) -> Result<(), Error> {
    let url = url::Url::parse(request_url)
        .map_err(|error| Error::msg(format!("Invalid URL: {error:?}")))?;
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

    Err(Error::msg(format!(
        "HTTP request to {} is not allowed",
        request_url
    )))
}

fn execute_request_with_extra_headers(
    client: &Client,
    request: &HttpRequest,
    body: Option<Vec<u8>>,
    timeout: Option<Duration>,
    extra_headers: &[(String, String)],
) -> Result<reqwest::blocking::Response, Error> {
    let method = Method::from_bytes(
        request
            .method
            .as_deref()
            .unwrap_or("GET")
            .to_uppercase()
            .as_bytes(),
    )
    .map_err(|error| Error::msg(format!("Invalid HTTP method: {error}")))?;

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
            Error::msg("timeout")
        } else {
            Error::msg(error.to_string())
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
) -> Result<Vec<u8>, Error> {
    let mut body = Vec::new();
    let max = max_http_response_bytes.unwrap_or(DEFAULT_MAX_HTTP_RESPONSE_BYTES);
    response
        .take(max + 1)
        .read_to_end(&mut body)
        .map_err(|error| Error::msg(error.to_string()))?;
    if body.len() > max as usize {
        return Err(Error::msg(format!(
            "HTTP response exceeds the configured maximum number of bytes: {max}"
        )));
    }
    Ok(body)
}

fn store_last_response(
    host_state: &Arc<Mutex<PluginHttpHostState>>,
    plugin_id: &str,
    status_code: u16,
    headers: BTreeMap<String, String>,
) -> Result<(), Error> {
    let mut host_state = host_state
        .lock()
        .map_err(|error| Error::msg(format!("plugin HTTP host state lock poisoned: {error}")))?;
    host_state.last_responses.insert(
        plugin_id.to_string(),
        PluginHttpLastResponse {
            status_code,
            headers,
        },
    );
    Ok(())
}

fn execute_byparr_request(
    client: &Client,
    policy: &IndexerProxyPolicy,
    request: &HttpRequest,
    original_body: Option<Vec<u8>>,
    original_timeout: Option<Duration>,
    max_http_response_bytes: Option<u64>,
) -> Result<ProxiedHttpResponse, Error> {
    if policy.config.provider_type != scryer_domain::IndexerProxyProviderType::Byparr {
        return Err(Error::msg("unsupported indexer proxy provider"));
    }
    if !policy.config.is_enabled {
        return Err(Error::msg("Indexer proxy is disabled for this indexer."));
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

    let response = client
        .post(&endpoint)
        .timeout(byparr_timeout)
        .json(&solver::byparr_solve_request(
            &request.url,
            policy.config.request_timeout_seconds,
        ))
        .send()
        .map_err(|error| {
            if error.is_timeout() {
                Error::msg(solver::BYPARR_TIMEOUT_MESSAGE)
            } else {
                Error::msg(solver::BYPARR_UNREACHABLE_MESSAGE)
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
        return Err(Error::msg(solver::BYPARR_UNAVAILABLE_MESSAGE));
    }

    let response_body = read_response_body(response, max_http_response_bytes)?;
    let solution = solver::parse_byparr_solution(&response_body)
        .map_err(|error| Error::msg(error.message()))?;

    let solution_status = solution.status.unwrap_or_else(|| byparr_status.as_u16());
    let solved_final_url = solution.url.as_deref().map(solver::sanitized_url_for_log);
    if solution_status == StatusCode::TOO_MANY_REQUESTS.as_u16() {
        tracing::warn!(
            indexer_id = policy.indexer_id.as_str(),
            proxy_config_id = policy.config.id.as_str(),
            status = solution_status,
            "Byparr reported target indexer rate limit"
        );
        return Err(Error::msg(solver::target_rate_limit_message(&solution)));
    }

    let solved_body = solution.response.clone().unwrap_or_default().into_bytes();
    if solver::solved_body_looks_rate_limited(&solved_body) {
        return Err(Error::msg(solver::target_rate_limit_message(&solution)));
    }
    if !(200..300).contains(&solution_status) {
        return Err(Error::msg(format!(
            "Byparr target request returned HTTP {solution_status}."
        )));
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
            client,
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
            return Err(Error::msg(solver::rate_limit_message_with_retry_after(
                retry_after,
            )));
        }
        if !status.is_success() {
            return Err(Error::msg(solver::BYPARR_NO_SOLUTION_MESSAGE));
        }
        let body = read_response_body(retry, max_http_response_bytes)?;
        return Ok(ProxiedHttpResponse {
            status_code: status.as_u16(),
            headers,
            body,
        });
    }

    Err(Error::msg(solver::BYPARR_NO_SOLUTION_MESSAGE))
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
}
