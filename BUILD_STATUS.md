# BUILD_STATUS

## Completed steps

- **Foundation → Audit**: Tauri 2.12/React 19/TS 6/Vite 8 (Node 24.21.0, Rust 1.97.1); schema v1–v11; spec §§1–7; native WebdriverIO E2E behind the test-only `e2e` feature (`check:isolation`).
- **Windows release 1.0.0 (latest)**:
  - Colour correction (spec §1): charcoal/grey tokens, blue accents only, neutral selected surfaces and placeholders, charcoal icon. Listen ASAP defaults to mustard (v11 keeps chosen colours).
  - NSIS per-user installer `MuDraft_<version>_windows_x64_setup.exe` with offline WebView2 (~127 MB). Signing is optional via CI secrets (`docs/WINDOWS.md`).
  - `Windows release` workflow builds the installer on `windows-latest` and runs `scripts/windows/verify-install.ps1`. On a `v*` tag it publishes the release.
  - Fixed a Listen List race that dropped the search from the URL.

## Checks run

- macOS 26.6.2: `npm run check` (197 web, 200 Rust), `npm run e2e` 10/10, and a 9-screen × 4 size/zoom sweep with axe (no contrast findings).
- Windows (`windows-latest` CI): `npm run check`. The native E2E passes 10/10 through tauri-driver on WebView2 153, run de-elevated (wry#1782). The installer is 210 MB, unsigned. Install, shortcuts, the production data folder, restart, upgrade from 0.9.0 and an uninstall that keeps the profile all pass; screenshots show the grey theme.

## Unverified

- Windows theme seen only in CI screenshots (start-up and dialog).
- The WebView2 offline-install path is not exercised, because runners already have WebView2.
- Manual screen-reader pass.

## Blockers

See `docs/RELEASE_BLOCKERS.md`.

## Next step

Signing decision; macOS distribution.
