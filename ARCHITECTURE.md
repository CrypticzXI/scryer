# Scryer Architecture

This document is the authoritative reference for agents modifying the scryer codebase. Read it before making any non-trivial change. When this document and the code disagree, the code is correct and this document should be updated.

Scryer is a media management system. It monitors a user's media library, searches indexers for releases matching their quality preferences, acquires releases via download engines, imports completed downloads into the library, and keeps the library organized.

---

## Core Rules

These are non-negotiable. Violating any of these will result in rejected changes.

### 1. Dependency direction is strictly enforced

```
scryer-domain        (types only, no behavior, no dependencies on other crates)
     ^
scryer-application   (business logic, defines repository/client traits)
     ^
scryer-infrastructure (implements traits with SQLite, HTTP, WASM)
     ^
scryer-interface     (GraphQL API, maps between app types and wire types)
     ^
scryer               (binary crate, wires everything together, runs the server)
```

- **Domain depends on nothing** (only serde, chrono, uuid, thiserror).
- **Application depends on domain only.** It defines traits (ports) that infrastructure implements.
- **Infrastructure depends on domain and application.** It implements the traits.
- **Interface depends on domain, application, and infrastructure.** The infrastructure dependency is a pragmatic exception for settings reads — do not expand it.
- **The binary crate depends on everything.** It constructs all concrete types and wires them together.

Never add an upward dependency. Never import infrastructure types into application. Never import interface types into application or infrastructure.

### 2. Memory must be bounded

Every in-memory collection, buffer, channel, and cache must have a defined upper bound. No unbounded growth. Specific rules:

- **Domain event queries must push filtering and pagination to SQL.** Never load the entire event table into memory. Use `DomainEventFilter` with `after_sequence` and `limit`.
- **Broadcast channels have explicit capacity.** Document why the capacity was chosen.
- **Background tasks must terminate.** Every spawned task must have a cancellation path — either a `CancellationToken`, checking `tx.send()` errors, or a `Receiver` that closes.
- **Caches need eviction.** If you add a cache, it needs a size limit and an eviction strategy.

### 3. No stringly-typed identifiers at internal boundaries

String constants used as identifiers (setting keys, event types, status codes) must be defined as enums or typed constants in exactly one place, with serde for serialization at system boundaries.

**Correct pattern — enum with serde:**
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoringPersona {
    Balanced,
    Audiophile,
    Efficient,
    Compatible,
}
```

**Correct pattern — typed constant in one place:**
```rust
// In scryer-application/src/settings_keys.rs (the SINGLE source of truth)
pub const SCORING_PERSONA_KEY: &str = "quality.scoring_persona";
```

**Wrong — same string literal in multiple files:**
```rust
// In file A:
const SCORING_PERSONA_KEY: &str = "quality.scoring_persona";
// In file B (drift risk):
const SCORING_PERSONA_KEY: &str = "quality.scoring_persona";
```

When crossing a system boundary (database, GraphQL, JSON), use serde traits. Internal code passes the enum; serialization happens at the boundary. The `settings/keys.rs` module in `scryer-application` is the canonical location for setting key constants. If you need a new setting key, add it there and import it everywhere.

**This extends to GraphQL.** Every domain enum exposed through GraphQL must have a corresponding `*Value` enum in `crates/scryer-interface/src/types.rs` with explicit `from_domain()` and `into_domain()` methods that exhaustively match all variants. The frontend must use these same enum values — never raw strings.

```rust
// types.rs — exhaustive mapping ensures compile-time safety
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum MediaFacetValue { Movie, Tv, Anime }

