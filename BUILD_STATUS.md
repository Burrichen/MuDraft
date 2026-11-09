# BUILD_STATUS

## Completed steps

- **Foundation → 1.1.0**: Tauri 2.12/React 19/TS 6/Vite 8 (Node 24.21.0, Rust 1.97.1); spec §§1–7; native E2E behind the test-only `e2e` feature.
- **Sync** and the **release workflow** (1.2.0): `src-tauri/src/sync/`, `docs/SYNC.md`, `.github/workflows/release.yml`, `docs/RELEASE.md`.
- **1.3.0 / 1.3.5**: published (PRs #5, #6).
- **1.4.0** (`release/v1.4.0`, uncommitted, version bumped; release notes list every change):
  - Schema v14 Next Up slots; v15 deletable Listen ASAP; v16 album release-group types.
  - Signed in-app updates (`tauri-plugin-updater`); Release adds `latest.json`.
  - Version in sidebar and Settings; MusicBrainz named only in Settings.
  - Stats: Top rated Albums/Tracks toggle; two cards removed.
  - Tag bubbles include → exclude → off; album About section removed.
  - Artist progress skips library singles/live; interviews, audiobooks, and audio dramas filtered; discographies sync (fetched data merges by newest fetch).
  - macOS fix: `Popover` survives WebKit click focus (`Popover.test.tsx`).

## Checks run

- `npm run check` (web and Rust suites, lint, format, clippy); see final report for counts.
- `check-version.mjs` (1.4.0); `latest.json` step dry-run with fake files.

## Blockers

- `TAURI_SIGNING_PRIVATE_KEY` secret must be added before the Release run (key at `~/.tauri/mudraft-updater.key`, outside the repo).
- A real update (1.4.0 → next) on Windows and macOS is unverified.
- `docs/RELEASE_BLOCKERS.md`: two-computer acceptance and real GitHub sync pending. Native E2E not re-run.

## Next step

Add the secret, review the app, commit, then push `release/v1.4.0`.
