# archive-extraction plugin fixture (RFC 123 WP2)

`plugin.wasm.zst` is the real `archive-extraction` command-model plugin, checked
in as a zstd blob following the builtin indexer convention
(`crates/scryer-plugins/builtins/*.wasm.zst`, decompressed at load time). The
real-artifact integration suite (`src/archive_real_artifact_tests.rs`)
decompresses it with `zstd::decode_all` and drives it through
`WasmArchiveExtractorClient::process`.

## Provenance

- Source repo: `github.com/scryer-media/scryer-plugins` @ commit `5b20a3f`
  (`archive_extractors/archive-extraction`, built on `scryer-plugin-pdk` v0.1).
- Build: `cargo build --profile plugin-release --target wasm32-wasip1`
  (baseline feature set — no `simd128`/`relaxed-simd`). Built + import-verified
  by the W2 lane; deterministic (a fresh rebuild reproduced the same blake3).
- Decompressed artifact:
  - size `2219185`
  - blake3 `c10aca8102ef9d5d7f860d6aa88e654aaa85aec7a529e223f0d5c00d903bbd0d`
  - sha256 `9620fb3b0d57522a7b7ddac4f8996b994ccb912207677dfea118bc0bd950fe88`
- `plugin.wasm.zst` (zstd -19): size `451355`,
  sha256 `bba06142d98eace046070eebe5578d4dad27f68b0f21a3987d22cf2d29f2e705`.

## ABI (frozen, RFC §5)

Imports exactly two host functions under `extism:host/user`
(`scryer_aes_cbc_decrypt`, `scryer_crc32`); every other import is
`wasi_snapshot_preview1`; exports include `_start` + `memory`. The
`abi_imports_match_frozen_contract` test is the drift tripwire.
