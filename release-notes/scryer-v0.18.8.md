# scryer-v0.18.8

AI generated release notes

## User-facing changes
- Improved validation for custom trusted certificates used by plugin HTTP settings. Scryer now rejects malformed PEM data and certificate entries that contain more than one X.509 certificate with clearer validation behavior.
- Tightened validation of stored local-admin password hashes. This improves checks that determine whether a usable administrator login exists and avoids treating malformed hashes as valid local credentials.
- Hardened backup-encryption metadata generation as part of ongoing security cleanup.

## Plugin and runtime updates
- Updated the Wasmtime plugin crypto host to use `crc-fast` in place of `crc32fast` while preserving the existing streaming CRC-32 behavior expected by plugins.

## Packaging and release engineering
- Fixed Windows WinGet release validation to correctly handle nested manifest directories during MSI publishing checks.
- Improved CI coverage for WebAssembly plugin fixtures and added the `wasm32-unknown-unknown` Rust target to release tooling.
- Added OSV scanner configuration and related release-hygiene updates.