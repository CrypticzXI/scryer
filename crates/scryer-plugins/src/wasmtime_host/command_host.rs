//! Binary `scryer:host/v1` imports for native command guests.
//!
//! A host call creates a bounded response handle. Guests can obtain its length,
//! copy it into their own memory, then drop it. The command host starts
//! fail-closed: adapters opt services in only with their existing descriptor
//! policy and configuration rather than command artifacts inheriting ambient
//! WASI authority.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use scryer_plugin_sdk::host::{
    HOST_ABI_MODULE, PluginConfigGetResponse, PluginHostRequest, PluginHostResponse,
    PluginHttpBatchResponse, PluginHttpResponse, PluginStateGetResponse,
    PluginStateMutationResponse,
};
use scryer_plugin_sdk::{PluginError, PluginErrorCode, PluginResult};
use wasmtime::{Caller, Linker, Memory};

use crate::plugin_http_host::{
    IndexerErrorCaptureContext, IndexerProxyPolicy, PluginHttpHost, PluginHttpRequest,
};
use crate::wasmtime_host::sandbox::HostCtx;

const MAX_RESPONSE_HANDLES: usize = 32;
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_STATE_BYTES: usize = 1024 * 1024;
const MAX_HTTP_BATCH_REQUESTS: usize = 256;
const HOST_HTTP_START_RATE_CEILING: u64 = 100;

static NEXT_HTTP_BATCH_ID: AtomicU64 = AtomicU64::new(1);
static HTTP_BATCH_LIMITER: OnceLock<Mutex<HttpBatchLimiter>> = OnceLock::new();

#[derive(Default)]
struct HttpBatchLimiter {
    buckets: HashMap<String, HttpBatchLimiterBucket>,
}

#[derive(Default)]
struct HttpBatchLimiterBucket {
    next_start: Option<Instant>,
    active_spacings: HashMap<u64, Duration>,
}

struct HttpBatchRegistration {
    id: u64,
    keys: Vec<String>,
}

impl HttpBatchRegistration {
    fn wait_for_start(&self, key: &str) -> Result<(), String> {
        let scheduled = {
            let mut limiter = HTTP_BATCH_LIMITER
                .get_or_init(|| Mutex::new(HttpBatchLimiter::default()))
                .lock()
                .map_err(|error| format!("plugin HTTP batch limiter lock poisoned: {error}"))?;
            let bucket = limiter
                .buckets
                .get_mut(key)
                .ok_or_else(|| "plugin HTTP batch limiter registration disappeared".to_string())?;
            let spacing = bucket
                .active_spacings
                .values()
                .copied()
                .max()
                .ok_or_else(|| "plugin HTTP batch limiter has no active rate".to_string())?;
            let now = Instant::now();
            let scheduled = bucket.next_start.unwrap_or(now).max(now);
            bucket.next_start = Some(scheduled + spacing);
            scheduled
        };
        let delay = scheduled.saturating_duration_since(Instant::now());
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
        Ok(())
    }
}

impl Drop for HttpBatchRegistration {
    fn drop(&mut self) {
        let Ok(mut limiter) = HTTP_BATCH_LIMITER
            .get_or_init(|| Mutex::new(HttpBatchLimiter::default()))
            .lock()
        else {
            return;
        };
        for key in &self.keys {
            if let Some(bucket) = limiter.buckets.get_mut(key) {
                bucket.active_spacings.remove(&self.id);
                if bucket.active_spacings.is_empty() {
                    limiter.buckets.remove(key);
                }
            }
        }
    }
}

