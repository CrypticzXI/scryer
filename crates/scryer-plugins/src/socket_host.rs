use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use extism::{Function, UserData, ValType, host_fn};
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use crate::types::{
    PluginDescriptor, SocketCloseRequest, SocketCloseResponse, SocketError, SocketErrorCode,
    SocketOpenRequest, SocketOpenResponse, SocketPermission, SocketReadRequest, SocketReadResponse,
    SocketResponse, SocketStartTlsRequest, SocketStartTlsResponse, SocketTlsMode,
    SocketWriteRequest, SocketWriteResponse, allowed_host_pattern_is_valid,
    socket_host_pattern_config_key,
};

const SOCKET_HOST_NAMESPACE: &str = "extism:host/user";
const MAX_OPEN_SOCKETS: usize = 4;
const MAX_READ_BYTES: usize = 64 * 1024;
const MAX_WRITE_BYTES: usize = 64 * 1024;
const MAX_TOTAL_READ_BYTES: usize = 2 * 1024 * 1024;
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_WRITE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub(crate) struct SocketHost {
    state: UserData<SocketHostState>,
}

impl SocketHost {
    pub(crate) fn disabled() -> Self {
        Self {
            state: UserData::new(SocketHostState::new(Vec::new())),
        }
    }

    pub(crate) fn from_descriptor(
        descriptor: &PluginDescriptor,
        config_json: Option<&str>,
    ) -> Self {
        Self {
            state: UserData::new(SocketHostState::new(resolve_permissions(
                &descriptor.socket_permissions,
                config_json,
            ))),
        }
    }

    pub(crate) fn functions(&self) -> Vec<Function> {
        let params = || [ValType::I64];
        let results = || [ValType::I64];

        vec![
            Function::new(
                "scryer_socket_open",
                params(),
                results(),
                self.state.clone(),
                scryer_socket_open,
            )
            .with_namespace(SOCKET_HOST_NAMESPACE),
            Function::new(
                "scryer_socket_read",
                params(),
                results(),
                self.state.clone(),
                scryer_socket_read,
            )
            .with_namespace(SOCKET_HOST_NAMESPACE),
            Function::new(
                "scryer_socket_write",
                params(),
                results(),
                self.state.clone(),
                scryer_socket_write,
            )
            .with_namespace(SOCKET_HOST_NAMESPACE),
            Function::new(
                "scryer_socket_starttls",
                params(),
                results(),
                self.state.clone(),
                scryer_socket_starttls,
            )
            .with_namespace(SOCKET_HOST_NAMESPACE),
            Function::new(
                "scryer_socket_close",
                params(),
                results(),
                self.state.clone(),
                scryer_socket_close,
            )
            .with_namespace(SOCKET_HOST_NAMESPACE),
        ]
    }

    pub(crate) fn cleanup(&self) {
        if let Ok(state) = self.state.get()
            && let Ok(mut state) = state.lock()
        {
            state.cleanup();
        }
    }
}

#[derive(Debug)]
struct SocketHostState {
    permissions: Vec<ResolvedSocketPermission>,
    sockets: HashMap<u32, OpenSocket>,
    next_handle: u32,
    total_read_bytes: usize,
}

impl SocketHostState {
    fn new(permissions: Vec<ResolvedSocketPermission>) -> Self {
        Self {
            permissions,
            sockets: HashMap::new(),
            next_handle: 1,
            total_read_bytes: 0,
        }
    }

    fn cleanup(&mut self) {
        self.sockets.clear();
        self.total_read_bytes = 0;
    }

    fn open(&mut self, request: SocketOpenRequest) -> Result<SocketOpenResponse, SocketError> {
        let host = normalize_host(&request.host);
        if host.is_empty() {
            return Err(socket_error(
                SocketErrorCode::ProtocolError,
                "socket host must not be empty",
            ));
        }

        if self.sockets.len() >= MAX_OPEN_SOCKETS {
            return Err(socket_error(
                SocketErrorCode::PermissionDenied,
                format!("socket handle limit of {MAX_OPEN_SOCKETS} reached"),
            ));
        }

        if !self.allows(&host, request.port, request.tls_mode) {
            return Err(socket_error(
                SocketErrorCode::PermissionDenied,
                format!(
                    "socket permission denied for {host}:{} using {:?}",
                    request.port, request.tls_mode
                ),
            ));
        }

        let read_timeout = timeout_or_default(request.read_timeout_ms, DEFAULT_READ_TIMEOUT);
        let write_timeout = timeout_or_default(request.write_timeout_ms, DEFAULT_WRITE_TIMEOUT);
        let connect_timeout =
            timeout_or_default(request.connect_timeout_ms, DEFAULT_CONNECT_TIMEOUT);

        let stream = connect_tcp(
            &host,
            request.port,
            connect_timeout,
            read_timeout,
            write_timeout,
        )?;
        let stream = match request.tls_mode {
            SocketTlsMode::Plain | SocketTlsMode::Starttls => SocketStream::Plain(stream),
            SocketTlsMode::Tls => SocketStream::Tls(Box::new(upgrade_tls(stream, &host)?)),
        };

        let handle = self.allocate_handle();
        self.sockets.insert(
            handle,
            OpenSocket {
                host,
                port: request.port,
                mode: request.tls_mode,
                stream,
            },
        );

        Ok(SocketOpenResponse { handle })
    }

