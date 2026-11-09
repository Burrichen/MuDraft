# BUILD_STATUS

## Completed steps

- **Foundation → 1.1.0**: Tauri 2.12/React 19/TS 6/Vite 8 (Node 24.21.0, Rust 1.97.1); spec §§1–7; native E2E behind the test-only `e2e` feature.
- **Sync** and the **release workflow** (1.2.0): `src-tauri/src/sync/`, `docs/SYNC.md`, `.github/workflows/release.yml`, `docs/RELEASE.md`.
- **1.3.0 / 1.3.5**: published (PRs #5, #6).
- **1.4.0**: published and Latest (PR #7, tag `v1.4.0`); `docs/release-notes/v1.4.0.md` lists the changes.
  - Schema v14 Next Up slots; v15 deletable Listen ASAP; v16 album release-group types.
  - Signed in-app updates (`tauri-plugin-updater`): the Release workflow publishes `latest.json` and the macOS `.app.tar.gz`, signed with the `TAURI_SIGNING_PRIVATE_KEY` secret.
  - Tag include/exclude filters; Top rated by albums or tracks; discographies sync; artist progress skips library singles and live recordings.

## Checks run

- Release run passed: test gates, live sync, Windows installer, DMG verified on Intel and Apple Silicon.
- CI passed after one rerun (a Windows paging test timed out at 5.17s; passed on rerun and in the Release gate).
- `latest.json` signatures verified against the embedded public key; `releases/latest/download/latest.json` serves 1.4.0.

## Blockers

- First real in-app update (1.4.0 → next) on Windows and macOS is unverified.
- Local macOS E2E runs can stall when the test window is in the background (WebDriver script timeouts); CI E2E passed.
- `docs/RELEASE_BLOCKERS.md`: two-computer acceptance and real GitHub sync pending.

## Next step

Install 1.4.0 on the other computers, then start the next version.
