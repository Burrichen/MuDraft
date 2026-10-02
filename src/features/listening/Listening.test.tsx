import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { AlbumDetail } from "../../services/album";
import {
  confirmCoverage,
  correctListen,
  deleteListen,
  describeListening,
  listeningSettings,
  localToday,
  logListen,
  restoreListen,
  setListeningSettings,
  undoLog,
  type ListenView,
  type Logged,
} from "../../services/listening";
import { ListenDialog } from "./ListenDialog";
import { ListeningPanel } from "./ListeningPanel";
import { ListeningSettings } from "./ListeningSettings";

vi.mock("../../services/listening", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/listening")>()),
  logListen: vi.fn(),
  undoLog: vi.fn(() => Promise.resolve()),
  correctListen: vi.fn(),
  deleteListen: vi.fn(() => Promise.resolve()),
  restoreListen: vi.fn(() => Promise.resolve()),
  confirmCoverage: vi.fn(() => Promise.resolve(2)),
  listeningSettings: vi.fn(),
  setListeningSettings: vi.fn(() => Promise.resolve()),
}));

const EDITION = "0190f5c3-0000-7000-8000-0000000000e1";

const LOGGED: Logged = {
  listenId: "l1",
  isFull: true,
  coverage: "tracks",
  coveredTracks: 2,
  addedToCollection: true,
  removedFromListenList: true,
  completedNextUp: false,
  undo: {
    listenId: "l1",
    priorHistoryId: null,
    restoreListenList: ["a1", EDITION],
    createdCollectionFor: EDITION,
    completedAttempt: null,
  },
};

function listen(over: Partial<ListenView> = {}): ListenView {
  return {
    id: "l1",
    editionId: EDITION,
    editionName: "Standard",
    listenedOn: "2024-03-02",
    kind: "first",
    isFull: true,
    coverage: "tracks",
    coveredTracks: 2,
    editionTracks: 2,
    fromNextUp: false,
    ...over,
  };
}

function detail(listens: ListenView[]): AlbumDetail {
  return {
    albumId: "a1",
    title: "Blue",
    editionId: EDITION,
    editions: [{ id: EDITION, name: "Standard", trackCount: 2 }],
    discs: [
      {
        number: 1,
        tracks: [
          { id: "t1", position: 1, title: "All I Want", listened: false },
          { id: "t2", position: 2, title: "My Old Man", listened: false },
        ],
      },
    ],
    listening: {
      summary: {
        lastListened: listens[0]?.listenedOn ?? null,
        listenCount: listens.length,
        undatedListens: 0,
        earlierUndated: false,
      },
      listens,
    },
  } as unknown as AlbumDetail;
}

describe("describeListening", () => {
  it("is transparent about unknown dates", () => {
    const base = { lastListened: null, listenCount: 0, undatedListens: 0, earlierUndated: false };
    expect(describeListening(base)).toBe("Not listened yet");
    expect(describeListening({ ...base, listenCount: 1, undatedListens: 1 })).toBe(
      "Listened · date unknown · 1 listen",
    );
    expect(describeListening({ ...base, earlierUndated: true })).toMatch(/dates unknown/);
    expect(describeListening({ ...base, lastListened: "2024-03-02", listenCount: 3 })).toMatch(
      /^Last listened .*2024 · 3 listens$/,
    );
  });

  it("uses the local calendar date", () => {
    expect(localToday(new Date(2026, 0, 5, 23, 59))).toBe("2026-01-05");
  });
});

