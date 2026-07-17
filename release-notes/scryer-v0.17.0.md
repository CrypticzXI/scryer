# Scryer 0.17.0 release notes

Scryer 0.17.0 is a major release centered on a new responsive web experience, personalized Discovery, smarter acquisition through indexer convergence, restartable multi-instance Arr imports, faster library workflows, and a much more capable plugin platform. It also includes a broad cleanup of the GraphQL API.

The release comparison is `scryer-v0.16.8..release-0.17.0`: 190 commits across 827 files, with 181,816 insertions and 46,556 deletions.

## Headliner features

### A new Scryer UI

The web application has been redesigned around one responsive shell and visual system rather than a collection of isolated page styles.

- A rebuilt header, grouped sidebar, mobile navigation, atmospheric backgrounds, consistent focus states, semantic status/facet colors, and shared panel/form/table/dialog styling now span login, setup, catalog, automation, integrations, system administration, and error/loading states.
- The shared component foundation adds consistent badges, icon actions, text actions, switches, radio groups, multi-selects, denser tables, richer selects, tooltips, scroll affordances, and mobile/desktop variants.
- Activity now uses responsive, action-complete queue rows with pause/resume, manual import, title assignment, ignore, failure/retry, and removal actions.
- Requests is a poster-backed triage surface with separate administrator and requester workflows, facet/status filtering, requester/library/profile/monitor context, and responsive approval, dismissal, modification, and cancellation actions.
- Calendar defaults to a redesigned month view at every breakpoint, adds month/week switching and a Today action, and presents facet-colored events, air times, filters, and event counts more clearly.
- System administration is divided into focused overview, service-log, audit-log, jobs, and recycle-bin experiences. The overview includes readiness/version, title and monitored counts, users, datastore/schema migration state, facet totals, recent events, and indexer activity.
- Settings has permission-aware subnavigation, breadcrumbs, consistent page headers, responsive edit rails, and integration-specific plugin management. Indexer, download-client, notification, and subtitle pages can install, enable, disable, upgrade, update-all, and uninstall the relevant provider plugins without leaving the page.
- A general CodeMirror-based editor now supports Rego, shell, JavaScript, and plain text, with diagnostics, readonly/auto-height modes, dark/light styling, and a textarea fallback while the editor chunk loads.

### Unified catalogs and title workspaces

Movies, series, and anime now share a richer catalog and title-management model.

- Poster, poster-plus-table, and compact-table modes replace separate, inconsistent catalog implementations. The active mode is remembered per content scope.
- Movie detail opens inline in the shared title workspace instead of navigating to the removed standalone movie-overview implementation.
- The workspace brings monitoring, automatic and interactive search, refresh/scan, rename, history, blocklist, on-disk file inspection, primary-file selection, external subtitles, deletion previews, and external metadata links into one place.
- Bulk selection supports monitor, unmonitor, edit, and previewed deletion, including optional on-disk deletion and resumable job state.
- Catalogs use server-side cursor pagination, projection-aware queries, abortable in-flight requests, infinite loading, stable row identity, and virtualized scroll restoration. The initial page is deliberately smaller instead of eagerly fetching hundreds of records.
- Filters cover monitored state, continuing/ended status, library, root, genre, theme/tag, year, and minimum rating. Server-provided option counts and ranges are retryable and debounced.
- Configurable columns and sort keys include added date, year, runtime, status, root, popularity, resolution, HDR, audio codec, size, episode progress, and source-specific ratings.
- Title sorting uses persisted ICU/CLDR collation keys with multilingual article stripping, Unicode normalization, and CJK width normalization for stable locale-aware pagination.
- Series and anime details load episode and media-file detail on demand. Expanded data survives reactive refreshes, and memoized episode/series-movie rows provide monitoring, queue state, search, file/subtitle management, blocklist, and history without the old all-files initial payload.
- Source-branded ratings support IMDb, Rotten Tomatoes critic and Popcornmeter, Metacritic critic and user, Letterboxd, TMDB, TVDB, Trakt, MyAnimeList, AniList, AniDB, MDBList, and additional aliases with deterministic normalization and ordering.
- Hydration persists popularity, normalized ratings, and canonical tags with source provenance plus adult/spoiler attributes. Combined metadata requests batch up to 50 movie/series identities, while interactive hydration refreshes recommendations inline and a small background worker pool makes recommendations due again after 24 hours.
- AniBridge hydration normalizes MAL, AniList, AniDB, Kitsu, Simkl, TVDB, TMDB, IMDb, and Trakt identities, replaces stale source-scoped identifiers/tags during rematch, and respects per-library title ownership throughout lookup and creation.

### Global Search

The old route-command palette has been replaced by one desktop/mobile Global Search surface.