fn register_http_batch(
    plugin_id: &str,
    urls: impl IntoIterator<Item = String>,
    starts: u32,
    interval_ms: u64,
) -> Result<HttpBatchRegistration, String> {
    if starts == 0 || interval_ms == 0 {
        return Err("plugin HTTP batch start rate must be positive".to_string());
    }
    let requested_spacing = Duration::from_millis(
        interval_ms
            .saturating_add(u64::from(starts) - 1)
            .checked_div(u64::from(starts))
            .unwrap_or(interval_ms),
    );
    let host_spacing = Duration::from_millis(1_000 / HOST_HTTP_START_RATE_CEILING);
    let spacing = requested_spacing.max(host_spacing);
    let id = NEXT_HTTP_BATCH_ID.fetch_add(1, Ordering::Relaxed);
    let keys = urls
        .into_iter()
        .map(|url| {
            let parsed = url::Url::parse(&url)
                .map_err(|error| format!("invalid plugin HTTP batch URL: {error}"))?;
            let host = parsed
                .host_str()
                .ok_or_else(|| "plugin HTTP batch URL is missing a host".to_string())?;
            let origin = match parsed.port_or_known_default() {
                Some(port) => format!("{}://{host}:{port}", parsed.scheme()),
                None => format!("{}://{host}", parsed.scheme()),
            };
            Ok(format!("{plugin_id}|{origin}"))
        })
        .collect::<Result<BTreeSet<_>, String>>()?
        .into_iter()
        .collect::<Vec<_>>();
    let mut limiter = HTTP_BATCH_LIMITER
        .get_or_init(|| Mutex::new(HttpBatchLimiter::default()))
        .lock()
        .map_err(|error| format!("plugin HTTP batch limiter lock poisoned: {error}"))?;
    for key in &keys {
        limiter
            .buckets
            .entry(key.clone())
            .or_default()
            .active_spacings
            .insert(id, spacing);
    }
    drop(limiter);
    Ok(HttpBatchRegistration { id, keys })
}

fn http_batch_limiter_key(plugin_id: &str, url: &str) -> Result<String, String> {
    let parsed = url::Url::parse(url)
        .map_err(|error| format!("invalid plugin HTTP batch URL: {error}"))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| "plugin HTTP batch URL is missing a host".to_string())?;
    let origin = match parsed.port_or_known_default() {
        Some(port) => format!("{}://{host}:{port}", parsed.scheme()),
        None => format!("{}://{host}", parsed.scheme()),
    };
    Ok(format!("{plugin_id}|{origin}"))
}

#[derive(Clone)]
pub(crate) struct CommandHost {
    state: Arc<Mutex<CommandHostState>>,
    services: Option<Arc<CommandHostServices>>,
    request_deadline: Option<Instant>,
}

struct CommandHostState {
    next_handle: u32,
    responses: HashMap<u32, Vec<u8>>,
}

struct CommandHostServices {
    plugin_id: String,
    config: BTreeMap<String, String>,
    state: Mutex<CommandState>,
    http: PluginHttpHost,
    timeout: Duration,
}

#[derive(Default)]
struct CommandState {
    values: BTreeMap<String, Vec<u8>>,
    bytes: usize,
}

impl CommandHost {
    pub(crate) fn disabled() -> Self {
        Self {
            state: Arc::new(Mutex::new(CommandHostState {
                next_handle: 1,
                responses: HashMap::new(),
            })),
            services: None,
            request_deadline: None,
        }
    }

