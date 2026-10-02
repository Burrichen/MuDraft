import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createTag, deleteTag, listTags, updateTag, type TagInfo } from "../../services/library";
import { renderApp } from "../../test/renderApp";
import { NativeError } from "../../transport/native";

vi.mock("../../services/library", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/library")>()),
  listTags: vi.fn(),
  createTag: vi.fn(),
  updateTag: vi.fn(),
  deleteTag: vi.fn(),
}));
vi.mock("../../services/preferences", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/preferences")>()),
  updatePreferences: vi.fn((patch: object) =>
    Promise.resolve({
      sidebarCollapsed: false,
      lastRoute: "/listen-list",
      albumLayout: "grid",
      ...patch,
    }),
  ),
}));
vi.mock("../../services/album", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/album")>()),
  artworkPreference: vi.fn(() =>
    Promise.resolve({ allowed: false, cache: { files: 0, bytes: 0 } }),
  ),
}));
vi.mock("../../services/listening", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/listening")>()),
  listeningSettings: vi.fn(() => Promise.resolve({ fullListenRemovesFromListenList: true })),
}));
vi.mock("../../services/health", () => ({
  checkHealth: vi.fn(() => new Promise(() => undefined)),
}));

const TAGS: TagInfo[] = [
  {
    id: "00000000-0000-7000-8000-000000000001",
    name: "Listen ASAP",
    color: "#f59e0b",
    builtin: true,
    albumCount: 4,
  },
  {
    id: "0190f5c3-0000-7000-8000-00000000000b",
    name: "Road trip",
    color: "#1d4ed8",
    builtin: false,
    albumCount: 3,
  },
  {
    id: "0190f5c3-0000-7000-8000-00000000000c",
    name: "Unused",
    color: "#34d399",
    builtin: false,
    albumCount: 0,
  },
];

beforeEach(() => {
  vi.mocked(listTags).mockResolvedValue(TAGS);
  vi.mocked(updateTag).mockImplementation((id, change) =>
    Promise.resolve({ ...(TAGS.find((t) => t.id === id) as TagInfo), ...change }),
  );
});

describe("tag management", () => {
  it("lists every tag including unused ones, with usage", async () => {
    renderApp("/settings");
    const list = await screen.findByRole("list", { name: "Your tags" });
    expect(within(list).getAllByRole("listitem")).toHaveLength(3);
    expect(within(list).getByText("Not used yet")).toBeInTheDocument();
    expect(within(list).getByText("On 3 albums")).toBeInTheDocument();
  });

  it("protects the built-in tag's name and existence but not its colour", async () => {
    renderApp("/settings");
    const name = await screen.findByLabelText("Name of Listen ASAP");
    expect(name).toHaveAttribute("readonly");
    expect(name).toHaveAccessibleDescription(/Its name is fixed/);
    const del = screen.getByRole("button", { name: "Delete tag Listen ASAP" });
    expect(del).toHaveAccessibleDescription("Built-in tags can’t be deleted.");
    const hex = screen.getByLabelText("Listen ASAP colour (hex)");
    await userEvent.clear(hex);
    await userEvent.type(hex, "#10b981");
    expect(updateTag).toHaveBeenLastCalledWith(TAGS[0]?.id, { color: "#10b981" });
  });

  it("creates tags with a colour and reports duplicates", async () => {
    vi.mocked(createTag)
      .mockResolvedValueOnce({
        id: "x",
        name: "Headphones",
        color: "#60a5fa",
        builtin: false,
        albumCount: 0,
      })
      .mockRejectedValueOnce(new NativeError("conflict", "a tag named “Road trip” already exists"));
    renderApp("/settings");
    const input = await screen.findByLabelText("New tag");
    await userEvent.type(input, "Headphones");
    await userEvent.click(screen.getByRole("button", { name: "Create tag" }));
    expect(createTag).toHaveBeenCalledWith("Headphones", "#60a5fa");
    expect(await screen.findByText("Created “Headphones”.")).toBeInTheDocument();
    await userEvent.type(input, "road trip");
    await userEvent.click(screen.getByRole("button", { name: "Create tag" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("already exists");
  });

  it("renames on blur and deletes after showing the impact", async () => {
    vi.mocked(deleteTag).mockResolvedValue(3);
    renderApp("/settings");
    const name = await screen.findByLabelText("Name of Road trip");
    await userEvent.clear(name);
    await userEvent.type(name, "Long drives{Enter}");
    expect(updateTag).toHaveBeenCalledWith(TAGS[1]?.id, { name: "Long drives" });

    await userEvent.click(screen.getByRole("button", { name: "Delete tag Road trip" }));
    const dialog = screen.getByRole("dialog", { name: "Delete “Road trip”?" });
    expect(dialog).toHaveTextContent(
      "It will be removed from 3 albums. The albums themselves stay.",
    );
    await userEvent.click(within(dialog).getByRole("button", { name: "Delete tag" }));
    expect(deleteTag).toHaveBeenCalledWith(TAGS[1]?.id);
    await waitFor(() => {
      expect(
        screen.getByText("Deleted “Road trip” and removed it from 3 albums."),
      ).toBeInTheDocument();
    });
  });
});
