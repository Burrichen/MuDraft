# MuDraft — Project Spec

MuDraft (MusicDraft) is a local-first desktop music backlog, one-album picker, and personal collection/review app. Sections 1–8 are the product contract; later steps refine implementation details without changing it.

## 1. Platform and design

- Tauri 2, React, TypeScript, Vite. SQLite accessed only through narrow Rust commands.
- No hosted backend, accounts, telemetry, subscription, or runtime Node server.
- Sleek dark navy/blue UI inspired by FDraft: collapsible sidebar, artwork cards, restrained motion.
- Navigation: Listen List, Next Up, Collection, Stats, Settings; nested Album and Artist pages.

## 2. Data

- Persistent artists, canonical albums, user-distinguished editions, edition tracklists, recording identities where known, global album tags, listen events, reviews, ratings, and metadata provenance.
- Standard and Deluxe editions may coexist when the user chooses; ordinary pressings must not create accidental duplicates.
- Artist credits support collaborations.

## 3. Acquisition

- Custom CSV is the only bulk-import format. Manual creation and metadata search also work.
- MusicBrainz supplies metadata; Cover Art Archive supplies optional cached artwork.
- Match review and editable metadata are required.
- Normalize source genres into 2–3 useful broad genres where supported. Never invent missing facts.

## 4. Next Up

- Exactly one current album.
- **Completely Random:** uniform selection from the Listen List.
- **Weighted Random:** uniform selection ONLY from albums with the built-in `Listen ASAP` tag. Do not change this to probability weighting or silently fall back.
- **Guided Recommendation:** one scrollable page with Decade, Genre, Tag. OR within a category, AND between categories. `Agnostic` is the default and is exclusive within its category. Offer represented decades, plus a current-year option when represented.
- Rerolls never repeat a result until that session's pool is exhausted.

## 5. Listening and ratings

- Collection allows direct manual additions and repeated listens without duplicate album entries.
- A full listen normally removes Listen List membership; this is configurable.
- Store real local listening dates; distinguish known dates from previously-listened history without dates.
- No Diary tab; Collection supports Recently Listened sorting.
- Album and track ratings: 0–5 in 0.5 steps. Zero is a rating, not unrated.
- Effective album rating: explicit album rating if set; otherwise the mean of rated tracks rounded to the nearest 0.5 (unrated tracks ignored).
- Keep written reviews and favourite tracks. Rating and listening are separate actions.

## 6. Artists and stats

- Artist pages: ranked discography; album and track completion bars with counts and percentages. Explain catalogue coverage and how editions are counted.
- Stats: equally prominent Listen List and Collection sections, including best-rated release year, genre, and artists.
- NO minimum sample sizes, Bayesian adjustments, or suppressed small-sample results. Show counts without penalizing them.

## 7. Portability and release

- Profile export/import preserves all meaningful user data, relationships, overrides, settings, and optionally artwork.
- Core use works offline.
- Ship Windows x64 NSIS setup `.exe` and universal macOS `.dmg`.
- Native platform CI builds, honest signing status, and recoverable storage are required.

## 8. Exclusions

External critic/community scores; RYM/Pitchfork integrations; Diary screen; streaming/playback integrations; playlists; social features; challenges; events; streaks; cloud sync; automatic updater.

## 9. Module map (planned)

- `src/` — React UI
  - `app/` shell, routing, sidebar; `theme/` design tokens
  - `features/` listen-list, next-up, collection, album, artist, stats, settings, import
  - `services/` typed, validated domain calls; `transport/` Tauri bridge (only IPC entry point)
- `src-tauri/src/` — Rust core
  - `commands/` narrow, validated Tauri commands
  - `db/` SQLite connection, migrations, transactions, backups
  - `domain/` ratings, dates, IDs, picker, stats, genre normalization (pure, unit-tested)
  - `library/` transactional persistence contracts (catalogue, personal data, selection, settings, provenance)
  - `import/` CSV parsing and match review; `metadata/` MusicBrainz + Cover Art Archive client and artwork cache
  - `portability/` profile export/import
- `.github/workflows/` — native Windows/macOS build CI

## 10. Planned sequence

1. Documentation (this step).
2. Scaffold Tauri 2 + React + TS + Vite; pinned toolchain, lint/test/CI skeleton.
3. SQLite schema, migrations, storage safety/recovery.
4. Artists, albums, editions, tracks, tags: Rust commands + tests.
5. App shell, theme, navigation, Listen List UI.
6. Manual creation; MusicBrainz search, match review, artwork cache.
7. Custom CSV import with match review.
8. Next Up modes and session reroll pool.
9. Listening events, Collection, ratings, reviews, favourite tracks.
10. Album and Artist pages with completion/coverage.
11. Stats.
12. Settings and profile export/import.
13. Release packaging, signing status, CI artifacts.
