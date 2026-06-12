# scryer service

This crate provides a first-version executable that exposes:

- `GET /` backend notice page (points to Next.js app)
- `POST /graphql` GraphQL endpoint

## Run

```bash
cd crates/scryer
cargo run
```

Run the SPA separately:

```bash
cd apps/scryer-web
npm install
npm run dev
```

The backend can expose the SPA URL in the root notice via:

```bash
SCRYER_WEB_UI_URL=http://127.0.0.1:3000
```
`SCRYER_WEB_DIST_DIR` controls where the service serves embedded static UI assets from.
When set (for example `${SCRYER_DIST=...}` by the build pipeline), the service serves the built SPA from that path at `/`.

Copy and edit the config template:

```bash
cp crates/scryer/.env.example .env
```

Load values before running (choose a preferred loader):

```bash
set -a
. .env
set +a
cargo run
```

Runtime bootstrap environment values (optional, used when DB settings are empty):

- `SCRYER_DB_PATH` (default `sqlite://file::memory:?mode=memory&cache=shared`)
- `SCRYER_BIND` (default `127.0.0.1:8080`)
- `SCRYER_BASE_PATH` (optional, defaults to `/`; set to `/scryer` to host behind a reverse-proxy path prefix)

Application configuration now lives in `settings_definitions` and `settings_values`.

Configuration reads and writes are exposed through typed GraphQL fields on `POST /graphql`,
including:

- `serviceSettings` / `updateServiceSettings(...)`
- `libraryPaths` / `updateLibraryPaths(...)`
- `mediaSettings(scope: ...)` / `updateMediaSettings(...)`
- `qualityProfileSettings` / `saveQualityProfileSettings(...)`
- `downloadClientRouting(scope: ...)` / `updateDownloadClientRouting(...)`
- `indexerRouting(scope: ...)` / `updateIndexerRouting(...)`

Common managed keys:

- `system.service.nzbget.url`
- `system.service.nzbget.username`
- `system.service.nzbget.password` (sensitive)
- `system.service.nzbget.dupe_mode`
- `media.media.movies.path`
- `media.media.series.path`

Legacy bootstrap settings (still supported as fallback):

- `SCRYER_NZBGET_URL` (default `http://127.0.0.1:6789`)
- `SCRYER_NZBGET_DUPE_MODE` / `SCRYER_NZBGET_DUPEMODE` (optional, defaults to `SCORE`)
- `SCRYER_NZBGET_USERNAME`
- `SCRYER_NZBGET_PASSWORD`
- `SCRYER_BASE_PATH` (optional; serves the UI, GraphQL, health, and WebSocket endpoints under that prefix)
- `SCRYER_WEB_UI_URL` (optional, default `http://127.0.0.1:3000`)
- `SCRYER_WEB_DIST_DIR` (optional, default `./crates/scryer/ui`)

MVP workflow: open the SPA on `http://127.0.0.1:3000` and use the nav/search experience for title add/queue actions.
`addTitleAndQueueDownload` should return success only when NZBGet accepts the exact NZB URL.

### NZBGet category routing

When queueing titles, Scryer now submits an NZBGet category derived from the title facet (`movie`, `series`, `anime`, or `other`).

For a standard completed-directory workflow:
- Configure NZBGet with matching category definitions for `movie`, `series`, `anime`, and `other`.
- Set category-specific `DestDir` under a common completed root (for example, `/data/completed/movie`, `/data/completed/series`, etc.).
- Configure your Servarr clients to monitor the completed directories and move final assets into your library destinations.
- Keep this scryer category on queued items as the routing key; NZBGet category should remain your integration point for mover semantics.

Data storage:
- SQLite is used with SQLx and runs through the bundled SQLite library from `libsqlite3-sys`.
- No system SQLite package is required at runtime for basic DB access.