impl MediaFacetValue {
    pub fn from_domain(facet: MediaFacet) -> Self {
        match facet {
            MediaFacet::Movie => Self::Movie,
            MediaFacet::Series => Self::Tv,
            MediaFacet::Anime => Self::Anime,
        }
    }
    pub fn into_domain(self) -> MediaFacet {
        match self {
            Self::Movie => MediaFacet::Movie,
            Self::Tv => MediaFacet::Series,
            Self::Anime => MediaFacet::Anime,
        }
    }
}
```

When a variant is added to or removed from a domain enum, the exhaustive `match` fails to compile, forcing the GraphQL schema to update in lockstep. The frontend `lib/types/` must mirror these enum values as TypeScript union types.

### 4. SQLite is the only persistence layer

All state lives in SQLite. No Redis, no external message queues, no filesystem state (except media files and the database file itself). The database uses WAL journal mode with a single-threaded command worker for writes (see Infrastructure section). Respect this — never bypass the command worker for writes.

### 5. No global mutable state outside of AppServices

All shared mutable state lives in `AppServices` fields (behind `Arc`, `RwLock`, `Mutex`, `broadcast`, `Notify`, or `Semaphore`). Never use `lazy_static`, `once_cell` at module scope for mutable state, or thread-local storage for business data. The `AppServices` struct is the single coordination point.

### 6. All user-visible strings go through i18n

Every string shown in the UI must use the `t()` translation function with a key defined in all locale files under `lib/i18n/locales/`. Never hardcode English strings in components.

### 7. No GraphQL caching

The urql client uses `requestPolicy: "network-only"` globally. There is no `cacheExchange`. Never add one. Never set a different `requestPolicy` on individual queries. Every query hits the network.

---

## Crate-by-Crate Architecture

### scryer-domain (`crates/scryer-domain/`)

Pure type library. Single file: `src/lib.rs`. No behavior beyond parsing helpers and Display impls.

**Aggregate hierarchy:**
```
Title (root aggregate)
  +-- Collection (Season, Movie, Arc, Interstitial, Specials)
  +-- Episode
  +-- TitleHistoryRecord
  +-- BlocklistEntry
  +-- ExternalId, TaggedAlias
