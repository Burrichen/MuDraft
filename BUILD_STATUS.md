# BUILD_STATUS

## Completed steps

- **Foundation → 1.4.0** (published, Latest; PR #7): Tauri 2.12/React 19/TS 6/Vite 8 (Node 24.21.0, Rust 1.97.1); spec §§1–7, GitHub sync, release workflow, signed in-app updates (`latest.json`), schema v14–v16. Details: `docs/release-notes/v1.4.0.md`.
- **Update 1 — Lists** (spec §11), beta 1.5.0: Lists page, schema v17 (`list`, `list_entry`), native Pitchfork importer, listened/Listen List status derived live, adding unlistened albums via reviewed MusicBrainz matches (no duplicates), lists in profile export and sync. Also: no edition chooser for a single edition; delete extra editions on the album page.

## Checks run

- Update 1, on the 1.2.0 base: web (254 tests), clippy, Rust tests. Parser run once against the live Pitchfork page: 100 albums.
- After porting onto 1.4.0: see the release run for 1.5.0.
- Not run: native E2E, macOS by hand, a live Pitchfork fetch from the app.

## Notes

- Git Bash duplicates vitest by path casing; run `npm test` from PowerShell. Build Rust with `CARGO_BUILD_JOBS=2`.
- Matching a whole list uses the CSV import page, about a second per album.

## Blockers

- First real in-app update (1.4.0 → next) on Windows and macOS is unverified.
- `docs/RELEASE_BLOCKERS.md`: two-computer acceptance and real GitHub sync pending.

## Next step

Test the 1.5.0 beta; then MusicBrainz-matching polish or the next version.
