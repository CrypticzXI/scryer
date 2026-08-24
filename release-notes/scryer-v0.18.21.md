# scryer-v0.18.21

AI generated release notes

## User-facing changes
- Improved qBittorrent post-import handling so seeding workflows and older qBittorrent setting combinations behave more consistently.
- Expanded recent download activity and completed download history from 100 to 300 items per client, reducing cases where useful entries disappeared too quickly in multi-client setups.
- Improved activity/history ordering across download clients by handling timestamps reported in seconds, milliseconds, or RFC3339 format more reliably.
- Fixed SABnzbd status handling so deleted items no longer linger as active downloads.
- Fixed SABnzbd history handling so unknown statuses remain in progress instead of being treated as completed.
- SABnzbd unpack/write failures, including disk-full style failures, now surface as warnings with clearer failure messaging.
- Built-in bundled plugins are now eligible for auto-update again, with safer rollback handling if an update fails.

## For plugin authors
- Published `scryer-plugin-sdk` `3.8.0`.
- Added download-client support for `mark_imported_non_destructive`, allowing plugins to acknowledge imports without destructive cleanup when supported.