describe("ListenDialog", () => {
  it("backdates, and a double click records one request", async () => {
    let resolve: (l: Logged) => void = () => undefined;
    vi.mocked(logListen).mockReturnValue(
      new Promise((r) => {
        resolve = r;
      }),
    );
    const onLogged = vi.fn();
    const user = userEvent.setup();
    render(
      <ListenDialog albumTitle="Blue" editionId={EDITION} onClose={vi.fn()} onLogged={onLogged} />,
    );
    const date = screen.getByLabelText("Date listened");
    await user.clear(date);
    await user.type(date, "2019-06-01");
    await user.click(screen.getByRole("radio", { name: "Re-listen" }));
    const save = screen.getByRole("button", { name: "Mark listened" });
    await user.dblClick(save);
    expect(logListen).toHaveBeenCalledTimes(1);
    expect(vi.mocked(logListen).mock.calls[0]?.[1]).toEqual({
      editionId: EDITION,
      listenedOn: "2019-06-01",
      kind: "relisten",
      earlierUndated: false,
      trackIds: null,
      attemptId: null,
    });
    resolve(LOGGED);
    await waitFor(() => {
      expect(onLogged).toHaveBeenCalledWith(LOGGED);
    });
  });

  it("records an unknown date instead of inventing one, and retries with the same key", async () => {
    vi.mocked(logListen).mockRejectedValueOnce(new Error("disk busy")).mockResolvedValue(LOGGED);
    const user = userEvent.setup();
    render(
      <ListenDialog albumTitle="Blue" editionId={EDITION} onClose={vi.fn()} onLogged={vi.fn()} />,
    );
    await user.click(screen.getByLabelText("I don’t know the date"));
    await user.click(screen.getByLabelText("I also listened before, but don’t know when"));
    await user.click(screen.getByRole("button", { name: "Mark listened" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("disk busy");
    await user.click(screen.getByRole("button", { name: "Mark listened" }));
    const calls = vi.mocked(logListen).mock.calls;
    expect(calls).toHaveLength(2);
    expect(calls[0]?.[0]).toBe(calls[1]?.[0]);
    expect(calls[0]?.[1]).toMatchObject({ listenedOn: null, earlierUndated: true });
  });

  it("rejects future dates and requires tracks in track mode", async () => {
    const user = userEvent.setup();
    render(
      <ListenDialog
        albumTitle="Blue"
        editionId={EDITION}
        tracks={[
          { id: "t1", label: "1 All I Want", listened: false },
          { id: "t2", label: "2 My Old Man", listened: true },
        ]}
        onClose={vi.fn()}
        onLogged={vi.fn()}
      />,
    );
    const save = screen.getByRole("button", { name: "Log tracks" });
    await user.click(save);
    expect(logListen).not.toHaveBeenCalled();
    await user.click(screen.getByLabelText(/All I Want/));
    const date = screen.getByLabelText("Date listened");
    await user.clear(date);
    await user.type(date, "2999-01-01");
    await user.click(save);
    expect(logListen).not.toHaveBeenCalled();
    vi.mocked(logListen).mockResolvedValue({ ...LOGGED, isFull: false });
    await user.clear(date);
    await user.type(date, "2020-01-01");
    await user.click(save);
    expect(vi.mocked(logListen).mock.calls[0]?.[1]).toMatchObject({ trackIds: ["t1"] });
  });
});

describe("ListeningPanel", () => {
  it("logs with an undo that reverts memberships", async () => {
    vi.mocked(logListen).mockResolvedValue(LOGGED);
    const user = userEvent.setup();
    render(<ListeningPanel detail={detail([])} />);
    expect(screen.getByText("Not listened yet")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Mark album listened" }));
    await user.click(screen.getByRole("button", { name: "Mark listened" }));
    expect(await screen.findByText(/Added to your Collection/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Undo" }));
    expect(undoLog).toHaveBeenCalledWith(LOGGED.undo);
  });

  it("corrects and deletes a listen with undo", async () => {
    vi.mocked(correctListen)
      .mockResolvedValueOnce({ listenedOn: "2024-03-02", kind: "first" })
      .mockResolvedValue({ listenedOn: null, kind: "relisten" });
    const user = userEvent.setup();
    render(<ListeningPanel detail={detail([listen()])} />);
    await user.click(screen.getByRole("button", { name: "Edit listen from 2024-03-02" }));
    await user.click(screen.getByLabelText("I don’t know the date"));
    await user.click(screen.getByRole("radio", { name: "Re-listen" }));
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(correctListen).toHaveBeenCalledWith("l1", { listenedOn: null, kind: "relisten" });
    await user.click(await screen.findByRole("button", { name: "Undo" }));
    expect(correctListen).toHaveBeenLastCalledWith("l1", {
      listenedOn: "2024-03-02",
      kind: "first",
    });

    await user.click(screen.getByRole("button", { name: "Delete listen from 2024-03-02" }));
    expect(deleteListen).toHaveBeenCalledWith("l1");
    expect(await screen.findByText("Listen deleted.")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Undo" }));
    expect(restoreListen).toHaveBeenCalledWith("l1");
  });

  it("asks before counting tracks for a listen logged without a tracklist", async () => {
    const user = userEvent.setup();
    render(<ListeningPanel detail={detail([listen({ coverage: "unknown", coveredTracks: 0 })])} />);
    expect(screen.getByText(/tracks unknown when logged/)).toBeInTheDocument();
    expect(confirmCoverage).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Count tracks as listened…" }));
    await user.click(screen.getByRole("button", { name: "Count all 2 tracks" }));
    expect(confirmCoverage).toHaveBeenCalledWith("l1");
  });
});

describe("ListeningSettings", () => {
  it("loads and saves the Listen List removal setting", async () => {
    vi.mocked(listeningSettings).mockResolvedValue({ fullListenRemovesFromListenList: true });
    const user = userEvent.setup();
    render(<ListeningSettings />);
    const toggle = screen.getByRole("switch", { name: /Remove albums from your Listen List/ });
    await waitFor(() => {
      expect(toggle).toBeChecked();
    });
    await user.click(toggle);
    expect(setListeningSettings).toHaveBeenCalledWith(false);
    expect(toggle).not.toBeChecked();
  });
});
