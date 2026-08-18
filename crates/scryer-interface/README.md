# `scryer-interface`

The GraphQL API. This crate composes the schema (`build_schema*`,
`export_schema_sdl`, `ApiContext`) from the per-area interface crates:

- `scryer-interface-core`: request context, dataloaders, `AppError` → GraphQL
  error-code mapping.
- `scryer-interface-query` / `scryer-interface-subscription`: root query and
  subscription resolvers.
- `scryer-interface-media` and `scryer-interface-media-types`: catalog/media
  payload types and mappers.
- `scryer-interface-acquisition`, `-import`, `-metadata`, `-security`,
  `-settings`, `-system`: mutations and payloads for those areas.

`src/bin/export-graphql-schema.rs` prints the SDL; the web app and SDK
generate their clients from it. Resolvers validate and map — product policy
lives in `scryer-application`.