- Results can interleave titles already in the library, metadata results outside the library, navigation/actions/settings commands, and movie/series/anime results.
- `All`, `In Library`, `Actions`, and facet tabs combine with additive filters and match ranking for exact, prefix, and keyword results.
- Metadata identities merge external IDs and filter titles already represented in the catalog. Authorization signatures invalidate stale catalog state when the signed-in user's library access changes.
- Results can open managed titles or start the correct add/request flow for unmanaged metadata, while respecting current library capabilities.
- Search no longer auto-opens over an existing dialog, and catalog arrays are equality-checked before replacement to avoid stale or duplicate results.

### Personalized Discovery

Discovery is now a persistent part of Scryer rather than a one-off metadata lookup.

- The new home combines a hero with trending, upcoming, new-on-streaming, new-on-physical, personalized weekly, genre, theme/tag, acclaimed, collection-completion, and anime-aware rails.
- Discovery detail and “More Like This” recommendations carry canonical identity, ratings, artwork, source context, and enough authorization information to add, request, or open the right title.
- Server-backed filters cover content type, genre, theme/tag, studio, year, and minimum rating. Duplicate, invalid, already-managed, and facet-inappropriate items are removed before display.
- Scryer stores Discovery snapshots, public-feed generations, incremental and pending changes, run history, acknowledgements, fingerprints, cursors, leases, and retry state so the experience survives restarts cleanly.
- Domain events are coalesced; large or ambiguous change sets escalate to a full snapshot. Public-feed work can continue while personalized context work is backed off or blocked by a scan.
- Discovery sync now handles paged ingest, acknowledgement recovery, full queues, terminal and transient failures, manual cooldowns, daily backstops, quiet startup periods, and stable per-installation jitter.
- Results apply library visibility and per-library permissions throughout, including cached recommendations. Owned items are excluded through normalized external identities, including movie-shaped items attached to series.
- “Not interested” is browser-local in 0.17: hidden IDs are kept in `localStorage`, and clearing browser storage or using another device resets them.

### Indexer convergence replaces per-item Wanted scheduling

Scryer no longer builds and polls a separate search schedule for every missing item. It now tracks whether each relevant indexer has covered a missing or cutoff-upgrade scope.

- Each routed indexer receives convergence credit only after an actual fired response, including a valid empty response. Disabled, skipped, deferred, or errored indexers do not count.
- A scope is converged only when every currently routed indexer has coverage for the current criteria fingerprint. Routing or criteria changes reopen the scope automatically.
- The fingerprint includes quality-profile identity/version, required audio languages, and match identity; subtitle preferences deliberately do not reopen acquisition coverage. Availability gates and recent/hot windows favor newly missing episodes, recently available movies, and recently added titles while a persisted cursor rotates through colder work.
- Convergence cycles are quota-aware, pause the affected facet during an active library scan, resume from the saved scope cursor, and mark only indexers that actually fired.
- Converged scopes return to RSS monitoring. Background and interactive searches use the same coverage accounting.
- Wanted is organized as Missing, Upgrades, Pending, and History, with searching, converged/“Watching RSS,” deferred, and hot/cold queue states plus routed/covered indexer information.
- Missing and upgrade searches create single-flight background acquisition-search jobs, expose processed/grabbed/failed progress, can be canceled, and remain visible while the browser session retains the job identity.
- Title-scoped interactive search requires `ManageTitles`; broader library/facet search requires `ManageCatalogSettings`; read-only wanted views remain scoped by library `View` access.
- Cutoff-unmet and pending-release reads are paged and bounded. Due pending releases are processed before fresh RSS work.
- One upstream scheduler now coordinates quotas, cooldowns, retry feedback, cost/value admission, and stable jitter across outbound clients. Public hosts default to 20 requests/second with burst 20, managed/local hosts to 10/second with burst 20, and loopback remains unthrottled.
- Automatic searches learn the best identifier strategy per title/indexer/facet, retain text fallback, suppress an ID strategy after three valid empty results when another strategy works, and re-probe suppressed strategies after seven days. Interactive searches are never suppressed by that learning.
- RSS is now scheduled per indexer instead of following the old wanted-row cadence. It targets 15-minute freshness, can stretch to 60 minutes under low quota, and probes an exhausted quota after six hours. Cold convergence yields before higher-value work while overdue RSS is protected from a saturated acquisition backlog.
- Scheduler state is persisted and flushed during graceful shutdown.

### Byparr indexer proxies

Indexers can reference a managed Byparr challenge-solving proxy.

