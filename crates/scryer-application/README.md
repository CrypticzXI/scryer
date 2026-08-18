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
- `src/lib_tests/`: crate-level test harness and shared test doubles
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

- `src/lib.rs` stays small: it re-exports the public surface and defines `AppError`; crate-level tests live in `src/lib_tests/`.
- Files are grouped by domain directory; start with a directory's `mod.rs` when choosing where to read next.
- Some large workflow files are `include!`d/`#[path]`-declared submodules of a domain `mod.rs`; `cargo fmt` does not see those, so format such hunks by hand.
