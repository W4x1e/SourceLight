import { act, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

vi.mock("../lib/api", () => ({
  api: { photoIptc: vi.fn(async () => null), getPhoto: vi.fn(async () => null) },
}));

import type { Photo } from "../types";
import PhotoMeta from "./PhotoMeta";

/** The capture block the Lightbox and the selection panel share. */
describe("PhotoMeta", () => {
  it("capitalises the genre as every other list of tags does", async () => {
    const photo = { id: 1, genre: "portrait", place: null, width: 6000, height: 4000 } as unknown as Photo;
    render(<PhotoMeta photo={photo} />);
    await act(async () => {});
    expect(screen.getByText("Portrait")).toBeTruthy();
    expect(screen.queryByText("portrait")).toBeNull();
  });
});
