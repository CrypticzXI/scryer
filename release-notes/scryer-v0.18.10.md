# scryer-v0.18.10

AI generated release notes

This release focuses on routing control, queue scalability, and import reliability.

## Highlights
- Added indexer-to-download-client mappings, so you can route results from specific indexers to the download client you want.
- Improved activity and queue handling for large queue sizes, with better paging and a more responsive queue view.
- Fixed several manual import issues, including an NZBGet import race and follow-up corrections for missing episode data.
- Restored deterministic artwork fallbacks so posters and other artwork stay consistent when preferred images are unavailable.
- Title history now includes source provider details for clearer tracking.
- Fixed acquisition and settings action gating so related controls open and close correctly.

## Fixes and Improvements
- Applied security fixes in media info parsing.
- Improved notification robustness and provider capability handling.
- Refined series and episode availability handling across overview and detail views.
- Polished catalog and title management flows, including quality-profile handling and add-to-catalog behavior.

## Maintenance
- Updated internal crate naming and supporting dependencies.
- Included CI and scorecard maintenance, plus tougher validation for published Winget installs.