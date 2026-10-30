import { describe, expect, it } from "vitest";
import {
  conflicts,
  DEFAULT_KEYMAP,
  eventToCombo,
  migrateKeymap,
  resolveAction,
  type ActionId,
} from "./keymap";

const ev = (init: KeyboardEventInit) => new KeyboardEvent("keydown", init);

describe("eventToCombo", () => {
  it("folds Ctrl and Cmd into one canonical `mod`", () => {
    expect(eventToCombo(ev({ key: "z", ctrlKey: true }))).toBe("mod+z");
    expect(eventToCombo(ev({ key: "z", metaKey: true }))).toBe("mod+z");
  });

  it("orders modifiers mod → shift and lowercases the key", () => {
    expect(eventToCombo(ev({ key: "Z", ctrlKey: true, shiftKey: true }))).toBe("mod+shift+z");
  });

  it("ignores a bare modifier press so holding Shift resolves to nothing", () => {
    expect(eventToCombo(ev({ key: "Shift", shiftKey: true }))).toBe("");
  });

  it("keeps arrows and digits verbatim", () => {
    expect(eventToCombo(ev({ key: "ArrowLeft" }))).toBe("arrowleft");
    expect(eventToCombo(ev({ key: "1" }))).toBe("1");
    expect(eventToCombo(ev({ key: "\\" }))).toBe("\\");
  });
});

describe("resolveAction", () => {
  it("matches a photo action in the library and lightbox", () => {
    expect(resolveAction(DEFAULT_KEYMAP, "library", ev({ key: "c" }))).toBe("flag-pick");
    expect(resolveAction(DEFAULT_KEYMAP, "lightbox", ev({ key: "5" }))).toBe("rate-5");
  });

  it("respects context — a develop key does not fire in the library", () => {
    expect(resolveAction(DEFAULT_KEYMAP, "develop", ev({ key: "z", ctrlKey: true }))).toBe(
      "develop-undo",
    );
    // `r` is the crop tool in Develop but nothing in the library.
    expect(resolveAction(DEFAULT_KEYMAP, "library", ev({ key: "r" }))).toBeNull();
    // Undo in Develop is the edit's, not the verdict's.
    expect(resolveAction(DEFAULT_KEYMAP, "develop", ev({ key: "d" }))).toBeNull();
  });

  it("gives Develop the verdict keys and its own undo", () => {
    expect(resolveAction(DEFAULT_KEYMAP, "develop", ev({ key: "c" }))).toBe("flag-pick");
    expect(resolveAction(DEFAULT_KEYMAP, "develop", ev({ key: "3" }))).toBe("rate-3");
    expect(resolveAction(DEFAULT_KEYMAP, "develop", ev({ key: "arrowright" }))).toBe("next-photo");
  });

  it("gives compare the photo keys plus its own set navigation", () => {
    // The focused frame in compare takes the same rating and flag keys as a
    // photo anywhere else.
    expect(resolveAction(DEFAULT_KEYMAP, "compare", ev({ key: "x" }))).toBe("flag-reject");
    expect(resolveAction(DEFAULT_KEYMAP, "compare", ev({ key: "3" }))).toBe("rate-3");
    expect(resolveAction(DEFAULT_KEYMAP, "compare", ev({ key: "k" }))).toBe("compare-keep");
    expect(resolveAction(DEFAULT_KEYMAP, "compare", ev({ key: "]" }))).toBe("compare-next-set");
    // …and the compare-only keys stay out of the library and lightbox.
    expect(resolveAction(DEFAULT_KEYMAP, "library", ev({ key: "k" }))).toBeNull();
    expect(resolveAction(DEFAULT_KEYMAP, "lightbox", ev({ key: "]" }))).toBeNull();
  });

  it("follows a rebind rather than the default", () => {
    const map = { ...DEFAULT_KEYMAP, "flag-pick": "f" as string } as Record<ActionId, string>;
    expect(resolveAction(map, "library", ev({ key: "f" }))).toBe("flag-pick");
    expect(resolveAction(map, "library", ev({ key: "c" }))).toBeNull();
  });

  it("ignores an unbound action (empty combo)", () => {
    // label-5 (purple) has no default binding.
    expect(DEFAULT_KEYMAP["label-5"]).toBe("");
    expect(resolveAction(DEFAULT_KEYMAP, "library", ev({ key: "" }))).toBeNull();
  });
});

