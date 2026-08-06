# scryer-v0.18.2

AI generated release notes

## Highlights

- No major end-user feature changes are included in this release.
- Improved release reliability with stronger Docker build validation and release publish contract checks.
- Updated the JavaScript toolchain, including Node.js v24 and refreshed npm dependencies.
- Simplified maintenance by removing the legacy `xtask` stack and retiring older development-only assets.

## Release Engineering

- Added validation for release build configuration to catch publish issues earlier.
- Updated GitHub Actions and related release verification workflows.
- Refined release tooling in `xtask-release` to better support the current release process.

## Platform and Dependency Updates

- Updated Debian base images to v13.
- Updated the `docker/dockerfile` tooling tag to v1.26.
- Refreshed npm dependencies and tightened dependency pinning for more predictable builds.

## Maintenance

- Removed outdated development stack files and legacy project guidance documents.
- Reduced repository overhead by cleaning up unused local development and observability configuration.