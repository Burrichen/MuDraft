# BUILD_STATUS

## Completed steps

- **Foundation → 1.1.0**: Tauri 2.12/React 19/TS 6/Vite 8 (Node 24.21.0, Rust 1.97.1); spec §§1–7; native E2E behind the test-only `e2e` feature.
- **Prompts 22–23 — private GitHub sync**: engine in `src-tauri/src/sync/`, Settings → GitHub Sync UI, `docs/SYNC.md`.
- **Prompt 24 — 1.2.0 release**: `.github/workflows/release.yml` builds the NSIS x64 setup `.exe` and a universal ad-hoc-signed `.dmg` from one commit into one draft release with `SHA256SUMS.txt`. Optional signing secrets. Docs: `docs/RELEASE.md`, `docs/MACOS.md`, `docs/RELEASE_ACCEPTANCE.md`.

- **1.3.0 release**: published and Latest (PR #5); schema v13.
- **1.3.5** (`release/v1.3.5`): Liked leaves the sidebar (opened from the Stats card). Stats leads with Unique albums in Collection and Total listens (dated + undated), and drops missing-data notes (undated, unrated, without genre/year, unknown length). No schema or sync change.

## Checks run

- `npm run check` (247 web tests, Rust suites) and `check-version.mjs` locally on `release/v1.3.5`.
- 1.3.0: Release run passed on Windows and macOS; installers matched `SHA256SUMS.txt`.

## Blockers

`docs/RELEASE_BLOCKERS.md`: real two-computer acceptance and real GitHub sync not yet run.

## Next step

Check the `release/v1.3.5` CI and Release runs, open the release PR, merge, then publish the v1.3.5 draft.
