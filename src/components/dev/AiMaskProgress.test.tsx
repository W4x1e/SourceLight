import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useStore } from "../../store";
import AiMaskProgress, { aiStageText, isAiMaskStage } from "./AiMaskProgress";

/**
 * The line an AI mask run shows in the masks panel. It has no percentage to
 * give, so what it must get right is the words for each step, staying out of
 * the way of a quick run, and handing over to a download's own bar.
 */
describe("AiMaskProgress", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    useStore.setState({ jobs: {} });
  });
  afterEach(() => vi.useRealTimers());

  it("names each step the backend reports, and only those", () => {
    expect(isAiMaskStage("loading")).toBe(true);
    expect(isAiMaskStage("nonsense")).toBe(false);
    expect(aiStageText("waiting", false)).toMatch(/graphics card/);
    expect(aiStageText("loading", false)).toMatch(/Loading the AI model/);
    expect(aiStageText("subject", false)).toMatch(/Finding the subject/);
    expect(aiStageText("encoding", true)).toMatch(/Reading the photo/);
    // Before any step: a refining click is an update, anything else starting.
    expect(aiStageText(null, true)).toMatch(/Updating the selection/);
    expect(aiStageText(null, false)).toMatch(/Getting ready/);
  });

  it("stays hidden for a quick run, then shows the step and the seconds", () => {
    const { rerender } = render(<AiMaskProgress busy stage={null} refining={false} />);
    expect(screen.queryByRole("progressbar")).toBeNull();
    act(() => void vi.advanceTimersByTime(350));
    expect(screen.getByRole("progressbar")).toBeTruthy();
    expect(screen.getByText(/Getting ready/)).toBeTruthy();
    rerender(<AiMaskProgress busy stage="subject" refining={false} />);
    act(() => void vi.advanceTimersByTime(2000));
    expect(screen.getByText(/Finding the subject/)).toBeTruthy();
    expect(screen.getByText("2 s")).toBeTruthy();
    rerender(<AiMaskProgress busy={false} stage="subject" refining={false} />);
    expect(screen.queryByRole("progressbar")).toBeNull();
  });

  it("gives way to a model download, which has its own bar", () => {
    useStore.setState({
      jobs: { "model:subject": { label: "Subject model", current: 1, total: 10 } as never },
    });
    render(<AiMaskProgress busy stage={null} refining={false} />);
    act(() => void vi.advanceTimersByTime(500));
    expect(screen.queryByRole("progressbar")).toBeNull();
  });
});
