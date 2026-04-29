# AGENTS Instructions

Architecture documentation lives in a separate repo: `github.com/scryer-media/scryer-docs`.

For code discovery in this repo, use the `agent-context` MCP first if your
environment provides it. Start with `list_scopes`, select the repo scope that
corresponds to this repository, and use a workspace/group scope for cross-repo
searches when available. Use `search_symbols` for exact definitions and
`search_code` for broader semantic/hybrid discovery. Treat shell search as
follow-up refinement only after MCP has identified the relevant files, unless
you are doing a narrow exact-string confirmation. If MCP is unavailable in your
environment, fall back to `rg` or other local search tools.

Use this order when making architectural decisions:

1. Confirm the requested change matches existing module boundaries.
2. Validate route/view and component boundaries in the frontend before touching UI files.
3. Validate layer boundaries in the backend before touching crates.
4. Keep changes consistent with existing ownership and naming conventions.

If runtime interfaces change (GraphQL, subscription payloads, gateway contract), update API contract documentation in the scryer-docs repo.

## Runtime troubleshooting quickstart (Docker Compose logs)

For local stack checks, use this sequence:

1. `docker compose -f docker-compose.dev.yml ps`
   Confirm service status and current container names.

2. `docker compose -f docker-compose.dev.yml logs --tail=200 scryer`
   Inspect service startup/runtime logs.

3. `docker compose -f docker-compose.dev.yml logs --tail=200 nodejs`
   Inspect frontend logs.

4. `docker compose -f docker-compose.dev.yml logs --tail=200 nzbget`
   Check backing service health when app errors indicate upstream failures.
