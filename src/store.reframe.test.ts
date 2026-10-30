import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./lib/api", () => ({
  api: {
    setDevelopStack: vi.fn(async (_label: string, stack: unknown) => ({ stack })),
    reframeDevelop: vi.fn(async (_label: string, stack: unknown) => ({ stack })),
  },
}));
vi.mock("./lib/dialogs", () => ({ toast: vi.fn(), confirm: vi.fn() }));

import { api } from "./lib/api";
import { useStore } from "./store";

const geometry = { crop: { x: 0, y: 0, width: 1, height: 1 }, straighten: 0, quarterTurns: 0, flip: false };
const spot = { targetX: 0.5, targetY: 0.5, sourceX: 0.6, sourceY: 0.5, radius: 0.02, feather: 0.5, heal: true };

/** A change of framing carries masks and spots along; any other edit
 *  is stored as it is. */
describe("adjust and framing", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useStore.setState({ develop: { photoId: 1, stack: { geometry, masks: [], spots: [spot] } } as never });
  });

  it("reframes when only the framing changed", async () => {
    const tighter = { ...geometry, crop: { x: 0.1, y: 0.1, width: 0.8, height: 0.8 } };
    await useStore.getState().adjust("Crop", { geometry: tighter });
    expect(api.reframeDevelop).toHaveBeenCalledOnce();
    expect(api.setDevelopStack).not.toHaveBeenCalled();
  });

  it("stores as it is when the spots changed too, or nothing local exists", async () => {
    await useStore.getState().adjust("Paste", { geometry: { ...geometry, straighten: 2 }, spots: [] });
    expect(api.setDevelopStack).toHaveBeenCalledOnce();

    useStore.setState({ develop: { photoId: 1, stack: { geometry, masks: [], spots: [] } } as never });
    await useStore.getState().adjust("Crop", { geometry: { ...geometry, flip: true } });
    expect(api.reframeDevelop).not.toHaveBeenCalled();
  });

  it("stores as it is when the framing didn't change", async () => {
    await useStore.getState().adjust("Exposure", { exposure: 0.5 } as never);
    expect(api.reframeDevelop).not.toHaveBeenCalled();
    expect(api.setDevelopStack).toHaveBeenCalledOnce();
  });
});
