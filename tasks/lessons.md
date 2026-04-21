# Lessons Learned

## Environment & User Trust
- **Never question the user's environment state.** When they say something is broken, the code is freshly built and the environment is clean. Go straight to debugging the code.
- **Never suggest restarts/rebuilds as a fix.** The user runs `stack-restart.sh` on every Rust change. Assume this is done.
- **Don't assume you know better than the user about their environment.** If they say the container was rebuilt, it was rebuilt.
- **Treat Cargo concurrency limits as repo-scoped, not workspace-shell-scoped, when this shell contains multiple independent repos.** Avoid concurrent Cargo work inside the same Rust repo, but do not block unrelated repo work when the user confirms the active jobs are in a different project.

## Destructive Actions
- **NEVER execute destructive actions on live services without explicit permission.** This includes deleting downloads, killing processes, dropping data, etc.
- Do not restart Docker containers, bounce the dev stack, or interfere with in-progress builds unless the user explicitly asks for that action. A container restart can invalidate active debugging/build work even if the code change itself is correct.
- Only suggest commands for destructive operations. Let the user decide and execute.
- Even if something looks like "test data" or "leftover" — it might be a valid in-progress item the user cares about.

## NZBGet Integration
- NZBGet v21 `append` API expects post-processing parameters as `[{"Name": "key", "Value": "val"}]`, NOT `[{"key": "val"}]`. The latter is silently ignored.
- The `extract_nzbget_parameters` function correctly parses `Name`/`Value` pairs — both sides must agree on the format.

## SQL / SQLite
- When adding JOINs to existing queries, prefix ALL column references in WHERE and ORDER BY with table aliases to avoid ambiguous column errors.
- Count queries without JOINs don't need prefixes — only fix the queries that actually have the JOIN.

## Full-Stack Schema Changes
- When adding fields to a GraphQL type, update ALL layers: migration → application types → infrastructure queries → interface types/mappers → **frontend TypeScript types** → frontend GraphQL queries.
- Frontend types are manually defined (no codegen). Check `TitleMediaFile` in movie-overview-container, `EpisodeMediaFile` in series-overview-container, and `MediaInfoFile` in media-info-badges for media file schema changes.
- Don't declare a schema change "done" until the frontend builds clean with the new fields flowing end-to-end.
- Treat each serialization boundary as authoritative for its own casing and enum values. Raw `payload_json` must be decoded as serde-shaped JSON, while top-level GraphQL envelope fields must be decoded through the GraphQL-mapped enums instead of assuming both surfaces use the same representation.

## UI Organization
- **Per-facet/per-category settings belong in the per-category section**, not in a general settings section. When a feature varies by media type (movie/series/anime), put its UI alongside the existing per-category controls (e.g. "Default category profiles"), not inside the profile editor's general scoring section.
- **Don't mix preset selection with fine-tuning knobs** in the same visible section. If a user picks a persona preset, showing override toggles right next to it is confusing — collapse advanced overrides behind a sub-`<details>`.

## Search Scoring
- **Validate persona promises against real release examples before shipping scoring tweaks.** If the UI says remux is an Audiophile concern, Balanced/Efficient/Compatible should not still reward remux-heavy anime releases through hidden defaults or oversized file bonuses.

## Tracked Downloads
- **When reconstructing tracked-download state, preserve a durable import-history fallback until terminal tracked state persistence is proven end-to-end.** Otherwise a completed import can be resurrected after restart and re-enter the workflow incorrectly.
- **In import verification, completion should be based on expected logical units being satisfied, not on the mere presence of rejected artifacts.** Extra rejected files must not block a fully satisfied download.

## Planning & Type Design
- **When converting string workflow states, follow the existing serde-enum pattern already used in the codebase instead of inventing new constant-only patterns.** Keep text serialization at the persistence boundary and make the enum authoritative inside Rust.
- **When moving conditional persistence logic out of SQL and into Rust, preserve the original atomicity guarantees.** If the old query did read/modify/write in one statement, replace it with a transaction or a single repository command, and keep low-level upserts defensive against accidental bypasses.
- **When the user narrows a task to backend planning or backend stabilization, do not broaden the work into frontend validation or install repair unless they ask for it.** Keep the active scope tight before major refactors.
- **Never create unsigned commits or tags.** Before any commit or release rewrite, verify the real SSH signer path works first; if a pushed unsigned commit must be fixed, rewrite from the earliest bad commit and re-sign every descendant plus any affected release tags.

## Indexer Search Contracts
- **Do not bake alias language or script policy into core search query construction.** Core should pass canonical title plus tagged alias context through to indexer plugins, and plugins should decide whether to prefer romanized Japanese, Korean aliases, or other provider-specific naming conventions.
- **Pass an explicit normalized facet through the plugin search contract.** Plugins should get `movie` / `series` / `anime` directly instead of reverse-engineering semantics from category strings or host-side ID heuristics.
- **Pass IDs through the host/plugin boundary as a filtered map, not fixed `imdb_id` / `tvdb_id` / `anidb_id` slots.** The host may filter to supported IDs for a strategy, but provider-specific query shaping from those IDs belongs inside the plugin.
- **When the user asks for logging via `RUST_LOG`, do not add plugin config fields or descriptor changes.** Prefer existing runtime log filtering and keep observability changes out of the plugin contract unless the user explicitly asks for new config surface.

## urql / Frontend Caching
- `cacheExchange` was removed from all urql clients — the network layer handles caching naturally.
- Don't add per-query `requestPolicy` overrides; the exchange-level removal is the correct fix.

