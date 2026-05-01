use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SocketTlsMode {
    Plain,
    Starttls,
    Tls,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SocketPermission {
    pub host_pattern: String,
    pub ports: Vec<u16>,
    pub tls_modes: Vec<SocketTlsMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SocketErrorCode {
    PermissionDenied,
    DnsFailed,
    ConnectTimeout,
    IoFailed,
    TlsVerificationFailed,
    StartTlsFailed,
    AuthFailed,
    RemoteClosed,
    ProtocolError,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SocketError {
    pub code: SocketErrorCode,
    pub message: String,
}

impl SocketError {
    pub fn new(code: SocketErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for SocketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for SocketError {}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
pub struct SocketResponse<T> {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<T>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<SocketError>,
}

impl<T> SocketResponse<T> {
    pub fn ok(value: T) -> Self {
        Self {
            ok: true,
            value: Some(value),
            error: None,
        }
    }

    pub fn error(code: SocketErrorCode, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            value: None,
            error: Some(SocketError::new(code, message)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SocketOpenRequest {
    pub host: String,
    pub port: u16,
    pub tls_mode: SocketTlsMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect_timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SocketOpenResponse {
    pub handle: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SocketReadRequest {
    pub handle: u32,
    pub max_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SocketReadResponse {
    pub data_base64: String,
    pub eof: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SocketWriteRequest {
    pub handle: u32,
    pub data_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SocketWriteResponse {
    pub bytes_written: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SocketStartTlsRequest {
    pub handle: u32,
    pub host: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SocketStartTlsResponse {
    pub handle: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SocketCloseRequest {
    pub handle: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SocketCloseResponse {
    pub closed: bool,
}

pub type SocketResult<T> = Result<T, SocketError>;

#[cfg(target_arch = "wasm32")]
fn encode_request<T: Serialize>(request: &T) -> SocketResult<String> {
    serde_json::to_string(request).map_err(|error| {
        SocketError::new(
            SocketErrorCode::ProtocolError,
            format!("failed to encode socket request: {error}"),
        )
    })
}

#[cfg(target_arch = "wasm32")]
fn decode_response<T: for<'de> Deserialize<'de>>(raw: &str) -> SocketResult<T> {
    let response: SocketResponse<T> = serde_json::from_str(raw).map_err(|error| {
        SocketError::new(
            SocketErrorCode::ProtocolError,
            format!("failed to decode socket response: {error}"),
        )
    })?;

    if response.ok {
        response.value.ok_or_else(|| {
            SocketError::new(
                SocketErrorCode::ProtocolError,
                "socket response was successful but missing a value",
            )
        })
    } else {
        Err(response.error.unwrap_or_else(|| {
            SocketError::new(
                SocketErrorCode::ProtocolError,
                "socket response failed without an error",
            )
        }))
    }
}

#[cfg(target_arch = "wasm32")]
mod guest {
    use super::*;
    use extism_pdk::host_fn;

    #[host_fn]
    extern "ExtismHost" {
        fn scryer_socket_open(input: String) -> String;
        fn scryer_socket_read(input: String) -> String;
        fn scryer_socket_write(input: String) -> String;
        fn scryer_socket_starttls(input: String) -> String;
        fn scryer_socket_close(input: String) -> String;
    }

    fn call_host<T: for<'de> Deserialize<'de>>(
        request: impl Serialize,
        f: unsafe fn(String) -> Result<String, extism_pdk::Error>,
    ) -> SocketResult<T> {
        let input = encode_request(&request)?;
        let raw = unsafe { f(input) }.map_err(|error| {
            SocketError::new(
                SocketErrorCode::ProtocolError,
                format!("socket host function failed: {error}"),
            )
        })?;
        decode_response(&raw)
    }

    pub fn socket_open(request: SocketOpenRequest) -> SocketResult<SocketOpenResponse> {
        call_host(request, scryer_socket_open)
    }

    pub fn socket_read(request: SocketReadRequest) -> SocketResult<SocketReadResponse> {
        call_host(request, scryer_socket_read)
    }

    pub fn socket_write(request: SocketWriteRequest) -> SocketResult<SocketWriteResponse> {
        call_host(request, scryer_socket_write)
    }

    pub fn socket_starttls(request: SocketStartTlsRequest) -> SocketResult<SocketStartTlsResponse> {
        call_host(request, scryer_socket_starttls)
    }

    pub fn socket_close(request: SocketCloseRequest) -> SocketResult<SocketCloseResponse> {
        call_host(request, scryer_socket_close)
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod guest {
    use super::*;

    fn unsupported<T>() -> SocketResult<T> {
        Err(SocketError::new(
            SocketErrorCode::Unsupported,
            "socket host functions are only available to wasm plugins",
        ))
    }

    pub fn socket_open(_request: SocketOpenRequest) -> SocketResult<SocketOpenResponse> {
        unsupported()
    }

    pub fn socket_read(_request: SocketReadRequest) -> SocketResult<SocketReadResponse> {
        unsupported()
    }

    pub fn socket_write(_request: SocketWriteRequest) -> SocketResult<SocketWriteResponse> {
        unsupported()
    }

    pub fn socket_starttls(
        _request: SocketStartTlsRequest,
    ) -> SocketResult<SocketStartTlsResponse> {
        unsupported()
    }

    pub fn socket_close(_request: SocketCloseRequest) -> SocketResult<SocketCloseResponse> {
        unsupported()
    }
}

pub use guest::{socket_close, socket_open, socket_read, socket_starttls, socket_write};