    fn read(&mut self, request: SocketReadRequest) -> Result<SocketReadResponse, SocketError> {
        if request.max_bytes == 0 {
            return Err(socket_error(
                SocketErrorCode::ProtocolError,
                "socket read max_bytes must be greater than zero",
            ));
        }
        if self.total_read_bytes >= MAX_TOTAL_READ_BYTES {
            return Err(socket_error(
                SocketErrorCode::ProtocolError,
                format!("socket total read limit of {MAX_TOTAL_READ_BYTES} bytes exceeded"),
            ));
        }

        let max_remaining = MAX_TOTAL_READ_BYTES - self.total_read_bytes;
        let max_bytes = request.max_bytes.min(MAX_READ_BYTES).min(max_remaining);
        let socket = self.socket_mut(request.handle)?;
        let mut buffer = vec![0_u8; max_bytes];
        let bytes_read = socket.stream.read(&mut buffer).map_err(map_io_error)?;
        self.total_read_bytes += bytes_read;
        buffer.truncate(bytes_read);

        Ok(SocketReadResponse {
            data_base64: STANDARD.encode(buffer),
            eof: bytes_read == 0,
        })
    }

    fn write(&mut self, request: SocketWriteRequest) -> Result<SocketWriteResponse, SocketError> {
        let data = STANDARD
            .decode(request.data_base64.as_bytes())
            .map_err(|error| {
                socket_error(
                    SocketErrorCode::ProtocolError,
                    format!("failed to decode socket write payload: {error}"),
                )
            })?;
        if data.len() > MAX_WRITE_BYTES {
            return Err(socket_error(
                SocketErrorCode::ProtocolError,
                format!("socket write payload exceeds {MAX_WRITE_BYTES} bytes"),
            ));
        }

        let socket = self.socket_mut(request.handle)?;
        socket.stream.write_all(&data).map_err(map_io_error)?;
        socket.stream.flush().map_err(map_io_error)?;

        Ok(SocketWriteResponse {
            bytes_written: data.len(),
        })
    }

    fn starttls(
        &mut self,
        request: SocketStartTlsRequest,
    ) -> Result<SocketStartTlsResponse, SocketError> {
        let requested_host = normalize_host(&request.host);
        let socket = self.sockets.get(&request.handle).ok_or_else(|| {
            socket_error(
                SocketErrorCode::RemoteClosed,
                format!("socket handle {} is not open", request.handle),
            )
        })?;
        if requested_host != socket.host {
            return Err(socket_error(
                SocketErrorCode::PermissionDenied,
                "STARTTLS host must match the connected socket host",
            ));
        }
        if socket.mode != SocketTlsMode::Starttls
            || !self.allows(&socket.host, socket.port, SocketTlsMode::Starttls)
        {
            return Err(socket_error(
                SocketErrorCode::PermissionDenied,
                format!(
                    "socket STARTTLS permission denied for {}:{}",
                    socket.host, socket.port
                ),
            ));
        }
        if !matches!(&socket.stream, SocketStream::Plain(_)) {
            return Err(socket_error(
                SocketErrorCode::StartTlsFailed,
                "socket is already using TLS",
            ));
        }

        let mut socket = self.take_socket(request.handle)?;
        let SocketStream::Plain(stream) = socket.stream else {
            unreachable!("socket stream was checked before removal");
        };
        socket.stream = SocketStream::Tls(Box::new(upgrade_tls(stream, &socket.host)?));
        self.sockets.insert(request.handle, socket);

        Ok(SocketStartTlsResponse {
            handle: request.handle,
        })
    }

    fn close(&mut self, request: SocketCloseRequest) -> SocketCloseResponse {
        SocketCloseResponse {
            closed: self.sockets.remove(&request.handle).is_some(),
        }
    }

    fn allows(&self, host: &str, port: u16, tls_mode: SocketTlsMode) -> bool {
        self.permissions
            .iter()
            .any(|permission| permission.allows(host, port, tls_mode))
    }

    fn allocate_handle(&mut self) -> u32 {
        let handle = self.next_handle;
        self.next_handle = self.next_handle.wrapping_add(1).max(1);
        handle
    }

