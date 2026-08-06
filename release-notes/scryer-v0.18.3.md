# scryer-v0.18.3

AI generated release notes

## Highlights

- No end-user feature changes are included in `0.18.3`.
- This release focuses on release automation and CI reliability.
- Maintenance-only releases are now handled more efficiently, with lighter validation for CI-only, docs-only, and web-only changes.

## Release Engineering

- Simplified the release flow by automatically classifying changes and choosing the appropriate validation scope instead of always running the full release path.
- Improved dry-run cache validation so mismatched release notes or missing required validation steps are caught before publish.
- Reused checked-in embedded plugin builtins for non-product releases to reduce unnecessary rebuild work during release preparation.

## CI and Verification

- Split cargo timing uploads into a dedicated artifact, making CI diagnostics easier to inspect without breaking the main artifact upload when timings are unavailable.
- Tightened OCI verification to explicitly check per-platform SBOM attestations for supported Linux image variants.
- Included additional test coverage around release validation scope selection and dry-run cache acceptance rules.