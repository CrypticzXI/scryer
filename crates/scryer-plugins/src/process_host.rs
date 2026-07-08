use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};

use crate::types::{PluginDescriptor, ProviderDescriptor};

pub(crate) const PROCESS_HOST_NAMESPACE: &str = "extism:host/user";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(25);
const MAX_STDIN_BYTES: usize = 64 * 1024;
const MAX_OUTPUT_BYTES: usize = 512 * 1024;
const MAX_ARGS: usize = 128;
const MAX_ENV_VARS: usize = 256;

#[derive(Clone)]
pub(crate) struct ProcessHost {
    state: Arc<Mutex<ProcessHostState>>,
}

impl ProcessHost {
    pub(crate) fn disabled() -> Self {
        Self {
            state: Arc::new(Mutex::new(ProcessHostState::new(Vec::new()))),
        }
    }

    pub(crate) fn from_descriptor(
        descriptor: &PluginDescriptor,
        config_json: Option<&str>,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(ProcessHostState::new(resolve_allowed_commands(
                descriptor,
                config_json,
            )))),
        }
    }

    pub(crate) fn call(&self, function: &str, input: String) -> Result<String, String> {
        if function != "scryer_process_exec" {
            return Err(format!("unsupported process host function: {function}"));
        }
        let state = self
            .state
            .lock()
            .map_err(|error| format!("process state lock poisoned: {error}"))?;
        let request = decode_input(input);
        Ok(encode_response(
            request.and_then(|request| state.execute(request)),
        ))
    }
}

#[derive(Debug)]
struct ProcessHostState {
    allowed_commands: Vec<String>,
}

impl ProcessHostState {
    fn new(allowed_commands: Vec<String>) -> Self {
        Self { allowed_commands }
    }

    fn execute(&self, request: ProcessExecRequest) -> Result<ProcessExecResponse, ProcessError> {
        validate_request(&request)?;
        if !self.command_allowed(&request.command) {
            return Err(process_error(
                ProcessErrorCode::PermissionDenied,
                format!("process permission denied for {}", request.command),
            ));
        }

        let stdin = decode_stdin(request.stdin_base64.as_deref())?;
        let timeout = request
            .timeout_ms
            .filter(|value| *value > 0)
            .map(Duration::from_millis)
            .unwrap_or(DEFAULT_TIMEOUT)
            .min(MAX_TIMEOUT);

        let mut command = Command::new(&request.command);
        command
            .args(&request.args)
            .envs(request.env.iter())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(working_directory) = request
            .working_directory
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            command.current_dir(working_directory);
        }

        let mut child = command.spawn().map_err(|error| {
            process_error(
                ProcessErrorCode::SpawnFailed,
                format!("failed to start {}: {error}", request.command),
            )
        })?;

        if let Some(mut child_stdin) = child.stdin.take() {
            let input = stdin.clone();
            thread::spawn(move || {
                let _ = child_stdin.write_all(&input);
            });
        }

        let stdout_handle = child
            .stdout
            .take()
            .map(|stdout| thread::spawn(move || read_limited(stdout)));
        let stderr_handle = child
            .stderr
            .take()
            .map(|stderr| thread::spawn(move || read_limited(stderr)));

        let deadline = Instant::now() + timeout;
        let (status_code, timed_out) = loop {
            match child.try_wait() {
                Ok(Some(status)) => break (status.code(), false),
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        break (None, true);
                    }
                    thread::sleep(POLL_INTERVAL);
                }
                Err(error) => {
                    return Err(process_error(
                        ProcessErrorCode::IoFailed,
                        format!("failed while waiting for {}: {error}", request.command),
                    ));
                }
            }
        };

        Ok(ProcessExecResponse {
            status_code,
            stdout_base64: join_reader(stdout_handle)?,
            stderr_base64: join_reader(stderr_handle)?,
            timed_out,
        })
    }

    fn command_allowed(&self, command: &str) -> bool {
        self.allowed_commands
            .iter()
            .any(|allowed| same_command_path(command, allowed))
    }
}