    fn socket_mut(&mut self, handle: u32) -> Result<&mut OpenSocket, SocketError> {
        self.sockets.get_mut(&handle).ok_or_else(|| {
            socket_error(
                SocketErrorCode::RemoteClosed,
                format!("socket handle {handle} is not open"),
            )
        })
    }

    fn take_socket(&mut self, handle: u32) -> Result<OpenSocket, SocketError> {
        self.sockets.remove(&handle).ok_or_else(|| {
            socket_error(
                SocketErrorCode::RemoteClosed,
                format!("socket handle {handle} is not open"),
            )
        })
    }
}

#[derive(Debug)]
struct OpenSocket {
    host: String,
    port: u16,
    mode: SocketTlsMode,
    stream: SocketStream,
}

#[derive(Debug)]
enum SocketStream {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ClientConnection, TcpStream>>),
}

impl Read for SocketStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.read(buffer),
            Self::Tls(stream) => stream.read(buffer),
        }
    }
}

impl Write for SocketStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.write(buffer),
            Self::Tls(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(stream) => stream.flush(),
            Self::Tls(stream) => stream.flush(),
        }
    }
}

#[derive(Debug)]
struct ResolvedSocketPermission {
    host_pattern: String,
    ports: Vec<u16>,
    tls_modes: Vec<SocketTlsMode>,
}

impl ResolvedSocketPermission {
    fn allows(&self, host: &str, port: u16, tls_mode: SocketTlsMode) -> bool {
        self.ports.contains(&port)
            && self.tls_modes.contains(&tls_mode)
            && host_matches_pattern(&self.host_pattern, host)
    }
}

fn resolve_permissions(
    permissions: &[SocketPermission],
    config_json: Option<&str>,
) -> Vec<ResolvedSocketPermission> {
    let config = config_json.and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());

    permissions
        .iter()
        .filter_map(|permission| {
            let host_pattern = resolve_host_pattern(&permission.host_pattern, config.as_ref())?;
            Some(ResolvedSocketPermission {
                host_pattern,
                ports: permission.ports.clone(),
                tls_modes: permission.tls_modes.clone(),
            })
        })
        .collect()
}

fn resolve_host_pattern(pattern: &str, config: Option<&serde_json::Value>) -> Option<String> {
    if let Some(key) = socket_host_pattern_config_key(pattern) {
        let value = config?
            .get(key)?
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        let host = normalize_host(value);
        if host.contains('*') || !allowed_host_pattern_is_valid(&host) {
            return None;
        }
        return Some(host);
    }

    let host = normalize_host(pattern);
    allowed_host_pattern_is_valid(&host).then_some(host)
}

fn host_matches_pattern(pattern: &str, host: &str) -> bool {
    if let Some(suffix) = pattern.strip_prefix("*.") {
        return host
            .strip_suffix(suffix)
            .is_some_and(|prefix| prefix.ends_with('.') && prefix.len() > 1);
    }
    pattern == host
}

fn normalize_host(host: &str) -> String {
    host.trim().trim_end_matches('.').to_ascii_lowercase()
}

fn timeout_or_default(value_ms: Option<u64>, default: Duration) -> Duration {
    value_ms
        .filter(|value| *value > 0)
        .map(Duration::from_millis)
        .unwrap_or(default)
}

fn connect_tcp(
    host: &str,
    port: u16,
    connect_timeout: Duration,
    read_timeout: Duration,
    write_timeout: Duration,
) -> Result<TcpStream, SocketError> {
    let addresses = (host, port).to_socket_addrs().map_err(|error| {
        socket_error(
            SocketErrorCode::DnsFailed,
            format!("failed to resolve {host}:{port}: {error}"),
        )
    })?;

    let mut saw_address = false;
    let mut last_error = None;
    for address in addresses {
        saw_address = true;
        match TcpStream::connect_timeout(&address, connect_timeout) {
            Ok(stream) => {
                stream.set_read_timeout(Some(read_timeout)).ok();
                stream.set_write_timeout(Some(write_timeout)).ok();
                return Ok(stream);
            }
            Err(error) => last_error = Some(error),
        }
    }

    if !saw_address {
        return Err(socket_error(
            SocketErrorCode::DnsFailed,
            format!("{host}:{port} did not resolve to any socket addresses"),
        ));
    }

    let error = last_error.unwrap_or_else(|| io::Error::other("connect failed"));
    Err(socket_error(
        if error.kind() == io::ErrorKind::TimedOut {
            SocketErrorCode::ConnectTimeout
        } else {
            SocketErrorCode::IoFailed
        },
        format!("failed to connect to {host}:{port}: {error}"),
    ))
}

