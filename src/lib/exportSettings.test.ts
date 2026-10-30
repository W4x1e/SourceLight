import { beforeEach, describe, expect, it } from "vitest";
import { DEFAULT_EXPORT, exportOptions, isPreset, renamePattern, withDefaults } from "./exportSettings";
import { useStore } from "../store";

const metadata = { location: true, names: false, aiCaption: false, aiKeywords: false };

/** Export remembers its settings and keeps named presets. */
describe("export settings", () => {
  beforeEach(() =>
    useStore.setState({ exportSettings: DEFAULT_EXPORT, exportPresets: [], exportMetadata: metadata }),
  );

  it("fills a setting added later from the defaults", () => {
    const old = { format: "png", wmImage: { imagePath: "logo.png" } } as never;
    const s = withDefaults(old);
    expect(s.format).toBe("png");
    expect(s.quality).toBe(92);
    expect(s.wmImage.imagePath).toBe("logo.png");
    expect(s.wmImage.anchor).toBe("bottom-right");
  });

  it("sends a watermark only when there is one to draw", () => {
    expect(exportOptions({ ...DEFAULT_EXPORT, wmMode: "image" }, metadata).watermark).toBeNull();
    const logo = { ...DEFAULT_EXPORT.wmImage, imagePath: "logo.png" };
    expect(exportOptions({ ...DEFAULT_EXPORT, wmMode: "image", wmImage: logo }, metadata).watermark).toEqual(logo);
    // A chosen logo is kept, but not drawn, while the watermark is off.
    expect(exportOptions({ ...DEFAULT_EXPORT, wmImage: logo }, metadata).watermark).toBeNull();
  });

  it("numbers names in front of the pattern", () => {
    expect(renamePattern(DEFAULT_EXPORT)).toBeNull();
    expect(renamePattern({ ...DEFAULT_EXPORT, rename: " {name}-web " })).toBe("{name}-web");
    expect(renamePattern({ ...DEFAULT_EXPORT, numbered: true })).toBe("{seq}-{name}");
  });

  it("saves, recognises, applies and deletes a preset", () => {
    const st = useStore.getState();
    st.setExportSettings({ size: 2048, sharpen: "standard", subfolder: "Web" });
    st.saveExportPreset("Web proofs");
    const web = useStore.getState().exportPresets[0]!;
    expect(web.name).toBe("Web proofs");
    expect(isPreset(web, useStore.getState().exportSettings, metadata)).toBe(true);

    st.setExportSettings({ size: null, format: "tiff" });
    expect(isPreset(web, useStore.getState().exportSettings, metadata)).toBe(false);
    st.applyExportPreset("Web proofs");
    expect(useStore.getState().exportSettings.size).toBe(2048);
    expect(useStore.getState().exportSettings.format).toBe("jpeg");

    // The same settings built in another key order are still the preset.
    const reordered = Object.fromEntries(Object.entries(useStore.getState().exportSettings).reverse()) as never;
    expect(isPreset(web, reordered, metadata)).toBe(true);
    // Metadata choices are part of it.
    expect(isPreset(web, useStore.getState().exportSettings, { ...metadata, names: true })).toBe(false);

    st.deleteExportPreset("Web proofs");
    expect(useStore.getState().exportPresets).toEqual([]);
  });

  it("replaces a preset saved under the same name", () => {
    const st = useStore.getState();
    st.saveExportPreset("Print");
    st.setExportSettings({ format: "tiff" });
    st.saveExportPreset("Print");
    const presets = useStore.getState().exportPresets;
    expect(presets).toHaveLength(1);
    expect(presets[0]!.settings.format).toBe("tiff");
  });
});
