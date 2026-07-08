//! Descriptor extraction for command-model archive artifacts (RFC 123 §8.2,
//! WP4 pulled early for the archive kind only).
//!
//! The new archive plugin is a wasip1 command binary: it exports `_start` and
//! `memory` but NOT `scryer_describe` / `scryer_archive_process`. Scryer's
//! Extism describe path (`plugin.call("scryer_describe")`) and its
//! export-existence validation would therefore reject it. This module detects
//! the command shape and runs describe through the wasmtime backing instead; the
//! four fleet kinds keep the Extism describe path untouched.

use scryer_application::{AppError, AppResult};
use scryer_plugin_sdk::{EXPORT_DESCRIBE, PluginDescriptor};
use wasmtime::{ExternType, Linker, Module, Store};

use crate::wasmtime_host::sandbox::{self, BareSandbox, HostCtx, HostLimits};
use crate::wasmtime_host::{crypto_host, engine, error};

/// Describe runs reuse the 10s describe budget of the Extism path.
const DESCRIBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Attempt to extract a descriptor from a command-model archive artifact.
///
/// Returns `None` when the artifact is NOT the command model, so the caller
/// falls back to the Extism describe path. Classification (RFC §8.2): the module
/// exports `_start` and does NOT export `scryer_describe`. `Some(Err(_))` means
/// it is the command model but describe failed (e.g. missing `memory` export or
/// a bad describe response).
pub(crate) fn command_model_describe(wasm: &[u8]) -> Option<Result<PluginDescriptor, String>> {
    // Cheap negative fast-path: every Extism fleet plugin exports
    // `scryer_describe`, whose name is present verbatim in the wasm export
    // section, so skip the wasmtime compile for those. The authoritative
    // classification below still uses wasmtime module exports for the command
    // case where it matters.
    if contains_bytes(wasm, EXPORT_DESCRIBE.as_bytes()) {
        return None;
    }

    let engine = engine::shared_engine();
    // If it will not even compile under wasmtime, we cannot classify it here;
    // let the Extism path report its own error.
    let module = Module::from_binary(engine, wasm).ok()?;

    let mut has_start = false;
    let mut has_describe = false;
    let mut has_memory = false;
    for export in module.exports() {
        match export.name() {
            "_start" => has_start = true,
            name if name == EXPORT_DESCRIBE => has_describe = true,
            "memory" => has_memory = matches!(export.ty(), ExternType::Memory(_)),
            _ => {}
        }
    }

    if !has_start || has_describe {
        // Extism reactor model (or not a command) — fall back.
        return None;
    }
    if !has_memory {
        return Some(Err(
            "archive command plugin must export a linear memory named 'memory'".to_string(),
        ));
    }

    Some(run_describe(&module).map_err(|error| error.to_string()))
}

fn run_describe(module: &Module) -> AppResult<PluginDescriptor> {
    let engine = engine::shared_engine();

    let mut linker: Linker<HostCtx> = Linker::new(engine);
    wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |ctx: &mut HostCtx| &mut ctx.wasi)
        .map_err(|error| {
            AppError::Repository(format!("failed to wire WASI for archive describe: {error:#}"))
        })?;
    // The command binary imports the §5 crypto ABI even though describe does not
    // call it — the imports must be satisfied to instantiate at all.
    crypto_host::add_to_linker(&mut linker).map_err(|error| {
        AppError::Repository(format!(
            "failed to register crypto host for archive describe: {error:#}"
        ))
    })?;

    let BareSandbox {
        wasi,
        stdout,
        stderr,
    } = sandbox::build_describe_sandbox();

    let mut store = Store::new(
        engine,
        HostCtx {
            wasi,
            limits: HostLimits::new(None),
        },
    );
    store.limiter(|ctx: &mut HostCtx| &mut ctx.limits);
    store.set_epoch_deadline(engine::deadline_ticks(DESCRIBE_TIMEOUT));

    let instance = linker.instantiate(&mut store, module).map_err(|error| {
        AppError::Repository(format!(
            "failed to instantiate archive plugin for describe: {error:#}"
        ))
    })?;
    let start = instance
        .get_typed_func::<(), ()>(&mut store, "_start")
        .map_err(|error| {
            AppError::Repository(format!("archive plugin is not a wasip1 command: {error:#}"))
        })?;

    let result = start.call(&mut store, ());
    let denied = store.data().limits.memory_denied;
    let stdout_bytes = stdout.contents();
    let stderr_bytes = stderr.contents();
    let stderr_tail = {
        let start = stderr_bytes.len().saturating_sub(4096);
        String::from_utf8_lossy(&stderr_bytes[start..]).into_owned()
    };

    error::interpret_start_result(result, denied).map_err(|failure| {
        let ctx = error::InvocationContext {
            plugin_id: "<archive-describe>",
            plugin_version: "",
            operation: "describe",
            budget: DESCRIBE_TIMEOUT,
            stderr_tail: &stderr_tail,
        };
        error::to_app_error(&failure, &ctx)
    })?;

    serde_json::from_slice::<PluginDescriptor>(&stdout_bytes).map_err(|error| {
        AppError::Repository(format!(
            "archive plugin describe returned invalid PluginDescriptor JSON: {error}"
        ))
    })
}

/// Naive substring search — good enough for a cheap negative pre-filter.
fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if haystack.len() < needle.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extism_reactor_shape_falls_back_to_none() {
        // A module exporting `scryer_describe` is the Extism reactor model — the
        // negative fast-path (substring on the export name) returns None so the
        // caller uses the Extism describe path.
        let wasm = wat::parse_str(
            r#"(module
                 (func (export "scryer_describe") (result i64) (i64.const 0))
                 (memory (export "memory") 1))"#,
        )
        .unwrap();
        assert!(command_model_describe(&wasm).is_none());
    }

    #[test]
    fn command_shape_without_memory_is_rejected() {
        // Exports `_start`, no `scryer_describe`, but no `memory` -> classified
        // as command-model and rejected for the missing memory export.
        let wasm = wat::parse_str(r#"(module (func (export "_start")))"#).unwrap();
        match command_model_describe(&wasm) {
            Some(Err(message)) => assert!(message.contains("memory"), "{message}"),
            other => panic!("expected Some(Err(missing memory)), got {other:?}"),
        }
    }

    #[test]
    fn non_command_module_falls_back_to_none() {
        // No `_start`, no `scryer_describe` -> not a command; fall back to None.
        let wasm = wat::parse_str(r#"(module (memory (export "memory") 1))"#).unwrap();
        assert!(command_model_describe(&wasm).is_none());
    }

    #[test]
    fn contains_bytes_matches() {
        assert!(contains_bytes(b"abcdef", b"cde"));
        assert!(!contains_bytes(b"abc", b"xyz"));
        assert!(contains_bytes(b"anything", b""));
        assert!(!contains_bytes(b"ab", b"abc"));
    }
}
