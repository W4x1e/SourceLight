import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { DialogHost } from "./dialogs";
import { askToSaveAsCopy } from "./saveAsCopy";

/** Save as copy does nothing at all when its question is cancelled. */
describe("Save as copy", () => {
  it("makes no copy when the name is cancelled", async () => {
    render(<DialogHost />);
    const save = vi.fn(async () => {});
    const asking = askToSaveAsCopy(save);
    (await screen.findByRole("button", { name: "Cancel" })).click();
    await asking;
    expect(save).not.toHaveBeenCalled();
  });

  it("makes an unnamed copy on OK with the box empty, and a named one with a name", async () => {
    render(<DialogHost />);
    const save = vi.fn(async (_: string | null) => {});
    let asking = askToSaveAsCopy(save);
    const ok = () => screen.getAllByRole("button").find((b) => b.textContent !== "Cancel")!;
    await screen.findByRole("textbox");
    ok().click();
    await asking;
    expect(save).toHaveBeenLastCalledWith(null);

    asking = askToSaveAsCopy(save);
    fireEvent.change(await screen.findByRole("textbox"), { target: { value: "Square crop" } });
    ok().click();
    await asking;
    expect(save).toHaveBeenLastCalledWith("Square crop");
  });
});
