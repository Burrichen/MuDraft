// Release metadata gate. Every version source must agree, the release notes must exist,
// and (when building from a tag) the tag must name that version. Prints `version=<v>`
// for GitHub Actions outputs.
import fs from "node:fs";

const read = (p) => fs.readFileSync(new URL(`../../${p}`, import.meta.url), "utf8");

const sources = {
  "package.json": JSON.parse(read("package.json")).version,
  "package-lock.json": JSON.parse(read("package-lock.json")).packages[""].version,
  "src-tauri/tauri.conf.json": JSON.parse(read("src-tauri/tauri.conf.json")).version,
  "src-tauri/Cargo.toml": /^\[package\][^[]*?^version = "([^"]+)"/ms.exec(
    read("src-tauri/Cargo.toml"),
  )?.[1],
  "src-tauri/Cargo.lock": /name = "mudraft"\nversion = "([^"]+)"/.exec(
    read("src-tauri/Cargo.lock"),
  )?.[1],
};

const versions = new Set(Object.values(sources));
if (versions.size !== 1 || versions.has(undefined)) {
  console.error("Version sources disagree:");
  for (const [file, v] of Object.entries(sources)) console.error(`  ${file}: ${v ?? "(missing)"}`);
  process.exit(1);
}
const [version] = versions;
if (!/^\d+\.\d+\.\d+$/.test(version)) {
  console.error(`"${version}" is not a plain MAJOR.MINOR.PATCH version`);
  process.exit(1);
}
const notes = `docs/release-notes/v${version}.md`;
if (!fs.existsSync(new URL(`../../${notes}`, import.meta.url))) {
  console.error(`Missing release notes: ${notes}`);
  process.exit(1);
}
if (process.env.GITHUB_REF_TYPE === "tag" && process.env.GITHUB_REF_NAME !== `v${version}`) {
  console.error(`Tag ${process.env.GITHUB_REF_NAME} does not match version ${version}`);
  process.exit(1);
}
console.log(`version=${version}`);
