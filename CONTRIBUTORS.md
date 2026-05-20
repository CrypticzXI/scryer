# Contributing to Scryer

This document covers the development setup, architecture, and workflows for contributors.

## Repo Layout

```
scryer/
  crates/
    scryer-domain/          # Domain models and traits
    scryer-application/     # Use cases and business logic
    scryer-infrastructure/  # External integrations (DB, HTTP, download clients)
    scryer-interface/       # GraphQL API and WebSocket layer
    scryer-release-parser/  # Release title parsing and scoring
    scryer-rules/           # OPA/Rego policy engine
    scryer-plugins/         # WASM plugin runtime and built-in plugins
    scryer-mediainfo/       # Media file analysis (ffprobe)
    scryer/                 # Binary entry point, CLI, server bootstrap
    scryer-mock-apis/       # Mock services for testing
  apps/
    scryer-web/             # Vite + React 19 + React Router 7 SPA
  docker/                   # Dockerfiles for build, dev, and release
  scripts/                  # Dev stack orchestration and release tooling
  docs/                     # Getting started guide and assets
```

## Design-First Workflow

Architecture documentation lives in the [scryer-docs](https://github.com/scryer-media/scryer-docs) sibling repo:

1. Update or add an intention in `intentions/`
2. Derive/adjust specs from that intention in `specs/`
3. Update `architecture/manifest.yaml` if structure changes
4. Record decisions in `adr/`
5. Implement code only after artifacts are aligned

## UI Architecture

The frontend is a **Vite + React 19 + React Router 7** single-page application with a unified media model behind the API, where media-type is a facet (`movie`, `series`, `anime`) filtered at UI boundaries.

- **UI primitives**: shadcn/ui components in `apps/scryer-web/components/ui/`
- **Theme**: Tailwind v4 with semantic color tokens in `apps/scryer-web/app/globals.css`
- **i18n**: All visible strings go through `t(...)` from `apps/scryer-web/lib/i18n/`
- **GraphQL**: urql with `network-only` policy (no client-side caching)

## Prerequisites

- **Rust** (stable toolchain) + Cargo
- **Node.js** 22+ and npm
- **Docker** and Docker Compose

## Git Hooks

`gitleaks` is required for commits in this repo.

After cloning, run:

```bash
git config core.hooksPath .githooks
```

The versioned `pre-commit` hook will block commits when `gitleaks` reports staged secrets or when staged diffs contain machine-local usernames or home-directory paths.

### macOS Privacy & Security

If `cargo build`, `cargo xtask`, or other Rust commands stall around `build-script-build`, macOS is likely blocking newly compiled local binaries from your terminal app.

Enable your terminal under `System Settings -> Privacy & Security -> Developer Tools`, then fully quit and reopen it.

`spctl developer-mode enable-terminal` only helps `Terminal.app`. If you use Ghostty, iTerm, WezTerm, or another terminal, you must allow that specific app in the Developer Tools list.

## Development Stack

The dev stack is orchestrated via Docker Compose:

```bash
cargo xtask stack up
```

Use `cargo xtask stack up --seed` when you also want the one-shot seed container to run.

This brings up:
- NZBGet container (download client for testing)
- Scryer Rust service (compiled and run inside the container)
- Vite dev server for the web UI
- Nginx reverse proxy combining both on port 3000

`cargo xtask stack up` recreates the Rust service container each time, so local testing
starts from a fresh Linux build tree by default.

To stop:

```bash
cargo xtask stack down
```

View logs:

```bash
cargo xtask stack logs
```

## Running Services Individually

### Scryer (Rust backend)

```bash
cargo run -p scryer
```

Environment is loaded from `crates/scryer/.env`.

### Web UI (Vite dev server)

```bash
cd apps/scryer-web && npm run dev
```

## Build & Test

```bash
# Rust
cargo build --workspace --locked
cargo nextest run --workspace --locked

# Frontend
cd apps/scryer-web && npm ci && npm run build

# Lint
cargo clippy --workspace --locked -- -D warnings
cd apps/scryer-web && npm run lint
```

## Release

From the repo root:

```bash
cargo xtask release          # patch bump
cargo xtask release --minor  # minor bump
cargo xtask release --dry-run
```

`cargo xtask release` handles: cargo update, audit, clippy, tests, npm audit fix, lint, version bumping all workspace crates, cargo check, signed tag, and push. CI builds and publishes the release on tag push. The legacy shell script is only a compatibility wrapper.

The root `cargo xtask` binary is intentionally thin. Release and migration
commands still keep their existing `cargo xtask ...` shape, but they delegate
to the dedicated `xtask-release` / `xtask-migrations` packages under the hood.
For advanced debugging you can also run `cargo xtask-release -- ...` or
`cargo xtask-migrations -- ...` directly.

`cargo xtask release --dry-run` is a mutating release rehearsal. It runs the same release-prep steps as a real app release, including `npm audit fix`, `cargo fmt`, `cargo clippy --fix`, lockfile refreshes, and the normal validation passes, but it stops before the Cargo version bump, signed tag, and push. If the dry run succeeds, xtask keeps those prep changes, commits them, and writes a reusable cache marker under `tmp/xtask-release-dry-run.json` plus cached bundled-plugin artifacts under `tmp/xtask-release-dry-run-builtins/`. A subsequent real `cargo xtask release` on the same clean commit and release args can reuse that cache only if the computed next tag still matches and the published central `catalog-v2.json` checksum is unchanged; otherwise xtask falls back to a full validation run.

## Reporting Issues

File bug reports and feature requests in the [GitHub Issues](https://github.com/scryer-media/scryer/issues) tab.
