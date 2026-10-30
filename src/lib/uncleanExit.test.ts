import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./api", () => ({
  api: { uncleanExit: vi.fn(), openLogFolder: vi.fn() },
}));
vi.mock("./dialogs", () => ({ confirm: vi.fn() }));

import { api } from "./api";
import { confirm } from "./dialogs";
import { reportUncleanExit } from "./uncleanExit";

describe("reportUncleanExit", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(api.openLogFolder).mockResolvedValue(null);
  });

  it("says nothing after a normal exit", async () => {
    vi.mocked(api.uncleanExit).mockResolvedValue(null);
    await reportUncleanExit();
    expect(confirm).not.toHaveBeenCalled();
  });

  it("names the kept log and opens its folder when asked", async () => {
    vi.mocked(api.uncleanExit).mockResolvedValue("unclean-exit-2026-10-02T14-27-03-812.log");
    vi.mocked(confirm).mockResolvedValue(true);
    await reportUncleanExit();
    expect(vi.mocked(confirm).mock.calls[0]![0].message).toContain(
      "unclean-exit-2026-10-02T14-27-03-812.log",
    );
    expect(api.openLogFolder).toHaveBeenCalled();
  });

  it("leaves the folder closed when dismissed", async () => {
    vi.mocked(api.uncleanExit).mockResolvedValue("unclean-exit-x.log");
    vi.mocked(confirm).mockResolvedValue(false);
    await reportUncleanExit();
    expect(api.openLogFolder).not.toHaveBeenCalled();
  });
});
