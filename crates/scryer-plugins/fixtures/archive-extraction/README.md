# archive-extraction plugin fixture (RFC 123 WP2)

`plugin.wasm.zst` is the real `archive-extraction` command-model plugin, checked
in as a zstd blob following the builtin indexer convention
(`crates/scryer-plugins/builtins/*.wasm.zst`, decompressed at load time). The
real-artifact integration suite (`src/archive_real_artifact_tests.rs`)
decompresses it with `zstd::decode_all` and drives it through
`WasmArchiveExtractorClient::process`.

## Provenance

- Source repo: `github.com/scryer-media/scryer-plugins`, release tag
  `plugins-v3/archive-extraction/v0.1.1`.
- Release asset: `plugin-v3.wasm.zst`.
- Decompressed artifact:
  - size `1753286`
  - blake3 `eed239c261ec45a51d1831bd6303c83819e9223b95d72f6e3216b8c33b2671c2`
  - sha256 `9d241b543faf5b6920eec3a9efdac86f6ff33f7d7d8ac7c32736d751a901837f`
- `plugin.wasm.zst`: size `461327`,
  sha256 `8523f30575ea9be0cce44ca6809e95e31d17b01078e04d255b4ec8ef4885a549`.

## ABI (frozen, RFC §5 + WP2.5)

Imports exactly three host functions under `extism:host/user`:
`scryer_aes_cbc_decrypt` (i64×5→i64) and `scryer_crc32` (i64×3→i64) — the frozen
§5 crypto pair — plus `scryer_par2_reconstruct` (i64×2→i64), the WP2.5
host-thread Reed–Solomon reconstruct dispatch (RFC §13.6 item 6). Every other
import is `wasi_snapshot_preview1`; exports include `_start` + `memory`. The
`abi_imports_match_frozen_contract` test is the drift tripwire.
