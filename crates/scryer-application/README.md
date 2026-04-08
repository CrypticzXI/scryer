# `scryer-application`

Business-logic crate for Scryer.

It owns:

- use-case orchestration over shared domain contracts
- repository, client, and provider interfaces
- cross-domain request/response contracts and helper glue
- background workflow entry points used by the binary crate

## Reading Guide

Start here when you are changing one of these areas:

- `src/lib.rs`: crate index, public re-exports, and `AppError`
- `src/lib_tests.rs`: root test harness and shared crate-level test doubles
- `src/ports.rs`: repository/client/provider traits
- `src/services.rs`: `AppServices` and `AppUseCase`
- `src/contracts.rs`: shared request/response DTOs and cross-domain structs
- `src/helpers.rs`: crate-wide helper functions and constants

Workflow directories:

- `src/acquisition/`: wanted-item search, pending releases, RSS follow-up
- `src/catalog/`: title lifecycle, discovery, hydration, title images, facets
- `src/library/`: scan, rename, filesystem walk, delete, title matching
- `src/import/`: completed downloads, manual import, post-processing, upgrades
- `src/integration/`: download queue polling, tracked downloads, indexer testing
- `src/media/`: media analysis and language helpers
- `src/quality/`: profiles, scoring, release parsing, dedup
- `src/events/`: activity and domain-event shaping
- `src/notifications/`: notification orchestration and dispatch
- `src/settings/`: settings workflows and key definitions
- `src/jobs/`: background job orchestration and housekeeping
- `src/rules/`: rule-set workflows and request shaping
- `src/plugins/`: plugin lifecycle and managed rules
- `src/security/`: auth, admin, and backup-related workflows
- `src/health/`: health and readiness flow
- `src/subtitles/`: subtitle orchestration, providers, search, sync, scoring

When a workflow file is still large, start with the nearby helper files before opening the main entrypoint:

- `src/library/discovery.rs`, `src/library/rename.rs`, and `src/library/scan/helpers.rs`
- `src/import/parameters.rs` and `src/import/title_resolution.rs`
- `src/acquisition/decision_helpers.rs` and `src/acquisition/search_queries.rs`

## Notes

- `src/lib.rs` is intentionally small again; crate-level tests now live in `src/lib_tests.rs`.
- Physical files are grouped by domain directory, and the main workflow modules now use descriptive internal names for the largest domains.
- A few path-based compatibility declarations still exist for smaller domains; start with the directory `mod.rs` guide when choosing where to read next.
