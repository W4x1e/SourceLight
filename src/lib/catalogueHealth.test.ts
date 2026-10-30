import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./api", () => ({
  api: { catalogueRecovery: vi.fn(), restoreNewestBackup: vi.fn(), restartApp: vi.fn() },
}));
vi.mock("./dialogs", () => ({ confirm: vi.fn(), toast: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

import { api } from "./api";
import { confirm } from "./dialogs";
import { reportRecovery } from "./catalogueHealth";
import { useStore } from "../store";

describe("reportRecovery", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useStore.setState({ showSettings: false });
  });

  it("says nothing when startup did not have to recover", async () => {
    vi.mocked(api.catalogueRecovery).mockResolvedValue(null);
    await reportRecovery();
    expect(confirm).not.toHaveBeenCalled();
  });

  it("names the backup it restored and where the damaged file went", async () => {
    vi.mocked(api.catalogueRecovery).mockResolvedValue({
      restored: {
        path: "C:/b/library-auto-x.db",
        filename: "library-auto-x.db",
        bytes: 1,
        modified: 1790338694,
        automatic: true,
      },
      damagedCopy: "C:/b/damaged/library-damaged-1.db",
    });
    vi.mocked(confirm).mockResolvedValue(false);
    await reportRecovery();
    const message = vi.mocked(confirm).mock.calls[0]![0].message!;
    expect(message).toContain("restored the backup from");
    expect(message).toContain("C:/b/damaged/library-damaged-1.db");
    // "See backups" opens where they are listed.
    expect(useStore.getState().settingsTab).toBe("library");
  });
});
