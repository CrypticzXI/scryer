# Scryer Architecture Manifesto

## Purpose

This document is the architectural contract for Scryer.

It is written for humans and agents alike. If you are adding, changing, or reviewing a feature, this document is the default source of truth for how that work should fit into the system.

This document is intentionally opinionated. Scryer should feel coherent because the architecture is coherent. We do not let each feature invent its own model, lifecycle, source of truth, or boundary rules.

If the code and this document diverge, stop and resolve the mismatch deliberately. Do not silently accept architectural drift. If the architecture changes intentionally, update this document in the same work. If runtime contracts change, also update the external documentation in the `scryer-docs` repo.

## What Scryer Is

Scryer is a single-node media management system for movies, series, and anime.

It monitors a user's library, searches indexers, evaluates releases against quality and policy rules, acquires releases through download engines, imports completed downloads into the library, fetches metadata and subtitles, and keeps the library organized.

The backend is authoritative. The web app is a projection client, not a second source of truth.

Scryer is intentionally:

- a single deployable binary
- SQLite-backed
- GraphQL-first
- event-driven internally
- plugin-extensible at specific boundaries
- optimized for one homelab node rather than distributed deployment

## Architectural Priorities

When tradeoffs appear, prefer:

- the smallest coherent change over the largest ambitious change
- coherence over local cleverness
- durability over convenience
- explicit flows over hidden coupling
- semantic models over storage-shaped models
- one strong path over multiple overlapping paths
- typed boundaries over stringly glue
- bounded memory over convenience shortcuts
- reducing code over growing code when functionality is preserved
- simple homelab operations over premature distributed complexity

## Non-Negotiable Principles

### 1. The Backend Is Authoritative

All durable truth lives in the backend.

That includes:

- title, collection, and episode state
- import and acquisition state
- quality and rules decisions
- user and notification settings
- domain event history
- plugin-mediated changes

The frontend may project, filter, and present state, but it does not invent durable truth. Plugins may extend behavior, but they do not become alternate authorities.

If a feature cannot clearly answer "who is authoritative for this state?", the design is not ready.

### 2. Dependency Direction Is Strict

Scryer uses explicit layer boundaries, and the dependency direction is enforced:

```text
scryer-domain         (types only)
     ^
scryer-application    (business logic, ports)
     ^
scryer-infrastructure (SQLite, HTTP, WASM implementations)
     ^
scryer-interface      (GraphQL boundary)
     ^
scryer                (binary wiring and startup)
```

Supporting crates such as `scryer-release-parser`, `scryer-mediainfo`, `scryer-rules`, and `scryer-plugins` exist below or beside this flow according to their concern, but they must not punch holes through it.

Rules:

- domain depends on nothing except foundational utility crates
- application depends on domain plus narrowly-scoped support crates that remain below the interface layer, such as parsing, mediainfo, and rules engines
- infrastructure depends on domain and application
- interface depends on domain and application, with the current narrow infrastructure dependency kept as an exception rather than a precedent
- the binary crate wires concrete implementations together

Never add an upward dependency. Never import infrastructure types into application. Never import interface types into application or infrastructure.

### 3. One Canonical Domain Event Spine

Scryer has one canonical domain event story.

Every meaningful state change must be representable as a durable domain event. Those events are the shared language for:

- GraphQL subscriptions
- activity projections
- notification dispatch
- history views
- background coordination

We do not create separate shadow event systems for different subsystems. We do not let notifications, subscriptions, and activity feeds each invent their own private notion of what happened.

If a feature changes durable state but emits no meaningful event, that feature is incomplete.

### 4. Durable Before Live

Live delivery matters, but durability comes first.

The rule is:

- durable state changes happen first
- durable domain events are recorded with them
- live wake-up and fanout happen after durable intent exists

This is why subscriptions re-query the database by cursor after wake-up rather than trusting transient in-memory delivery.

A real-time system that cannot explain what happened after a restart is not reliable enough for Scryer.

### 5. The API Describes Intent, Not Storage

Scryer uses a semantic GraphQL API.

Queries should describe the views the frontend needs. Mutations should describe business actions. Subscriptions should describe meaningful streams of change.

We do not expose raw tables, ad hoc status strings, plugin internals, or SQLite implementation details merely because doing so is easy.

The public model should reflect how people think about wanted titles, releases, imports, library items, jobs, notifications, and settings, not how the storage happens to be laid out.

