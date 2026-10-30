import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../lib/api", () => ({
  api: {
    exportBatch: vi.fn(),
    showFolder: vi.fn(() => Promise.resolve()),
    cancelExport: vi.fn(),
    phoneOutboxStart: vi.fn(),
    phoneOutboxQueue: vi.fn(),
  },
}));

import { api } from "../lib/api";
import type { BatchRequest } from "../lib/batchExport";
import { useStore } from "../store";
import ExportQueuePanel from "./ExportQueuePanel";

const exportBatch = vi.mocked(api.exportBatch);
const showFolder = vi.mocked(api.showFolder);

const request = (patch: Partial<BatchRequest> = {}): BatchRequest => ({
  ids: [1, 2, 3],
  files: ["a.arw", "b.arw", "c.arw"],
  directory: "D:/out",
  options: {} as never,
  rename: null,
  subfolder: "Web",
  ...patch,
});

/** Batch export's queue tells what happened to each file. */
describe("ExportQueuePanel", () => {
  beforeEach(() => useStore.setState({ exportJob: null }));
  afterEach(() => vi.clearAllMocks());

  it("marks a failed file as failed, with its reason, not with a tick", () => {
    useStore.getState().startExportJob(request());
    useStore.getState().noteExportItem(1, null);
    useStore.getState().noteExportItem(2, "the file is locked");
    useStore.getState().updateExportProgress(2, 3, "c.arw");
    render(<ExportQueuePanel />);
    expect(screen.getByText("the file is locked")).toBeTruthy();
  });

  it("keeps every failure from the outcome, and offers them again into the same folder", async () => {
    useStore.getState().startExportJob(request());
    useStore.getState().finishExportJob("Exported 1, 2 failed", {
      written: 1,
      failed: 2,
      errors: ["b.arw: locked", "c.arw: no denoise model"],
      cancelled: false,
      failures: [
        { photoId: 2, error: "locked" },
        { photoId: 3, error: "no denoise model" },
      ],
      directory: "D:/out/Web",
    });
    render(<ExportQueuePanel />);
    expect(screen.getByText("locked")).toBeTruthy();
    expect(screen.getByText("no denoise model")).toBeTruthy();

    exportBatch.mockResolvedValue({
      written: 2,
      failed: 0,
      errors: [],
      cancelled: false,
      failures: [],
      directory: "D:/out/Web",
    });
    fireEvent.click(screen.getByText("Retry failed"));
    await vi.waitFor(() => expect(exportBatch).toHaveBeenCalled());
    // Only the failed two, into the folder the batch wrote to, with no second
    // subfolder inside it.
    expect(exportBatch).toHaveBeenCalledWith([2, 3], "D:/out/Web", {}, null, null);

    fireEvent.click(screen.getByRole("button", { name: /^Show in / }));
    expect(showFolder).toHaveBeenCalledWith("D:/out/Web");
  });

  it("offers no retry for numbered files, which would be numbered again from 001", () => {
    useStore.getState().startExportJob(request({ rename: "{seq}-{name}" }));
    useStore.getState().finishExportJob("Exported 2, 1 failed", {
      written: 2,
      failed: 1,
      errors: ["b.arw: locked"],
      cancelled: false,
      failures: [{ photoId: 2, error: "locked" }],
      directory: "D:/out/Web",
    });
    render(<ExportQueuePanel />);
    expect(screen.queryByText("Retry failed")).toBeNull();
    expect(screen.getByRole("button", { name: /^Show in / })).toBeTruthy();
  });

  // Its folder is the outbox's: retrying into it would never reach the phone.
  it("offers neither retry nor the folder for Send to phone", () => {
    useStore.getState().startExportJob(request({ toPhone: true, subfolder: null }));
    useStore.getState().finishExportJob("Exported 2, 1 failed", {
      written: 2,
      failed: 1,
      errors: ["b.arw: locked"],
      cancelled: false,
      failures: [{ photoId: 2, error: "locked" }],
      directory: "C:/data/outbox/1791019815000",
    });
    render(<ExportQueuePanel />);
    expect(screen.getByText("locked")).toBeTruthy();
    expect(screen.queryByText("Retry failed")).toBeNull();
    expect(screen.queryByRole("button", { name: /^Show in / })).toBeNull();
  });

  it("sends to the phone through a folder of the outbox, then queues it", async () => {
    const { sendToPhone } = await import("../lib/batchExport");
    vi.mocked(api.phoneOutboxStart).mockResolvedValue("C:/data/outbox/1");
    vi.mocked(api.phoneOutboxQueue).mockResolvedValue(2);
    exportBatch.mockResolvedValue({
      written: 2,
      failed: 0,
      errors: [],
      cancelled: false,
      failures: [],
      directory: "C:/data/outbox/1",
    });
    await sendToPhone({ ids: [1, 2], files: ["a.arw", "b.arw"], options: {} as never, rename: null });
    expect(exportBatch).toHaveBeenCalledWith([1, 2], "C:/data/outbox/1", {}, null, null);
    expect(api.phoneOutboxQueue).toHaveBeenCalledWith("C:/data/outbox/1");
    expect(useStore.getState().exportJob?.request.toPhone).toBe(true);
    expect(useStore.getState().exportJob?.running).toBe(false);
  });

  it("refuses a second batch while one runs", async () => {
    const { runBatch } = await import("../lib/batchExport");
    useStore.getState().startExportJob(request());
    await runBatch(request({ ids: [9], files: ["z.arw"] }));
    expect(exportBatch).not.toHaveBeenCalled();
    expect(useStore.getState().exportJob?.request.ids).toEqual([1, 2, 3]);
  });
});
