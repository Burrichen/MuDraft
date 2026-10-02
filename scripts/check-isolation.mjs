// Fails if the test-only WebDriver plugin is reachable from a normal build for any target.
// Also checks that it IS present with `--features e2e`, so a renamed crate can't pass vacuously.
import { spawnSync } from "node:child_process";

const PLUGIN = "tauri-plugin-wdio-webdriver";
const tree = (...extra) =>
  spawnSync("cargo", ["tree", "--target", "all", "-e", "normal", "-i", PLUGIN, ...extra], {
    cwd: new URL("../src-tauri/", import.meta.url),
    encoding: "utf8",
  });

const normal = tree();
if (normal.status === 0) {
  console.error(`${PLUGIN} is in the release dependency graph:\n${normal.stdout}`);
  process.exit(1);
}
if (!/did not match any packages/.test(normal.stderr)) {
  console.error(`Unexpected cargo tree failure:\n${normal.stderr}`);
  process.exit(1);
}
const e2e = tree("--features", "e2e");
if (e2e.status !== 0) {
  console.error(`Expected ${PLUGIN} with --features e2e:\n${e2e.stderr}`);
  process.exit(1);
}
console.log(`${PLUGIN}: absent from normal builds, present only with --features e2e.`);
