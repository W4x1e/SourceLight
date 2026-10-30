import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../lib/api", () => ({ api: {} }));
vi.mock("../lib/dialogs", () => ({ toast: vi.fn(), confirm: vi.fn() }));

import { useStore } from "../store";
import { setLocale } from "../i18n";
import type { Photo } from "../types";
import PhotoMenu from "./PhotoMenu";

const photo = (id: number, over: Partial<Photo> = {}): Photo =>
  ({ id, rating: 0, flag: 0, colorLabel: 0, filename: `DSC0${id}.ARW`, path: `C:/p/DSC0${id}.ARW`, ...over }) as Photo;

/** Draws the menu and lets its placement and focus settle. */
const show = async () => {
  const r = render(<PhotoMenu />);
  await act(async () => {});
  return r;
};

/** The stars in the menu's rating row that are drawn lit. */
const lit = () =>
  screen
    .getAllByRole("menuitem", { name: /^Rate \d/ })
    .filter((b) => b.className.includes("text-accent")).length;

/** The right-click menu on a photo. */
describe("PhotoMenu", () => {
  beforeEach(() => {
    useStore.setState({
      photos: [photo(1, { rating: 4 }), photo(2, { rating: 4 }), photo(3, { rating: 2 })],
      searchResults: null,
      copiedFrom: null,
    });
  });
  afterEach(async () => {
    act(() => useStore.setState({ photoMenu: null }));
    await setLocale("en");
  });

  it("shows the rating the photos share, and the hovered one over it", async () => {
    useStore.getState().openPhotoMenu(10, 10, [1, 2], "browse");
    await show();
    expect(lit()).toBe(4);
    fireEvent.mouseEnter(screen.getByRole("menuitem", { name: "Rate 2" }));
    expect(lit()).toBe(2);
  });

  it("shows no rating when the photos disagree", async () => {
    useStore.getState().openPhotoMenu(10, 10, [1, 3], "browse");
    await show();
    expect(lit()).toBe(0);
  });

  it("offers Compare for two or more photos, and not for one", async () => {
    useStore.getState().openPhotoMenu(10, 10, [1, 2], "browse");
    const { unmount } = await show();
    expect(screen.getByRole("menuitem", { name: "Compare" })).toBeTruthy();
    unmount();
    useStore.setState({ photoMenu: null });
    useStore.getState().openPhotoMenu(10, 10, [1], "browse");
    await show();
    expect(screen.queryByRole("menuitem", { name: "Compare" })).toBeNull();
  });

  it("is in the interface's language throughout", async () => {
    await setLocale("de");
    useStore.setState({ copiedFrom: { id: 3, filename: "DSC03.ARW" } });
    useStore.getState().openPhotoMenu(10, 10, [1], "browse");
    await show();
    const words = screen.getAllByRole("menuitem").map((b) => b.textContent ?? "");
    for (const english of ["Find similar colours", "Copy image", "Play slideshow", "Edit capture time", "Copy location", "Paste"])
      expect(words.some((w) => w.includes(english))).toBe(false);
    expect(words).toContain("Ähnliche Farben finden");
    expect(words).toContain("Ort von DSC03.ARW einfügen");
  });
});