### 6. Single-Node SQLite Runtime Is Deliberate

Scryer is SQLite-first by design.

This is not a placeholder for a future distributed architecture. It is the deliberate operating model for the product.

That means:

- all state lives in SQLite
- we do not add Redis, external queues, or alternate persistence systems
- write contention is handled through the existing serialized write path
- backup, restore, upgrade, and portability matter
- operational simplicity matters

The database is part of the product's reliability story, not an implementation detail to ignore.

### 7. Memory Must Be Bounded

Every in-memory collection, channel, cache, buffer, and loop must have an explicit bound and a clear lifecycle.

That means:

- event queries push filtering and pagination to SQL
- broadcast channels have explicit capacity
- caches have eviction
- background tasks have shutdown paths
- no feature is allowed to assume "the dataset is probably small enough"

If memory growth is unbounded, the feature is not done.

### 8. Shared Mutable Runtime State Must Be Explicit

Shared mutable runtime coordination must stay explicit and centralized.

We do not spread mutable global state across module-level singletons, thread-local business state, or hidden coordination points. Runtime channels, trackers, wake handles, and caches must have an obvious owner and an obvious construction path.

Scryer keeps feature dependencies in grouped `AppServices` structs and keeps mutable coordination in `AppRuntimeState`. New work should preserve that split instead of pushing runtime state back into the dependency graph.

### 9. Typed Boundaries Are Mandatory

Scryer should not rely on free-floating string identifiers at internal boundaries.

Setting keys, event types, statuses, enum-like values, and GraphQL-exposed domain states should be represented by:

- Rust enums with serde where appropriate
- typed constants defined once where enum modeling is not appropriate
- explicit boundary mappings at GraphQL and persistence edges

Internal code should pass typed values. Serialization happens at the boundary, not in the middle of the codebase.

### 10. Frontend Boundaries Are Real

The frontend is not allowed to become a second application layer.

The rules are:

- containers own data fetching and hook composition
- views are presentational
- GraphQL is the boundary, not shared internal backend types
- all user-visible strings go through i18n
- urql remains network-only; there is no GraphQL cache layer

The web app is a projection client over backend truth. It should stay that way.

### 11. Plugins Are Guests, Not Peers

Plugins are powerful extensions of Scryer, not alternate cores of Scryer.

That means plugins must be:

- capability-based
- sandboxed
- versioned
- observable
- host-mediated

Plugins do not get to bypass policy, mutate core storage directly, or invent hidden side channels that the rest of the system cannot observe.

If a plugin needs new power, the answer is to design a new host capability or hook, not to punch a hole through the architecture.

### 12. Security and Permissions Are Core Behavior

Authentication, authorization, encryption bootstrap, admin flows, and external integration credentials are not wrappers around the real system. They are part of the real system.

Every feature must have a clear answer to:

- who can do this
- on whose behalf it runs
- what external systems it can touch
- what user data or secrets it can expose
- how it behaves across restart and recovery

### 13. Solve Problems With the Least Necessary Code

Scryer should not equate progress with code growth.

New features should be implemented with the smallest amount of code that cleanly solves the problem and fits the architecture.

That means:

- prefer extending an existing coherent path over creating a parallel subsystem
- prefer deleting, simplifying, or consolidating code when that preserves behavior
- prefer a narrow solution that fits the real requirement over a broad framework for hypothetical future needs
- treat reduction in overall codebase size, while retaining functionality, as a success

This is not a license for clever compression or unreadable shortcuts. The goal is not fewer lines at any cost. The goal is less unnecessary code, less duplication, and less surface area to maintain.

If a feature adds a large amount of code, the change should be able to explain why that code is truly necessary and why a smaller design would not have worked.

## Durable Domain Commitments

These are the product areas that must remain explicit as Scryer grows.

They are here so the codebase does not slowly collapse into a giant generic "media manager" blob.

### Facets and Media Taxonomy Must Stay Explicit

Scryer treats movies, series, and anime as first-class facets. That is not superficial labeling.

The title, collection, and episode model is part of the architecture. Matching, organization, imports, release evaluation, and UI behavior all depend on it.

Facet-specific behavior should go through explicit facet handling rather than match statements and one-off exceptions scattered throughout the system.

### Acquisition, Import, and Library Organization Are Distinct Domains

