# archive-extraction plugin fixture (RFC 123 WP2)

`plugin.wasm.zst` is the real `archive-extraction` command-model plugin, checked
in as a zstd blob following the builtin indexer convention
(`crates/scryer-plugins/builtins/*.wasm.zst`, decompressed at load time). The
real-artifact integration suite (`src/archive_real_artifact_tests.rs`)
decompresses it with `zstd::decode_all` and drives it through
`WasmArchiveExtractorClient::process`.

## Provenance

- Source artifact:
  `archive_extractors/archive-extraction/target/variants/baseline/wasm32-wasip1/plugin-release/archive_extraction_archive_extractor.wasm`.
- Decompressed artifact:
  - size `1839967`
  - blake3 `ad09621d749d7a88e195fcdef2d58b050b58bf9c1d5d94ed387075db3d1517f5`
  - sha256 `bdda433d83d47d494e5bf246c3be2e28fd9db6775f9128b70c98c6a73df7da23`
- `plugin.wasm.zst`: size `424816`,
  sha256 `d24cd828ac77f7117201a9688366c95da64432641d8d01d8a4cd01900b3ce980`.

## ABI (frozen, RFC §5)

Imports exactly two host functions under `extism:host/user`:
`scryer_aes_cbc_decrypt` (i64x5 -> i64) and `scryer_crc32` (i64x3 -> i64), the
frozen §5 crypto pair. Every other import is `wasi_snapshot_preview1`; exports
include `_start` + `memory`. The `abi_imports_match_frozen_contract` test is the
drift tripwire.

> **Migration (weaver-unrar host-abi rename, 2026-07):** the embedder-neutral
> ABI renames these imports to `host_aes_cbc_decrypt` / `host_crc32` (namespace
> stays `extism:host/user` via weaver-unrar's `host-abi-extism` feature). This
> blob predates the rename and still imports `scryer_*`; the wasmtime host
> registers both the `host_*` names and the `scryer_*` aliases during the
> window. When this blob is rebuilt against the renamed weaver-unrar release,
> update the two names above, flip `abi_imports_match_frozen_contract` to expect
> `host_*`, and drop the host-side `scryer_*` aliases in
> `wasmtime_host/crypto_host.rs`.
