import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("./api", () => ({ api: { exportSpaceCheck: vi.fn() } }));
vi.mock("./dialogs", () => ({ confirm: vi.fn() }));

import { api } from "./api";
import { confirm } from "./dialogs";
import { roomToExport } from "./exportSpace";

const check = vi.mocked(api.exportSpaceCheck);
const ask = vi.mocked(confirm);

const options = {
  format: "jpeg" as const,
  quality: 92,
  maxEdge: null,
  sharpen: "standard" as const,
  watermark: null,
  textWatermark: null,
};

/** The pre-flight free-space check before a batch export. */
describe("roomToExport", () => {
  afterEach(() => {
    vi.clearAllMocks();
    vi.restoreAllMocks();
  });

  it("goes ahead without asking when the export fits", async () => {
    check.mockResolvedValue(null);
    expect(await roomToExport([1, 2], "D:/out", options)).toBe(true);
    expect(ask).not.toHaveBeenCalled();
  });

  it("asks, naming both sizes, when it may not fit — and does what the user says", async () => {
    check.mockResolvedValue({ needed: "3.2 GB", free: "1.1 GB" });
    ask.mockResolvedValue(false);
    expect(await roomToExport([1, 2], "D:/out", options)).toBe(false);
    const message = ask.mock.calls[0]![0].message;
    expect(message).toContain("3.2 GB");
    expect(message).toContain("1.1 GB");

    ask.mockResolvedValue(true);
    expect(await roomToExport([1, 2], "D:/out", options)).toBe(true);
  });

  it("never blocks an export because the check itself failed", async () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    check.mockRejectedValue(new Error("no such drive"));
    expect(await roomToExport([1], "Z:/gone", options)).toBe(true);
    expect(ask).not.toHaveBeenCalled();
  });
});
