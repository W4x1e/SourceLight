import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../lib/api", () => ({
  api: {
    aiKeywords: vi.fn(async () => []),
    iptcSelection: vi.fn(async () => null),
    listPeople: vi.fn(async () => []),
    photoPeople: vi.fn(async () => []),
    photoIptc: vi.fn(async () => null),
    getPhoto: vi.fn(async () => null),
    listPlaces: vi.fn(async () => []),
  },
  fileUrl: (p: string) => p,
}));
vi.mock("../lib/dialogs", () => ({ toast: vi.fn(), confirm: vi.fn(), prompt: vi.fn() }));

import { toast } from "../lib/dialogs";
import { useStore } from "../store";
import type { Photo } from "../types";
import SelectionPanel from "./SelectionPanel";

const photo = (id: number): Photo =>
  ({ id, filename: `DSC0${id}.ARW`, rating: 0, flag: 0, colorLabel: 0, subjects: [], placeId: null, width: 6000, height: 4000 }) as unknown as Photo;

const show = async () => {
  render(<SelectionPanel />);
  await act(async () => {});
};

const enter = (el: HTMLElement, text: string) => {
  fireEvent.change(el, { target: { value: text } });
  fireEvent.keyDown(el, { key: "Enter" });
};

/** What the selection panel says after it changes the whole selection. */
describe("SelectionPanel changes", () => {
  const assign = vi.fn(async (_name: string) => {});
  const addTag = vi.fn(async (_name: string) => {});
  beforeEach(() => {
    useStore.setState({
      photos: [photo(1), photo(2), photo(3)],
      searchResults: null,
      focusId: 1,
      assignPlaceToSelection: assign,
      addTagToSelection: addTag,
    });
  });
  afterEach(() => {
    vi.clearAllMocks();
    act(() => useStore.setState({ selected: new Set() }));
  });

  it("says how far a place reached when the panel shows only one of the photos", async () => {
    useStore.setState({ selected: new Set([1, 2, 3]) });
    await show();
    const input = screen.getByPlaceholderText("Set place for selection…") as HTMLInputElement;
    await act(async () => enter(input, "  Karoo "));
    expect(assign).toHaveBeenCalledWith("Karoo");
    expect(toast).toHaveBeenCalledWith("Put “Karoo” on 3 photos.", "success");
    expect(input.value).toBe("");
  });

  it("says nothing extra for one photo, whose panel shows the result", async () => {
    useStore.setState({ selected: new Set([1]) });
    await show();
    await act(async () => enter(screen.getByPlaceholderText("Add a keyword…"), "fox"));
    expect(addTag).toHaveBeenCalledWith("fox");
    expect(toast).not.toHaveBeenCalled();
  });

  it("says why a change failed, and keeps what was typed", async () => {
    addTag.mockRejectedValueOnce(new Error("the catalogue is read-only"));
    useStore.setState({ selected: new Set([1, 2]) });
    await show();
    const input = screen.getByPlaceholderText("Add a keyword to 2 photos…") as HTMLInputElement;
    await act(async () => enter(input, "fox"));
    expect(toast).toHaveBeenCalledWith("the catalogue is read-only", "error");
    expect(input.value).toBe("fox");
  });
});

const seven = { id: 7, filename: "DSC07.ARW", subjects: [], thumbPath: null } as unknown as Photo;
const hideButton = () => screen.queryByRole("button", { name: "Hide this panel until you select a photo" });

/** The selection panel's "Nothing selected" slot on a roomy window. */
describe("SelectionPanel with nothing selected", () => {
  const width = window.innerWidth;
  beforeEach(() => {
    window.innerWidth = 1400;
    useStore.setState({ selected: new Set(), focusId: null, photos: [seven], searchResults: null, emptyPanelHidden: false });
  });
  afterEach(() => {
    cleanup();
    window.innerWidth = width;
    useStore.setState({ selected: new Set(), emptyPanelHidden: false });
    vi.clearAllMocks();
  });

  it("closes, stays closed, and comes back on Undo", () => {
    const { rerender } = render(<SelectionPanel />);
    expect(screen.getByText("Nothing selected")).toBeTruthy();

    fireEvent.click(hideButton()!);
    expect(screen.queryByText("Nothing selected")).toBeNull();
    expect(useStore.getState().emptyPanelHidden).toBe(true);
    rerender(<SelectionPanel />);
    expect(screen.queryByText("Nothing selected")).toBeNull();

    const undo = vi.mocked(toast).mock.calls[0]![2]!;
    expect(undo.label).toBe("Undo");
    act(() => undo.run());
    expect(screen.getByText("Nothing selected")).toBeTruthy();
  });

  it("still opens for a selection once the empty slot is closed", async () => {
    useStore.setState({ emptyPanelHidden: true });
    render(<SelectionPanel />);
    expect(screen.queryByText("Nothing selected")).toBeNull();

    // Async, so the sections' backend calls settle inside act.
    await act(async () => useStore.setState({ selected: new Set([7]), focusId: 7 }));
    expect(screen.getByText("DSC07.ARW")).toBeTruthy();
    expect(hideButton()).toBeNull();

    act(() => useStore.setState({ selected: new Set(), focusId: null }));
    expect(screen.queryByText("Nothing selected")).toBeNull();
  });
});