- Proxy configurations include base URL, timeout, enablement, health, last error, create/edit/test/delete actions, and explicit direct-versus-proxy routing per indexer.
- Challenge responses invoke the bounded `request_solution_v1` flow. Scryer applies an allowlisted subset of returned headers/cookies, caches solved sessions with TTLs, records proxy health, sanitizes errors, and feeds typed rate-limit signals back to the shared scheduler.
- Missing or disabled assignments remain visible with warnings instead of being silently rewritten.
- Managed child indexers inherit their parent's proxy choice. An enabled proxy cannot be disabled or deleted while enabled indexers depend on it; base URLs/timeouts are normalized and the default timeout is 60 seconds.
- Newznab protocol errors 100–107, 200–203, 300, 500–501, 900, and 910 are translated into actionable validation messages instead of opaque repository failures.
- Proxy configuration and revision participate in indexer-client cache invalidation and backup/restore.

### Restartable multi-instance Sonarr, Radarr, and Prowlarr import

External import is now a five-step Connect, Libraries, Quality, Sources, and Summary workflow.

- Multiple Sonarr and Radarr instances plus Prowlarr can be connected and validated independently. Arr warmups run concurrently, Sonarr episode loading is bounded to 16 requests per instance and two active instances at once, and duplicate clients/indexers are merged across direct and Arr-linked sources.
- Source roots can be dragged or assigned into new Scryer libraries, returned to the unassigned pool, supplemented with manual roots, and remapped from the source-visible path to the path visible on the Scryer host while retaining provenance.
- Each mapped library receives a quality profile and scoring persona before imported clients/indexers are selected and finalization begins.
- Imported configuration can include roots, clients, indexers, titles, naming, media-management permissions, metadata providers, quality/request profiles, title quality/availability/language/tags, monitoring, NFO/Plexmatch behavior, anime-special handling, and Linux chmod/chown settings.
- Sonarr v5 and earlier Sonarr/Radarr route differences are handled explicitly. Redirects remain disabled, including cross-host redirects.
- Non-secret browser state is retained in `sessionStorage`. API keys and imported client/indexer secrets are excluded from browser persistence and stored in an owner-scoped server secret draft.
- Secret drafts are encrypted and require the database encryption key. Warmed source evidence suggests monitoring, profiles, NFO/Plexmatch, naming, and Unix permission settings only after at least three samples and 85% agreement; conflicting evidence is reported, and only an allowlisted safe subset is applied automatically.
- Warmups use resumable server sessions, while monitor snapshots are persisted as session-scoped chunks. Expired or pruned sessions return the operator to reconnection rather than spinning forever, and created-library IDs survive retries so finalization does not create duplicates.
- Large source discovery persists paged snapshot chunks with progress, scan hints, cancellation/removal, and a serialized apply guard. Terminal warmup sessions expire after two hours, and stale non-apply chunks are cleaned without turning temporary setup secrets into permanent catalog state.
- The wizard will not finish until every source is mapped, required credentials are present, and monitored-status warmups succeed. If another administrator owns the active draft, it must be explicitly replaced.

### Faster, concurrent, and more observable library workflows

- Library enumeration, title matching, metadata hydration, and file analysis now stream progress instead of waiting for whole phases to complete.
- Walking and analysis share global concurrency limits, but each library can still scan or cancel independently—even when several libraries belong to the same facet.
- Sonarr/Radarr hints and fuzzy metadata lookups are batched to gateway limits; multi-root scans share one session; empty libraries report deterministic zero totals.
- Background scans are additive, while interactive full scans can reconcile removals. Background refresh runs every two hours with a stable per-facet initial jitter spread across six hours.
- Source signatures based on platform file modification state allow unchanged files to skip repeat analysis. Legacy rows missing a signature are backfilled without forcing unnecessary reanalysis.
- Duplicate files are represented as primary/additional copies; invalid multiple-primary state is repaired; full paths disambiguate identical leaf hints; unreadable files remain pending rather than being falsely imported.
- Scan toasts show library/facet identity, phase counts, ETA, completion/review/failure state, cancel/background actions, and links to affected titles or unmatched imports. Missed events are reconciled against authoritative job records.
- Pending-import review now uses complete title metadata and deduplicated external IDs rather than only a TVDB ID. Movie matches leave the list immediately; series/anime matches can use parsed season, episode, and absolute-number hints for explicit binding.
- Match, bind, and ignore actions are guarded per item to prevent duplicate mutations. Existing title folder paths are preserved during stale or missing-file resolution.

### Plugin SDK 3.6 and a native Wasmtime host

Scryer's plugin platform expands substantially in 0.17.

