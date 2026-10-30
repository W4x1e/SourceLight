import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { LutInfo, PresetGroupInfo } from "../bindings";
import DevelopPresets from "./DevelopPresets";

const groups: PresetGroupInfo[] = [
  {
    id: "film",
    name: "Film",
    description: "Colour film looks, from soft negatives to slide film.",
    kind: "style",
    presets: [
      { id: "film-warm", name: "Warm", description: "Warm highlights, lifted blacks, grain and a little glow." },
      { id: "film-slide", name: "Slide", description: "Colour slide film: deep blacks, clean whites and saturated colour." },
    ],
  },
  {
    id: "portrait",
    name: "Portrait",
    description: "Flattering skin, with the background kept quiet.",
    kind: "subject",
    presets: [{ id: "portrait-golden", name: "Golden", description: "Warm, late-afternoon light on skin." }],
  },
];

const luts: LutInfo[] = [
  {
    name: "ember.cube",
    title: "Ember",
    blurb: "Warm and dark, for firelight and tungsten.",
    family: "Cinema",
    grouped: false,
    imported: false,
  } as LutInfo,
];

function panel(over: Partial<Parameters<typeof DevelopPresets>[0]> = {}) {
  const props: Parameters<typeof DevelopPresets>[0] = {
    groups,
    presetId: "",
    presetAmount: 1,
    onUseBuiltin: vi.fn(),
    onAmountChange: vi.fn(),
    onAmountCommit: vi.fn(),
    builtinPreviews: {},
    onWantPreviews: vi.fn(),
    suggestions: [],
    suggesting: false,
    onRefreshSuggestions: vi.fn(),
    onApplySuggestion: vi.fn(),
    saved: [],
    savedPreviews: {},
    onUseSaved: vi.fn(),
    onRemoveSaved: vi.fn(),
    onStoreSaved: vi.fn(),
    luts,
    lutName: "",
    lutIntensity: 1,
    lutPreviews: {},
    onChooseLut: vi.fn(),
    onRemoveLut: vi.fn(),
    onImportLut: vi.fn(),
    onGroupLut: vi.fn(),
    onUngroupLut: vi.fn(),
    onLutIntensityChange: vi.fn(),
    onLutIntensityCommit: vi.fn(),
    onTabShown: vi.fn(),
    aspect: 1.5,
    onClose: vi.fn(),
    ...over,
  };
  render(<DevelopPresets {...props} />);
  return props;
}

describe("DevelopPresets", () => {
  beforeEach(() => localStorage.clear());
  afterEach(() => vi.clearAllMocks());

  it("opens on a folded list of groups, by style and by subject", () => {
    const p = panel();
    expect(screen.getByText("By style")).toBeTruthy();
    expect(screen.getByText("By subject")).toBeTruthy();
    expect(screen.getByText("Colour film looks, from soft negatives to slide film.")).toBeTruthy();
    // Folded: no preset tiles, and nothing rendered yet.
    expect(screen.queryByRole("button", { name: /^Warm$/ })).toBeNull();
    expect(p.onWantPreviews).not.toHaveBeenCalled();
  });

  it("asks for a group's previews when it is opened, and applies a preset", () => {
    const p = panel();
    fireEvent.click(screen.getByRole("button", { name: /Film/ }));
    expect(p.onWantPreviews).toHaveBeenCalledWith(["film-warm", "film-slide"]);
    fireEvent.click(screen.getByText("Slide"));
    expect(p.onUseBuiltin).toHaveBeenCalledWith("film-slide");
  });

  it("shows what is on with its amount, and takes it off", () => {
    const p = panel({ presetId: "film-warm", presetAmount: 0.8 });
    expect(screen.getByText("Film: Warm")).toBeTruthy();
    expect(screen.getByText("Amount")).toBeTruthy();
    fireEvent.click(screen.getByText("Remove"));
    expect(p.onUseBuiltin).toHaveBeenCalledWith("");
  });

  it("finds presets by name or description, opening the groups they are in", () => {
    const p = panel();
    fireEvent.change(screen.getByPlaceholderText("Find a preset…"), { target: { value: "skin" } });
    expect(screen.getByText("Golden")).toBeTruthy();
    expect(p.onWantPreviews).toHaveBeenCalledWith(["portrait-golden"]);
  });

  it("keeps LUTs on their own tab", () => {
    const p = panel({ lutName: "ember.cube" });
    expect(screen.queryByText("Ember")).toBeNull();
    fireEvent.click(screen.getByRole("tab", { name: "LUTs" }));
    expect(p.onTabShown).toHaveBeenLastCalledWith("luts");
    expect(screen.getAllByText("Ember").length).toBeGreaterThan(0);
    expect(screen.getByText("Intensity")).toBeTruthy();
  });
});
