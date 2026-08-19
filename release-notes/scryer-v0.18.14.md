# scryer-v0.18.14

AI generated release notes

## What's Changed

- Fixed a UI issue where manual import copy progress could be overwritten on blocked downloads.
- When a manual import is queued or actively copying, the download row now stays in an importing state, continues showing live transfer progress, and keeps conflicting actions disabled until the import finishes.
- Improved state handling so stale finished import results do not incorrectly repaint a blocked download while manual review is still required.

## Internal

- Added coverage for blocked-download manual import state projection and bridged download queue behavior.