# Release blockers (updated for 1.0.0, 2026-10-02)

## Open — must resolve before a public release

1. **Signing**: Windows 1.0.0 is an unsigned personal release (SmartScreen may warn). CI signs when `WINDOWS_CERTIFICATE` secrets exist. There is no macOS Developer ID/notarization and no macOS distribution yet.
2. **WebView2 offline install path**: bundled, but not exercised in CI, because runners already have WebView2. Test on a clean Windows VM.
3. **Manual assistive-technology pass**: VoiceOver/NVDA walkthrough not done. Automated axe-core and keyboard sweeps are clean except item 4.
4. **Card title target size**: axe flags the title link (18 px). The real target is the whole card (`::after` cover link), so it is likely a false positive. Confirm in the manual pass.

## Accepted / monitor

- `npm audit`: 0 production vulnerabilities. High-severity findings are in dev-only WebdriverIO tooling (driver/browser downloaders); not shipped. Update when fixed upstream.
- `core:default` capability is broader than MuDraft uses; narrowing it is a hardening option, not a known issue.
- List queries are N+1 per album but measured fine at 5,000 albums (below). Re-measure above 20,000.

## Resolved for 1.0.0

- Windows native E2E: 10/10 in CI through tauri-driver. The embedded WebDriver plugin is macOS-only because 1.4.0 doesn't build against Tauri 2.12's WebView2 crates.
- The installer passes install, upgrade, restart and uninstall checks on a Windows runner (`scripts/windows/verify-install.ps1`).

## Fixed in this audit

- Restored archive artwork must be a real image (magic bytes). The `artwork:` protocol re-checks name and type before serving.
- Back navigation lost list search/sort/filters: the view now lives in the URL.
- Large lists render 120 cards at a time with an explicit "Show more".
- Artist links were too small to tap (now 24 px). The Stats table region is now keyboard-scrollable.

## Measurements (not guarantees)

Mac15,6, Apple M3 Pro, 18 GB, macOS 26.6.2. Fixture: 5,000 albums, 60,000 tracks, 8,000 listens (`src-tauri/tests/perf.rs`).

- **Rust, release:** open DB 0.3 ms; Listen List 50 ms; Collection 81–94 ms; Stats 159 ms; Next Up 3 ms; Artist page 65 ms; export 193 ms; RSS 104 MB.
- **App, debug `e2e` build:** launch to shell 0.5–1.0 s; first 120 cards 0.24–0.76 s; search (incl. 200 ms debounce) 0.24 s; Collection 0.5 s; Stats 0.55 s. App process ~140 MB, web content 145–199 MB.
