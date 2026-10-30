import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../lib/api", () => ({ api: {} }));
vi.mock("../lib/dialogs", () => ({ toast: vi.fn(), confirm: vi.fn() }));

import { useStore } from "../store";
import { TOUR } from "../lib/tour";
import { t } from "../i18n";
import GuidedTour from "./GuidedTour";

/** Lets the step's requestAnimationFrame lookups run. */
const frames = async (n = 25) => {
  for (let i = 0; i < n; i++) await act(() => new Promise((r) => requestAnimationFrame(() => r(null))));
};

const title = () => screen.getByRole("dialog").querySelector("h2")?.textContent;

/** The first-run guided tour. */
describe("GuidedTour", () => {
  beforeEach(() => {
    useStore.setState({ mode: "places", tourSeen: false, tour: null });
    useStore.getState().startTour();
  });
  afterEach(() => {
    useStore.setState({ tour: null, mode: "browse" });
  });

  it("starts on the welcome card and walks forward and back", () => {
    render(<GuidedTour />);
    expect(title()).toBe(t(TOUR[0]!.title));
    fireEvent.click(screen.getByRole("button", { name: "Show me" }));
    expect(title()).toBe(t(TOUR[1]!.title));
    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(title()).toBe(t(TOUR[0]!.title));
  });

  it("switches to each step's workspace", async () => {
    useStore.getState().tourGo(TOUR.findIndex((s) => s.id === "cull"));
    render(<GuidedTour />);
    await frames(1);
    expect(useStore.getState().mode).toBe("cull");
  });

  it("skipping marks it seen and returns to where the user was", () => {
    render(<GuidedTour />);
    fireEvent.click(screen.getByRole("button", { name: "Show me" }));
    fireEvent.click(screen.getByRole("button", { name: "Skip tour" }));
    const s = useStore.getState();
    expect(s.tour).toBeNull();
    expect(s.tourSeen).toBe(true);
    expect(s.mode).toBe("places");
  });

  it("Escape ends it; arrow keys step it", () => {
    render(<GuidedTour />);
    fireEvent.keyDown(window, { key: "ArrowRight" });
    expect(useStore.getState().tour?.step).toBe(1);
    fireEvent.keyDown(window, { key: "ArrowLeft" });
    expect(useStore.getState().tour?.step).toBe(0);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(useStore.getState().tour).toBeNull();
    expect(useStore.getState().tourSeen).toBe(true);
  });

  it("keeps keys from reaching the workspace underneath", () => {
    const underneath = vi.fn();
    window.addEventListener("keydown", underneath);
    render(<GuidedTour />);
    fireEvent.keyDown(window, { key: "x" });
    fireEvent.keyDown(window, { key: "1" });
    expect(underneath).not.toHaveBeenCalled();
    window.removeEventListener("keydown", underneath);
  });

  it("ends with Done on the last step", () => {
    useStore.getState().tourGo(TOUR.length - 1);
    render(<GuidedTour />);
    expect(screen.queryByRole("button", { name: "Skip tour" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(useStore.getState().tourSeen).toBe(true);
  });

  it("spotlights a target that is there, and centres the card when it is not", async () => {
    const settings = document.createElement("button");
    settings.dataset.tour = "rail-settings";
    settings.getBoundingClientRect = () =>
      ({ left: 4, top: 700, width: 76, height: 56, right: 80, bottom: 756, x: 4, y: 700 }) as DOMRect;
    document.body.appendChild(settings);
    useStore.getState().tourGo(TOUR.length - 1);
    const { getByTestId } = render(<GuidedTour />);
    await frames(1);
    const hole = getByTestId("guided-tour").firstElementChild as HTMLElement;
    expect(hole.style.left).toBe("-2px");
    expect(hole.style.width).toBe("88px");

    settings.remove();
    act(() => useStore.getState().tourGo(TOUR.length - 2));
    await frames();
    expect(hole.style.width).toBe("0px");
  });
});