#[derive(Debug, Deserialize)]
struct ProcessExecRequest {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    working_directory: Option<String>,
    #[serde(default)]
    stdin_base64: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
struct ProcessExecResponse {
    status_code: Option<i32>,
    stdout_base64: String,
    stderr_base64: String,
    timed_out: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProcessErrorCode {
    PermissionDenied,
    SpawnFailed,
    IoFailed,
    ProtocolError,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProcessError {
    code: ProcessErrorCode,
    message: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
struct ProcessResponse<T> {
    ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    value: Option<T>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<ProcessError>,
}

impl<T> ProcessResponse<T> {
    fn ok(value: T) -> Self {
        Self {
            ok: true,
            value: Some(value),
            error: None,
        }
    }
}

fn resolve_allowed_commands(
    descriptor: &PluginDescriptor,
    config_json: Option<&str>,
) -> Vec<String> {
    let ProviderDescriptor::Notification(notification) = &descriptor.provider else {
        return Vec::new();
    };
    if !notification.capabilities.requires_host_process {
        return Vec::new();
    }

    let mut commands = Vec::new();
    if let Some(config) =
        config_json.and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
    {
        for key in ["path", "command", "executable", "script_path"] {
            if let Some(value) = config
                .get(key)
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                commands.push(value.to_string());
            }
        }
    }

    if notification.provider_type.eq_ignore_ascii_case("synology") {
        commands.push("/usr/syno/bin/synoindex".to_string());
    }

    commands.sort();
    commands.dedup();
    commands
}

fn validate_request(request: &ProcessExecRequest) -> Result<(), ProcessError> {
    if request.command.trim().is_empty() {
        return Err(process_error(
            ProcessErrorCode::ProtocolError,
            "process command must not be empty",
        ));
    }
    if request.args.len() > MAX_ARGS {
        return Err(process_error(
            ProcessErrorCode::ProtocolError,
            format!("process arg count exceeds {MAX_ARGS}"),
        ));
    }
    if request.env.len() > MAX_ENV_VARS {
        return Err(process_error(
            ProcessErrorCode::ProtocolError,
            format!("process environment count exceeds {MAX_ENV_VARS}"),
        ));
    }
    Ok(())
}

fn decode_stdin(stdin_base64: Option<&str>) -> Result<Vec<u8>, ProcessError> {
    let Some(stdin_base64) = stdin_base64 else {
        return Ok(Vec::new());
    };
    let bytes = STANDARD.decode(stdin_base64.as_bytes()).map_err(|error| {
        process_error(
            ProcessErrorCode::ProtocolError,
            format!("failed to decode process stdin: {error}"),
        )
    })?;
    if bytes.len() > MAX_STDIN_BYTES {
        return Err(process_error(
            ProcessErrorCode::ProtocolError,
            format!("process stdin exceeds {MAX_STDIN_BYTES} bytes"),
        ));
    }
    Ok(bytes)
}

fn same_command_path(left: &str, right: &str) -> bool {
    let left = left.trim();
    let right = right.trim();
    if left == right {
        return true;
    }

    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => Path::new(left) == Path::new(right),
    }
}

fn read_limited(mut reader: impl Read) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(output);
        }
        let remaining = MAX_OUTPUT_BYTES.saturating_sub(output.len());
        if remaining > 0 {
            output.extend_from_slice(&buffer[..read.min(remaining)]);
        }
    }
}

fn join_reader(
    handle: Option<thread::JoinHandle<io::Result<Vec<u8>>>>,
) -> Result<String, ProcessError> {
    let Some(handle) = handle else {
        return Ok(String::new());
    };
    let bytes = handle
        .join()
        .map_err(|_| process_error(ProcessErrorCode::IoFailed, "process output reader panicked"))?
        .map_err(|error| {
            process_error(
                ProcessErrorCode::IoFailed,
                format!("failed to read process output: {error}"),
            )
        })?;
    Ok(STANDARD.encode(bytes))
}

fn process_error(code: ProcessErrorCode, message: impl Into<String>) -> ProcessError {
    ProcessError {
        code,
        message: message.into(),
    }
}

fn decode_input(input: String) -> Result<ProcessExecRequest, ProcessError> {
    serde_json::from_str(&input).map_err(|error| {
        process_error(
            ProcessErrorCode::ProtocolError,
            format!("failed to decode process host request: {error}"),
        )
    })
}

fn encode_response<T>(result: Result<T, ProcessError>) -> String
where
    T: Serialize,
{
    let response = match result {
        Ok(value) => ProcessResponse::ok(value),
        Err(error) => ProcessResponse {
            ok: false,
            value: None,
            error: Some(error),
        },
    };

    serde_json::to_string(&response).unwrap_or_else(|error| {
        serde_json::to_string(&ProcessResponse::<()> {
            ok: false,
            value: None,
            error: Some(process_error(
                ProcessErrorCode::ProtocolError,
                format!("failed to encode process host response: {error}"),
            )),
        })
        .unwrap_or_else(|_| "{\"ok\":false}".to_string())
    })
}
