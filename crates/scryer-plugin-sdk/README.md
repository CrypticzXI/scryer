# scryer-plugin-sdk

[![crates.io](https://img.shields.io/crates/v/scryer-plugin-sdk.svg)](https://crates.io/crates/scryer-plugin-sdk)
[![docs.rs](https://docs.rs/scryer-plugin-sdk/badge.svg)](https://docs.rs/scryer-plugin-sdk)

> [!WARNING]
> **Deprecated for direct plugin development.** New Scryer plugins should use
> [`scryer-plugin-pdk`](https://crates.io/crates/scryer-plugin-pdk), which
> provides the supported guest runtime and re-exports these contract types.

This crate remains published as the shared compatibility, wire-contract, and
schema layer used by the PDK, Scryer itself, and existing SDK-based plugins.

This crate defines Scryer's plugin descriptors, configuration fields,
capabilities, request and response payloads, compatibility checks, host-service
messages, and generated JSON Schema. It does not run a plugin by itself.

## Use the PDK for plugins

- New plugins should depend on `scryer-plugin-pdk`, not this crate directly.
- Existing Extism-export plugins may continue using this SDK while they migrate
  to the PDK.
- Host integrations, catalog tools, and schema consumers may still use this
  crate directly when they only need the underlying contract types.

## What is included

- `PluginDescriptor` and typed provider descriptors for indexers, download
  clients, notifications, subtitles, archive extraction, and subtitle sync.
- Configuration metadata, declared host permissions, provider capabilities,
  and scoring policies.
- Typed request, response, result, and error payloads for every plugin family.
- Torrent, networking, HTTP, notification, and native host-service contracts.
- SDK and host-version compatibility helpers.
- A generated JSON Schema bundle for registry validation and non-Rust tooling.

## Legacy Extism indexer example

The following example documents the older SDK-only integration for maintainers
of existing plugins. Do not use it as the starting point for a new plugin; use
the PDK instead.

Create a Rust library that produces a WebAssembly `cdylib`:

```toml
[package]
name = "my-scryer-indexer"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
extism-pdk = "1.4"
scryer-plugin-sdk = "3.7"
serde_json = "1"
```

Export `scryer_describe` plus the operations advertised by the descriptor:

```rust
use extism_pdk::*;
use scryer_plugin_sdk::*;

#[plugin_fn]
pub fn scryer_describe(_input: String) -> FnResult<String> {
    let descriptor = PluginDescriptor {
        id: "my-indexer".into(),
        name: "My Indexer".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        sdk_version: SDK_VERSION.into(),
        sdk_constraint: current_sdk_constraint(),
        socket_permissions: vec![],
        provider: ProviderDescriptor::Indexer(IndexerDescriptor {
            provider_type: "my-indexer".into(),
            provider_aliases: vec![],
            source_kind: IndexerSourceKind::Generic,
            capabilities: IndexerCapabilities::default(),
            scoring_policies: vec![],
            config_fields: vec![],
            allowed_hosts: vec![],
            rate_limit_seconds: None,
        }),
    };

    Ok(serde_json::to_string(&descriptor)?)
}

#[plugin_fn]
pub fn scryer_indexer_search(input: String) -> FnResult<String> {
    let _request: PluginSearchRequest = serde_json::from_str(&input)?;
    let response = PluginSearchResponse::default();
    Ok(serde_json::to_string(&PluginResult::Ok(response))?)
}
```

Build it with:

```console
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown
```

For real implementations, declare the capabilities and configuration fields
you support, return typed results, and list every network destination the plugin
needs in its descriptor. Scryer enforces the declared host and socket
permissions at runtime.

## Compatibility

Always populate descriptors from the SDK instead of hard-coding compatibility
values:

```rust
sdk_version: SDK_VERSION.to_string(),
sdk_constraint: current_sdk_constraint(),
```

Scryer validates this contract before loading a plugin. A plugin should only
advertise exports and capabilities it actually implements.

## Schema and examples

- [API documentation](https://docs.rs/scryer-plugin-sdk)
- [SDK source and schema bundle](https://github.com/scryer-media/scryer/tree/main/crates/scryer-plugin-sdk)
- [First-party plugins and scaffolding](https://github.com/scryer-media/scryer-plugins)

Print the generated schema from a Scryer checkout with:

```console
cargo run -p scryer-plugin-sdk --example print-schema
```

## License

GPL-3.0-only
