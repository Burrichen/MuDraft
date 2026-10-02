/**
 * Starts and stops the real MuDraft binary (built with `--features e2e`) for WebdriverIO's
 * standalone mode: the app's embedded WebDriver server listens on `PORT`, and tests talk
 * to it directly. Restarting is a real process restart against the same data folder.
 */
import { type ChildProcess, spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
export const ROOT = path.resolve(here, "..");
export const PORT = 4445;
export const BINARY = path.join(
  ROOT,
  "src-tauri/target/debug",
  process.platform === "win32" ? "mudraft.exe" : "mudraft",
);

/** One working folder per run, created by the launcher and inherited by workers. */
export function workDir(): string {
  if (!process.env.MUDRAFT_E2E_WORK) {
    process.env.MUDRAFT_E2E_WORK = fs.mkdtempSync(path.join(os.tmpdir(), "mudraft-e2e-"));
  }
  return process.env.MUDRAFT_E2E_WORK;
}

export const paths = () => ({
  data: path.join(workDir(), "data"),
  exportFile: path.join(workDir(), "profile.mudraft"),
  log: path.join(workDir(), "app.log"),
});

let app: ChildProcess | null = null;

async function ready(timeoutMs: number): Promise<void> {
  const until = Date.now() + timeoutMs;
  while (Date.now() < until) {
    try {
      const res = await fetch(`http://127.0.0.1:${String(PORT)}/status`);
      if (res.ok) return;
    } catch {
      // not listening yet
    }
    await new Promise((r) => setTimeout(r, 250));
  }
  throw new Error(`MuDraft's WebDriver server did not start; see ${paths().log}`);
}

export async function startApp(): Promise<void> {
  if (!fs.existsSync(BINARY)) throw new Error(`Build first: npm run e2e:build (${BINARY})`);
  const p = paths();
  const log = fs.openSync(p.log, "a");
  app = spawn(BINARY, [], {
    stdio: ["ignore", log, log],
    env: {
      ...process.env,
      TAURI_WEBDRIVER_PORT: String(PORT),
      MUDRAFT_DATA_DIR: p.data,
      MUDRAFT_E2E_FIXTURES: path.join(here, "fixtures/musicbrainz"),
      MUDRAFT_E2E_CSV: path.join(here, "fixtures/albums.csv"),
      MUDRAFT_E2E_EXPORT: p.exportFile,
      MUDRAFT_E2E_RESTORE: p.exportFile,
    },
  });
  await ready(60_000);
}

export async function stopApp(): Promise<void> {
  const child = app;
  app = null;
  if (!child || child.exitCode !== null) return;
  const exited = new Promise((r) => child.once("exit", r));
  child.kill();
  await exited;
}
