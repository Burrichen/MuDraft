/**
 * Component-test fixtures only. ESLint forbids importing src/test/** from app code, so
 * these can never reach a build or a user's profile.
 */
import type { AlbumSummary } from "../../components/AlbumCard";

export const FIXTURE_ALBUMS: readonly AlbumSummary[] = [
  {
    albumId: "0190f5c3-0000-7000-8000-000000000001",
    title: "Fixture Album One",
    artists: [{ id: "0190f5c3-0000-7000-8000-0000000000a1", name: "Fixture Artist" }],
    year: 1997,
    rating: 7,
  },
  {
    albumId: "0190f5c3-0000-7000-8000-000000000002",
    title:
      "An Extremely Long Fixture Album Title That Keeps Going Well Past Any Reasonable Card Width (Deluxe Anniversary Remaster)",
    artists: [
      {
        id: "0190f5c3-0000-7000-8000-0000000000a2",
        name: "First Collaborating Fixture Artist With A Long Name",
      },
      { id: "0190f5c3-0000-7000-8000-0000000000a3", name: "Second Fixture Artist" },
    ],
    year: null,
    editionName: "Deluxe",
    artworkUrl: "data:image/gif;base64,R0lGODlhAQABAAAAACw=",
    rating: 0,
  },
  {
    albumId: "0190f5c3-0000-7000-8000-000000000003",
    title: "Unrated Fixture",
    artists: [],
    year: 2024,
    rating: null,
  },
];
