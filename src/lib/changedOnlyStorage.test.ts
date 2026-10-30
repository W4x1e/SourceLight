import { describe, expect, it, vi } from "vitest";
import { changedOnlyStorage } from "./changedOnlyStorage";

function fakeStorage() {
  const data = new Map<string, string>();
  return {
    getItem: vi.fn((k: string) => data.get(k) ?? null),
    setItem: vi.fn((k: string, v: string) => void data.set(k, v)),
    removeItem: vi.fn((k: string) => void data.delete(k)),
  } as unknown as Storage & { setItem: ReturnType<typeof vi.fn> };
}

describe("changedOnlyStorage", () => {
  it("writes a value once, and again only when it changes", () => {
    const backing = fakeStorage();
    const s = changedOnlyStorage(backing);
    s.setItem("k", "a");
    s.setItem("k", "a");
    s.setItem("k", "a");
    expect(backing.setItem).toHaveBeenCalledTimes(1);
    s.setItem("k", "b");
    expect(backing.setItem).toHaveBeenCalledTimes(2);
  });

  it("doesn't rewrite what it just read", () => {
    const backing = fakeStorage();
    backing.setItem("k", "saved");
    backing.setItem.mockClear();
    const s = changedOnlyStorage(backing);
    expect(s.getItem("k")).toBe("saved");
    s.setItem("k", "saved");
    expect(backing.setItem).not.toHaveBeenCalled();
  });

  it("writes again after a removal", () => {
    const backing = fakeStorage();
    const s = changedOnlyStorage(backing);
    s.setItem("k", "a");
    s.removeItem("k");
    s.setItem("k", "a");
    expect(backing.setItem).toHaveBeenCalledTimes(2);
  });
});
