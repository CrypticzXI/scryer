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
  - size `1839962`
  - blake3 `20c987cfe0258b665408717907549ed0c721e53cb9eaf78a39d5ebdc7ed193b0`
  - sha256 `873090a80f6859072cdc542d21350d3a53831f797520265836073b007322d579`
- `plugin.wasm.zst`: size `424824`,
  sha256 `2a7335e9320c980af996b897fd9879e2ceb0302ac5b16a07dd381d68b70758b0`.

## ABI (frozen, RFC §5)

Imports exactly two host functions under `extism:host/user`:
`host_aes_cbc_decrypt` (i64x5 -> i64) and `host_crc32` (i64x3 -> i64), the
frozen §5 crypto pair. Every other import is `wasi_snapshot_preview1`; exports
include `_start` + `memory`. The `abi_imports_match_frozen_contract` test is the
drift tripwire.

The Wasmtime host retains the pre-rename `scryer_*` aliases during the plugin
upgrade window so installed archive plugin artifacts built with weaver-unrar
0.2.x continue to load.
