# `scryer-infrastructure`

Adapters that implement the ports declared in `scryer-application`:

- `storage/`, `workflow/stores/`, `settings/`, `users/`, `security/`: SQL
  persistence (SQLite by default via the bundled `libsqlite3-sys`; PostgreSQL
  supported) behind the repository traits.
- `downloads/clients/`: download-client integrations (NZBGet, SABnzbd, Weaver,
  plugin-backed clients) and the prioritized routing/failover router.
- `indexers/`, `discovery/`, `metadata/`: indexer clients, release discovery,
  and metadata providers.
- `notifications/`, `oauth/`, `media/`, `customization/`: notification
  dispatch, OAuth flows, media probing glue, UI customization storage.
- `workflow/file_importer.rs`: filesystem moves/hardlinks/copies for imports.

Business policy stays in `scryer-application`; these adapters translate at the
boundary and must not grow decision logic of their own.
