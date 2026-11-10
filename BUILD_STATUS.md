# BUILD_STATUS

## Completed steps

- **Foundation → 1.4.0**: Tauri 2.12/React 19/TS 6/Vite 8 (Node 24.21.0, Rust 1.97.1); sync, signed in-app updates, release workflow. Latest release: v1.4.0.
- **`apk` branch (unreleased)**: Android APK beside the installers; `docs/ANDROID.md`.
  - Stats removed in the APK only; Liked takes its nav slot.
  - Android file picker (`commands/files.rs`), Keystore token, bundled-roots TLS (`net.rs`), update check opening the release APK, safe-area/back/keyboard handling.
  - Release workflow `android` job (only with signing secrets).

## Checks run

- `npm run check` (web 267 tests, Rust) and `check:isolation` pass.
- arm64 release APK on an API 36 emulator: CSV import and template save, profile export and restore, MusicBrainz search, update check, Keystore token save, Back, insets, keyboard.

## Blockers

- Not run: custom-artwork picker, add-album-by-search commit, a real GitHub sync, the update download, the `android` CI job, a real phone.
- No release signing key yet (`docs/ANDROID.md`).
- Narrow screens: wide tables (CSV rows) are cramped.
- First real in-app update (1.4.0 → next) unverified; `docs/RELEASE_BLOCKERS.md` items pending.

## Next step

Create the signing key and secrets, run the Release workflow, test on a real phone, then merge `apk`.
