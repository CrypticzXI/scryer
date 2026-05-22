use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::sync::{Arc, LazyLock, Mutex};

use extism::{CurrentPlugin, Error, Function, Manifest, UserData, Val, ValType};
use extism_manifest::HttpRequest;
use glob::Pattern;
use reqwest::Method;
use reqwest::blocking::Client;

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
    max_http_response_bytes: Option<u64>,
    last_responses: HashMap<String, PluginHttpLastResponse>,
}

#[derive(Clone, Default)]
struct PluginHttpLastResponse {
    status_code: u16,
    headers: BTreeMap<String, String>,
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

        let client = build_plugin_http_client(&state.extra_ca_bundle_pem).map_err(Error::msg)?;
        state.cached_client = Some(client.clone());
        Ok(client)
    }
}

pub(crate) fn host_functions(manifest: &Manifest) -> Vec<Function> {
    let state = UserData::new(PluginHttpHostState {
        runtime: shared_plugin_http_runtime(),
        allowed_hosts: manifest.allowed_hosts.clone(),
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
    let mut host_state = host_state
        .lock()
        .map_err(|error| Error::msg(format!("plugin HTTP host state lock poisoned: {error}")))?;
    host_state
        .last_responses
        .insert(plugin_id.clone(), PluginHttpLastResponse::default());

    enforce_allowed_hosts(host_state.allowed_hosts.as_deref(), &request.url)?;

    let client = host_state.runtime.client()?;
    let timeout = current.time_remaining();
    let response = execute_request(&client, &request, body, timeout)?;
    let status_code = response.status().as_u16();

    if response.status().is_success() {
        let headers = response_headers(&response);
        let body = read_response_body(response, host_state.max_http_response_bytes)?;
        host_state.last_responses.insert(
            plugin_id,
            PluginHttpLastResponse {
                status_code,
                headers,
            },
        );
        current.memory_set_val(&mut output[0], body)?;
        return Ok(());
    }

    host_state.last_responses.insert(
        plugin_id,
        PluginHttpLastResponse {
            status_code,
            headers: BTreeMap::new(),
        },
    );
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

fn execute_request(
    client: &Client,
    request: &HttpRequest,
    body: Option<Vec<u8>>,
    timeout: Option<std::time::Duration>,
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
        builder = builder.header(name, value);
    }
    if let Some(timeout) = timeout {
        builder = builder.timeout(timeout);
    }
    if let Some(body) = body {
        builder = builder.body(body);
    }

    builder.send().map_err(|error| {
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

fn build_plugin_http_client(extra_ca_bundle_pem: &str) -> Result<Client, String> {
    scryer_outbound_http::install_default_rustls_provider();
    let mut builder = reqwest::blocking::Client::builder();
    if !extra_ca_bundle_pem.trim().is_empty() {
        builder = builder.tls_certs_merge(uploaded_root_certificates(extra_ca_bundle_pem)?);
    }
    builder
        .build()
        .map_err(|error| format!("failed to build plugin HTTP client: {error}"))
}

fn uploaded_root_certificates(bundle_pem: &str) -> Result<Vec<reqwest::Certificate>, String> {
    if bundle_pem.trim().is_empty() {
        return Ok(Vec::new());
    }

    let certificates = reqwest::Certificate::from_pem_bundle(bundle_pem.as_bytes())
        .map_err(|error| format!("failed to parse uploaded trusted certificate bundle: {error}"))?;
    if certificates.is_empty() {
        return Err(
            "uploaded trusted certificate bundle did not contain any X.509 certificates"
                .to_string(),
        );
    }
    Ok(certificates)
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
    fn add_uploaded_certificates_accepts_valid_pem_bundle() {
        let certificates = uploaded_root_certificates(TEST_PLUGIN_HTTP_CA_CERT_PEM)
            .expect("valid trusted certificate bundle");

        assert_eq!(certificates.len(), 1);
    }

    #[test]
    fn build_plugin_http_client_accepts_empty_trust_bundle() {
        build_plugin_http_client("").expect("default trust bundle should build");
    }

    #[test]
    fn build_plugin_http_client_accepts_uploaded_certificates() {
        build_plugin_http_client(TEST_PLUGIN_HTTP_CA_CERT_PEM)
            .expect("uploaded trust bundle should build");
    }

    #[test]
    fn add_uploaded_certificates_rejects_non_certificate_pem_items() {
        let error = uploaded_root_certificates(
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
