# scryer-v0.18.3

AI generated release notes

## Highlights

- No end-user application changes are included in `v0.18.3`.
- Simplified maintenance releases for docs-only and CI-only changes by skipping unnecessary full application validation.
- Improved release reliability with stronger CI artifact handling and tighter multi-architecture container verification.

## Release Engineering

- `cargo xtask release` now classifies changes as full, web-only, or CI/docs-only and runs the required validation scope for each case.
- Docs-only and CI-only releases can reuse checked-in builtin plugin assets instead of rebuilding them, while still enforcing release hygiene and dry-run cache validation.
- Dry-run cache reuse now checks the expected validation scope before allowing a release to proceed.

## CI and Verification

- GitHub Actions now uploads Cargo timing artifacts separately so missing timing output does not fail the main artifact upload.
- OCI verification was tightened to validate per-platform attestation manifests and confirm SPDX SBOM coverage for both published Linux image variants.