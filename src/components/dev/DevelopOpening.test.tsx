import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../../lib/api", () => ({
  api: { closeDevelop: vi.fn(() => Promise.resolve()), openDevelop: vi.fn(() => new Promise(() => {})) },
  fileUrl: () => "",
}));
vi.mock("../../lib/dialogs", () => ({ toast: vi.fn(), confirm: vi.fn() }));

import { useStore } from "../../store";
import type { Photo } from "../../types";
import DevelopOpening from "./DevelopOpening";

const photo = (id: number): Photo =>
  ({ id, rating: 0, flag: 0, colorLabel: 0, filename: `DSC0${id}.ARW`, path: `C:/p/DSC0${id}.ARW` }) as Photo;

/** Develop while its first photo is still opening: up at once, saying so. */
describe("DevelopOpening", () => {
  beforeEach(() => {
    useStore.setState({ photos: [photo(1), photo(2)], searchResults: null, develop: null, developWanted: 1 });
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("names the photo at once, and says it is opening once that takes a moment", () => {
    vi.useFakeTimers();
    render(<DevelopOpening />);
    expect(screen.getByText("DSC01.ARW")).toBeTruthy();
    // Not for an open that is over in a blink.
    expect(screen.queryByRole("status")).toBeNull();
    act(() => void vi.advanceTimersByTime(300));
    expect(screen.getByRole("status").textContent).toContain("Opening DSC01.ARW");
  });

  it("takes the keyboard from the grid that opened it", () => {
    const cell = document.createElement("button");
    document.body.append(cell);
    cell.focus();
    const { container, unmount } = render(<DevelopOpening />);
    expect(document.activeElement).toBe(container.querySelector(".develop-shell"));
    unmount();
    expect(document.activeElement).toBe(cell);
    cell.remove();
  });

  it("goes back to the library on Escape", () => {
    render(<DevelopOpening />);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(useStore.getState().developWanted).toBeNull();
  });

  it("steps on with the arrows, from the photo asked for", () => {
    render(<DevelopOpening />);
    fireEvent.keyDown(window, { key: "ArrowRight" });
    expect(useStore.getState().developWanted).toBe(2);
  });
});
