# BUILD_STATUS

## Completed steps

- **Foundation → Artist pages**: Tauri 2.12/React 19/TS 6/Vite 8 (Node 24.21.0, npm 11.19.0, Rust 1.97.1); schema v1–v10; shell; MusicBrainz; CSV import; lists; tags; album pages; ratings; listening; Next Up; artist catalogue and pages.
- **Stats selectors (latest, no charts)**: `library/stats.rs` and `stats_overview`, plus `src/services/stats.ts`.
  - Listen List: per-album counts, artists, years/decades (unknown apart), overlapping genres, tags, Listen ASAP, waiting times, known/unknown runtime.
  - Collection: listen counts by kind, date status and month/year; canonical scores (`ratings::album_score`, shared with Artist pages); best year (original), decade, genre and artist with sample counts and all tied winners; favourites; per-session deduplicated runtime with coverage.
  - Next Up: shown/skipped/completed/pending by stored source.

## Checks run (macOS 26.6.2)

- `npm run check`: 162 web tests; clippy clean; 184 Rust tests (hand-calculated fixture, empty data, ties).
- No schema change; desktop launch not repeated.

## Unverified

- Live MusicBrainz in the desktop app; Windows build and CI.

## Blockers

None.

## Next step

Stats charts on the Stats page.
