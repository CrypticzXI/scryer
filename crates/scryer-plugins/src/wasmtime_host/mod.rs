//! Native wasmtime host for the archive-extractor plugin (RFC 123 §7.2, WP1).
//!
//! Replaces the Extism execution path for the archive kind: a process-wide
//! engine with epoch cancellation and the full wasm feature surface
//! ([`engine`]), a per-invocation WASI p1 sandbox with a memory cap
//! ([`sandbox`]), the frozen zero-copy crypto/CRC host ABI ([`crypto_host`]),
//! the stdin/stdout command protocol ([`invoke`]), and trap→`AppError` mapping
//! ([`error`]). Everything else in the archive pipeline (path sandboxing, COW
//! staging, providers, SDK v3.3 shapes) is untouched — this owns only the
//! instantiate/run layer beneath `WasmArchiveExtractorClient::process`.

mod crypto_host;
mod describe;
mod engine;
mod error;
mod invoke;
mod par2_host;
mod sandbox;

pub(crate) use describe::command_model_describe;
pub(crate) use invoke::{ArchiveInvocation, process_archive};
