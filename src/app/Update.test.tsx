import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { VersionSettings } from "../features/settings/VersionSettings";
import { checkHealth } from "../services/health";
import { checkForUpdate, type AvailableUpdate } from "../transport/updater";
import { UpdateProvider } from "./UpdateProvider";

vi.mock("../transport/updater", () => ({ checkForUpdate: vi.fn() }));
vi.mock("../services/health", () => ({ checkHealth: vi.fn() }));

const NOTES = `MuDraft 1.5.0: faster picks.

- **Next Up** remembers more.
- Settings shows the version.`;

function available(install: AvailableUpdate["install"]): AvailableUpdate {
  return { version: "1.5.0", currentVersion: "1.4.0", notes: NOTES, install };
}

beforeEach(() => {
  vi.mocked(checkHealth).mockResolvedValue({
    appVersion: "1.4.0",
    profile: "production",
    dataDir: "/data",
    schemaVersion: 14,
    sqliteVersion: "3.53.2",
  });
});

describe("update prompt", () => {
  it("offers a newer version at startup with its patch notes, and installs on request", async () => {
    const user = userEvent.setup();
    let fail = true;
    const install = vi.fn<AvailableUpdate["install"]>((onProgress) => {
      onProgress(0.5);
      return fail ? Promise.reject(new Error("signature mismatch")) : new Promise(() => undefined);
    });
    vi.mocked(checkForUpdate).mockResolvedValue(available(install));
    render(
      <UpdateProvider autoCheck unavailable={null}>
        <p>app</p>
      </UpdateProvider>,
    );
    const dialog = await screen.findByRole("dialog", { name: "MuDraft 1.5.0 is available" });
    expect(dialog).toHaveTextContent("You have MuDraft 1.4.0");
    expect(
      within(dialog)
        .getAllByRole("listitem")
        .map((li) => li.textContent),
    ).toEqual(["Next Up remembers more.", "Settings shows the version."]);
    expect(dialog).not.toHaveTextContent("**");

    await user.click(within(dialog).getByRole("button", { name: "Update and Restart" }));
    expect(install).toHaveBeenCalledTimes(1);
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "Couldn’t update: signature mismatch. Nothing was changed.",
    );

    fail = false;
    await user.click(within(dialog).getByRole("button", { name: "Try Again" }));
    expect(await within(dialog).findByText("Downloading… 50%")).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Later" })).toHaveAttribute(
      "aria-disabled",
      "true",
    );
  });

  it("stays quiet when the startup check fails or there is nothing new", async () => {
    vi.mocked(checkForUpdate).mockRejectedValueOnce(new Error("offline"));
    const { unmount } = render(
      <UpdateProvider autoCheck unavailable={null}>
        <p>app</p>
      </UpdateProvider>,
    );
    await waitFor(() => {
      expect(checkForUpdate).toHaveBeenCalledTimes(1);
    });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    unmount();
  });

  it("lets Settings check on demand and reports the result", async () => {
    const user = userEvent.setup();
    render(
      <UpdateProvider autoCheck={false} unavailable={null}>
        <VersionSettings />
      </UpdateProvider>,
    );
    expect(await screen.findByText("MuDraft 1.4.0")).toBeInTheDocument();
    expect(checkForUpdate).not.toHaveBeenCalled();

    vi.mocked(checkForUpdate).mockResolvedValueOnce(null);
    await user.click(screen.getByRole("button", { name: "Check for Updates" }));
    expect(await screen.findByText("You have the latest version.")).toBeInTheDocument();

    vi.mocked(checkForUpdate).mockRejectedValueOnce(new Error("GitHub is unreachable"));
    await user.click(screen.getByRole("button", { name: "Check for Updates" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Couldn’t check for updates: GitHub is unreachable",
    );

    vi.mocked(checkForUpdate).mockResolvedValueOnce(available(vi.fn()));
    await user.click(screen.getByRole("button", { name: "Check for Updates" }));
    expect(
      await screen.findByRole("dialog", { name: "MuDraft 1.5.0 is available" }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Later" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.getByText("MuDraft 1.5.0 is available.")).toBeInTheDocument();
  });
});