    pub(crate) fn for_download_client(
        plugin_id: String,
        config: BTreeMap<String, String>,
        allowed_hosts: Vec<String>,
        timeout: Duration,
        max_http_response_bytes: Option<u64>,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(CommandHostState {
                next_handle: 1,
                responses: HashMap::new(),
            })),
            services: Some(Arc::new(CommandHostServices {
                plugin_id,
                config,
                state: Mutex::new(CommandState::default()),
                http: PluginHttpHost::new(allowed_hosts, None, None, max_http_response_bytes),
                timeout,
            })),
            request_deadline: None,
        }
    }

    /// Build the host services for a command-ABI indexer.
    ///
    /// Indexers differ from download clients in exactly two ways, and both live
    /// in egress: a configured indexer proxy has to wrap every request, and the
    /// managed-destination cooldown key has to be carried so a shared upstream
    /// (a Prowlarr parent, say) throttles as one destination rather than once
    /// per child. Everything else — descriptor-bound config, plugin state, the
    /// timeout — is identical, so this mirrors `for_download_client` rather
    /// than growing it another two arguments that every caller passes `None` to.
    pub(crate) fn for_indexer(
        plugin_id: String,
        config: BTreeMap<String, String>,
        allowed_hosts: Vec<String>,
        indexer_proxy_policy: Option<IndexerProxyPolicy>,
        destination_cooldown_key: Option<String>,
        timeout: Duration,
        max_http_response_bytes: Option<u64>,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(CommandHostState {
                next_handle: 1,
                responses: HashMap::new(),
            })),
            services: Some(Arc::new(CommandHostServices {
                plugin_id,
                config,
                state: Mutex::new(CommandState::default()),
                http: PluginHttpHost::new(
                    allowed_hosts,
                    indexer_proxy_policy,
                    destination_cooldown_key,
                    max_http_response_bytes,
                ),
                timeout,
            })),
            request_deadline: None,
        }
    }

    /// Clone this host for one command invocation and bind HTTP calls to the
    /// invocation's remaining wall-clock budget.
    pub(crate) fn for_invocation(&self, timeout: Duration) -> Self {
        let now = Instant::now();
        Self {
            state: Arc::clone(&self.state),
            services: self.services.clone(),
            request_deadline: Some(now.checked_add(timeout).unwrap_or(now)),
        }
    }

    fn remaining_http_timeout(&self, maximum: Duration) -> Result<Duration, String> {
        let Some(deadline) = self.request_deadline else {
            return Ok(maximum);
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("command plugin HTTP deadline exhausted".to_string());
        }
        Ok(remaining.min(maximum))
    }

    pub(crate) fn rate_limit_message(&self) -> Option<String> {
        let services = self.services.as_ref()?;
        services
            .http
            .rate_limit_message(&services.plugin_id)
            .ok()
            .flatten()
    }

    pub(crate) fn begin_indexer_error_capture(&self, context: IndexerErrorCaptureContext) {
        if let Some(services) = self.services.as_ref() {
            services.http.begin_indexer_error_capture(context);
        }
    }

    pub(crate) fn finish_indexer_error_capture(&self, operation_failed: bool) {
        if let Some(services) = self.services.as_ref() {
            services.http.finish_indexer_error_capture(operation_failed);
        }
    }

    fn call(&self, encoded_request: &[u8]) -> Result<u32, String> {
        let request: PluginHostRequest = postcard::from_bytes(encoded_request)
            .map_err(|error| format!("invalid postcard host request: {error}"))?;
        let response = self.service_request(request);
        let encoded = postcard::to_allocvec(&response)
            .map_err(|error| format!("failed to encode host response: {error}"))?;
        if encoded.len() > MAX_RESPONSE_BYTES {
            return Err(format!(
                "encoded host response exceeds {MAX_RESPONSE_BYTES} bytes"
            ));
        }
        let mut state = self
            .state
            .lock()
            .map_err(|error| format!("command host lock poisoned: {error}"))?;
        if state.responses.len() >= MAX_RESPONSE_HANDLES {
            return Err(format!(
                "command host response handle limit of {MAX_RESPONSE_HANDLES} reached"
            ));
        }
        let handle = state.next_handle;
        state.next_handle = state.next_handle.wrapping_add(1).max(1);
        state.responses.insert(handle, encoded);
        Ok(handle)
    }

    fn service_request(&self, request: PluginHostRequest) -> PluginHostResponse {
        let Some(services) = &self.services else {
            return unsupported_response(request);
        };
        match request {
            PluginHostRequest::ConfigGet(request) => {
                PluginHostResponse::ConfigGet(PluginResult::Ok(PluginConfigGetResponse {
                    value: services.config.get(&request.key).cloned(),
                }))
            }
            PluginHostRequest::StateGet(request) => {
                let result = services
                    .state
                    .lock()
                    .map_err(|error| error.to_string())
                    .map(|state| PluginStateGetResponse {
                        value: state.values.get(&request.key).cloned(),
                    });
                PluginHostResponse::StateGet(result.map_or_else(service_error, PluginResult::Ok))
            }
            PluginHostRequest::StateSet(request) => {
                let result = services
                    .state
                    .lock()
                    .map_err(|error| error.to_string())
                    .and_then(|mut state| set_state_value(&mut state, request.key, request.value));
                PluginHostResponse::StateSet(result.map_or_else(service_error, |changed| {
                    PluginResult::Ok(PluginStateMutationResponse { changed })
                }))
            }
            PluginHostRequest::StateDelete(request) => {
                let result = services
                    .state
                    .lock()
                    .map_err(|error| error.to_string())
                    .map(|mut state| {
                        let changed = state.values.remove(&request.key).is_some();
                        state.bytes = state
                            .values
                            .iter()
                            .map(|(key, value)| key.len() + value.len())
                            .sum();
                        changed
                    });
                PluginHostResponse::StateDelete(result.map_or_else(service_error, |changed| {
                    PluginResult::Ok(PluginStateMutationResponse { changed })
                }))
            }
            PluginHostRequest::Http(request) => {
                let response = self
                    .remaining_http_timeout(services.timeout)
                    .and_then(|timeout| {
                        services.http.request(
                            &services.plugin_id,
                            PluginHttpRequest {
                                url: request.url,
                                method: request.method,
                                headers: request.headers,
                            },
                            (!request.body.is_empty()).then_some(request.body),
                            timeout,
                        )
                    })
                    .and_then(|body| {
                        let status = services.http.status_code(&services.plugin_id)?;
                        Ok(PluginHttpResponse {
                            status,
                            headers: services
                                .http
                                .headers(&services.plugin_id)?
                                .unwrap_or_default(),
                            body,
                        })
                    });
                PluginHostResponse::Http(response.map_or_else(service_error, PluginResult::Ok))
            }
            PluginHostRequest::HttpBatch(batch) => {
                let response = (|| {
                    if batch.requests.len() > MAX_HTTP_BATCH_REQUESTS {
                        return Err(format!(
                            "plugin HTTP batch contains {} requests; maximum is {MAX_HTTP_BATCH_REQUESTS}",
                            batch.requests.len()
                        ));
                    }
                    let timeout = self.remaining_http_timeout(services.timeout)?;
                    let registration = register_http_batch(
                        &services.plugin_id,
                        batch.requests.iter().map(|request| request.url.clone()),
                        batch.desired_start_rate.starts,
                        batch.desired_start_rate.interval_ms,
                    )?;
                    let results = std::thread::scope(|scope| {
                        let mut handles = Vec::with_capacity(batch.requests.len());
                        for (index, request) in batch.requests.into_iter().enumerate() {
                            let key = http_batch_limiter_key(&services.plugin_id, &request.url)?;
                            registration.wait_for_start(&key)?;
                            let http = &services.http;
                            let item_plugin_id =
                                format!("{}#http-batch-{}-{index}", services.plugin_id, registration.id);
                            handles.push(scope.spawn(move || {
                                let result = http
                                    .request(
                                        &item_plugin_id,
                                        PluginHttpRequest {
                                            url: request.url,
                                            method: request.method,
                                            headers: request.headers,
                                        },
                                        (!request.body.is_empty()).then_some(request.body),
                                        timeout,
                                    )
                                    .and_then(|body| {
                                        let (status, headers) =
                                            http.take_response_metadata(&item_plugin_id)?;
                                        Ok(PluginHttpResponse {
                                            status,
                                            headers,
                                            body,
                                        })
                                    });
                                result.map_or_else(service_error, PluginResult::Ok)
                            }));
                        }
                        Ok::<_, String>(
                            handles
                                .into_iter()
                                .map(|handle| {
                                    handle.join().unwrap_or_else(|_| {
                                        service_error("plugin HTTP batch worker panicked".to_string())
                                    })
                                })
                                .collect::<Vec<_>>(),
                        )
                    })?;
                    Ok(PluginHttpBatchResponse { results })
                })();
                PluginHostResponse::HttpBatch(
                    response.map_or_else(service_error, PluginResult::Ok),
                )
            }
            request => unsupported_response(request),
        }
    }

    fn response_len(&self, handle: u32) -> Option<usize> {
        self.state.lock().ok()?.responses.get(&handle).map(Vec::len)
    }

    fn response(&self, handle: u32) -> Option<Vec<u8>> {
        self.state.lock().ok()?.responses.get(&handle).cloned()
    }

    fn drop_response(&self, handle: u32) {
        if let Ok(mut state) = self.state.lock() {
            state.responses.remove(&handle);
        }
    }
}