describe("conflicts", () => {
  it("finds a clash only within an overlapping context", () => {
    // "1" is rate-1 (library+lightbox). Assigning "1" to rate-2 clashes.
    expect(conflicts(DEFAULT_KEYMAP, "rate-2", "1")).toContain("rate-1");
    // develop-crop lives only in Develop, so reusing a compare-only key there
    // does not clash — but the rating keys now reach Develop too.
    expect(conflicts(DEFAULT_KEYMAP, "develop-crop", "k")).toEqual([]);
    expect(conflicts(DEFAULT_KEYMAP, "develop-crop", "1")).toContain("rate-1");
  });
});

describe("clearing the grid's selection", () => {
  it("is Escape in the library, where nothing else wants it", () => {
    expect(resolveAction(DEFAULT_KEYMAP, "library", ev({ key: "Escape" }))).toBe("clear-selection");
    // The overlays keep Escape for closing themselves.
    expect(resolveAction(DEFAULT_KEYMAP, "lightbox", ev({ key: "Escape" }))).toBe("close-overlay");
    expect(conflicts(DEFAULT_KEYMAP, "clear-selection", "escape")).toEqual([]);
  });
});

describe("keys that work anywhere", () => {
  it("resolve in the global context, and clash with a key in any other", () => {
    expect(resolveAction(DEFAULT_KEYMAP, "global", ev({ key: "k", ctrlKey: true }))).toBe("open-palette");
    expect(resolveAction(DEFAULT_KEYMAP, "global", ev({ key: "2", ctrlKey: true }))).toBe("mode-cull");
    expect(resolveAction(DEFAULT_KEYMAP, "library", ev({ key: "k", ctrlKey: true }))).toBeNull();
    expect(conflicts(DEFAULT_KEYMAP, "develop-crop", "mod+k")).toContain("open-palette");
    expect(conflicts(DEFAULT_KEYMAP, "open-palette", "c")).toContain("flag-pick");
  });
});

describe("migrateKeymap", () => {
  it("puts pick and reject side by side for a new install", () => {
    expect(DEFAULT_KEYMAP["flag-pick"]).toBe("c");
    expect(DEFAULT_KEYMAP["flag-reject"]).toBe("x");
  });

  it("moves a saved keymap still on the old pick key to the new one", () => {
    const saved = { ...DEFAULT_KEYMAP, "flag-pick": "p" };
    expect(migrateKeymap(saved)["flag-pick"]).toBe("c");
  });

  it("keeps a key the person chose", () => {
    const saved = { ...DEFAULT_KEYMAP, "flag-pick": "q" };
    expect(migrateKeymap(saved)["flag-pick"]).toBe("q");
  });

  it("leaves the old key alone when the new one is already bound to something else", () => {
    const saved = { ...DEFAULT_KEYMAP, "flag-pick": "p", "open-develop": "c" };
    expect(migrateKeymap(saved)["flag-pick"]).toBe("p");
  });

  it("fills in actions an old install never saved", () => {
    expect(migrateKeymap(undefined)).toEqual(DEFAULT_KEYMAP);
  });
});

/** Other keyboard layouts. */
describe("keys on other layouts", () => {
  const keymap = DEFAULT_KEYMAP;
  it("rates and labels from the AZERTY number row, Shift or not", () => {
    // Unshifted, the 1 key types "&"; shifted, it types "1".
    expect(resolveAction(keymap, "cull", ev({ key: "&", code: "Digit1" }))).toBe("rate-1");
    expect(resolveAction(keymap, "cull", ev({ key: "1", code: "Digit1", shiftKey: true }))).toBe("rate-1");
    expect(resolveAction(keymap, "cull", ev({ key: "ç", code: "Digit9" }))).toBe("label-4");
  });

  it("reads AltGr as no modifier, so QWERTZ's AltGr+ß is a backslash", () => {
    const altGr = ev({ key: "\\", code: "Minus", ctrlKey: true, altKey: true });
    Object.defineProperty(altGr, "getModifierState", { value: (k: string) => k === "AltGraph" });
    expect(eventToCombo(altGr)).toBe("\\");
    expect(resolveAction(keymap, "develop", altGr)).toBe(resolveAction(keymap, "develop", ev({ key: "\\" })));
  });

  it("still prefers what the key types where both match", () => {
    expect(resolveAction(keymap, "cull", ev({ key: "2", code: "Digit2" }))).toBe("rate-2");
  });
});
