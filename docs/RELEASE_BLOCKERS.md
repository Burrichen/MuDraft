# Release blockers (updated for 1.2.0, 2026-10-21)

## Open — must resolve before publishing 1.2.0

1. **Two-computer acceptance**: `docs/RELEASE_ACCEPTANCE.md` hasn't been run on a real Mac and a real Windows PC with the draft's files. In particular, GitHub Sync has never been run against real GitHub.
2. **Live sync CI**: the `live-sync` job skips until a dedicated private test repository and token are configured (`docs/RELEASE.md`).

## Open — acceptable for a personal release, required for a public one

3. **Signing**: Windows is unsigned (SmartScreen may warn). macOS is ad-hoc signed and not notarized, so users must click Open Anyway once. Optional secrets enable Authenticode and Developer ID signing and notarization. The Developer ID path is untested.
4. **WebView2 offline install path**: bundled, but not exercised in CI, because runners already have WebView2. Test on a clean Windows VM.
5. **macOS Gatekeeper prompt for a downloaded file**: CI copies have no quarantine flag, so the approval flow is only checked by hand.
6. **Manual assistive-technology pass**: no VoiceOver/NVDA walkthrough yet. axe-core checks (jsdom, including the sync section) and keyboard tests are clean.
7. **Card title target size**: axe flags the 18 px title link, but the whole card is the target. Likely a false positive; confirm in the manual pass.

## Accepted / monitor

- `npm audit`: 0 production vulnerabilities. Dev-only WebdriverIO tooling findings are not shipped.
- `core:default` capability is broader than MuDraft uses; narrowing it is hardening, not a known issue.
- The `macos-15-intel` runner (used to run the Intel slice) is GitHub's last x86_64 macOS image, supported until about August 2027. After that, the x86_64 slice can only be inspected and run under Rosetta.
- List queries are N+1 per album but fine at 5,000 albums. Re-measure above 20,000.

## Measurements (not guarantees)

Mac15,6, Apple M3 Pro, 18 GB, macOS 26.6.2. Fixture: 5,000 albums, 60,000 tracks, 8,000 listens (`src-tauri/tests/perf.rs`).

- **Rust, release:** open DB 0.3 ms; Listen List 50 ms; Collection 81–94 ms; Stats 159 ms; Next Up 3 ms; Artist page 65 ms; export 193 ms; RSS 104 MB.
- **App, debug `e2e` build:** launch to shell 0.5–1.0 s; first 120 cards 0.24–0.76 s; search 0.24 s; Collection 0.5 s; Stats 0.55 s.
- **1.2.0 universal DMG:** 13.9 MiB (app 30.6 MiB).
