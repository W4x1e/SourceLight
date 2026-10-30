import { renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

const renderEditedPreview = vi.fn(async (id: number) => `/thumbs/edited/${id}_tag.jpg`);
vi.mock("./api", () => ({ api: { renderEditedPreview: (id: number) => renderEditedPreview(id) } }));

import { useEditedPath } from "./useEditedPath";
import type { Photo } from "../types";

const photo = (p: Partial<Photo>) => ({ id: 1, edited: false, editedPath: null, ...p }) as Photo;

/** The grid and loupe show a photo as edited, rendering it when it has no render yet. */
describe("useEditedPath", () => {
  it("asks nothing for an unedited photo", () => {
    const { result } = renderHook(() => useEditedPath(photo({ id: 2 })));
    expect(result.current).toBeNull();
    expect(renderEditedPreview).not.toHaveBeenCalled();
  });

  it("uses the render already on the row", () => {
    const { result } = renderHook(() => useEditedPath(photo({ id: 3, edited: true, editedPath: "/r.jpg" })));
    expect(result.current).toBe("/r.jpg");
    expect(renderEditedPreview).not.toHaveBeenCalled();
  });

  it("renders an edited photo that has none, once per row", async () => {
    const p = photo({ id: 4, edited: true });
    const { result } = renderHook(() => useEditedPath(p));
    renderHook(() => useEditedPath(p));
    await waitFor(() => expect(result.current).toBe("/thumbs/edited/4_tag.jpg"));
    expect(renderEditedPreview).toHaveBeenCalledTimes(1);
  });

  it("keeps the camera's preview when the render fails", async () => {
    renderEditedPreview.mockRejectedValueOnce("no decoder");
    const { result } = renderHook(() => useEditedPath(photo({ id: 5, edited: true })));
    await waitFor(() => expect(renderEditedPreview).toHaveBeenCalledWith(5));
    expect(result.current).toBeNull();
  });
});