- The supported SDK moves to 3.6 and adds query facets, path/tag configuration fields, richer download-add context, archive-extractor contracts, and subtitle-sync command models.
- Scryer replaces the Extism host dependency with a native Wasmtime compatibility host for the existing Extism-PDK ABI, including config, vars, logging, HTTP, socket, process, caps, and epoch cancellation support.
- New command-model hosts provide sandboxed archive extraction and subtitle synchronization with read-only inputs, writable outputs, path/symlink escape protection, timeouts, memory/output limits, describe-time validation, and archive crypto helpers.
- Archive extraction is selected dynamically by supported format instead of using bundled ZIP/7z/RAR libraries. Archived downloads can remain blocked until an Archive Extraction plugin is installed and enabled.
- Scryer continues to own archive-set discovery, first-volume selection, PAR2 filename recovery/repair preparation, destination validation, and import. Extraction uses a hidden destination-side staging directory, cleans stale/completed staging state, and requires a resolved destination. Output is capped at 20,000 files, 20,000 directories, 40,000 total entries, and 2 TiB expanded data; traversal/symlink escapes are rejected and extraction must yield usable video.
- Staging prefers clone/copy-on-write. Ordinary non-clone extraction refuses files above 64 MiB instead of silently duplicating very large archives. Titleless archives can be identified from the extracted video and moved to the resolved title, including a cross-device fallback.
- Plugin-backed download clients can receive resolved NZB bodies or URLs, magnet URIs, and prefetched torrent payloads in the form each client supports.
- Subtitle searches/downloads now use the shared upstream scheduler. Automated fan-out omits deferred providers; interactive deferral returns temporary-unavailable semantics; rate-limit and retry feedback updates the common cooldown state.
- Enhanced subtitle sync reuses stored container/video/audio/subtitle/chapter metadata instead of probing the media again. ZIP/7z/RAR subtitle bundles use the archive plugin, while TAR and supported single-file compression remain local; plugin output is bounded and symlinks are ignored.
- Provider-declared setup fields support secret, boolean, number, text, select, tag, path/file, default, and host-bound values.
- Provider artwork identifies indexers, download clients, notifications, subtitles, archive extractors, and other integrations throughout setup and settings.

### Hardened plugin trust boundaries

- Host-process and raw-socket capabilities are restricted to first-party/official plugins; uploaded, manually installed, unverified, and verified-community plugins cannot request host-process execution.
- Spawned processes receive a sanitized `PATH`; `LD_*` and `DYLD_*` variables are removed; output readers are bounded.
- Plugin HTTP uses DNS pinning and redirect revalidation and blocks cloud-metadata/link-local destinations, including `metadata.google.internal` and IPv4 `169.254.0.0/16`.
- RFC1918, loopback, and IPv6 ULA destinations remain available for self-hosted integrations.
- Plugin catalog status is cached rather than probing the network on every read. Installed plugins remain manageable during catalog outages, incompatible uninstalled entries are hidden, and the highest compatible release is selected from the complete release set.

### Reactive Refresh v2, persistent notices, and per-user display settings

- One `domainEventFeed` subscription now drives scoped refresh predicates by title, event type, stream kind, or facet. A persisted sequence cursor fills gaps after reconnect.
- Reconnect starts after three seconds; after three consecutive failures the UI falls back to 30-second polling until the feed recovers. Refresh aliases are debounced and epoch-aware to avoid broad over-refresh storms.
- The shell owns persistent automatic-backup-key warnings, SMG/Scryer compatibility/update banners, and navigation badges for pending imports/requests, manual-import work, plugin updates, and the running Scryer version.
- Enrolled SMG integrations persist deprecated/incompatible-version and available-Scryer-update notices. Compatibility is refreshed on a stable six-hour phase, with the first periodic check held for 30 minutes after startup; enrollment incompatibility is recorded immediately.
- Per-user settings cover light/dark theme, regional or ISO/24-hour dates, highlight and secondary colors, contrast, reduced motion, sponsor visibility, density, sidebar mode, default landing page, and ordered per-view table columns.
- Accent choices generate the full runtime color palette; preference loading resets correctly across authentication-session changes and avoids user-scoped requests on the unauthenticated login page.

### Media permissions and richer rename templates

- Media/library settings add opt-in Unix permission application, common file/folder mode presets, validated custom octal values, symbolic previews, file-mode derivation from folder mode, and optional group/GID ownership.
- On Unix, hardlink import now requires the source UID to match the Scryer process UID; foreign-owned sources are copied and destination ownership is verified. Chmod/chown is best-effort, and applying it to a hardlink also changes the shared download-side inode.
- Folder and filename templates are edited separately with facet-aware validation, autocomplete, live token references, escaped literal braces, numeric padding, filter chaining, `season_order`, `absolute_episode`, `truncate:N`, and whitespace replacement filters.

## Upgrade considerations

### Plan for a longer first startup

