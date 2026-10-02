import { useLocation, useNavigate } from "react-router";
import { labelForPath } from "./navigation";

interface BackState {
  from?: unknown;
}

/**
 * Reliable back navigation for nested pages. When the page was reached from inside the
 * app, go back in history (preserving scroll and state). When it was opened directly —
 * e.g. restored after restart — there is no in-app history, so go to `fallback` instead
 * of leaving the app or doing nothing.
 */
export function useBack(fallback: string): { label: string; goBack: () => void } {
  const navigate = useNavigate();
  const location = useLocation();
  const raw = (location.state as BackState | null)?.from;
  const from = typeof raw === "string" && location.key !== "default" ? raw : null;
  return {
    label: `Back to ${labelForPath(from ?? fallback)}`,
    goBack: () => {
      if (from) void navigate(-1);
      else void navigate(fallback, { replace: true });
    },
  };
}
