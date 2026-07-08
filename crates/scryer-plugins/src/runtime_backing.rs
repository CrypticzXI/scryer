//! Runtime-strategy seam (RFC 123 §7.1, WP0).
//!
//! A `PluginInstanceSpec` describes an instantiation in runtime-agnostic terms;
//! a `PluginRuntimeBacking` says which runtime executes it. Adapters express
//! their sandbox/timeout requirements through the spec instead of building an
//! `extism::Manifest` by hand, so a second runtime (the native wasmtime archive
//! host) can be selected without disturbing the four fleet kinds.
//!
//! 0.17.0 is deliberately minimal: an enum, not a trait hierarchy. The archive
//! extractor selects `WasmtimeArchive`; everything else stays on `Extism` via
//! the unchanged `build_plugin*` builders. WP3 (RFC §8.1) generalises backing
//! selection by artifact tag once a second wasmtime consumer exists.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use scryer_plugin_sdk::{PluginDescriptor, PluginKind};

/// One preopened directory mapping for a plugin instance.
#[derive(Debug, Clone)]
pub(crate) struct PreopenSpec {
    pub(crate) host_path: PathBuf,
    pub(crate) guest_path: String,
    /// `false` = read-only (Extism `ro:` prefix / wasmtime `DirPerms::READ`).
    pub(crate) writable: bool,
}

impl PreopenSpec {
    pub(crate) fn read_only(host_path: impl Into<PathBuf>, guest_path: impl Into<String>) -> Self {
        Self {
            host_path: host_path.into(),
            guest_path: guest_path.into(),
            writable: false,
        }
    }

    pub(crate) fn writable(host_path: impl Into<PathBuf>, guest_path: impl Into<String>) -> Self {
        Self {
            host_path: host_path.into(),
            guest_path: guest_path.into(),
            writable: true,
        }
    }
}

/// Runtime-agnostic description of a single plugin invocation (RFC §7.1).
#[derive(Clone)]
pub(crate) struct PluginInstanceSpec {
    /// The verified artifact bytes (from `LoadedPlugin::materialize_wasm()`).
    pub(crate) wasm: Arc<Vec<u8>>,
    pub(crate) preopens: Vec<PreopenSpec>,
    pub(crate) timeout: Duration,
    /// Hard memory cap; `None` = the runtime's default cap.
    pub(crate) memory_max_bytes: Option<usize>,
    /// Allowed network hosts — Extism/http-host kinds only; empty for archive.
    pub(crate) allowed_hosts: Vec<String>,
}

impl PluginInstanceSpec {
    /// Build the Extism `Manifest` for this spec.
    ///
    /// This is the bridge that lets the Extism backing wrap the existing
    /// `build_plugin*` builders unchanged (RFC §7.1): read-only preopens use
    /// Extism's `ro:` host-path prefix, and `allowed_hosts` feed the Scryer http
    /// host. (`memory_max_bytes` has no Extism analogue — the wasmtime backing
    /// enforces it via `StoreLimits`.)
    pub(crate) fn extism_manifest(&self) -> extism::Manifest {
        let mut manifest = extism::Manifest::new([extism::Wasm::data((*self.wasm).clone())])
            .with_timeout(self.timeout);
        for preopen in &self.preopens {
            let host_path = if preopen.writable {
                preopen.host_path.display().to_string()
            } else {
                format!("ro:{}", preopen.host_path.display())
            };
            manifest = manifest.with_allowed_path(host_path, preopen.guest_path.clone());
        }
        for host in &self.allowed_hosts {
            manifest = manifest.with_allowed_host(host);
        }
        manifest
    }
}

/// Which runtime executes a `PluginInstanceSpec` (RFC §7.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PluginRuntimeBacking {
    /// The frozen Extism runtime — every fleet kind, plus descriptor extraction.
    Extism,
    /// The native wasmtime archive host (RFC §7.2).
    WasmtimeArchive,
}

impl PluginRuntimeBacking {
    /// Backing selected for a descriptor in 0.17.0: the archive extractor runs
    /// natively on wasmtime; every other kind stays on Extism. WP3 (RFC §8.1)
    /// replaces this with artifact-tag-driven selection.
    pub(crate) fn for_descriptor(descriptor: &PluginDescriptor) -> Self {
        match descriptor.kind() {
            PluginKind::ArchiveExtractor => Self::WasmtimeArchive,
            _ => Self::Extism,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_preopen_uses_ro_prefix_in_manifest() {
        let spec = PluginInstanceSpec {
            wasm: Arc::new(b"\0asm".to_vec()),
            preopens: vec![
                PreopenSpec::read_only("/host/source", "/scryer/source"),
                PreopenSpec::writable("/host/output", "/scryer/output"),
            ],
            timeout: Duration::from_secs(42),
            memory_max_bytes: Some(1024),
            allowed_hosts: vec!["example.com".to_string()],
        };
        // Manifest construction must not panic and must round-trip through the
        // Extism manifest builder (the WP0 Extism-backing bridge).
        let manifest = spec.extism_manifest();
        let json = serde_json::to_string(&manifest).expect("manifest serialises");
        assert!(json.contains("ro:/host/source"), "ro prefix: {json}");
        assert!(json.contains("/host/output"), "rw path present: {json}");
        assert!(!json.contains("ro:/host/output"), "rw path must not be ro");
        assert!(json.contains("example.com"), "allowed host present: {json}");
    }
}
