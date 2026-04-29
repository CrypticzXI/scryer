# Scryer Repo Guidance

This file is intentionally repo-facing. Machine-local workflow preferences,
personal agent habits, and workstation-specific instructions belong in the
workspace-level `AGENTS.md`, not here.

## Project map

- Backend crates live under `crates/`:
  - `scryer-domain`: shared domain types and invariants
  - `scryer-application`: orchestration and use cases
  - `scryer-infrastructure`: persistence, external clients, and adapters
  - `scryer-interface`: GraphQL and API mapping
  - `scryer`: service entrypoint
- Frontend lives in `apps/scryer-web/`:
  - routes and containers under `src/` and `components/`
  - shared UI primitives under `components/ui/`
  - translations under `lib/i18n/locales/`
- Build and release automation lives in `cargo xtask`
- Docker assets live under `docker/`
- CI lives under `.github/workflows/`

## Documentation and planning

- Architecture documents, plans, and ADRs live in
  `github.com/scryer-media/scryer-docs`.
- Before making an architectural change, check whether a relevant plan or ADR
  already exists there.
- If a change affects runtime contracts or documented behavior, update the docs
  repo in the same workstream.

## Change discipline

- Preserve existing layer boundaries. Prefer extending the canonical flow in the
  owning layer over introducing a parallel path.
- Keep changes small and ownership-consistent. Avoid spreading the same logic
  across multiple entry points when one shared orchestration path should own it.
- In the web app, route all user-visible copy through `t(...)` and locale files.
- Reuse existing UI primitives before introducing bespoke controls or patterns.
- When touching a cross-cutting behavior, trace all public entry points for that
  concern before deciding the implementation shape.

## Verification

- Run the narrowest checks that prove the change, then broaden when the risk
  justifies it.
- Direct Cargo commands should use `--locked`.
- Useful commands:

```bash
cargo build --workspace --locked
cargo test --workspace --locked

cargo xtask --help
cargo xtask ci clippy
cargo xtask stack up
cargo xtask stack up --seed
cargo xtask release --dry-run

cd apps/scryer-web && npm ci && npm run build
```

- Do not add `--locked` to `cargo xtask ci clippy`; run that command exactly as
  written.

## Local stack troubleshooting

When debugging the local development stack, these commands are the standard
starting point:

```bash
docker compose -f docker-compose.dev.yml ps
docker compose -f docker-compose.dev.yml logs --tail=200 scryer
docker compose -f docker-compose.dev.yml logs --tail=200 nodejs
docker compose -f docker-compose.dev.yml logs --tail=200 nzbget
./scripts/stack-restart.sh
```
