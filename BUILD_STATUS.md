# BUILD_STATUS

## Completed steps

- **Foundation → 1.1.0**: Tauri 2.12/React 19/TS 6/Vite 8 (Node 24.21.0, Rust 1.97.1); spec §§1–7; Windows installer and release workflow; native E2E behind the test-only `e2e` feature.
- **Prompts 22–23 — private GitHub sync**: engine in `src-tauri/src/sync/` (schema v12 `sync_state`), deterministic `sync/` format, three-way merge, conflicts, backups, fast-forward commits, `keyring-core` token store, `fs4` free-space check. Settings → GitHub Sync (`src/features/sync/`): setup steps, connect/test/token/disconnect, status, progress events, first-sync preview (Upload/Download/Merge), per-field conflict choices, recovery advice. User guide: `docs/SYNC.md`.

## Checks run (macOS 26.6.2 only)

- `npm run check` passes: 239 web tests (including axe in jsdom) and 150 Rust lib tests, among them 30 sync tests against mocked GitHub with two separate profiles.
- macOS Keychain round trip passes (`os_secure_store`, ignored by default).
- Native E2E (WebDriver, macOS WebKit): 11/11, including a Settings → GitHub Sync step that goes through the real bridge without contacting GitHub.
- Not run: real GitHub (`live_` test needs `MUDRAFT_SYNC_TEST_*`), anything on Windows, Windows Credential Manager.

## Unverified

- Real GitHub API and token scopes; sync UI on Windows/WebView2; visual review of the sync section; screen-reader pass.

## Blockers

See `docs/RELEASE_BLOCKERS.md`. Sync needs a real two-computer check with a private repository.

## Next step

Run the live test and the Windows CI checks; manual Mac + Windows sync trial.