```

**Key types by area:**

| Area | Types |
|------|-------|
| Identity | `Id` (UUID wrapper, also `new_rego_safe()` for OPA-compatible IDs), `ExternalId`, `MediaFacet` (Movie/Series/Anime) |
| Catalog | `Title`, `NewTitle`, `Collection`, `CollectionType`, `Episode`, `EpisodeType`, `InterstitialMovieMetadata` |
| Downloads | `DownloadQueueItem`, `DownloadQueueState`, `TrackedDownloadState`, `TrackedDownloadStatus`, `TitleMatchType`, `CompletedDownload` |
| Import | `ImportStatus`, `ImportType`, `ImportDecision`, `ImportSkipReason`, `ImportStrategy`, `ImportResult`, `ImportRecord` |
| Policy | `PolicyInput`, `PolicyOutput`, `PolicyScoringEntry`, `RuleSet` |
| Events | `DomainEvent`, `DomainEventPayload`, `DomainEventType`, `DomainEventStream`, `DomainEventFilter` |
| Notifications | `NotificationEventType`, `NotificationChannelConfig`, `NotificationSubscription` |
| Users | `User`, `Entitlement` |
| Files | `VIDEO_EXTENSIONS`, `SUBTITLE_EXTENSIONS`, `ARCHIVE_EXTENSIONS`, helpers like `is_video_file()` |

**Domain invariants encoded in types:**
- `TrackedDownloadState::is_terminal()` — Imported/Failed/Ignored survive restarts; non-terminal states are re-derived.
- `ImportArtifactResult::counts_as_imported()` — AlreadyPresent counts toward completion.
- Tags prefixed `scryer:` preserve case (they encode structured config); all others are lowercased.
- `Id::new_rego_safe()` generates IDs compatible with OPA Rego package naming.

### scryer-application (`crates/scryer-application/`)

Business logic layer. The largest crate. Defines all repository/client traits and implements all use cases.

**Central types:**

`AppServices` is the dependency injection container. All dependencies are `Arc<dyn Trait>`. Constructed in the binary crate's `main.rs`. There is no DI framework — it's manual constructor injection.

`AppUseCase` wraps `AppServices` with auth config and facet registry:
```rust
pub struct AppUseCase {
    pub services: AppServices,
    pub auth: JwtAuthConfig,
    pub facet_registry: Arc<FacetRegistry>,
}
```

**Use case pattern:** Each domain workflow entry module adds an `impl AppUseCase` block for a specific concern. The primary workflows now use descriptive internal module names such as `library_workflow`, `import_workflow`, `catalog_workflow`, and `acquisition_workflow`, with helper modules colocated beside them under grouped domain directories. Methods take `&self` plus `actor: &User` for authorization. This is the only pattern for adding business logic — never put business logic in infrastructure or interface crates.

**Crate map:** `lib.rs` is the crate index; repository/client/provider traits live in `ports.rs`; shared DTOs live in `contracts.rs`; dependency wiring lives in `services.rs`; domain workflow files live under grouped directories such as `acquisition/`, `catalog/`, `library/`, and `import/`.

| Module | Physical location |
|------|---------------|
| `library_workflow` | `crates/scryer-application/src/library/library.rs` |
| `import_workflow` | `crates/scryer-application/src/import/import.rs` |
| `catalog_workflow` | `crates/scryer-application/src/catalog/catalog.rs` |
| `acquisition_workflow` | `crates/scryer-application/src/acquisition/acquisition.rs` |
| `app_usecase_integration` | `crates/scryer-application/src/integration/integration.rs` |
| `app_usecase_rss` | `crates/scryer-application/src/acquisition/rss.rs` |
| `app_usecase_discovery` | `crates/scryer-application/src/catalog/discovery.rs` |
| `app_usecase_settings` | `crates/scryer-application/src/settings/settings.rs` |
| `app_usecase_plugins` | `crates/scryer-application/src/plugins/plugins.rs` |
| `app_usecase_rules` | `crates/scryer-application/src/rules/rules.rs` |
| `app_usecase_activity` | `crates/scryer-application/src/events/activity_api.rs` |
| `app_usecase_jobs` | `crates/scryer-application/src/jobs/jobs.rs` |
| helper modules | colocated beside workflow files, e.g. `library/rename.rs`, `library/discovery.rs`, `import/title_resolution.rs`, `import/parameters.rs`, `acquisition/search_queries.rs`, and `acquisition/decision_helpers.rs` |

**Repository traits** are defined in `ports.rs`, covering every persistence concern. Each trait is `#[async_trait] + Send + Sync + 'static`. The application layer only knows these traits, never the concrete SQLite implementation.

**Background loops** — async loops, all taking `(AppUseCase, CancellationToken)`:

| Loop | Purpose |
|------|---------|
| `start_background_acquisition_poller` | Polls wanted items, runs search-and-grab |
| `start_download_queue_poller` | Polls download clients, drives tracked download state machine, triggers imports |
| `start_background_library_refresh_loop` | Central job scheduler |
| `start_background_hydration_loop` | Fetches metadata from gateway for unhydrated titles |
| `start_background_post_hydration_title_scan_workers` | Scans library after metadata hydration |
| `start_background_subtitle_poller` | Searches for missing subtitles |
| `start_background_poster_loop` | Fetches poster images |
| `start_background_banner_loop` | Fetches banner images |
| `start_background_fanart_loop` | Fetches fanart images |
| `start_notification_dispatcher` | Tails domain events, dispatches to notification plugins |

Coordination: Most loops use `tokio::select!` between cancellation and a `Notify` wake signal or `broadcast::Receiver`. The `*_wake` Notify handles allow immediate wake-up when new work arrives.

**Domain event system:**

Events are the primary mechanism for cross-concern communication. The flow:

1. Use case creates a `NewDomainEvent` via factory functions in `domain_events.rs`
2. `AppServices::append_domain_event()` persists it and broadcasts the sequence number
3. Subscribers (notification dispatcher, GraphQL subscriptions) wake on the broadcast, re-query the DB for actual events
4. Projection functions in `event_views.rs` transform events into view models (ActivityEvent, HistoryEvent, TitleHistoryRecord, etc.)

