# MuDraft

Local-first desktop music backlog, one-album picker, and collection. Tauri 2 + React + TypeScript + Vite; SQLite (bundled, no system install) behind narrow Rust commands.

Product contract: [PROJECT_SPEC.md](PROJECT_SPEC.md). Current state: [BUILD_STATUS.md](BUILD_STATUS.md).

## Toolchain (pinned)

| Tool | Version | Pinned in                             |
| ---- | ------- | ------------------------------------- |
| Node | 24.21.0 | `.nvmrc`, `package.json` `engines`    |
| npm  | 11.19.0 | `package.json` `packageManager`       |
| Rust | 1.97.1  | `rust-toolchain.toml` (auto-installs) |

## Setup — macOS

```sh
xcode-select --install                          # Apple command line tools
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
nvm install && nvm use                          # or any manager honouring .nvmrc
npm ci
npm run dev
```

## Setup — Windows (PowerShell)

Requires Microsoft C++ Build Tools ("Desktop development with C++") and WebView2 (preinstalled on Windows 10/11).

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools --override "--wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
winget install --id Rustlang.Rustup
winget install --id CoreyButler.NVMforWindows
nvm install 24.21.0; nvm use 24.21.0
npm ci
npm run dev
```

## Scripts

| Script                 | Purpose                                               |
| ---------------------- | ----------------------------------------------------- |
| `npm run dev`          | Desktop app with hot reload (development storage)     |
| `npm run build`        | Release build + installers (NSIS / DMG)               |
| `npm run build:native` | Release binary only, no installer                     |
| `npm run check`        | Type-check, lint, format check, web tests, Rust tests |
| `npm run format`       | Apply Prettier and rustfmt                            |

`npm run web:dev` is a browser preview only; it cannot reach Rust and is not desktop validation.

## Storage

Bundle ID `app.mudraft.desktop`. Data lives in the per-user local app-data dir, never the install dir:

- Production (release builds): `~/Library/Application Support/app.mudraft.desktop` / `%LOCALAPPDATA%\app.mudraft.desktop`
- Development (debug builds): sibling `app.mudraft.desktop.dev`
- Tests/automation: debug builds honour an absolute `MUDRAFT_DATA_DIR`; release builds ignore it. Rust tests use temp dirs.

## Metadata (MusicBrainz)

Searches and lookups run natively in Rust (`src-tauri/src/metadata/`); the renderer has no network access. Requests share one queue at ≤1/second, send `MuDraft/<version> ( https://github.com/Burrichen/MuDraft )`, time out after 15 s, retry at most 3 times with backoff (honouring `Retry-After`), and are cached in the app database so recent results work offline. Broad-genre vocabulary and mappings live in `src-tauri/src/metadata/genres.rs`.

A live smoke test is excluded from normal runs: `cargo test --manifest-path src-tauri/Cargo.toml live_musicbrainz -- --ignored`.

## CSV import

Listen List → Import CSV. Required columns: Album, Artist. Optional: Year, Edition, Tags (separated by `;`), MusicBrainz Release Group ID, MusicBrainz Release ID. UTF-8 (BOM optional) or UTF-16 with BOM; quoted fields may contain commas and line breaks. Artist names are never split. Rows are staged and previewed; nothing changes until Import. Format details: `src-tauri/src/csv_import/mod.rs`.
