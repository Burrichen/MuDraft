import type { Logged } from "../../services/listening";

export function loggedMessage(l: Logged): string {
  const parts = [
    l.isFull
      ? "Listen logged."
      : `Logged ${String(l.coveredTracks)} track${l.coveredTracks === 1 ? "" : "s"}.`,
  ];
  if (l.addedToCollection) parts.push("Added to your Collection.");
  if (l.removedFromListenList) parts.push("Removed from your Listen List.");
  if (l.completedNextUp) parts.push("Next Up pick completed.");
  return parts.join(" ");
}
