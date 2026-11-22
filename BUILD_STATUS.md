# BUILD_STATUS

## Completed steps

- **Foundation → 1.4.0** (published; PR #7): Tauri 2.12/React 19/TS 6/Vite 8 (Node 24.21.0, Rust 1.97.1); spec §§1–7, GitHub sync, release workflow, signed in-app updates (`latest.json`), schema v14–v16.
- **Update 1 — Lists** (spec §11), 1.5.0 (published, Latest; PR #8): Lists page, schema v17, native Pitchfork importer, status derived live, reviewed MusicBrainz matches (no duplicates), lists in export and sync; no edition chooser for one edition; delete extra editions.
- **Android APK** (`docs/ANDROID.md`), merged from the `apk` branch: no Stats (Liked takes its slot), Lists included, document picker, Keystore token, bundled TLS roots (the list importer uses `net.rs`), update check opens the release APK. The Release workflow `android` job builds it when the `ANDROID_KEYSTORE_*` secrets exist.

## Checks run

- Lists on the 1.4.0 base: web, clippy and Rust tests; the 1.5.0 release run passed.
- After merging `apk`: typecheck, lint, prettier, 282 web tests, rustfmt, clippy `-D warnings`, all Rust tests, isolation check pass.
- Not run: native E2E, macOS by hand, live Pitchfork fetch in the app, APK on a phone.

## Notes

- Run `npm test` from PowerShell, not Git Bash. Build Rust with `CARGO_BUILD_JOBS=2`.
- Android signing key: `~/.mudraft-android/` and the `ANDROID_KEYSTORE_*` secrets (new in 1.5.0; the 1.4.0 APK used another key). Back up.

## Blockers

- First real in-app update (1.4.0 → next) is unverified.
- `docs/RELEASE_BLOCKERS.md`: two-computer acceptance and real GitHub sync pending.

## Next step

Test 1.5.0 on Windows and the APK on a phone.
