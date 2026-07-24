# scryer-v0.17.2

## Highlights
- Fixed a regression that prevented NZBs from being grabbed from indexers whose download links respond with an HTTP redirect (for example a 301 or 302). Outbound download and API requests now follow trusted redirects again, so grabs that failed with "nzb download failed with status 301 Moved Permanently" succeed.

## Additional updates
- Outbound HTTP requests once again follow redirects by default, with per-hop rate limiting and a bounded hop limit preserved. Untrusted title-image fetches continue to reject redirects.
- Refreshed TRaSH Guides release-group data to keep release evaluation and scoring current.