Searching for a release, grabbing it, tracking it in download clients, importing completed downloads, and maintaining the organized library are related concerns, but they are not the same concern.

Scryer must keep these boundaries explicit:

- acquisition decides what to search and grab
- integration tracks external download client state
- import decides what completed downloads mean and how they enter the library
- library management decides how the durable library is organized and maintained

When these responsibilities blur, the code becomes hard to reason about and regressions multiply.

### Metadata and Artwork Are First-Class Domains

Metadata is not an incidental side effect.

Scryer must treat metadata and artwork as real domains with clear ownership over:

- hydration from the metadata gateway
- title and episode identity enrichment
- images and visual assets
- local metadata integration where applicable
- post-hydration follow-up work

### Quality, Scoring, and Rules Are First-Class

Release parsing, quality scoring, rule evaluation, and upgrade policy are central product behavior.

They should not be hidden behind opaque helper chains or casually duplicated in multiple workflows. Scryer wins or loses on this logic being understandable and reliable.

### Notifications Are Projections, Not Alternate Truth

Notifications matter, but they are downstream of domain facts.

Notification text, delivery channels, and plugin dispatch are projections over domain events. They must not become a second event system or a hidden source of state transitions.

### Operations Are Part of the Product

Startup, migrations, key bootstrap, backup and restore, job execution, health reporting, and version upgrades are not side chores.

They are part of the product and must remain legible in the architecture.

### Plugin Lifecycle Is Part of the Core Product

The plugin runtime is not enough by itself.

Scryer should keep plugin discovery, configuration, compatibility, installation, built-in plugin behavior, and notification/indexer/download-client plugin lifecycles explicit and observable.

### Verification Should Follow Domain Boundaries

Tests should grow with the architecture rather than against it.

Scryer should prefer:

- domain-focused test suites
- boundary tests for GraphQL and plugin interfaces
- end-to-end tests for the canonical flow from action to durable state to event to projection
- parallel test directory structures over large inline test blocks in production files

What Scryer should avoid is one catch-all test bucket that mirrors no domain and teaches contributors nothing about where new verification belongs.

## Patterns

Patterns are reusable implementation rules that help contributors make local decisions without re-litigating architecture every time.

### 1. Serde Defines Enum Boundaries

Rust enums are the canonical representation of discrete state in Scryer, and their serde form is the default boundary contract when those enums cross persistence or API boundaries.

In practice:

- enums stay typed internally
- persistence stores the serialized form rather than ad hoc encodings
- GraphQL gets explicit mapped enums instead of raw strings
- internal code does not normalize or re-stringify values just to pass them around

Serialized enum variants are durable contract surface. Renaming them is a migration concern, not a casual refactor.

### 2. GraphQL Enums Must Mirror Domain Enums Explicitly

Every domain enum exposed through GraphQL must have a corresponding GraphQL enum with explicit `from_domain()` and `into_domain()` conversions.

This is a deliberate compile-time safety mechanism. When a domain enum changes, the mapping should fail loudly until the GraphQL boundary is updated in lockstep.

Never pass enum-shaped state through GraphQL as free-form strings or `serde_json::Value`.

### 3. Resolver Pattern Is Fixed

Resolvers are boundary adapters, not business logic hosts.

The resolver pattern is:

1. extract application context and actor
2. map GraphQL input into domain input
3. delegate to `AppUseCase`
4. map domain output back into GraphQL output

Resolvers should not perform policy decisions, repository access, or orchestration logic themselves.

### 4. Workflow Modules Grow by Domain

Business logic should grow in grouped domain modules such as acquisition, catalog, import, library, notifications, quality, rules, and settings.

Do not reintroduce giant horizontal buckets like:

- generic `services/`
- cross-domain `usecases/`
- helper junk drawers that quietly become alternative architecture

If a workflow grows too large, split further inside that domain rather than creating a cross-domain sink.

### 5. Tests Should Live Beside Domains, Not Inside Production Files

Scryer should prefer a parallel test tree that mirrors the domain structure rather than scattering large `#[cfg(test)] mod tests` blocks through production files.

Small truly local tests are not forbidden, but the default convention is:

- behavior and regression tests live in `tests/` trees
- test file names describe scenarios and contracts
- production files stay focused on production code

This is a deliberate convention even though inline Rust tests are common elsewhere.

### 6. Settings Keys Have One Source of Truth

