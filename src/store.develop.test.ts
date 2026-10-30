import { describe, expect, it, vi } from "vitest";

// Each open resolves when the test says so, so their order is the test's.
const pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: unknown) => void }>();
const closeDevelop = vi.fn(() => Promise.resolve());
vi.mock("./lib/api", () => ({
  api: {
    openDevelop: (photoId: number) =>
      new Promise((resolve, reject) => pending.set(photoId, { resolve, reject })),
    closeDevelop: () => closeDevelop(),
    saveDevelopAsCopy: () => Promise.resolve({ photoId: 21, frame: 1 }),
  },
}));
vi.mock("./lib/dialogs", () => ({ toast: vi.fn(), confirm: vi.fn() }));

import { toast } from "./lib/dialogs";
import { developShown, useStore } from "./store";

const info = (photoId: number) => ({ photoId, frame: 1 }) as never;

/**
 * Stepping fast asks for several photos before the first is open:
 * the one asked for last is what Develop shows, and an answer for one passed
 * over, or the backend saying it was overtaken, changes nothing.
 */
describe("openDevelop", () => {
  it("shows the photo asked for last, whatever order the answers come in", async () => {
    const first = useStore.getState().openDevelop(1);
    const second = useStore.getState().openDevelop(2);
    expect(useStore.getState().developWanted).toBe(2);

    pending.get(2)!.resolve(info(2));
    await second;
    pending.get(1)!.resolve(info(1));
    await first;
    expect(useStore.getState().develop?.photoId).toBe(2);
  });

  it("says nothing when an open was overtaken", async () => {
    const first = useStore.getState().openDevelop(3);
    const second = useStore.getState().openDevelop(4);
    pending.get(3)!.reject("superseded by a later open");
    await first;
    pending.get(4)!.resolve(info(4));
    await second;
    expect(useStore.getState().developError).toBeNull();
    expect(useStore.getState().develop?.photoId).toBe(4);
  });
});

/** A photo that won't open says why, and doesn't throw away the one open.
 *  */
describe("a failed open", () => {
  it("keeps Develop on the open photo and says why in its banner", async () => {
    const shown = useStore.getState().openDevelop(10);
    pending.get(10)!.resolve(info(10));
    await shown;
    const failing = useStore.getState().openDevelop(11);
    pending.get(11)!.reject("the file isn't there — if it's on a drive that isn't connected, connect it");
    await failing;
    expect(useStore.getState().develop?.photoId).toBe(10);
    expect(useStore.getState().developError).toContain("isn't there");
    expect(useStore.getState().developWanted).toBe(11);
  });

  it("says why with a toast when nothing was open", async () => {
    useStore.setState({ develop: null });
    vi.mocked(toast).mockClear();
    const failing = useStore.getState().openDevelop(12);
    pending.get(12)!.reject("LibRaw could not read the file");
    await failing;
    expect(useStore.getState().develop).toBeNull();
    expect(vi.mocked(toast)).toHaveBeenCalledWith(expect.stringContaining("LibRaw"), "error");
    // And Develop, up while it was opening, goes again.
    expect(developShown(useStore.getState())).toBe(false);
  });
});

/** Develop is on screen from the moment it is asked for: a raw takes a second
 *  or more to open, and nothing used to show in the meantime. */
describe("the first open", () => {
  it("shows Develop before the photo has opened", async () => {
    useStore.setState({ develop: null, developWanted: null });
    expect(developShown(useStore.getState())).toBe(false);
    const opening = useStore.getState().openDevelop(30);
    expect(useStore.getState().develop).toBeNull();
    expect(developShown(useStore.getState())).toBe(true);
    pending.get(30)!.resolve(info(30));
    await opening;
    expect(developShown(useStore.getState())).toBe(true);
    expect(useStore.getState().develop?.photoId).toBe(30);
  });

  it("closes the core's photo again when Develop was left while it opened", async () => {
    useStore.setState({ develop: null, developWanted: null });
    closeDevelop.mockClear();
    const opening = useStore.getState().openDevelop(31);
    // Escape, or Library, before the raw had decoded.
    useStore.setState({ developWanted: null });
    pending.get(31)!.resolve(info(31));
    await opening;
    expect(useStore.getState().develop).toBeNull();
    expect(closeDevelop).toHaveBeenCalledTimes(1);
  });

  it("leaves the core alone when another photo was asked for instead", async () => {
    useStore.setState({ develop: null, developWanted: null });
    closeDevelop.mockClear();
    const first = useStore.getState().openDevelop(32);
    const second = useStore.getState().openDevelop(33);
    pending.get(32)!.resolve(info(32));
    await first;
    expect(closeDevelop).not.toHaveBeenCalled();
    pending.get(33)!.resolve(info(33));
    await second;
    expect(useStore.getState().develop?.photoId).toBe(33);
  });
});

/** Saving a copy moves Develop onto it: nothing is left "opening". */
describe("saveDevelopAsCopy", () => {
  it("leaves no open pending for the original", async () => {
    const shown = useStore.getState().openDevelop(20);
    pending.get(20)!.resolve(info(20));
    await shown;
    useStore.setState({ reload: () => Promise.resolve() } as never);
    await useStore.getState().saveDevelopAsCopy(null);
    expect(useStore.getState().develop?.photoId).toBe(21);
    expect(useStore.getState().developWanted).toBe(21);
  });
});
