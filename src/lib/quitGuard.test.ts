import { describe, expect, it } from "vitest";
import { unfinishedWork } from "./quitGuard";
import { useStore } from "../store";

const state = (patch: Partial<ReturnType<typeof useStore.getState>>) =>
  ({ ...useStore.getState(), exportJob: null, jobs: {}, ...patch }) as ReturnType<typeof useStore.getState>;

describe("quitting", () => {
  it("goes without a question when nothing would be cut off", () => {
    expect(unfinishedWork(state({}))).toBeNull();
    // Analysis resumes on the next launch: not worth holding a quit for.
    expect(
      unfinishedWork(state({ jobs: { faces: { label: "Analysing faces", current: 3, total: 70, unit: "count" } } })),
    ).toBeNull();
  });

  it("asks during an export, saying how much is left", () => {
    const q = unfinishedWork(
      state({
        exportJob: {
          request: { ids: [1, 2, 3], files: ["a", "b", "c"], directory: "", options: {} as never, rename: null, subfolder: null },
          done: 1,
          message: "",
          running: true,
          summary: null,
          results: {},
          directory: null,
        },
      }),
    );
    expect(q?.title).toBe("Quit while exporting?");
    expect(q?.message).toContain("2 of 3 photos haven't been exported yet");
    expect(q?.danger).toBe(true);
  });

  it("asks during a card import", () => {
    const q = unfinishedWork(
      state({ jobs: { import: { label: "Importing", current: 40, total: 100, unit: "count" } } }),
    );
    expect(q?.title).toBe("Quit while importing from the card?");
    expect(q?.message).toContain("40 of 100 files copied");
  });
});

/** A download starts again from the beginning, so quitting asks. */
describe("unfinishedWork while downloading", () => {
  it("asks before cutting a model download off", () => {
    const q = unfinishedWork(
      state({ jobs: { "model:clip": { label: "Downloading model", current: 400e6, total: 800e6, unit: "bytes" } } }),
    );
    expect(q?.title).toBe("Quit while downloading?");
  });

  it("doesn't ask about a download that already failed, or about indexing", () => {
    expect(
      unfinishedWork(
        state({
          jobs: {
            "model:clip": { label: "Downloading model", current: 1, total: 2, unit: "bytes", error: "broken" },
            index: { label: "Indexing", current: 1, total: 9, unit: "count" },
          },
        }),
      ),
    ).toBeNull();
  });
});
