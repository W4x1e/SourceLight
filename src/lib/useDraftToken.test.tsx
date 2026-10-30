import { act, render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { EditStack } from "../bindings";

// Each hand-over resolves when the test says so, so what is in flight is
// exactly what the test can see.
const sent: { draft: EditStack | null; resolve: (n: number) => void }[] = [];
vi.mock("./api", () => ({
  api: {
    setDevelopDraft: (draft: EditStack | null) =>
      new Promise<number>((resolve) => sent.push({ draft, resolve })),
  },
}));

import { useDraftToken } from "./useDraftToken";

const stack = (exposure: number) => ({ exposure }) as unknown as EditStack;

function Rig({ draft, seen }: { draft: EditStack | null; seen: (t: number | null) => void }) {
  seen(useDraftToken(draft));
  return null;
}

/**
 * A drag hands the backend its draft for the readback preview: one at
 * a time, always heading for the newest, and the token the URL asks for comes
 * back; at rest there is no token, so the preview shows the committed frame.
 */
describe("useDraftToken", () => {
  it("sends one draft at a time, skips the ones overtaken, and ends on the newest", async () => {
    let token: number | null = null;
    const seen = (t: number | null) => (token = t);
    const a = stack(0.1);
    const view = render(<Rig draft={a} seen={seen} />);
    expect(sent.map((s) => s.draft)).toEqual([a]);

    // Two more moves while the first is in flight: only the last goes next.
    const b = stack(0.2);
    const c = stack(0.3);
    view.rerender(<Rig draft={b} seen={seen} />);
    view.rerender(<Rig draft={c} seen={seen} />);
    expect(sent).toHaveLength(1);

    await act(async () => sent[0]!.resolve(1));
    expect(token).toBe(1);
    expect(sent.map((s) => s.draft)).toEqual([a, c]);
    await act(async () => sent[1]!.resolve(2));
    expect(token).toBe(2);

    // Committed: no draft, so no token, and nothing more is sent.
    view.rerender(<Rig draft={null} seen={seen} />);
    expect(token).toBeNull();
    expect(sent).toHaveLength(2);
  });
});
