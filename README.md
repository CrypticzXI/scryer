# Scryer

<p align="center">
  <a href="https://github.com/scryer-media/scryer/releases"><img src="https://img.shields.io/github/v/release/scryer-media/scryer" alt="Release" /></a>
  <a href="https://ghcr.io/scryer-media/scryer"><img src="https://img.shields.io/badge/container-ghcr.io-blue" alt="Container" /></a>
</p>

[![Scryer overview](docs/img/scryer-overview.webp)](https://www.scryer.media/scryer/)


<h3 align="center">
    <a href="https://www.scryer.media/scryer/docs/getting-started/">Getting Started Guide</a>
</h3>

<p align="center">
For more information about the tool, please visit the <a href="https://www.scryer.media/scryer">official webiste</a>
</p>

## What Scryer Is

Scryer is a self-hosted media management application for movies, TV series, and anime.

At a high level, it:

- monitors libraries and tracked titles
- searches for releases through pluggable providers
- evaluates releases against quality and rules policies
- coordinates downloads and imports
- organizes files for downstream media servers
- manages subtitles
- deeply multi-lingual, when you select your chosen language, your content gets updated as well (limited to upstream language content availability)

Conceptually it is "Sonarr + Radarr, with some extra bits from other *arr tools", however Scryer is a machine-code compiled binary that runs very efficiently compared to the *arr tools.

*Scryer was written from scratch and has no affiliation with the Servarr tools*

## Technical Overview

Scryer ships as a single Rust binary with:

- an embedded web UI
- a GraphQL API
- SQLite-backed application state
- a plugin runtime for indexers, download clients, subtitle providers, and notifications

## Architecture

```text
┌─────────────────────────────────────────┐
│  scryer binary                          │
│  ┌───────────┐  ┌────────────────────┐  │
│  │ Web UI    │  │ GraphQL API        │  │
│  └───────────┘  └────────────────────┘  │
│  ┌───────────────────────────────────┐  │
│  │ Application layer                 │  │
│  │ acquisition · import · subtitles  │  │
│  │ rename · post-processing · rules  │  │
│  └───────────────────────────────────┘  │
│  ┌───────────────────────────────────┐  │
│  │ Plugin System (WASM)              │  │
│  └───────────────────────────────────┘  │
│  ┌───────────────────────────────────┐  │
│  │ Storage (SQLite)                  │  │
│  └───────────────────────────────────┘  │
└─────────────────────────────────────────┘
         │                     │
    ┌────┴─────┐         ┌─────┴──────┐
    │ Metadata │         │ Indexers & │
    │  API     │         │ Clients    │
    └──────────┘         └────────────┘
```

## Docker

Scryer publishes a first-party container image:

- `ghcr.io/scryer-media/scryer:latest` auto-selects the best embedded Linux payload for the host CPU while keeping portable plus optimized Linux payloads in one image

The Docker contract is intentionally small:

- Persist app data in `/config`
- Use `PUID` / `PGID` when you want the container to re-own `/config` and then drop privileges
- `TZ` defaults to `Etc/UTC`
- `UMASK` is optional and accepts standard octal values such as `022`
- `--user=1000:1000` and `--read-only=true` are both supported

### docker-compose

```yaml
services:
  scryer:
    image: ghcr.io/scryer-media/scryer:latest
    container_name: scryer
    environment:
      - PUID=1000
      - PGID=1000
      - TZ=Etc/UTC
      - UMASK=022 # optional
      - SCRYER_METADATA_GATEWAY_GRAPHQL_URL=https://smg.scryer.media/graphql
    volumes:
      - /path/to/scryer/config:/config
      - /path/to/your/movies:/data/movies
      - /path/to/your/series:/data/series
    ports:
      - 8080:8080
    restart: unless-stopped
```

### docker run

```bash
docker run -d \
  --name=scryer \
  -e PUID=1000 \
  -e PGID=1000 \
  -e TZ=Etc/UTC \
  -e UMASK=022 \
  -e SCRYER_METADATA_GATEWAY_GRAPHQL_URL=https://smg.scryer.media/graphql \
  -p 8080:8080 \
  -v /path/to/scryer/config:/config \
  -v /path/to/your/movies:/data/movies \
  -v /path/to/your/series:/data/series \
  --restart unless-stopped \
  ghcr.io/scryer-media/scryer:latest
```

If you run the container as root, the entrypoint will re-own `/config` to `PUID` / `PGID` and then drop privileges before starting `scryer`. If you run with `--user=1000:1000`, make sure the bind mount is already owned by that uid/gid because the ownership repair path is skipped in non-root mode.

For hardened deployments, `scryer` supports `--read-only=true` as long as `/config` remains writable.

## Development

- [Contributors guide](CONTRIBUTORS.md)
- [Architecture notes](ARCHITECTURE.md)
- [Issues](https://github.com/scryer-media/scryer/issues)

For installation, upgrade guidance, and end-user documentation, use the website links at the top of this file.

---
*All media images courtesy of [thetvdb](https://thetvdb.com/)*
