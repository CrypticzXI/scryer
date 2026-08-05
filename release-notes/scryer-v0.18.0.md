# scryer-v0.18.0

AI generated release notes

## Highlights

Scryer 0.18.0 focuses on smoother day-to-day use across discovery, imports, downloads, and library management, with a notable push on Windows packaging and platform reliability.

### User-facing improvements

- Discovery browsing is more polished, with fixes to organization, classification, and general UI behavior on the discovery page.
- Rules settings now include locale pack controls, making it easier to tune discovery and matching behavior for your library.
- Manual import has been expanded with better visibility and selection handling, improving control over ambiguous or partial imports.
- Recycle Bin and library cleanup flows are more robust. Media deletion and restore operations now run as jobs, improving reliability and making long-running actions easier to track.
- Download client integration is more reliable, especially for SABnzbd and NZBGet queue/history handling, along with fixes for externally managed download scenarios.
- Activity and queue surfaces were refined to reduce noise, including better handling of external download activity.
- Windows support improved with a new MSI installer path and additional packaging validation.

### Plugin and platform updates

- Added a new plugin command host, expanding what plugins can do and improving the plugin runtime surface.
- Updated the plugin SDK and related schemas for newer command/runtime capabilities.
- Improved import, queue, and tracked download plumbing across the app for more consistent backend behavior.
- Updated GraphQL schema and related queries/mutations to support the new flows.

### Reliability and security

- Hardened recycle-bin behavior and backup/security-related flows.
- Improved release signing, provenance validation, supply-chain checks, and CI coverage.
- Added stronger Windows artifact validation and broader release pipeline safeguards.
- Toolchain and dependency updates improve baseline stability for this release.

## Notable areas of change

- Discovery page fixes and organization improvements
- Better download client queue/history integration
- Manual import workflow improvements
- Recycle Bin job-based delete/restore operations
- Windows MSI installer support
- Plugin command host and SDK updates
- Release, signing, and CI hardening