fn set_state_value(state: &mut CommandState, key: String, value: Vec<u8>) -> Result<bool, String> {
    let prior = state
        .values
        .get(&key)
        .map(|value| key.len() + value.len())
        .unwrap_or(0);
    let next = state
        .bytes
        .checked_sub(prior)
        .and_then(|bytes| bytes.checked_add(key.len() + value.len()))
        .ok_or_else(|| "command plugin state byte accounting overflow".to_string())?;
    if next > MAX_STATE_BYTES {
        return Err(format!(
            "command plugin state exceeds {MAX_STATE_BYTES} bytes"
        ));
    }
    state.values.insert(key, value);
    state.bytes = next;
    Ok(true)
}

fn unsupported_error() -> PluginError {
    PluginError {
        code: PluginErrorCode::Unsupported,
        public_message: "this command plugin host service is not configured".to_string(),
        // `PluginError` predates the postcard host ABI and omits `None` fields
        // during serialization. Keep its optional fields present on this ABI.
        debug_message: Some(String::new()),
        retry_after_seconds: Some(0),
        details: None,
    }
}

fn unsupported<T>() -> PluginResult<T> {
    PluginResult::Err(unsupported_error())
}

fn service_error<T>(message: String) -> PluginResult<T> {
    PluginResult::Err(PluginError {
        code: PluginErrorCode::Temporary,
        public_message: "command plugin host service failed".to_string(),
        debug_message: Some(message),
        retry_after_seconds: Some(0),
        details: None,
    })
}

