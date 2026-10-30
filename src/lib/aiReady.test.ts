import { afterEach, describe, expect, it, vi } from "vitest";

const refreshAi = vi.fn(async () => {});
vi.mock("./api", () => ({ api: { ensureAiReady: vi.fn(), setAiSettings: vi.fn(), turnAiOn: vi.fn() } }));
vi.mock("./dialogs", () => ({ toast: vi.fn(), confirm: vi.fn() }));
vi.mock("../store", () => ({ useStore: { getState: () => ({ refreshAi }) } }));

import { api } from "./api";
import { confirm, toast } from "./dialogs";
import { ensureAi } from "./aiReady";

const ready = vi.mocked(api.ensureAiReady);
const blocked = (why: string) =>
  ({ ready: false, blocked: why, needs: [], askFirst: false, metered: false }) as unknown as Awaited<
    ReturnType<typeof api.ensureAiReady>
  >;

/** A block with no way round it from here says why. */
describe("ensureAi when blocked", () => {
  afterEach(() => vi.clearAllMocks());

  it("says why", async () => {
    ready.mockResolvedValue(blocked("On-device AI is switched off"));
    expect(await ensureAi("tagging")).toBe(false);
    expect(toast).toHaveBeenCalledWith("On-device AI is switched off", "error");
  });
});

/** With AI switched off, every feature is refused, and offers to turn it back
 *  on rather than showing a red error. */
describe("ensureAi with AI off", () => {
  afterEach(() => vi.clearAllMocks());
  const off = { ...blocked("On-device AI is switched off"), provider: "off" } as unknown as Awaited<
    ReturnType<typeof api.ensureAiReady>
  >;
  const isReady = { ...blocked(""), ready: true, blocked: null } as unknown as Awaited<
    ReturnType<typeof api.ensureAiReady>
  >;

  it("turns it on when asked, and carries on", async () => {
    ready.mockResolvedValueOnce(off).mockResolvedValueOnce(isReady);
    vi.mocked(confirm).mockResolvedValueOnce(true);
    expect(await ensureAi("selectSubject")).toBe(true);
    // As it was before, not plain built-in tags.
    expect(api.turnAiOn).toHaveBeenCalledOnce();
    expect(toast).not.toHaveBeenCalled();
  });

  it("leaves it off on No, quietly", async () => {
    ready.mockResolvedValueOnce(off);
    vi.mocked(confirm).mockResolvedValueOnce(false);
    expect(await ensureAi("search")).toBe(false);
    expect(api.turnAiOn).not.toHaveBeenCalled();
    expect(toast).not.toHaveBeenCalled();
  });

  it("shows a failure's words without 'Error:' in front", async () => {
    ready.mockRejectedValueOnce(new Error("The download of clip didn't arrive intact — try again"));
    expect(await ensureAi("search")).toBe(false);
    expect(toast).toHaveBeenCalledWith("The download of clip didn't arrive intact — try again", "error");
  });
});
