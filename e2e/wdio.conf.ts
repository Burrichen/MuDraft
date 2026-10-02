/**
 * Native end-to-end tests: WebdriverIO (standalone mode) drives the real MuDraft binary,
 * through the server `--features e2e` embeds on macOS or `tauri-driver` on Windows/Linux
 * (see ./app.ts). Each run uses a fresh temporary data folder; MusicBrainz is served from
 * recorded fixtures and native file dialogs are replaced by fixed paths. None of this is
 * compiled into release builds.
 *
 * Build: `npm run e2e:build`   Run: `npm run e2e`
 */
import path from "node:path";
import { fileURLToPath } from "node:url";
import { PORT, capabilities, startApp, stopApp, workDir } from "./app";

const here = path.dirname(fileURLToPath(import.meta.url));
// Created in the launcher; workers inherit it, so every session shares one profile.
workDir();

export const config: WebdriverIO.Config = {
  runner: "local",
  hostname: "127.0.0.1",
  port: PORT,
  specs: [path.join(here, "specs/**/*.e2e.ts")],
  maxInstances: 1,
  capabilities: [capabilities],
  framework: "mocha",
  reporters: ["spec"],
  mochaOpts: { ui: "bdd", timeout: 120_000 },
  waitforTimeout: 15_000,
  logLevel: "warn",
  // In the worker, so the spec can restart the same app process.
  beforeSession: startApp,
  afterSession: stopApp,
};
