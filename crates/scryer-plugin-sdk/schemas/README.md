# Scryer Plugin SDK Schemas

`plugin-sdk-v1.schema.json` is the committed JSON Schema bundle for the
current SDK ABI family. The filename is retained for registry compatibility.
It contains `$defs` for the descriptor, config fields, and each request/response
payload family used by indexer, download-client, notification, and subtitle plugins.
SDK 2.1 subtitle sync payloads include inline base64 subtitle bytes and rewritten
subtitle bytes while preserving the existing string-in/string-out Wasm export ABI.

The Rust SDK remains the source of truth for first-party plugins. These schemas are
for registry validation, fixture tooling, and non-Rust plugin authors.
