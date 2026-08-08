# scryer-v0.18.7

AI generated release notes

## User-facing changes
- No user-facing changes are included in this release.
- Runtime behavior, features, and application functionality are unchanged from `scryer-v0.18.6`.

## Internal improvements
- Hardened release CI for Windows `winget` install validation by extracting manifests into an isolated directory before verification.
- This reduces validation cross-contamination and makes release checks more reliable.