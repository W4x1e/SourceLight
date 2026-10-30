import { describe, expect, it } from "vitest";
import { accelerator } from "./appMenu";
import { DEFAULT_KEYMAP, actionEvent, resolveAction } from "./keymap";

describe("menu accelerators", () => {
  it("carry modifier combos across in Tauri's spelling", () => {
    expect(accelerator("mod+s")).toBe("CmdOrCtrl+S");
    expect(accelerator("mod+shift+r")).toBe("CmdOrCtrl+Shift+R");
    expect(accelerator("mod+\\")).toBe("CmdOrCtrl+Backslash");
    expect(accelerator("alt+arrowleft")).toBe("Alt+Left");
  });

  it("leave bare keys off the menu, where they would fire while typing", () => {
    expect(accelerator("3")).toBeUndefined();
    expect(accelerator("shift+x")).toBeUndefined();
    expect(accelerator("escape")).toBeUndefined();
    expect(accelerator("")).toBeUndefined();
  });
});

describe("menu-sent actions", () => {
  it("resolve to their own action where it applies, bound or not", () => {
    // Purple has no key by default: the menu is the only way to it.
    expect(DEFAULT_KEYMAP["label-5"]).toBe("");
    expect(resolveAction(DEFAULT_KEYMAP, "library", actionEvent("label-5"))).toBe("label-5");
    expect(resolveAction(DEFAULT_KEYMAP, "lightbox", actionEvent("rate-3"))).toBe("rate-3");
  });

  it("do nothing where the action does not apply, as its key would not", () => {
    expect(resolveAction(DEFAULT_KEYMAP, "develop", actionEvent("compare-keep"))).toBeNull();
    expect(resolveAction(DEFAULT_KEYMAP, "library", actionEvent("develop-crop"))).toBeNull();
  });
});
