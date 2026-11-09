import { createContext, useContext } from "react";
import type { AvailableUpdate } from "../transport/updater";

export type UpdateStatus =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "current" }
  | { kind: "available"; update: AvailableUpdate }
  | { kind: "error"; message: string };

export interface UpdateValue {
  status: UpdateStatus;
  /** Check now and, when a newer version exists, show the update prompt. */
  checkNow: () => void;
  /** Why checking is unavailable in this build, if it is. */
  unavailable: string | null;
}

export const UpdateContext = createContext<UpdateValue | null>(null);

export function useUpdates(): UpdateValue {
  const value = useContext(UpdateContext);
  if (!value) throw new Error("useUpdates must be used inside UpdateProvider");
  return value;
}