The 0.17 database upgrade is materially heavier than a typical patch migration. Back up the datastore, leave migrations enabled, avoid interrupting the first start, and allow additional database, disk, and metadata-gateway activity.

- Migration 140 converges prerelease schemas and runs Rust hooks for schema normalization, title-image blob migration, and a full-title ICU/CLDR catalog-sort-key backfill.
- The image hook reads legacy variants in 128-row pages, hashes and deduplicates their bytes into content-addressed blobs, replaces the variant table, and clears local paths for invalid cache rows. SQLite can also rebuild external-import snapshot chunks for session-scoped keys.
- The rollup creates the persistence for Discovery, canonical metadata tags and ratings, restartable external-import secrets/snapshots, more-like-this, upstream scheduling, workflow-operation indexes, and user UI/table settings. Existing genres are projected into canonical metadata tags.
- Migrations 141–146 add indexer proxy configuration and scope/indexer coverage, drop retired wanted scheduling columns and event outboxes, stop retaining raw Discovery pages, and add a personalized-Discovery index.
- SQLite and PostgreSQL fresh-install baselines move from migration 122 to 140.
- After database migration, a one-time background metadata rehydration runs across the full title catalog. It records completion, retries on a later startup after failure, and can increase metadata-gateway traffic until complete.
- The sort-key backfill updates every existing title in a transaction. Its bytes depend on the bundled ICU/CLDR data version, so future collation-data changes may require another regeneration.

### GraphQL clients must migrate with the server

0.17 intentionally changes the GraphQL contract. Do not pair the 0.17 frontend with a 0.16 server, and regenerate/update custom API clients and browser extensions.

This is a large contract change: the repository checker reports 559 breaking and 396 dangerous changes. Most come from the uppercase enum cutover (350 removed and 376 added enum values), followed by 142 removed fields and 47 field-kind changes. The remainder includes seven required inputs, six removed arguments, six removed types, one type-kind change, 17 optional inputs, and three optional arguments.

Major migrations include:

- lowercase/camel-case enum values to canonical uppercase values across facets, permissions, activity/domain events, downloads/imports, jobs, plugins, providers, monitoring, settings, and wanted state;
- legacy boolean/opaque mutation results to semantic payloads and typed result objects;
- stringly typed actor, stream, collection, episode, execution-mode, transfer-phase, queue-scope, and similar fields to enums or unions;
- string-encoded job/history/scoring/WebAuthn and related payloads to GraphQL JSON scalars, and parallel provider configuration value fields to the `ProviderConfigFieldValue` union;
- page payloads to `items`, `totalCount`, and `hasMore`, including history's `records` to `items`, while many delete/accept/clear/dismiss/restore mutations now return the affected identity or timestamp instead of a redundant success boolean;
- legacy wanted-search mutations to `triggerAcquisitionSearch`, `acquisitionSearchJob`, and `cancelAcquisitionSearch`;
- Sonarr/Radarr-specific monitor warmup inputs to typed Arr sources, warmup sessions, mappings, connection validation, and secret drafts;
- `cutoffUnmetTitles` to paged `cutoffUnmetTitlesPage`;
- removal/replacement of `libraryScanSession`, singular `mediaServerConnection`, and older monitor-warmup roots;
- new Discovery, catalog filter, indexer proxy, UI settings, episode/collection, and pending-import search operations.

The full compatibility output is retained with the release-note audit evidence.

### Review acquisition settings and schedules

- `acquisition.sync_interval_seconds` and `acquisition.batch_size` no longer drive the runtime.
- `acquisition.long_tail_backfill_max_scopes_per_cycle` defaults to `500`; it limits evaluated scopes per cycle rather than acting as an HTTP rate limiter.
- `acquisition.long_tail_reconverge_days` defaults to `0` (off) and is a break-glass stale-coverage backstop.
- `acquisition.poll_interval_seconds` remains.
- The cutover performs a best-effort, idempotent seed that treats legacy searches from the prior 14 days as covered across their routed indexers. This avoids an immediate full-library search storm; coverage reopens when requirements change or a failed search/download needs retry.
- Metadata Refresh and Wanted Sync jobs are removed. Discovery Sync has startup/dynamic behavior plus a daily backstop; RSS is scheduler-paced per indexer; background library refresh changes from hourly to every two hours.
- Metadata refresh is now lifecycle-driven: monitored active episodic titles and prerelease movies are due after 12 hours; active unmonitored titles and recently released movies after 24 hours; inactive episodic titles after 14 days; and older movies after 30 days. Background passes cap work at 100 titles, while user-forced refresh is uncapped.

### Environment-variable changes

Only one new environment variable is intended as a normal production tuning control:

- `SCRYER_OUTBOUND_HOST_RPS` is a new operator-facing override for the positive finite public-host request rate. The public burst remains 2; local/managed and loopback profiles are unchanged.

