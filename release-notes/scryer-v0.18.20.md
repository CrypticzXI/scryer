# scryer-v0.18.20

AI generated release notes

This release focuses on download-client reliability and responsiveness for slower or heavily loaded setups.

## Highlights

- Download-client feedback reads now run in parallel across multiple clients while preserving configured priority in the returned results.
- Queue, history, recent activity, and completed-download polling have been hardened to better tolerate slow responses and large client datasets.
- Backoff handling after polling failures is smarter, reducing repeated retries against unhealthy clients and helping status views recover more cleanly.
- Slow download clients are less likely to trigger temporary `queue status is temporarily unavailable` failures during normal polling.

## Configuration

- Added `SCRYER_DOWNLOAD_CLIENT_FEEDBACK_TIMEOUT_SECS` to control the overall download-client feedback timeout.
- The default feedback timeout is now `300` seconds, which is better suited to large or busy download clients.
- NZBGet, SABnzbd, and plugin-backed download clients now have more room to complete large status and queue reads before timing out.

## Included changes

- Hardened download-client feedback polling.
- Increased download-client timeouts and enabled parallel polling across multiple clients.