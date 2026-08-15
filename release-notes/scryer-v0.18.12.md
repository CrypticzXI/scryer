# scryer-v0.18.12

AI generated release notes

Scryer 0.18.12 adds first-class Emby support and improves library scanning, imports, and everyday navigation.

## Highlights

- Added first-class Emby integration for media-server setup and external-account authentication, including local and Emby Connect servers, account linking and invitations, protected user avatars, and migration from earlier MediaBrowser-compatible settings.
- Reworked the Calendar with improved month and week views, clearer event presentation and title links, responsive layouts, and a remembered view preference.
- Refined the Settings, Wanted, sidebar, login, profile, indexer, download-client, and media-server experiences for clearer configuration and smoother navigation.

## Library and Import Reliability

- Fixed episodic `.strm` imports being rejected as samples when the pointer file is smaller than the normal episodic video-size threshold.
- Enforced single-title ownership of library folders so scans, imports, renames, and title deletion cannot silently assign the same folder to multiple titles; ownership conflicts are surfaced for review.
- Improved series and series-movie import matching so episode-scoped files, upgrades, and the single primary file selected from scan results remain consistent.
- Added visible “scanned” title-history events and improved tracked-download recovery so completed acquisitions are reconciled more reliably after polling or restart.
- Rehydrated authoritative original-language metadata for existing and newly refreshed titles, improving language-aware quality and release decisions.
- Improved manual-import source handling and download-client compatibility across completed-download workflows.

## Authentication and Interface Improvements

- Emby authentication now supports servers that do not require a TOTP code while retaining TOTP support when configured.
- Improved authenticated-session persistence, passkey handling, external-account invitations, and media-server avatar loading.
- Polished subtitle-language selection, release-search results, pending-import feedback, title tables, season views, and activity details.

## Maintenance

- Expanded GraphQL schema documentation and compatibility coverage.
- Improved Windows MSVC compiler discovery and strengthened CI build and security checks.
