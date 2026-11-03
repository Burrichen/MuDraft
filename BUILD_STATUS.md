# BUILD_STATUS

## Completed steps

- **Foundation → 1.1.0**: Tauri 2.12/React 19/TS 6/Vite 8 (Node 24.21.0, Rust 1.97.1); spec §§1–7; native E2E behind the test-only `e2e` feature.
- **Prompts 22–23 — private GitHub sync**: engine in `src-tauri/src/sync/`, Settings → GitHub Sync UI, `docs/SYNC.md`.
- **Prompt 24 — 1.2.0 release**: `.github/workflows/release.yml` builds the NSIS x64 setup `.exe` and a universal ad-hoc-signed `.dmg` from one commit into one draft release with `SHA256SUMS.txt`. Optional signing secrets. Docs: `docs/RELEASE.md`, `docs/MACOS.md`, `docs/RELEASE_ACCEPTANCE.md`.

- **1.3.0 release** (`release/v1.3.0`): PR #4 merged (editable genres with reset, whole-album logging, Listen List track marks, track highlights, stats fixes; schema v13). 1.2.0 was never published; 1.3.0 supersedes it.

## Checks run

- PR #4 CI (run 41609205017): check and native E2E pass on Windows and macOS. macOS E2E restore step flaked once (missing `backups`), passed on re-run.
- `npm run check` and `check-version.mjs` locally on `release/v1.3.0`.
- 1.2.0 platform checks: see Release run 39595028911. Skipped: `live-sync`. Untested: signing paths, Gatekeeper prompt.

## Blockers

`docs/RELEASE_BLOCKERS.md`: real two-computer acceptance and real GitHub sync not yet run.

## Next step

Check the `release/v1.3.0` Release run, run `docs/RELEASE_ACCEPTANCE.md` with the draft v1.3.0 files, merge the release PR, then publish. Delete the stale v1.2.0 draft.
