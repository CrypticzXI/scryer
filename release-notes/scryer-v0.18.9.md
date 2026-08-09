# scryer-v0.18.9

AI generated release notes

This patch release focuses on better automatic acquisition behavior, broader media format coverage, and more resilient indexer handling.

## What's Changed

- RSS-driven acquisitions now respect library quality profiles, so automatic grabs better match each library's configured quality rules.
- Media info support has been expanded to cover `WMV`, `OGV`, and `FLV`, improving detection and metadata parsing for those formats.
- Prowlarr-managed indexers are more resilient: failures in one child indexer no longer disrupt sibling sources as broadly, reducing partial search outages.
- Indexer troubleshooting is clearer in Settings, with the latest error message now surfaced alongside the last error timestamp.

## Maintenance

- Windows package validation in CI now accepts non-fatal Winget manifest validation warnings during release checks.