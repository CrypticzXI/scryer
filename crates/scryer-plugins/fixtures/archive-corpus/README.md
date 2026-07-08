# archive test corpus (RFC 123 WP2)

Real archive fixtures for the integration suite
(`src/archive_real_artifact_tests.rs`), COPIED verbatim (never modified) from the
scryer-plugins harness and the rarpar reference repo. Encrypted-RAR password is
`testpass123` (rarpar `generate_encrypted.sh`; `-p` data-only encryption).

## enc-rar/ — encrypted RAR (host AES + CRC ABI)

| file | provenance | notes |
|---|---|---|
| `rar4_enc_store.rar` | scryer-plugins `xtask/tests/fixtures/archive_host/rar` | RAR4 store, decrypts to `small.txt` |
| `small.txt` | idem (rarpar `originals/small.txt`) | expected plaintext (290 B) for the RAR4 store fixture |
| `rar5_enc_lz.rar` | rarpar `weaver-unrar/tests/fixtures/rar5` | RAR5 `-m3` LZ + encryption, decrypts to `compressible.txt` |
| `compressible.txt` | rarpar `originals/compressible.txt` | expected plaintext (226943 B) for the RAR5 LZ fixture |

## plain-rar4/ — plain RAR4

| file | provenance | notes |
|---|---|---|
| `rar4_multifile_lz.rar` | rarpar `weaver-unrar/tests/fixtures/rar4` | 3 members (hello.txt, second.txt, zeros_64k.bin), LZ |
| `rar4_store.rar` | idem | single stored member (small.txt) |

## par2-rar5/ — plain multi-volume RAR5 + PAR2 recovery set

Copied from rarpar `weaver-par2/tests/fixtures/rar5_lz_plain`. The 6 `.part*.rar`
volumes are a plain multi-volume RAR5 that extracts to
`rar5_lz_plain_clip.mkv` (1109271 B). The `.par2` main packet + two recovery
volumes (`vol00+2`, `vol02+2`) drive PAR2 verify and repair-then-extract (via the
adapter's COW staging path).

Byte-correctness oracles: encrypted/plain-store RAR compared to the exact original
plaintext; compressed RAR members cross-checked by recomputing crc32 of the
extracted bytes against the archive's own header CRC that the plugin reports.
