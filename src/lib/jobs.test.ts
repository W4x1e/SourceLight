import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("./api", () => ({ api: { cancelJob: vi.fn(), cancelCaptioning: vi.fn() } }));

import { api } from "./api";
import { canCancel, cancelJob, isCancelled } from "./jobs";

/** Cancelling long jobs. */
describe("jobs", () => {
  afterEach(() => vi.clearAllMocks());

  it("tells a cancel from a failure, however it arrives", () => {
    expect(isCancelled(new Error("Cancelled"))).toBe(true); // a rejected command
    expect(isCancelled("Cancelled")).toBe(true); // a progress event's `error`
    expect(isCancelled(new Error("downloading clip: Cancelled"))).toBe(true); // with context
    expect(isCancelled(new Error("connection reset"))).toBe(false);
    expect(isCancelled(new Error("Cancelled halfway, then retried and failed"))).toBe(false);
    expect(isCancelled(null)).toBe(false);
  });

  it("offers Cancel on exactly the jobs that can stop", () => {
    for (const id of ["model:clip", "vision-runtime", "vlm-model", "index", "faces", "people", "caption", "denoise"]) {
      expect(canCancel(id), id).toBe(true);
    }
    for (const id of ["quality", "model:", "export", "scan"]) {
      expect(canCancel(id), id).toBe(false);
    }
  });

  it("routes captioning to its own command and everything else to the registry", async () => {
    await cancelJob("caption");
    expect(api.cancelCaptioning).toHaveBeenCalledOnce();
    expect(api.cancelJob).not.toHaveBeenCalled();

    await cancelJob("model:clip");
    expect(api.cancelJob).toHaveBeenCalledWith("model:clip");
  });
});