The remaining changes are specialized or internal:

- `SCRYER_DOWNLOAD_QUEUE_POLL_INTERVAL_SECS` and `SCRYER_DOWNLOAD_QUEUE_RECENT_HISTORY_POLL_INTERVAL_SECS` are test/diagnostic overrides, not normal production tuning controls.
- `SCRYER_NFO_PROFILE_ROOTS`, `SCRYER_NFO_PROFILE_LIMIT`, `SCRYER_WALK_PROFILE_ROOTS`, and `SCRYER_WALK_PROFILE_LIMIT` are used by profiling harnesses.
- The legacy notification-plugin child-process environment no longer includes `SCRYER_TITLE_GENRES`; this was not an operator configuration toggle.

### Configure a download client explicitly

Scryer no longer creates an implicit NZBGet client from legacy service settings. If no download client is configured, the queue is empty instead of silently falling back to NZBGet.

Legacy routing/category values are copied into their current settings when the current value is absent, but operators who relied only on the implicit fallback should verify an enabled download client before upgrading.

### Plugin compatibility and archive extraction

- The runtime SDK constraint is now `>=3.6.0,<4`. Plugin descriptors and builds must be compatible with SDK 3.6.
- Capability policy is stricter, especially for host-process, raw socket, process environment, HTTP egress, DNS, redirects, and cloud-metadata destinations.
- Archive extraction is no longer bundled into the application dependency graph. Install and enable a compatible Archive Extraction plugin before importing archived downloads.
- Backups include installed plugin state and expanded portable application data, but runtime trust/compatibility checks still apply after restore.

### Backup and restore scope changed

Backups now preserve durable convergence coverage and cursor state, indexer learning and proxy configuration, canonical metadata ratings/tags, and per-user UI/table settings.

Discovery data, more-like-this results, and all centralized title-image cache records are deliberately omitted. After a restore, Scryer will synchronize Discovery again and its bounded background image workers will download and process artwork from the restored remote source URLs. Remote artwork remains available while local variants are rebuilt, but operators should expect a temporary increase in network and image-processing activity.

Temporary external-import secret drafts and upstream quota/cooldown/RSS cadence state also remain non-portable. This backup ownership correction requires no GraphQL, environment-variable, or database migration changes.

### UI routes and local state

- Canonical routes now include `/automation/wanted/items`, `/automation/acquisition`, `/integrations/indexers`, `/integrations/download-clients`, `/system/users`, `/system/backup`, `/system/recycle-bin`, `/logs`, and `/logs/audit`.
- Documented 0.16 aliases—including `/wanted`, older `/settings/*` integration/backup/recycle-bin paths, `/system/audit`, and `/movies/overview`—redirect while preserving query strings and hashes. Unknown roots/invalid subsections show not-found instead of silently falling back.
- The standalone movie overview and import-history views are replaced by the title workspace and Wanted History. Recycle Bin is grouped with System pages.
- Discovery dismissals, catalog view modes, and acquisition-search job continuity use browser-local/session storage and do not roam across devices.
- Fresh/unauthenticated UI settings default to Dark. Stored `PRIDE`/`SYSTEM` values are still understood by the wire mapping, but Pride is no longer offered in the standard theme selector.

## Bug fixes and hardening

### Downloads and queueing

- SABnzbd now checks its queue and history after an ambiguous `addfile` response instead of replaying the request or failing over to another client. Explicit `status: false` responses remain definitive rejections, preventing duplicate submissions.
- Titles containing SABnzbd-illegal characters are normalized for reconciliation, and the compatible SABnzbd API path is resolved correctly.
- Staged NZBs are reused across failover, cleaned up after final failure, and not refetched unnecessarily. Username/password authentication remains supported.
- The router sends only the NZB, torrent, or magnet forms that the selected client declares compatible.
- Byparr/proxy responses stay on their configured origin, are rechecked against outbound-target policy, and are capped at 64 MiB. Torrent bodies are capped at 32 MiB and require a valid top-level bencoded `info` dictionary; HTML challenge/login/error pages are rejected before submission.
- API errors now distinguish rejected, ambiguous, unavailable, plugin-required, canceled, and temporarily unavailable operations; temporary deferrals can include `Retry-After`.
- Release queue actions use typed episode, episode-set, collection, series-movie, or title scope. Orphan scopes cannot be converted into a lossy queue request.
- Pending force-grab no longer claims success when the server returns `grabbed: false`.

### Acquisition targeting and recovery

