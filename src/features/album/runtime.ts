import { formatDuration, type AlbumDetail } from "../../services/album";

/** Total of known lengths; says so when some are unknown rather than counting them as zero. */
export function runtimeText(runtime: AlbumDetail["runtime"]): string {
  if (runtime.trackCount === 0) return "No tracklist";
  const tracks = `${String(runtime.trackCount)} track${runtime.trackCount === 1 ? "" : "s"}`;
  if (runtime.knownMs === 0) return `${tracks} · length unknown`;
  if (runtime.unknownTracks > 0) {
    const n = runtime.unknownTracks;
    return `${tracks} · at least ${formatDuration(runtime.knownMs)} (${String(n)} track${n === 1 ? "" : "s"} of unknown length)`;
  }
  return `${tracks} · ${formatDuration(runtime.knownMs)}`;
}