fn unsupported_response(request: PluginHostRequest) -> PluginHostResponse {
    match request {
        PluginHostRequest::ConfigGet(_) => PluginHostResponse::ConfigGet(unsupported()),
        PluginHostRequest::StateGet(_) => PluginHostResponse::StateGet(unsupported()),
        PluginHostRequest::StateSet(_) => PluginHostResponse::StateSet(unsupported()),
        PluginHostRequest::StateDelete(_) => PluginHostResponse::StateDelete(unsupported()),
        PluginHostRequest::Http(_) => PluginHostResponse::Http(unsupported()),
        PluginHostRequest::HttpBatch(_) => PluginHostResponse::HttpBatch(unsupported()),
        PluginHostRequest::SocketOpen(_) => PluginHostResponse::SocketOpen(unsupported()),
        PluginHostRequest::SocketRead(_) => PluginHostResponse::SocketRead(unsupported()),
        PluginHostRequest::SocketWrite(_) => PluginHostResponse::SocketWrite(unsupported()),
        PluginHostRequest::SocketStartTls(_) => PluginHostResponse::SocketStartTls(unsupported()),
        PluginHostRequest::SocketClose(_) => PluginHostResponse::SocketClose(unsupported()),
        PluginHostRequest::ProcessExec(_) => PluginHostResponse::ProcessExec(unsupported()),
    }
}

pub(crate) fn add_to_linker(linker: &mut Linker<HostCtx>) -> wasmtime::Result<()> {
    linker.func_wrap_async(HOST_ABI_MODULE, "scryer_host_call", host_call)?;
    linker.func_wrap(
        HOST_ABI_MODULE,
        "scryer_host_response_len",
        host_response_len,
    )?;
    linker.func_wrap(
        HOST_ABI_MODULE,
        "scryer_host_response_read",
        host_response_read,
    )?;
    linker.func_wrap(
        HOST_ABI_MODULE,
        "scryer_host_response_drop",
        host_response_drop,
    )?;
    Ok(())
}

fn host_call(
    mut caller: Caller<'_, HostCtx>,
    (request_ptr, request_len): (i32, i32),
) -> Box<dyn std::future::Future<Output = i32> + Send + '_> {
    let request = read_memory(&mut caller, request_ptr, request_len);
    let command_host = caller.data().command_host.clone();
    Box::new(async move {
        let Ok(request) = request else {
            return 0;
        };
        tokio::task::spawn_blocking(move || command_host.call(&request))
            .await
            .ok()
            .and_then(Result::ok)
            .and_then(|handle| i32::try_from(handle).ok())
            .unwrap_or(0)
    })
}

fn host_response_len(caller: Caller<'_, HostCtx>, handle: i32) -> i32 {
    let Ok(handle) = u32::try_from(handle) else {
        return -1;
    };
    caller
        .data()
        .command_host
        .response_len(handle)
        .and_then(|len| i32::try_from(len).ok())
        .unwrap_or(-1)
}

fn host_response_read(
    mut caller: Caller<'_, HostCtx>,
    handle: i32,
    destination_ptr: i32,
    destination_len: i32,
) -> i32 {
    let Ok(handle) = u32::try_from(handle) else {
        return -1;
    };
    let Some(response) = caller.data().command_host.response(handle) else {
        return -1;
    };
    if response.len() > usize::try_from(destination_len).unwrap_or(0) {
        return -1;
    }
    if write_memory(&mut caller, destination_ptr, &response).is_err() {
        return -1;
    }
    i32::try_from(response.len()).unwrap_or(-1)
}

fn host_response_drop(mut caller: Caller<'_, HostCtx>, handle: i32) {
    if let Ok(handle) = u32::try_from(handle) {
        caller.data_mut().command_host.drop_response(handle);
    }
}

fn memory(caller: &mut Caller<'_, HostCtx>) -> Result<Memory, String> {
    caller
        .get_export("memory")
        .and_then(|export| export.into_memory())
        .ok_or_else(|| "command plugin did not export memory".to_string())
}

fn checked_range(pointer: i32, len: i32) -> Result<(usize, usize), String> {
    let start = usize::try_from(pointer).map_err(|_| "negative memory pointer".to_string())?;
    let len = usize::try_from(len).map_err(|_| "negative memory length".to_string())?;
    let end = start
        .checked_add(len)
        .ok_or_else(|| "memory range overflow".to_string())?;
    Ok((start, end))
}