- RSS evaluates the current derived target at match time and materializes state only for an anchored decision/grab. Completed submissions suppress an initial missing search without incorrectly suppressing a scored upgrade; active submissions suppress both.
- Season packs are attempted only when at least two selected episodes share a season. A viable/pending pack prevents redundant episode searches, while a failed pack reopens and falls back to the episode scopes.
- Failed downloads reopen convergence. Standby candidates survive client snapshots and can be recovered; replacement, conflict, manual reopen, cancellation, diagnostics, and mismatch recovery no longer depend on legacy wanted rows.
- Automatic episode fan-out deduplicates structured *nab shapes only when semantically equivalent, while text indexers retain distinct absolute-number, `SxxEyy`, and season searches. Cancellation propagates and callers can restrict work to uncovered indexers.
- Anime movie/series-movie searches use the owning title's tags and language context, fixing dual-audio inference, and Discovery excludes series-movie results already present in the library.

### Library scans, files, and imports

- Scan state is keyed by library as well as facet, fixing one library being shown busy because another same-facet library is scanning.
- Title detail refreshes preserve already-loaded episode/file/recommendation data and prune only entries that genuinely disappeared.
- File deletion updates episode and series-movie maps consistently; series-movie expansion fetches its actual media association.
- Specials headings localize correctly even when upstream labels are not English, and aggregate owned/monitored/total counts are preferred when available.
- Same-title sibling duplication, clean-folder pollution, ambiguous leaf-path hints, multiple-primary records, and redundant analysis are handled more defensively.
- Movie evidence can use a stable direct-child scan instead of unnecessary recursion. Folder discovery ignores recycle/trash/system/cache, trailer, extras, featurette, trickplay, Plex version/theme-music, and DOS 8.3 paths while retaining valid non-UTF-8 entries; symlinked folders are deduplicated and recursive depth remains bounded.
- NFO matching no longer treats a bare movie/show `<id>` as provider identity. Explicit `uniqueid`, provider attributes/URLs, Jellyfin identity, normalized provider IDs, BOM/Plexmatch content, and scan hints are prioritized without letting episode IDs become title identity.
- Pending imports do not race match/bind/ignore mutations or create incomplete titles from identifier-only selections.
- Import placement snapshots and revalidates the source and destination parent, retries transient copy failures, verifies copied content before move cleanup, refuses cleanup after path replacement, and removes a destination whose ownership verification fails. Transfer progress is heartbeat-aware, and newly created folders receive their configured permissions.
- Active import identity is scoped by client/system/download identity; reused client item IDs clear stale terminal state, episode-set signatures persist, large client lookups are chunked, and uncataloged duplicate-file skips no longer suppress a later valid import.
- Title create-or-get and pending-import binding are transactional, preserve identical external identities independently per library, and restrict linked series-movie lookup to the parent title's permitted libraries.
- Stale pending-import rows and missing loose files no longer clear an existing title's valid folder path.
- Invalid rename-template tokens are rejected before save; upgrade housekeeping correctly handles encoded media roots and staged replacement cleanup.

### Post-download validation, audio, and subtitles

- Automated imports reject provably bad media: ordinary movie/episode samples around 20 seconds, zero/indeterminate runtimes, unknown content below one minute, and files changed after probing. Short-form titles above their expected floor remain valid.
- Manually queued imports can bypass the automated sample gate, and forced manual replacement can intentionally accept a lower score. MediaInfo rescoring, user rules, and canonical analysis persistence run before final acceptance.
- Required-audio validation distinguishes absence from uncertainty. Stream tags and unambiguous track-title tokens are normalized; codec tokens and ambiguous two-letter words are ignored; `LAT` maps to Latin-American Spanish. Missing required audio is rejected only when every stream is resolved and proves absence, while uncertain evidence produces `audio_language_warning`.
- Release-name hints can satisfy unresolved tracks. `DUAL` consistently means English plus inferred original language for non-anime titles; anime tags preserve English/Japanese inference for anime movies.
- Subtitle scores are clamped to display percentages. Archive selection retains episode/language/format ranking, and sync records alignment, split/no-split scores, selected frame-rate ratio, consistency, and explicit skip reason. Stored audio stream names improve sync and rules targeting.
- MediaInfo retains human-readable stream/track titles even when the language tag is missing or `und`, improving language fallback, display, subtitle sync, and rule matching.

### Search, catalog, metadata, and media requests

- Search avoids stale/duplicate results after authorization changes and filters metadata identities already represented by any owner copy.
- Poster failures degrade to a deterministic generated treatment instead of broken/unstable image slots.
- Artwork refreshes normalize relative/duplicate-separator URLs and commit processed bytes only if the title still points to the requested source. Content-addressed blobs deduplicate identical images, reject digest conflicts, wait for the last reference before collection, and repair polluted local URLs or invalid variants through remote fallback/reprocessing.
- Local/TMDB poster and backdrop URLs select appropriate variants while preserving cache-version behavior.
- TVDB links use correct movie/series slug routes and a safe fallback.
- Media requests hydrate missing external identity, facet, artwork, and year before duplicate checks, preventing partial identifiers from bypassing existing-library/request detection.
- Duplicate external title rematches are rejected.
- Catalog sort and filter state respects library-view permissions and handles multilingual titles, CJK width, source ratings, and root validity consistently.

