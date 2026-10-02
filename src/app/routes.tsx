import { Navigate, type RouteObject } from "react-router";
import { AlbumPage } from "../features/album/AlbumPage";
import { ArtistPage } from "../features/artist/ArtistPage";
import { CollectionPage } from "../features/collection/CollectionPage";
import { CsvImportPage } from "../features/import/CsvImportPage";
import { ListenListPage } from "../features/listen-list/ListenListPage";
import { NextUpPage } from "../features/next-up/NextUpPage";
import { SettingsPage } from "../features/settings/SettingsPage";
import { StatsPage } from "../features/stats/StatsPage";
import { Shell } from "./Shell";

export const routes: RouteObject[] = [
  {
    path: "/",
    element: <Shell />,
    children: [
      { index: true, element: <Navigate to="/listen-list" replace /> },
      { path: "listen-list", element: <ListenListPage /> },
      { path: "listen-list/import", element: <CsvImportPage /> },
      { path: "next-up", element: <NextUpPage /> },
      { path: "collection", element: <CollectionPage /> },
      { path: "stats", element: <StatsPage /> },
      { path: "settings", element: <SettingsPage /> },
      { path: "albums/:albumId", element: <AlbumPage /> },
      { path: "artists/:artistId", element: <ArtistPage /> },
      { path: "*", element: <Navigate to="/listen-list" replace /> },
    ],
  },
];
