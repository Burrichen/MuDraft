import { createContext, useContext } from "react";
import type { UiPreferences, UiPreferencesPatch } from "../services/preferences";

export interface PreferencesValue {
  prefs: UiPreferences;
  /** Applies immediately; persisted in order. Failures revert and set `problem`. */
  update: (patch: UiPreferencesPatch) => void;
  /** Why preferences are not being saved, if they are not. */
  problem: string | null;
  dismissProblem: () => void;
}

export const PreferencesContext = createContext<PreferencesValue | null>(null);

export function usePreferences(): PreferencesValue {
  const value = useContext(PreferencesContext);
  if (!value) throw new Error("usePreferences must be used inside PreferencesProvider");
  return value;
}
