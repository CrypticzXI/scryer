# `scryer-domain`

Shared entities, value objects, and invariants used by every other crate:
titles and media facets (`movie`, `series`, `anime`), episodes/collections,
download queue and completed-download models, import results, quality and
release types, settings and permission types, plus the title sort/normalization
helpers.

Rules of the crate:

- No IO, no persistence, no transport concerns; other crates depend on it,
  it depends on none of them (see [ARCHITECTURE.md](../../ARCHITECTURE.md)).
- Serde shapes here are persisted and sent over the wire, so field renames keep
  `alias`es for anything already stored (for example
  `CompletedDownload.release_name`, formerly `nzb_name`).
