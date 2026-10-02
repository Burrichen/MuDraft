import { useCallback, useMemo, useRef, useState, type ReactNode } from "react";
import {
  updatePreferences,
  type UiPreferences,
  type UiPreferencesPatch,
} from "../services/preferences";
import { PreferencesContext, type PreferencesValue } from "./preferencesContext";

interface Props {
  initial: UiPreferences;
  /** Set when preferences could not be loaded; nothing is written in that case. */
  loadError?: string | null;
  children: ReactNode;
}

export function PreferencesProvider({ initial, loadError = null, children }: Props) {
  const [prefs, setPrefs] = useState(initial);
  const [problem, setProblem] = useState<string | null>(
    loadError
      ? `Interface settings could not be loaded (${loadError}). Changes won't be saved.`
      : null,
  );
  const confirmed = useRef(initial);
  // Latest optimistic state, kept in step with every setPrefs below.
  const current = useRef(initial);
  const queue = useRef<Promise<void>>(Promise.resolve());
  const canSave = loadError === null;

  const update = useCallback(
    (patch: UiPreferencesPatch) => {
      const keys = Object.keys(patch) as (keyof UiPreferences)[];
      if (keys.every((k) => current.current[k] === patch[k])) return; // no-op: nothing to save
      current.current = { ...current.current, ...patch };
      setPrefs(current.current);
      if (!canSave) return;
      queue.current = queue.current.then(async () => {
        try {
          confirmed.current = await updatePreferences(patch);
        } catch (err) {
          const revert = Object.fromEntries(keys.map((k) => [k, confirmed.current[k]]));
          current.current = { ...current.current, ...revert };
          setPrefs(current.current);
          setProblem(
            `Couldn't save interface settings: ${err instanceof Error ? err.message : String(err)}`,
          );
        }
      });
    },
    [canSave],
  );

  const dismissProblem = useCallback(() => {
    setProblem(null);
  }, []);

  const value = useMemo<PreferencesValue>(
    () => ({ prefs, update, problem, dismissProblem }),
    [prefs, update, problem, dismissProblem],
  );
  return <PreferencesContext value={value}>{children}</PreferencesContext>;
}
