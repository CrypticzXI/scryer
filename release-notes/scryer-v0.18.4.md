# scryer-v0.18.4

AI generated release notes

## Highlights

- No end-user feature changes are included in `0.18.4`.
- Release packages and container images are now published more consistently.
- Release preparation now keeps dependency state stable during TRaSH sync steps.

## Release Engineering

- Updated the release workflow so the TRaSH Guides sync runs with Cargo's locked dependency resolution, preventing unintended `Cargo.lock` churn during release prep.
- Improved release determinism for maintenance releases by keeping release-time helper tasks from introducing unrelated dependency drift.

## CI and Publishing

- Split release package uploads from build diagnostics, so successful archives are published cleanly while logs and timing artifacts remain available separately for troubleshooting.
- Relaxed missing-file handling for optional diagnostic artifacts while still requiring the actual release package to exist.
- Deduplicated generated container image tags in the release workflow, improving consistency for published images.