# scryer-v0.18.13

AI generated release notes

This release focuses on import reliability, clearer search feedback, and safer matching for ambiguous titles.

## Highlights
- Manual Import now carries the original grab’s release evidence through review and execution, improving title resolution, quality scoring, and recovery for interrupted manual imports.
- Direct movie manual imports now import the primary video only and skip samples or extras instead of trying to process every file in the download.
- Manual import suggestions are smarter for episodic and series-movie downloads, including better default episode selection and automatic preselection of the matching series-movie target.
- Episode search in Series Overview now shows live indexer progress, final search summaries, and failed indexers, making interactive search easier to follow while results are still arriving.

## More fixes
- Release blocklists are now truly title-scoped. Blocking a release for one title will no longer hide it from another title, and removing the blocklist entry takes effect immediately.
- Title matching was tightened for remakes, year-qualified duplicates, and other shared-name edge cases, reducing false matches and improving “title already owns another folder” scenarios.
- Pending Imports now gives clearer ownership-conflict guidance, and download category routing behaves more predictably when a download no longer matches the active route.
- Magnet identity normalization, completed-download verification, and indexer proxy probing were hardened to reduce edge-case failures in search and import flows.

## Platform and developer updates
- Plugin SDK documentation and schema were refreshed.
- CI and release workflows were updated around trusted publishing, provenance validation, and general pipeline hygiene.