When a setting is identified by a string key, that key must be defined exactly once in the canonical settings key module and imported everywhere else.

Never duplicate setting key string literals across bootstrap, application logic, interface, and frontend code.

## Canonical Feature Flow

New backend features should fit the same general shape:

1. Accept a semantic action or request.
2. Extract actor and context at the boundary.
3. Validate permissions, policy, and invariants in `AppUseCase`.
4. Change authoritative state through application ports.
5. Persist a durable domain event describing what happened.
6. Wake subscribers and downstream processors.
7. Re-query and project durable state into GraphQL, activity, history, or notifications.

Not every feature touches every stage equally, but the shape should remain recognizable.

If a feature needs a second side channel because it does not fit this flow, that is a design smell and should be challenged.

## What Good Additions Look Like

A feature fits Scryer well when it:

- has one obvious source of truth
- respects the crate dependency direction
- emits clear domain events
- uses GraphQL as a semantic boundary rather than a storage mirror
- keeps memory bounded
- keeps acquisition, import, library, metadata, notifications, and rules in their proper homes
- remains understandable after restart, retry, or replay
- solves the problem without unnecessary new layers or wrappers

## Red Flags

Stop and reconsider when a change introduces any of these patterns:

- upward crate dependencies
- durable state changes with no corresponding domain event
- load-all-then-filter event queries
- plugin logic that bypasses host-mediated boundaries
- raw strings standing in for typed state at internal boundaries
- resolver or frontend code containing business logic
- acquisition/import/library behavior duplicated in multiple places
- unbounded channels, queues, or caches
- notification logic becoming a second event system
- new abstractions that exist mostly to feel more "architected"

## Crate Responsibilities

### `scryer-domain`

Pure type library.

Responsibilities:

- core model types such as titles, collections, episodes, users, events, policies, notifications, and files
- domain enums and value objects
- basic parsing helpers and lightweight invariants

Non-responsibilities:

- business orchestration
- persistence
- HTTP or GraphQL concerns

The title hierarchy remains a core part of the architecture:

```text
Title
  +-- Collection
  +-- Episode
  +-- TitleHistoryRecord
  +-- BlocklistEntry
```

### `scryer-application`

Business logic layer.

Responsibilities:

- defines repository and integration ports
- owns `AppServices` and `AppUseCase`
- implements workflow logic for acquisition, catalog, import, library, jobs, notifications, rules, settings, health, and security
- owns the domain event factory and projection logic
- runs background loops by orchestrating ports rather than directly touching infrastructure details

Important structural rules:

- `AppServices` is a private grouped dependency graph organized by concern such as catalog, library, integrations, workflow, config, customization, notifications, identity, and events
- `AppRuntimeState` owns mutable coordination such as channels, trackers, wakes, and bounded caches
- `AppUseCase` is the main application-facing orchestration surface
- workflow entry modules stay grouped by domain
- business logic belongs here, not in infrastructure or interface crates

Key grouped workflow areas today include:

- `acquisition/`
- `catalog/`
- `events/`
- `import/`
- `integration/`
- `jobs/`
- `library/`
- `notifications/`
- `quality/`
- `rules/`
- `settings/`
- `subtitles/`

Domain events are persisted facts first and consumer projections second.

### `scryer-infrastructure`

Implements application ports and external integrations.

Responsibilities:

- smaller SQLite store implementations grouped by concern
- shared SQLite runtime through `DbRuntime` / `SqliteServices`
- narrow command-worker path only for the operations still requiring serialized execution
- read-query modules
- download client integrations
- metadata gateway integration
- multi-indexer orchestration
- WASM/plugin host integration support where applicable

Persistence rules:

- reads query the pool directly through the store/query modules
- serialized execution is reserved for the remaining operations that truly need it
- SQL is parameterized and explicit
- dynamic SQL goes through `QueryBuilder`
- migrations are embedded and applied at startup

### `scryer-interface`

GraphQL boundary using `async-graphql`.

Responsibilities:

- query, mutation, and subscription resolvers
- GraphQL input/output types
- mapping between domain types and wire types
- auth extraction and request context
- settings graph bridge

Rules:

- no business logic in resolvers
- explicit enum mapping
- subscriptions tail domain events via cursor and re-query
- no raw JSON pass-through for structured domain contracts

### `scryer`

Binary crate and startup orchestration.

Responsibilities:

