import { callNative, NativeError } from "../transport/native";

export type StorageProfile = "production" | "development";

export interface HealthReport {
  appVersion: string;
  profile: StorageProfile;
  dataDir: string;
  schemaVersion: number;
  sqliteVersion: string;
}

function isHealthReport(value: unknown): value is HealthReport {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.appVersion === "string" &&
    (v.profile === "production" || v.profile === "development") &&
    typeof v.dataDir === "string" &&
    typeof v.schemaVersion === "number" &&
    Number.isInteger(v.schemaVersion) &&
    typeof v.sqliteVersion === "string"
  );
}

export async function checkHealth(): Promise<HealthReport> {
  const raw = await callNative("health_check");
  if (!isHealthReport(raw)) {
    throw new NativeError("invalid_response", "Health check returned an unexpected shape");
  }
  return raw;
}
