# scryer-v0.18.16

AI generated release notes

## What's Changed

- Manual replacement imports now show live copy progress when Scryer is upgrading an existing file, so long-running copy steps no longer appear idle.
- Interactive release search is clearer in both title and episode views. Scryer now reports how many indexers were searched, calls out failed and skipped indexers separately, and no longer shows an `unknown` publish date when one was not provided.
- Startup behavior is smoother during database upgrades. While migrations are running, Scryer serves an upgrade screen, keeps `/health` healthy for orchestrators, and adds `/health/ready` for automation that must wait until the full app is serving.
- Windows desktop installs now use the Scryer icon more consistently in the tray and installer-created shortcut, with a tray icon loading fix included.

## Upgrade Notes

- Title folder ownership migrations were hardened for safety. Scryer now quarantines the earlier destructive migration path and applies a non-destructive replacement that preserves `media_files` records and only assigns folder ownership when it can be resolved safely.

## Other Improvements

- Getting started documentation now includes clearer health check guidance for Docker, Compose, and Kubernetes deployments.
- Windows build and packaging steps were cleaned up for faster, more reliable release builds.