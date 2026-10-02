# BUILD_STATUS

## Completed steps

- **Foundation → Profile export/restore**: Tauri 2.12/React 19/TS 6/Vite 8 (Node 24.21.0, npm 11.19.0, Rust 1.97.1); schema v1–v10; every product feature in spec §§1–7.
- **Audit and hardening (latest)**:
  - Native E2E: WebdriverIO 9.32 in standalone mode against the real binary built with cargo feature `e2e`. That feature embeds `tauri-plugin-wdio-webdriver`, MusicBrainz fixtures and dialog overrides; `check:isolation` keeps them out of normal builds.
  - Rust journey test from CSV to restore, plus a large-library measurement test.
  - Fixes: artwork image checks on restore and serve, back-navigation state kept in the URL, paged card rendering, link target size, focusable Stats table.
  - `docs/RELEASE_BLOCKERS.md` holds the blocker list and measurements.

## Checks run (macOS 26.6.2)

- `npm run check`: 182 web tests; clippy clean (also with `--features e2e`); 199 Rust tests.
- `npm run e2e`: 10/10 native journey steps, three consecutive runs.
- Sweep of 9 screens × 4 sizes/zooms: no overflow; focus visible; axe-core clean except a likely false positive (blocker 4).

## Unverified

- CI `e2e` job and Windows runs (not pushed); manual screen-reader pass.

## Blockers

See `docs/RELEASE_BLOCKERS.md` (signing, Windows E2E, manual AT pass).

## Next step

Release packaging, signing status, CI artifacts.
