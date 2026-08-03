# scryer-v0.17.3

AI generated release notes

Scryer 0.17.3 is a patch release focused on smarter release ranking, more reliable RSS and import flows, and safer compatibility handling across artwork, plugins, and GraphQL tooling.

## Highlights

- TRaSH-based scoring is now derived more directly from upstream guide data, improving release ranking and block behavior. This release also adds opt-in managed locale packs for French (`MULTi VF`, `MULTi VO`, `VOSTFR`), plus updated German and Asian locale handling. These locale packs ship disabled by default.
- Release parsing now captures streaming-service, audio-language, and subtitle-language signals more accurately, which improves required-audio rules, locale-aware matching, and release selection.
- RSS sync, pending release review, and release search received a broad round of fixes, reducing missed or incorrectly handled grabs.
- Manual import and completed-download matching were tightened up, including fixes for ambiguous titles, series-movie edge cases, and setup wizard drag-and-drop and validation.
- Title artwork can now be hosted through Scryer’s image proxy for third-party use, with follow-up fixes for poster links, title image handling, and cache behavior.
- Discovery, wanted, activity, and settings views received a follow-up polish pass, including smoother review and onboarding flows.

## Platform and compatibility

- Plugin loading now validates embedded artifacts against the host ABI, reducing incompatible plugin and runtime failures.
- GraphQL schema compatibility checks were tightened, including explicit compatibility overrides and release override forwarding for release workflows.
- Wasmtime cache reuse and release tooling were fixed up for more reliable builds and release test runs.
- This release also includes assorted CI, build, RSS gate, and release-parser edge-case fixes.