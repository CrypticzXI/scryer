# Scryer Plugin SDK Schemas

`plugin-sdk-v1.schema.json` is the committed JSON Schema bundle for the SDK-v1 ABI.
It contains `$defs` for the descriptor, config fields, and each request/response
payload family used by indexer, download-client, notification, and subtitle plugins.

The Rust SDK remains the source of truth for first-party plugins. These schemas are
for registry validation, fixture tooling, and non-Rust plugin authors.
