import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { likedTracks, setFavourite } from "../../services/ratings";
import { renderApp } from "../../test/renderApp";

vi.mock("../../services/ratings", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/ratings")>()),
  likedTracks: vi.fn(),
  setFavourite: vi.fn(() => Promise.resolve()),
}));
vi.mock("../../services/health", () => ({
  checkHealth: vi.fn(() => new Promise(() => undefined)),
}));

const ALBUM = "0190f5c3-0000-7000-8000-000000000001";

describe("Liked", () => {
  it("lists liked tracks on their own page and can unlike one", async () => {
    vi.mocked(likedTracks).mockResolvedValue([
      {
        trackId: "t1",
        title: "Airbag",
        lengthMs: 284_000,
        albumId: ALBUM,
        albumTitle: "OK Computer",
        credit: "Radiohead",
        editionName: "Standard",
      },
    ]);
    renderApp("/liked");
    const table = await screen.findByRole("region", { name: "Liked tracks" });
    expect(within(table).getByText("Airbag")).toBeInTheDocument();
    expect(within(table).getByText("Radiohead")).toBeInTheDocument();
    expect(within(table).getByRole("link", { name: "OK Computer" })).toHaveAttribute(
      "href",
      `/albums/${ALBUM}`,
    );
    expect(screen.getByText("1 liked track.")).toBeInTheDocument();
    await userEvent.click(within(table).getByRole("button", { name: "Like: Airbag" }));
    expect(setFavourite).toHaveBeenCalledWith("t1", false);
  });

  it("explains how to like tracks when there are none", async () => {
    vi.mocked(likedTracks).mockResolvedValue([]);
    renderApp("/liked");
    expect(await screen.findByText("No liked tracks yet")).toBeInTheDocument();
  });
});
