# archive-extraction plugin fixture (RFC 123 WP2)

`plugin.wasm.zst` is the real `archive-extraction` command-model plugin, checked
in as a zstd blob following the builtin indexer convention
(`crates/scryer-plugins/builtins/*.wasm.zst`, decompressed at load time). The
real-artifact integration suite (`src/archive_real_artifact_tests.rs`)
decompresses it with `zstd::decode_all` and drives it through
`WasmArchiveExtractorClient::process`.

## Provenance

- Source repo: `github.com/scryer-media/scryer-plugins` @ commit `5b20a3f` plus
  the RFC 123 **WP2.5** working tree (host-thread PAR2 reconstruction: the new
  `src/par2_host_solver.rs` + repair-path wiring in `src/main.rs`, built on
  `scryer-plugin-sdk` 3.4 / `scryer-plugin-pdk` v0.1). This artifact **replaces**
  the W2 reconstruction-incapable baseline (which imported only the two §5 crypto
  fns); it now dispatches the Reed–Solomon solve to the host via
  `scryer_par2_reconstruct`.
- Build: `cargo build --profile plugin-release --target wasm32-wasip1`
  (baseline feature set — no `simd128`/`relaxed-simd`). Built + import-verified by
  the WP2.5 plugin lane (Agent P).
- Decompressed artifact:
  - size `2241308`
  - blake3 `c6aa722a0c816b0a8cdc4dcabb05d77b462a5488e9767036ecfc2389b1cd9442`
  - sha256 `914d0d62af06775a6f98ac9870b1532266994177e4ea265789216f767c87ce9a`
- `plugin.wasm.zst` (zstd -19): size `457912`,
  sha256 `1677b6e9cab83b4ba9ea4082d9214e32a784edfb7e622c44b5bff2e8e49fe157`.

## ABI (frozen, RFC §5 + WP2.5)

Imports exactly three host functions under `extism:host/user`:
`scryer_aes_cbc_decrypt` (i64×5→i64) and `scryer_crc32` (i64×3→i64) — the frozen
§5 crypto pair — plus `scryer_par2_reconstruct` (i64×2→i64), the WP2.5
host-thread Reed–Solomon reconstruct dispatch (RFC §13.6 item 6). Every other
import is `wasi_snapshot_preview1`; exports include `_start` + `memory`. The
`abi_imports_match_frozen_contract` test is the drift tripwire.
