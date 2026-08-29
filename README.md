<p align="center">
  <a href="https://github.com/scryer-media/scryer/releases"><img src="https://img.shields.io/github/v/release/scryer-media/scryer" alt="Release" /></a>
  <a href="https://securityscorecards.dev/viewer/?uri=github.com/scryer-media/scryer"><img src="https://api.scorecard.dev/projects/github.com/scryer-media/scryer/badge" alt="OpenSSF Scorecard" /></a>
  <a href="https://www.bestpractices.dev/projects/14165"><img src="https://www.bestpractices.dev/projects/14165/badge"></a>
</p>
<p align="center">
  <a href="https://www.scryer.media/scryer/donate/"><img src="https://img.shields.io/badge/Donate-%E2%9D%A4%EF%B8%8F-db61a2?logo=githubsponsors&logoColor=white" alt="Donate to Scryer" /></a>
  <a href="https://www.reddit.com/r/scryer_media/"><img src="https://img.shields.io/badge/Reddit-r%2Fscryer__media-FF4500?logo=reddit&logoColor=white" alt="Scryer on Reddit" /></a>
  <a href="https://discord.gg/SQmtZTanqm"><img src="https://img.shields.io/badge/Discord-Join%20the%20community-5865F2?logo=discord&logoColor=white" alt="Scryer on Discord" /></a>
</p>
<hr/>
<!-- A/B alternative: separate logo and wordmark.
<p align="center">
  <img src="apps/scryer-web/public/scryer-logo.svg" alt="Scryer logo" width="240" />
  <br />
  <img src="apps/scryer-web/public/scryer-wordmark.svg" alt="Scryer" width="480" />
</p>
-->

<p align="center">
  <img src="docs/img/scryer-introduction.webp" alt="Introducing Scryer" width="960" />
  <img src="docs/img/scryer-overview.webp" alt="Introducing Scryer" width="960" href="https://www.scryer.media/scryer/" target="_blank"/>
</p>


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
- searches for releases through indexers
- evaluates releases against quality and rules policies
- coordinates downloads and imports
- organizes you files for media servers
- manages subtitles (finds missing ones, time aligns when needed)
- deeply multi-lingual, interface and metadata (where available)
- helps you discover new media based on trends and what you already have

Conceptually it is "Sonarr + Radarr + Seerr + Bazarr, with some extra bits from other *arr tools", however Scryer is a single machine-code compiled binary that runs very efficiently compared to the *arr tools.

Scryer also handles Anime much better than the Arr tools, it uses multiple anime datasources (anidb, anilist, MAL, etc), understands the nuances of anime season and episode numbering, handles episode and multiseason packs cleanly, and even knows which episodes and movies are filler or canon!

*Scryer was written from scratch and has no affiliation with the Servarr tools*

## Technical Overview

Scryer ships as a single Rust binary with:

- an embedded web UI
- a GraphQL API
- SQLite-backed application state
- a plugin runtime for indexers, download clients, subtitle providers, and notifications

## Architecture

![How Scryer fits into your media system](docs/img/scryer-architecture.webp)

## Docker

Scryer publishes a first-party container image:

- `ghcr.io/scryer-media/scryer:latest`
- `ghcr.io/scryer-media/scryer:<minor>-latest`
   - `15-latest` for the `0.15.x` line

For Docker installation, Compose examples, environment variables, volumes, and
deployment notes, see the [Docker install
docs](https://www.scryer.media/scryer/docs/getting-started/#docker-compose).

## Development

- [Contributing guide](CONTRIBUTING.md)
- [Architecture notes](ARCHITECTURE.md)
- [Issues](https://github.com/scryer-media/scryer/issues)

For installation, upgrade guidance, and end-user documentation, use the website links at the top of this file.