Event streams partition events by routing key: `Global`, `Title { title_id }`, `LibraryScan { session_id }`, `JobRun { run_id }`, `DownloadQueueItem { item_id }`.

**Events must be pure domain facts.** A domain event records what happened — not what any particular consumer wants to hear. Never shape an event payload to suit a specific subscriber (e.g., adding notification-specific fields, formatting messages for the activity feed, or omitting data because "the UI doesn't need it"). The publisher's only job is to faithfully record the domain fact with enough context to be self-describing. Each subscriber is responsible for interpreting the event for its own use case: the notification dispatcher maps events to user-facing notification text, `event_views.rs` projects events into activity feed items and title history records, and GraphQL subscriptions build their own view models. If a subscriber needs data that isn't in the event, the answer is either to enrich the event payload at the source (if the data is a domain fact) or to look it up in the subscriber (if it's presentation concern).

**Quality scoring pipeline:**

```
Release title string
    |
    v
scryer-release-parser::parse_release_metadata()
    |  -> ParsedReleaseMetadata (quality, source, codec, audio, languages, flags)
    v
quality_profile::evaluate_against_profile()
    |  -> QualityProfileDecision (score, scoring_log, allowed/blocked)
    |  Uses ScoringWeights from ScoringPersona (Balanced/Audiophile/Efficient/Compatible)
    v
scryer-rules::UserRulesEvaluator::evaluate()
    |  -> Score adjustments from user-authored Rego rules
    v
acquisition_policy::evaluate_upgrade()
    |  -> Accept/Reject based on score delta, cooldown, tier
    v
Final grab/reject decision
```

**Facet system:**

`FacetHandler` trait provides polymorphic behavior per media type (Movie, Series, Anime). `FacetRegistry` maps `MediaFacet` -> `Arc<dyn FacetHandler>`. This is how facet-specific behavior (rename templates, download categories, library paths) is dispatched without match statements everywhere.

### scryer-infrastructure (`crates/scryer-infrastructure/`)

Implements all repository traits and external integrations.

**`SqliteServices`** is the single concrete type implementing all repository traits. Two patterns for database access:

1. **Write path: Command worker.** `DbCommand` is a large enum with a variant per write operation. A single tokio task loops on an `mpsc` channel, processing commands sequentially. This serializes all writes, eliminating SQLite write contention without explicit locking. Every write goes through `db_call!` macro -> channel send -> oneshot reply.

2. **Read path: Direct pool queries.** Read-only queries call `sqlx::query()` directly on the connection pool. Query functions live in `queries/*.rs` modules, all following the pattern `pub(crate) async fn xxx_query(pool: &SqlitePool, ...) -> AppResult<T>`.

**SQL patterns:**
- Raw parameterized SQL with `sqlx::query("...").bind(x)`. All parameters are positional `?` placeholders.
- Dynamic queries via `sqlx::QueryBuilder<Sqlite>` for variable WHERE clauses.
- Manual row mapping via `row.try_get("column")`. No derive macros, no ORM.
- `ON CONFLICT DO UPDATE` extensively for upsert semantics.
- Migrations in `crates/scryer/src/db/migrations/`, embedded at compile time via `sqlx::migrate!`.

**Download clients:**

| Client | Protocol | Implementation |
|--------|----------|---------------|
| Weaver | GraphQL + WebSocket | `weaver.rs` (submit), `weaver_subscription.rs` (real-time queue via WS, fallback to HTTP polling) |
| NZBGet | JSON-RPC | `nzbget.rs` |
| SABnzbd | REST | `sabnzbd.rs` |

**Multi-indexer search orchestrator** (`multi_indexer.rs`):
- Fans out searches across all enabled indexers in parallel
- Two-tier strategy: primary (ID-based) then fallback (freetext, only if primary returns zero)
- Per-indexer rate limiting with configurable intervals
- Exponential backoff (5min → 24h) on indexer errors, de-escalates on success
- RSS feed caching via `Arc<OnceCell>` for concurrent callers
- Anime alias handling for better search coverage

**Metadata gateway** (`metadata_gateway.rs`):
- GraphQL over HTTP with APQ (Automatic Persisted Queries)
- mTLS enrollment with P-256 keypair + CSR
- Instance authentication via ECDSA-signed request headers
- **Always batch requests when possible.** The gateway is a shared service with real cost per round-trip. If you need metadata for multiple titles, use a batch query rather than N individual requests. Prefer bulk endpoints over loops.

### scryer-interface (`crates/scryer-interface/`)

GraphQL API layer using `async-graphql`.

**Structure:**
- `query.rs` — query resolvers
- `mutation/` — mutation modules composed via `#[derive(MergedObject)]`
- `subscription.rs` — subscription endpoints returning `BoxStream`
- `types.rs` — GraphQL payload and input types with bidirectional `from_domain()`/`into_domain()` mapping
- `mappers.rs` — Pure functions converting domain types to GraphQL payloads
- `settings_graph.rs` — Settings system bridge (hierarchical key-value with scope inheritance)
- `context.rs` — Request context, auth extraction

**Resolver pattern (every resolver follows this exactly):**
```rust
async fn some_query(&self, ctx: &Context<'_>, input: SomeInput) -> GqlResult<SomePayload> {
    let app = app_from_ctx(ctx)?;
    let actor = actor_from_ctx(ctx)?;
    let domain_input = input.into_domain();
    let result = app.some_use_case(&actor, domain_input).await.map_err(to_gql_error)?;
    Ok(from_domain_type(result))
}
```

Never put business logic in resolvers. They extract context, delegate to `AppUseCase`, and map types.

**Subscription pattern:** All subscriptions use `stream::unfold` with a cursor tracking the last-seen domain event sequence. On broadcast wake, they re-query the DB for events after the cursor. This is crash-tolerant — missed events are recovered from the DB.

**Type mapping rules:**
- Every domain enum exposed to GraphQL gets a corresponding `*Value` enum in `types.rs` with explicit `from_domain()`/`into_domain()` match arms.
- This provides compile-time safety when domain types change.
- Never use `serde_json::Value` or raw strings to pass structured data through GraphQL.

### scryer (binary crate, `crates/scryer/`)

Startup orchestration. `main.rs` handles:

1. CLI dispatch, data dir resolution, env file loading
2. Tracing setup (stdout + ring buffer for WS log streaming)
3. Splash server (starts listening immediately during bootstrap)
4. Database initialization with migrations
5. Settings seeding from definitions and environment
6. Encryption key bootstrap
7. Version upgrade detection
8. Facet registry construction
9. All `Arc<dyn Trait>` construction — repositories from `SqliteServices`, download clients, indexer client with WASM plugins, metadata gateway
10. `AppUseCase` assembly
11. GraphQL schema construction
12. Background task spawning (all loops listed above)
13. Axum router: `/graphql` (POST), `/graphql/ws` (WebSocket), `/health`, `/images/titles/...`, `/admin/...`, UI fallback

### Supporting Crates

**scryer-release-parser** — Token-based release name parser. Input: raw release title string. Output: `ParsedReleaseMetadata` with quality, source, codec, audio, languages, episode info, release group, flags, confidence score. Used in both the acquisition pipeline (pre-download scoring) and the import pipeline (file identification).

**scryer-mediainfo** — Pure Rust media file analyzer. No external ffprobe dependency. Native parsers for MKV, MP4, AVI, TS containers. Extracts video codec/resolution/HDR/DV, all audio streams with language/channels/codec, subtitle streams, chapter markers. Used during library scan and import for ground-truth media analysis.

**scryer-rules** — Pure-Rust OPA/Rego engine via `regorus`. `UserRulesEngine` holds pre-compiled rules in `Arc<RwLock<>>`. `UserRulesEvaluator` is a cheap per-batch clone. Rules receive `input.release` (parsed metadata), `input.profile` (quality settings), `input.context` (title/facet/tags), `input.builtin_score` (pre-rule score), and optionally `input.file` (post-download media analysis). Rules output `score_entry[code] := delta`. Custom builtins: `scryer.block_score()`, `scryer.size_gib()`, `scryer.lang_matches()`, `scryer.normalize_source()`, `scryer.normalize_codec()`.

**scryer-plugins** — WASM plugin system via Extism (wasmtime). Three plugin types: indexers, download clients, notification providers. Plugins export `describe()` (returns descriptor with capabilities, config schema) and type-specific functions. Built-in plugins are compiled into the binary. Plugins run in sandboxed wasmtime with a timeout and network restricted to declared hosts.

---

## Frontend Architecture (`apps/scryer-web/`)

### Stack

React, React Router, urql, Tailwind CSS, shadcn/ui (New York variant), Vite, TypeScript (strict). Check `package.json` for current versions. React Compiler runs in production builds only.

### Component Architecture

**Container/View pattern.** Strictly separated:

- **Containers** (`components/containers/`) own data fetching, mutations, and hook composition. They pass pure data and callbacks to views as props. All containers are `React.memo` wrapped and lazy-loaded.
- **Views** (`components/views/`) are presentational. They receive all data as props. No `useClient()`, no direct GraphQL calls.
- **Common** (`components/common/`) are shared components used across views.
- **UI** (`components/ui/`) are shadcn/ui primitives. Managed via `npx shadcn@latest add`. Don't hand-edit these.

### State Management

No global state store. Data flows through:

1. **URL** — View, section, title ID, episode ID all derived from pathname. Navigation via `useNavigate()`.
2. **React Context** — `TranslateContext` (i18n), `GlobalStatusContext` (toasts), `SearchContext` (global search), `LibraryScanProgressContext` (scan banners).
3. **Custom hooks** (`lib/hooks/`) abstracting domain concerns. Containers compose hooks; views consume the results.
4. **Auth** — JWT in `sessionStorage`. Token extracted from JWT payload for user info.

### GraphQL

- Queries and mutations are raw template strings in `lib/graphql/queries.ts` and `mutations.ts` with fragment composition.
- Types are hand-written in `lib/types/`. No codegen. Manually kept in sync with the backend schema.
- WebSocket subscriptions via `graphql-ws` with exponential backoff retry.
- **Runtime base path injection:** `index.html` has placeholder tokens replaced by the Rust server at serve time, enabling sub-path deployment without rebuild.

### Facet Registry

Media types are data-driven via `FACET_REGISTRY` array. Adding a new media type means adding one entry — nav, routing, settings, search behavior all derive from it.

### Styling

- Tailwind CSS v4 with PostCSS
- Semantic CSS custom properties in `globals.css` (`--color-background`, `--color-primary`, etc.)
- Three themes: Light, Dark, Pride (via `next-themes` with class strategy)
- Dark theme uses atmospheric gradients with frosted-glass card effects
- Icons: Lucide React throughout

---

## Key Patterns and Conventions

### Adding a new setting

1. Add the key constant to `crates/scryer-application/src/settings/keys.rs`
2. Import the constant everywhere it's used (bootstrap, application, interface)
3. Add a `ServiceSettingSeed` entry in `crates/scryer/src/settings_bootstrap.rs`
4. Add load/persist logic in `crates/scryer-application/src/settings/settings.rs`
5. Expose via GraphQL in `settings_graph.rs` (load) and `mutation/settings.rs` (persist)
6. Add frontend type, query fields, and UI

### Adding a new domain event type

1. Add variant to `DomainEventType` enum in `scryer-domain` — update `as_str()`, `parse()`, and `all()`
2. Add payload struct and `DomainEventPayload` variant in `scryer-domain`
3. Add factory function in `crates/scryer-application/src/events/domain_events.rs`
4. Add projection handling in `crates/scryer-application/src/events/event_views.rs` (activity view, history view, etc.)
5. If it should trigger notifications, add to `build_notification()` in `crates/scryer-application/src/notifications/dispatcher.rs`

### Adding a new repository trait

1. Define the trait in `crates/scryer-application/src/ports.rs`
2. Add a field for it in `AppServices`
3. Implement it on `SqliteServices` in `scryer-infrastructure/src/repositories.rs`
4. Add query functions in the appropriate `queries/*.rs` module
5. Add `DbCommand` variants in `commands.rs` for write operations
6. Add a null implementation in `null_repositories.rs` for testing
7. Wire the concrete implementation in `main.rs`

### Adding a new GraphQL query/mutation

1. Add the method to `AppUseCase` in the appropriate workflow file under `crates/scryer-application/src/`
2. Add GraphQL payload/input types in `types.rs` with `from_domain()`/`into_domain()`
3. Add mapper functions in `mappers.rs`
4. Add the resolver in `query.rs` or the appropriate `mutation/*.rs` file
5. Follow the exact resolver pattern: extract context -> delegate to app -> map types
6. Add frontend query/mutation string, types, and UI

### Error handling

- `AppResult<T> = Result<T, AppError>` throughout the application layer
- `AppError` variants: `Unauthorized`, `Validation`, `NotFound`, `Repository`
- In GraphQL resolvers: `.map_err(to_gql_error)?` converts to `async_graphql::Error`
- In background loops: log errors with `tracing`, never crash the loop
- Domain event appends in fire-and-forget contexts use `let _ = app.services.append_domain_event(...).await;`
- Domain event appends in critical paths use `?` for propagation

### Testing

- **Integration tests** live in `crates/scryer/tests/`. They use real in-memory SQLite with all migrations applied, plus wiremock for HTTP mocks.
- **`TestContext`** provides a fully wired `AppUseCase` with test helpers: `schema_exec()` for direct GraphQL, `gql()` for HTTP-level tests.
- **No mock framework.** All test doubles are hand-rolled structs implementing `#[async_trait]` traits.
- **Null repositories** in `null_repositories.rs` return empty results or errors for every method. Used as defaults in `AppServices::with_default_channels()`.
- **Always use `cargo nextest run`** instead of `cargo test` so tests run in parallel.

---

## Build and CI

```bash
# Rust (never run concurrently in the same workspace)
cargo build --workspace --locked
cargo nextest run --workspace --locked

# Frontend
cd apps/scryer-web && npm ci && npm run build

# Release (ALWAYS use the script, never manually)
./scripts/release.sh          # patch bump
./scripts/release.sh --minor  # minor bump
./scripts/release.sh --dry-run
```

CI is tag-triggered via `.github/workflows/scryer.yml`. The release script handles: cargo update, cargo audit, clippy (with `--all-targets`), tests, npm audit fix, lint, version bumping all workspace crates, cargo check, signed tag, and push.

---

## Things That Will Bite You

1. **Concurrent Cargo invocations** balloon `target/` by hundreds of gigabytes. Always check `ps aux | grep cargo` first.
2. **The domain event table is append-only with no automatic retention.** Queries must filter at the SQL level, never load-all-then-filter.
3. **TrackedDownloadState is re-derived on restart** for non-terminal states. If you're adding a new state, decide if it's terminal (persists) or transient (re-derived).
4. **Tags prefixed `scryer:`** are structured config (e.g., `scryer:quality-profile:abc123`). They have case-preserving semantics. Don't normalize them.
5. **The WASM plugin runtime is `!Send`.** Plugin calls go through `spawn_blocking` with a mutex. Don't try to hold a plugin handle across `.await` points.
6. **Settings use hierarchical scope inheritance** (global -> facet -> title). The `__inherit__` sentinel value means "use the parent scope's value." Don't confuse it with null/empty.
7. **The metadata gateway is an independent service** with its own deployment lifecycle. Schema changes there must be deployed before scryer queries new fields.
8. **`babel-plugin-react-compiler` only runs in production builds.** Dev builds may behave differently if you accidentally rely on compiler optimizations.
