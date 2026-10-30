import { beforeEach, describe, expect, it } from "vitest";
import { backPast, depthNow, gonePast, pushStep, replaceStep } from "./back";

/** Resolves once Back (or `history.go`) has landed. */
const landed = () => new Promise((done) => window.addEventListener("popstate", done, { once: true }));

describe("Back steps", () => {
  beforeEach(() => history.replaceState(null, ""));

  it("count up from the first step", () => {
    expect(depthNow()).toBe(0);
    expect(pushStep("loupe")).toBe(1);
    expect(pushStep("syncSheet")).toBe(2);
    expect(history.state).toEqual({ syncSheet: true, depth: 2 });
  });

  it("keep the depth when one step stands in for another", () => {
    pushStep("syncSheet");
    replaceStep("scrollTest");
    expect(history.state).toEqual({ scrollTest: true, depth: 1 });
  });

  it("leave a photo open when Back closes a sheet above it", async () => {
    const loupe = pushStep("loupe");
    const sheet = pushStep("syncSheet");
    const back = landed();
    history.back();
    await back;
    expect(gonePast(sheet)).toBe(true);
    expect(gonePast(loupe)).toBe(false);
  });

  it("go past a step and those above it at once", async () => {
    const loupe = pushStep("loupe");
    const menu = pushStep("libraryMenu");
    const back = landed();
    backPast(loupe);
    await back;
    expect(gonePast(menu)).toBe(true);
    expect(gonePast(loupe)).toBe(true);
    expect(depthNow()).toBe(loupe - 1);
  });

  it("know no step is never gone past", () => {
    expect(gonePast(null)).toBe(false);
  });
});