## Metadata Gateway / Library Scan
- When the user asks for deterministic scan debugging, stop inferring from aggregate counters and add targeted instrumentation that proves tracked-title attachment, unhydrated-title selection, hydration outcomes, and projection completion end-to-end.
- If movie scan metadata hydration looks effectively one-at-a-time while SMG batch search is fast, inspect whether the scan loop is awaiting per-file finalization or media analysis between title attachments. That upstream serialization can starve the hydration loop and collapse bulk metadata fetches into batch sizes of 1-2.
- When analyzing SMG rate limits, distinguish the coalesced background hydration path from the per-candidate library-scan preload path. Bulk hydration coalescing can still exist while scan preload search fanout remains uncoalesced; scan-time request bursts should be fixed in `preload_*_library_scan_candidates`.
- When the user wants a first-class SMG batch search API, implement a dedicated SMG GraphQL field and keep Scryer's transport on that field instead of expanding client-side `searchTvdb(...)` aliases.
- When SMG only has one real client, trim shared search contracts to the fields Scryer actually consumes instead of preserving a generalized rich payload that forces unnecessary hydration or localization work.
- For SMG GraphQL schema changes, always run `go run github.com/99designs/gqlgen@v0.17.87 generate` and verify both generated files update before calling the work finished.
- Batched metadata requests should bypass APQ entirely. Their variable/cardinality entropy makes persisted-query cache hits unlikely, so the APQ GET and registration round-trip is just wasted work.
- Long-running full-library scan triggers should start background work and return immediately. Holding the GraphQL request open couples scan lifetime to client connection lifetime and makes disconnect-driven cancellation orphan scan sessions.
- Keep library scan batching chunk-local. Do not buffer the entire discovered file or folder set before processing matches, imports, or queued title scans, or scan progress will stall until discovery completes and the library will appear to load late.
- When scan-time metadata discovery depends on external batch search requests, process each resolved batch through the rest of the pipeline immediately and publish progress between awaits. Do not make the UI wait for the whole metadata fanout before advancing work.
- Keep user-visible scan denominators stable. A phase can accumulate more work internally while discovery continues, but `*_total_known` should stay false until the denominator the UI will render is final; otherwise the progress percentage moves backward and feels broken.
- If a scan phase stays indeterminate until essentially the terminal event, do not render it as a progress bar in the toast. Keep backend tracking if it is still useful internally, but hide the user-facing bar.
- Library scans can attach pre-existing titles that still need hydration. Those titles do not pass through title creation, so scan tracking must explicitly wake the background hydration loop when it starts counting them, or metadata progress can stall with counted titles never entering hydration.
- For runtime library-scan tracking, workers must emit durable scan fact events and a single backend coordinator must subscribe to the central domain-event bus to project tracker state and publish compatibility scan snapshots. Do not let worker code or coordinator entrypoints mutate the in-memory scan tracker directly.
- Final scan-phase reconciliation must read the projected durable session state, not worker-local counters. If the worker assumes its local counts are authoritative, the durable event stream can still end short of terminal title/file counts and leave the session stuck in `running` even after the worker logs completion.
- Transient SQLite busy handling in library scans must stay fully internal. Route hot scan writes through the serialized `DbCommand` lane and keep retrying there until they succeed so scan summaries, unmatched items, and UI contracts never surface a separate deferred state.
- Prefer extending the existing serialized `DbCommand` write lane for hot SQLite write paths like title create-or-get and title image replacement. Reuse existing scan-aware wait patterns for background workers before inventing a heavier global coordinator.

## Frontend Reactive Refresh
- Debouncing websocket-driven refreshes is an action-spooling problem, not a subscriber-coupling problem. Events should be collected by action type and deduped so a 300ms flush performs one instance of each action, instead of each subscriber independently firing duplicate network requests.
- Do not scope reactive batching to one container. Event-driven refreshes must enqueue into a shared root-level spool so title lists, title overviews, import history, and any future reactive query actions can collapse into the same aliased GraphQL flush.

## Frontend Search Scope
- When the user narrows a search UX fix to title-owned search surfaces, do not broaden it back out to global search, add-title flows, or result queue actions. Gate the actual title search entrypoints the user named.

## Frontend Mobile Layouts
- When fixing mobile regressions in shared views, inspect the dedicated mobile branch of the component instead of assuming the desktop table layout covers it. Reuse shared action-tone helpers so card actions and table actions stay visually aligned.

## Frontend Toast Boundaries
- If Sonner toast content depends on app-level React context, the `Toaster` must be mounted beneath those providers. A global toaster in `src/main.tsx` cannot safely render route-level contexts like translation or library-scan state.
- For scan-toast phase bars, avoid amber/orange for normal file-analysis progress unless the user explicitly wants a warning-like accent. Prefer the app's established purple family when the user asks for an in-theme alternative.

## Benchmarking / Probes
- When the user asks for quick live probes, use simple one-shot measurements against the real endpoint for the exact sizes they named. Do not spend time reconstructing earlier benchmark commands, mining shell history, or building a reusable harness unless they explicitly ask for deeper benchmarking.
- If the user names a specific environment or host for a live probe, use that exact target instead of assuming a local service.

## Library Query Extraction
- When the user wants movie scan queries to prefer filenames, do not invent a separate filename parser. Reuse the release parser's normalized movie title output and only adjust precedence between parser output and folder fallback.

## Import / Rename Boundary
- When rename tokens depend on facts already produced by the import gate or media probe, move rename/path selection below that existing probe/rescore boundary instead of adding a second pre-rename scan. Fix the staging/order of the pipeline before introducing another analyzer call.
