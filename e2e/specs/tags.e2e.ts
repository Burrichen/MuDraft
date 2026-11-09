/**
 * Tagging an album from its page through the real native app (macOS WKWebView or
 * Windows WebView2) and real SQLite: each tick in the picker applies at once and stays.
 */
async function go(hash: string) {
  await browser.execute((h: string) => {
    window.location.hash = h;
  }, hash);
}

const byName = (name: string) => {
  const q = name.includes("'") ? `"${name}"` : `'${name}'`;
  return $(
    `//*[(self::button or self::a or @role='button')` +
      ` and (normalize-space(.)=${q} or @aria-label=${q})]`,
  );
};

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

/** The picker's checkbox for `tag`. */
const tagBox = (tag: string) =>
  $(`//label[contains(@class,'tag-picker-option')][.//*[normalize-space(.)='${tag}']]//input`);

const albumLink = () => $('a[href^="#/albums/"]');

describe("Tagging from the album page", () => {
  // Runs alone or after the journey (same data folder): import only if nothing is listed.
  it("has an album to tag", async () => {
    await go("#/listen-list");
    await browser.waitUntil(async () => {
      const text = await $("body").getText();
      return text.includes("Your Listen List is empty") || (await albumLink().isExisting());
    });
    if (!(await albumLink().isExisting())) {
      await go("#/listen-list/import");
      if (await byName("Not now").isExisting()) await click("Not now");
      await click("Choose CSV file…");
      await click("Look up 3 rows");
      await click("Import 3 rows to Listen List");
      await waitForText("Import complete");
      await go("#/listen-list");
      await albumLink().waitForExist();
    }
  });

  it("applies and removes a tag with one tick each, and the tick stays", async () => {
    await albumLink().click();
    const edit = $('button[aria-label^="Edit tags for"]');
    await edit.waitForExist();
    await edit.click();
    await tagBox("Listen ASAP").waitForExist();
    const before = await tagBox("Listen ASAP").isSelected();

    await tagBox("Listen ASAP").click();
    await browser.waitUntil(async () => (await tagBox("Listen ASAP").isSelected()) !== before, {
      timeoutMsg: "the Listen ASAP tick didn't stick",
    });
    await tagBox("Listen ASAP").click();
    await browser.waitUntil(async () => (await tagBox("Listen ASAP").isSelected()) === before, {
      timeoutMsg: "the Listen ASAP untick didn't stick",
    });
    await click("Done");
  });
});