fn upgrade_tls(
    stream: TcpStream,
    host: &str,
) -> Result<StreamOwned<ClientConnection, TcpStream>, SocketError> {
    let server_name = ServerName::try_from(host.to_string()).map_err(|error| {
        socket_error(
            SocketErrorCode::TlsVerificationFailed,
            format!("invalid TLS server name {host}: {error}"),
        )
    })?;
    let connection = ClientConnection::new(tls_config()?, server_name).map_err(|error| {
        socket_error(
            SocketErrorCode::TlsVerificationFailed,
            format!("failed to create TLS connection: {error}"),
        )
    })?;
    let mut tls_stream = StreamOwned::new(connection, stream);

    while tls_stream.conn.is_handshaking() {
        tls_stream
            .conn
            .complete_io(&mut tls_stream.sock)
            .map_err(|error| {
                socket_error(
                    SocketErrorCode::TlsVerificationFailed,
                    format!("TLS handshake failed: {error}"),
                )
            })?;
    }

    Ok(tls_stream)
}

fn tls_config() -> Result<Arc<ClientConfig>, SocketError> {
    let native = rustls_native_certs::load_native_certs();
    let mut roots = RootCertStore::empty();
    let (added, _) = roots.add_parsable_certificates(native.certs);
    if added == 0 || roots.is_empty() {
        return Err(socket_error(
            SocketErrorCode::TlsVerificationFailed,
            "no platform TLS root certificates were available",
        ));
    }

    Ok(Arc::new(
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    ))
}

fn decode_input<T>(input: String) -> Result<T, SocketError>
where
    T: for<'de> serde::Deserialize<'de>,
{
    serde_json::from_str(&input).map_err(|error| {
        socket_error(
            SocketErrorCode::ProtocolError,
            format!("failed to decode socket host request: {error}"),
        )
    })
}

fn encode_response<T>(result: Result<T, SocketError>) -> String
where
    T: serde::Serialize,
{
    let response = match result {
        Ok(value) => SocketResponse::ok(value),
        Err(error) => SocketResponse {
            ok: false,
            value: None,
            error: Some(error),
        },
    };

    serde_json::to_string(&response).unwrap_or_else(|error| {
        serde_json::to_string(&SocketResponse::<()> {
            ok: false,
            value: None,
            error: Some(socket_error(
                SocketErrorCode::ProtocolError,
                format!("failed to encode socket host response: {error}"),
            )),
        })
        .unwrap_or_else(|_| "{\"ok\":false}".to_string())
    })
}

fn socket_error(code: SocketErrorCode, message: impl Into<String>) -> SocketError {
    SocketError {
        code,
        message: message.into(),
    }
}

fn map_io_error(error: io::Error) -> SocketError {
    let code = match error.kind() {
        io::ErrorKind::UnexpectedEof
        | io::ErrorKind::ConnectionAborted
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::BrokenPipe
        | io::ErrorKind::NotConnected => SocketErrorCode::RemoteClosed,
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => SocketErrorCode::ConnectTimeout,
        _ => SocketErrorCode::IoFailed,
    };
    socket_error(code, error.to_string())
}

host_fn!(scryer_socket_open(state: SocketHostState; input: String) -> String {
    let state = state.get()?;
    let mut state = state
        .lock()
        .map_err(|error| extism::Error::msg(format!("socket state lock poisoned: {error}")))?;
    let request = decode_input(input);
    Ok(encode_response(request.and_then(|request| state.open(request))))
});

host_fn!(scryer_socket_read(state: SocketHostState; input: String) -> String {
    let state = state.get()?;
    let mut state = state
        .lock()
        .map_err(|error| extism::Error::msg(format!("socket state lock poisoned: {error}")))?;
    let request = decode_input(input);
    Ok(encode_response(request.and_then(|request| state.read(request))))
});

host_fn!(scryer_socket_write(state: SocketHostState; input: String) -> String {
    let state = state.get()?;
    let mut state = state
        .lock()
        .map_err(|error| extism::Error::msg(format!("socket state lock poisoned: {error}")))?;
    let request = decode_input(input);
    Ok(encode_response(request.and_then(|request| state.write(request))))
});

host_fn!(scryer_socket_starttls(state: SocketHostState; input: String) -> String {
    let state = state.get()?;
    let mut state = state
        .lock()
        .map_err(|error| extism::Error::msg(format!("socket state lock poisoned: {error}")))?;
    let request = decode_input(input);
    Ok(encode_response(request.and_then(|request| state.starttls(request))))
});

host_fn!(scryer_socket_close(state: SocketHostState; input: String) -> String {
    let state = state.get()?;
    let mut state = state
        .lock()
        .map_err(|error| extism::Error::msg(format!("socket state lock poisoned: {error}")))?;
    let request = decode_input(input);
    Ok(encode_response(request.map(|request| state.close(request))))
});
