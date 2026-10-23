# BUILD_STATUS

## Completed steps

- **Foundation → 1.1.0**: Tauri 2.12/React 19/TS 6/Vite 8 (Node 24.21.0, Rust 1.97.1); spec §§1–7; native E2E behind the test-only `e2e` feature.
- **Prompts 22–23 — private GitHub sync**: engine in `src-tauri/src/sync/`, Settings → GitHub Sync UI, `docs/SYNC.md`.
- **Prompt 24 — 1.2.0 release**: `.github/workflows/release.yml` builds the NSIS x64 setup `.exe` and a universal ad-hoc-signed `.dmg` from one commit into one draft release with `SHA256SUMS.txt`. Optional signing secrets. Docs: `docs/RELEASE.md`, `docs/MACOS.md`, `docs/RELEASE_ACCEPTANCE.md`.

## Checks run (Release run 39595028911, commit e37c0f7)

- Test gates (Windows, macOS): `npm run check`, 239 web + 151 Rust tests, including the two-device acceptance journey; real Keychain and Credential Manager round trips.
- Windows: installed the published 1.1.0 → Start-menu launch → restart → upgrade to 1.2.0 → uninstall, profile kept.
- DMG on Apple Silicon (arm64 and Rosetta x86_64) and Intel runners: layout, plist, `lipo`, ad-hoc `codesign`, LaunchServices launch, restart, upgrade from a 0.9.0-labelled build of the same code (no earlier macOS release exists), uninstall.
- Draft `v1.2.0` assets and checksums inspected; not published.
- Skipped: `live-sync` (no test secrets). Untested: Developer ID and Authenticode signing paths, quarantined-download Gatekeeper prompt.

## Blockers

`docs/RELEASE_BLOCKERS.md`: real two-computer acceptance and real GitHub sync not yet run.

## Next step

Run `docs/RELEASE_ACCEPTANCE.md` with the draft's files, then publish.
