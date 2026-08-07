# scryer-v0.18.6

AI generated release notes

Scryer 0.18.6 is a maintenance release focused on build and release reliability.

## Highlights
- Improved build stability for image-processing support by pinning `zune-core` to a known-good version.
- Tightened release packaging so `Cargo.lock` is refreshed and revalidated during version bumps.
- No intended application, API, or behavior changes are included in this release.

## Changes
- Added an explicit `zune-core` dependency to the image-processing feature set to prevent upstream dependency drift from breaking image-related builds.
- Updated the release workflow to regenerate the lockfile after version bumps and keep shipped package metadata in sync.
- Improved metadata-only release validation handling for release preparation and lockfile-only updates.