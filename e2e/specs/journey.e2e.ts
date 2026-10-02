/**
 * The product journey through the real native app and real SQLite:
 * CSV → match → tag → guided pick → reroll → rate tracks (explicit zero) → log listen →
 * Collection → Artist/Stats → export → restart (persistence) → restore.
 */
import fs from "node:fs";
import path from "node:path";
import { paths, startApp, stopApp, workDir } from "../app";

const work = workDir;

async function go(hash: string) {
  await browser.execute((h: string) => {
    window.location.hash = h;
  }, hash);
}

/**
 * A button or link by its visible text or aria-label. XPath's `normalize-space(.)` joins
 * text split across nodes (e.g. "Import {count} rows"), which `aria/` lookups don't.
 */
const byName = (name: string) => {
  const q = name.includes("'") ? `"${name}"` : `'${name}'`;
  return $(
    `//*[(self::button or self::a or @role='button' or @role='switch' or @role='progressbar')` +
      ` and (normalize-space(.)=${q} or @aria-label=${q})]`,
  );
};

// The embedded driver's "displayed"/"clickable" checks depend on viewport and window
// focus, so wait for the element to exist, bring it into view, then click it.
async function click(name: string) {
  await byName(name).waitForExist();
  await byName(name).scrollIntoView({ block: "center" });
  await byName(name).click();
}

async function waitForText(text: string) {
  await browser.waitUntil(async () => (await $("body").getText()).includes(text), {
    timeoutMsg: `"${text}" never appeared`,
  });
}

describe("MuDraft native journey", () => {
  it("starts on an empty, healthy profile", async () => {
    await go("#/listen-list");
    await waitForText("Storage OK");
    await waitForText("Your Listen List is empty");
  });

  it("asks before downloading artwork, and respects “Not now”", async () => {
    await go("#/listen-list/import");
    await waitForText("Download album artwork?");
    await click("Not now");
    await browser.waitUntil(async () => !(await $("dialog[open]").isExisting()));
  });

  it("imports a CSV with MusicBrainz IDs and a manual-only row", async () => {
    await go("#/listen-list/import");
    await click("Choose CSV file…");
    // Headers are recognised, so no column step. Look up the two rows with IDs.
    await click("Look up 3 rows");
    await click("Import 3 rows to Listen List");
    await waitForText("Import complete");
    await go("#/listen-list");
    await waitForText("Homogenic");
    await waitForText("Basement Demo");
    // One Listen List entry per album, however many editions were imported.
    await waitForText("2 albums");
  });

  it("picks with Guided Recommendation and rerolls without repeating", async () => {
    await go("#/next-up");
    await click("Guided Recommendation");
    await $("aria/Tag").$("button*=Listen ASAP").click();
    await waitForText("1 album match");
    await click("Find an album");
    await waitForText("Your Next Up");
    await waitForText("Why this album");
    await click("Reroll");
    await waitForText("You’ve seen all 1 matching album");
  });

  it("rates tracks with an explicit zero", async () => {
    await click("Open Album");
    await waitForText("Tracklist");
    await click("Set Rating for Hunter to 0 stars");
    await waitForText("0 of 5 stars");
  });

  it("logs the pick as listened and adds it to the Collection", async () => {
    await go("#/next-up");
    await click("Mark Listened");
    await click("Mark listened");
    await waitForText("Next Up pick completed");
    await go("#/collection");
    await waitForText("Homogenic");
    await waitForText("Last listened");
  });

  it("shows completion on the artist page and the zero rating in Stats", async () => {
    await click("Björk");
    await waitForText("Albums listened in full");
    // Both bars show X/Y and a percentage.
    await waitForText("1 / 1 · 100%");
    await go("#/stats");
    await waitForText("Best artist");
    await waitForText("0 of 5 stars from 1 rated album");
  });

  it("exports the profile to a file", async () => {
    await go("#/settings");
    await click("Export profile…");
    await waitForText("Saved “profile.mudraft”");
    expect(fs.statSync(path.join(work(), "profile.mudraft")).size).toBeGreaterThan(0);
  });

  it("keeps everything after the app restarts", async () => {
    await stopApp();
    await startApp();
    await browser.reloadSession();
    await go("#/collection");
    await waitForText("Homogenic");
    await go("#/listen-list");
    await waitForText("Basement Demo");
  });

  it("restores the exported profile after a preview, keeping a backup", async () => {
    await go("#/settings");
    await click("Restore from a profile…");
    await waitForText("Replace your profile?");
    await click("Replace my profile");
    await waitForText("Profile restored");
    // Mark this page, then wait for a fresh, loaded one before moving on.
    await browser.execute(() => {
      (window as unknown as { __beforeReload?: boolean }).__beforeReload = true;
    });
    await click("Reload MuDraft");
    await browser.waitUntil(() =>
      browser.execute(
        () =>
          !(window as unknown as { __beforeReload?: boolean }).__beforeReload &&
          document.readyState === "complete" &&
          document.body.innerText.includes("Storage OK"),
      ),
    );
    await go("#/collection");
    await waitForText("Homogenic");
    const backups = fs.readdirSync(path.join(paths().data, "backups"));
    expect(backups.some((f) => f.startsWith("before-restore-") && f.endsWith(".mudraft"))).toBe(
      true,
    );
  });
});