fn read_memory(
    caller: &mut Caller<'_, HostCtx>,
    pointer: i32,
    len: i32,
) -> Result<Vec<u8>, String> {
    let (start, end) = checked_range(pointer, len)?;
    let memory = memory(caller)?;
    if end > memory.data_size(&*caller) {
        return Err("memory range is out of bounds".to_string());
    }
    let mut bytes = vec![0; end - start];
    memory
        .read(&*caller, start, &mut bytes)
        .map_err(|error| format!("failed to read guest memory: {error}"))?;
    Ok(bytes)
}

fn write_memory(
    caller: &mut Caller<'_, HostCtx>,
    pointer: i32,
    bytes: &[u8],
) -> Result<(), String> {
    let start = usize::try_from(pointer).map_err(|_| "negative memory pointer".to_string())?;
    let end = start
        .checked_add(bytes.len())
        .ok_or_else(|| "memory range overflow".to_string())?;
    let memory = memory(caller)?;
    if end > memory.data_size(&*caller) {
        return Err("memory range is out of bounds".to_string());
    }
    memory
        .write(&mut *caller, start, bytes)
        .map_err(|error| format!("failed to write guest memory: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invocation_http_budget_uses_only_remaining_time() {
        let host = CommandHost::disabled();
        assert_eq!(
            host.remaining_http_timeout(Duration::from_secs(5))
                .expect("unbound host uses its configured maximum"),
            Duration::from_secs(5)
        );

        let active = host.for_invocation(Duration::from_secs(30));
        let remaining = active
            .remaining_http_timeout(Duration::from_secs(5))
            .expect("active invocation retains a request budget");
        assert!(!remaining.is_zero());
        assert!(remaining <= Duration::from_secs(5));

        let expired = host.for_invocation(Duration::ZERO);
        assert_eq!(
            expired
                .remaining_http_timeout(Duration::from_secs(5))
                .unwrap_err(),
            "command plugin HTTP deadline exhausted"
        );
    }

    #[test]
    fn overlapping_http_batches_share_the_most_restrictive_active_rate() {
        let url = "https://batch-rate.example.test/search";
        let key = http_batch_limiter_key("batch-rate-test", url).expect("valid limiter key");
        let three_per_second = register_http_batch(
            "batch-rate-test",
            [url.to_string()],
            3,
            1_000,
        )
        .expect("first batch should register");
        let two_per_second = register_http_batch(
            "batch-rate-test",
            [url.to_string()],
            2,
            1_000,
        )
        .expect("overlapping batch should register");

        let effective_spacing = HTTP_BATCH_LIMITER
            .get_or_init(|| Mutex::new(HttpBatchLimiter::default()))
            .lock()
            .expect("limiter lock")
            .buckets
            .get(&key)
            .expect("shared bucket")
            .active_spacings
            .values()
            .copied()
            .max();
        assert_eq!(effective_spacing, Some(Duration::from_millis(500)));

        drop(two_per_second);
        let effective_spacing = HTTP_BATCH_LIMITER
            .get_or_init(|| Mutex::new(HttpBatchLimiter::default()))
            .lock()
            .expect("limiter lock")
            .buckets
            .get(&key)
            .expect("remaining bucket")
            .active_spacings
            .values()
            .copied()
            .max();
        assert_eq!(effective_spacing, Some(Duration::from_millis(334)));

        drop(three_per_second);
        assert!(
            !HTTP_BATCH_LIMITER
                .get_or_init(|| Mutex::new(HttpBatchLimiter::default()))
                .lock()
                .expect("limiter lock")
                .buckets
                .contains_key(&key)
        );
    }

    #[test]
    fn disabled_host_returns_typed_unsupported_response() {
        let host = CommandHost::disabled();
        let request = postcard::to_allocvec(&PluginHostRequest::ConfigGet(
            scryer_plugin_sdk::host::PluginConfigGetRequest {
                key: "base_url".to_string(),
            },
        ))
        .unwrap();
        let handle = host.call(&request).unwrap();
        let response: PluginHostResponse =
            postcard::from_bytes(&host.response(handle).unwrap()).unwrap();
        assert!(matches!(
            response,
            PluginHostResponse::ConfigGet(PluginResult::Err(PluginError {
                code: PluginErrorCode::Unsupported,
                ..
            }))
        ));
    }
}
