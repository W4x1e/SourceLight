import { describe, expect, it } from "vitest";
import { takenWhy } from "./CatalogueBackups";

const backup = (filename: string, automatic = true) => ({
  path: `/b/${filename}`,
  filename,
  bytes: 1,
  modified: 1,
  automatic,
});

describe("why a backup was taken", () => {
  it("names each kind from its filename", () => {
    expect(takenWhy(backup("library-daily-1790600000.db"))).toBe("daily");
    expect(takenWhy(backup("library-auto-v24-1790552558.db"))).toBe("before an upgrade");
    expect(takenWhy(backup("library-auto-before-retag-1.db"))).toBe("before re-tagging");
    expect(takenWhy(backup("library-auto-before-folder-removal-1.db"))).toBe("before a library change");
    expect(takenWhy(backup("library-manual-1.db", false))).toBe("manual");
  });
});
