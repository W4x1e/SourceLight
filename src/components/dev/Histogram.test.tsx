import { act, render, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../../lib/api", () => ({ api: { developHistogram: vi.fn() } }));

import type { EditStack } from "../../bindings";
import { api } from "../../lib/api";
import Histogram from "./Histogram";

const m = vi.mocked(api);
const bins = { r: [1, 2, 3], g: [1, 2, 3], b: [1, 2, 3], shadowClip: 0, highlightClip: 0 };
const drag = (exposure: number) => ({ exposure }) as EditStack;

describe("Histogram", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    m.developHistogram.mockResolvedValue(bins as never);
  });

  it("follows a drag, one request at a time, then the committed edit", async () => {
    let finish: () => void = () => undefined;
    m.developHistogram.mockImplementation(
      () => new Promise((r) => (finish = () => r(bins as never))),
    );
    const { rerender } = render(<Histogram frame={1} draft={null} />);
    await waitFor(() => expect(m.developHistogram).toHaveBeenCalledWith(null));

    // A drag while the first is still being made: only the newest follows.
    rerender(<Histogram frame={1} draft={drag(0.5)} />);
    rerender(<Histogram frame={1} draft={drag(1)} />);
    expect(m.developHistogram).toHaveBeenCalledTimes(1);
    await act(async () => finish());
    await waitFor(() => expect(m.developHistogram).toHaveBeenCalledTimes(2));
    expect(m.developHistogram).toHaveBeenLastCalledWith(drag(1));

    // Released: the committed edit, by frame.
    await act(async () => finish());
    rerender(<Histogram frame={2} draft={null} />);
    await waitFor(() => expect(m.developHistogram).toHaveBeenCalledTimes(3));
    expect(m.developHistogram).toHaveBeenLastCalledWith(null);
  });

  it("keeps going after a failed request", async () => {
    m.developHistogram.mockRejectedValueOnce("no photo is open");
    const { rerender } = render(<Histogram frame={1} draft={null} />);
    await waitFor(() => expect(m.developHistogram).toHaveBeenCalledTimes(1));
    rerender(<Histogram frame={2} draft={null} />);
    await waitFor(() => expect(m.developHistogram).toHaveBeenCalledTimes(2));
  });
});