### Authentication and authorization

- A stale authenticated token rejected by authless proof is cleared and retried through the logged-out web-client proof flow instead of trapping bootstrap/login.
- MFA-enrollment tokens are not accepted as ordinary authenticated sessions.
- Authentication bootstrap is deduplicated, permission claims are normalized, rate-limited startup requests retry sanely, and login shows a specific rate-limit message.
- Configuration MFA step-up state, expiry, forced challenges, and settings refresh are centralized so sensitive actions cannot reuse an expired token.
- Security reauthentication uses the current signed-in username and requests only the password, preventing a different identity from being supplied during confirmation.
- Jellyfin login now follows required MFA enrollment, including pending invites for an existing user.
- Permission changes revoke prior tokens until re-login, and library-scoped media/recycle-bin actions enforce the target library grant in addition to global capability.
- Pending-import title search and resolution require `ManageTitles` on the unmatched item's library, while indexer-proxy management requires `ManageSystemSettings`.

### Plugins and integrations

- Descriptor-owned username, password, and API-key values no longer get overwritten by empty legacy fields. Blank optional secrets are omitted and typed number/boolean/tag values preserve their type.
- Plugin installation reports checksum, SDK metadata, compatibility, and other structured errors with actionable messages.
- Registry refreshes are coalesced and briefly suppressed after completion so subscriptions do not launch redundant refresh work.
- Download-client drafts deep-clone dynamic config and save only against the selected provider's declared fields.
- Disk-space health checks are correctly Unix-only, and multi-library health roots are deduplicated by normalized path.

### Storage and restart recovery

- Daily SQLite maintenance runs a full `VACUUM` only when at least 2,000 pages and 10% of the database are free, avoiding a costly no-op on every maintenance tick. Unreferenced image blobs are pruned in bounded batches.
- Startup ownership repair continues past vanished entries, unreadable submounts, metadata failures, and individual chown failures, then reports the aggregate count and first error instead of abandoning later siblings.
- Job-backed workflow rows left queued/running/discovering by a process restart are marked failed with an interruption reason and completion time, and stale progress is cleared so jobs and acquisition views cannot remain active forever.

### Release parsing and generated service aliases

- Episode ranges such as `S3.01-02`, `S3-01-02`, and `Season 1 - 001-020` parse correctly.
- Fused daily dates, month-name dates in either order, fused/decimal frame rates, CAMRip/HDCAM, DTS-HD-MA hyphenation, accented tokens, and canonical edition casing are recognized.
- Ambiguous AMZN/NF WEBRip names normalize to WEB-DL.
- Dolby Vision by itself no longer implies HDR, while explicit HDR10, HDR10+, and DV-HDR still do.
- Eight-digit bracket tokens remain checksums; ordinary title numbers, color depth, and PROPER/REPACK revisions are no longer misclassified as frame rate, season data, or editions.
- TRaSH Guides streaming-service alias generation strips regex word-boundary escapes and rejects digit/one-letter/generic source tokens, alternation fragments, and rename-template artifacts that previously produced false aliases.

### UI correctness and feedback

- Routine form-login denials of the authless web-client proof probe log at debug instead of flooding warning logs; rarer proxy/cross-site failures remain warnings.
- Quality-profile background refetches use an apply epoch and preserve an operator's in-progress draft.
- Scan and job status use authoritative uppercase terminal state, preventing finished work from remaining active in the UI.
- History, file, and subtitle dates now follow the selected account format.
- Title history consumes the semantic `items` payload and canonical event types; jobs derive active-run state from the authoritative jobs payload.
- Recycle-bin failures distinguish quarantine from successful deletion instead of displaying a false success.
- Rego diagnostics compensate for the hidden import line, and post-processing scripts receive the correct shell-language editor.
- Rename templates preserve escaped literal braces and apply `truncate:N` and chained whitespace/padding filters in the documented order.
- Setup path selection distinguishes files from folders, validates fresh media paths before continuing, allows revisiting completed steps, and presents clearer restore/setup recovery state.

## Audit scope

These notes were built from an exhaustive path-partitioned audit of the complete `scryer-v0.16.8..release-0.17.0` diff. Generated baselines, contracts, assets, fixtures, tests, deleted implementations, and internal refactors were included in the accounting rather than filtered out by perceived risk.