- CLI dispatch
- environment and data-dir bootstrap
- tracing and logging setup
- database initialization and migrations
- settings seeding
- encryption key bootstrap
- service construction
- GraphQL schema construction
- background task spawning
- axum router construction

### Supporting Crates

#### `scryer-release-parser`

Token-based release title parser used in acquisition and import flows.

#### `scryer-mediainfo`

Pure-Rust media analyzer providing ground-truth media inspection without an external `ffprobe` dependency.

#### `scryer-rules`

Pure-Rust policy/rules engine used for user-authored rule evaluation.

#### `scryer-plugins`

WASM plugin system for indexers, download clients, and notification providers. Plugins run under capability and runtime restrictions rather than as trusted peers.

## Frontend Architecture

The frontend lives under `apps/scryer-web/`.

### Stack

React, React Router, urql, Tailwind CSS, shadcn/ui, Vite, and strict TypeScript.

### Component Structure

Scryer uses a strict container/view pattern:

- containers own data fetching, mutations, and hook composition
- views are presentational
- common components are shared app-level UI
- `components/ui/` contains primitive UI building blocks rather than product logic

### State Rules

The frontend does not have a general-purpose global state store.

State flows through:

- URL-derived routing state
- focused React contexts
- custom hooks by concern
- JWT-backed auth state in session storage

### GraphQL Rules

- query and mutation documents are explicit
- types are hand-maintained and must stay aligned with backend enums and payloads
- subscriptions use `graphql-ws`
- urql stays network-only

### Frontend Product Rules

- all user-visible strings go through i18n
- frontend types mirror backend contract values explicitly
- media facets stay data-driven through the facet registry
- the frontend remains a projection client rather than a second business layer

## Contributor Conventions

### Adding a New Setting

1. Add the key constant to the canonical settings key module.
2. Use that constant everywhere instead of duplicating string literals.
3. Seed the setting during bootstrap.
4. Load and persist it in application logic.
5. Expose it through the GraphQL boundary if needed.
6. Add frontend types and UI if it is user-facing.

### Adding a New Domain Event Type

1. Add the type and payload in `scryer-domain`.
2. Add factory support in the application event factory.
3. Add projection support for activity/history or other views as needed.
4. Add notification projection support if it should dispatch externally.
5. Keep the event a pure domain fact rather than a consumer-specific payload.

### Adding a New Repository Trait

1. Define the trait in application ports.
2. Add it to the smallest relevant grouped dependency struct inside `AppServices`.
3. Implement it in infrastructure.
4. Add explicit query support.
5. Add serialized write-path support only if the write genuinely requires it.
6. Add testing/null implementations as needed.
7. Wire the concrete implementation in the binary crate.

### Adding a New GraphQL Query or Mutation

1. Add the use case to the appropriate application workflow.
2. Add GraphQL types and mappings.
3. Add the resolver following the fixed resolver pattern.
4. Add frontend query/mutation documents, types, and UI.

## Testing

Testing should mirror domain and boundary ownership.

Rules:

- prefer parallel `tests/` trees over large inline test blocks in production files
- use real in-memory SQLite with migrations where integration behavior matters
- use hand-rolled test doubles rather than a mock framework
- keep null/default repository implementations available for wiring focused tests
- use `cargo nextest run` rather than `cargo test`

Integration coverage should validate the canonical flow from API action to durable state to domain event to projection or subscription.

## Build and Release

Use the release script instead of ad hoc release commands.

Typical commands:

```bash
cargo build --workspace --locked
cargo nextest run --workspace --locked
cd apps/scryer-web && npm ci && npm run build
./scripts/release.sh
```

## Things That Will Bite You

1. Concurrent Cargo invocations can explode workspace disk usage.
2. The domain event table is append-only; queries must filter at SQL level.
3. Non-terminal tracked download state is re-derived on restart.
4. `scryer:`-prefixed tags carry structured semantics and preserve case.
5. The WASM plugin runtime is not `Send`; do not treat it like a normal async dependency.
6. Settings use scope inheritance and the `__inherit__` sentinel.
7. The metadata gateway has its own deployment lifecycle and compatibility window.
8. Frontend dev behavior can differ from production because the React Compiler only runs in production builds.

## Final Rule

A good change should make Scryer more legible, not merely more capable.

When in doubt, choose the design that preserves typed boundaries, keeps the event story coherent, respects the crate layering, and solves the real problem with the least necessary code.
