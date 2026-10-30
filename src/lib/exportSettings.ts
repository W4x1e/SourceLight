import type {
  BitDepth,
  ExportOptions,
  MetadataChoices,
  RenderingIntent,
  SharpenLevel,
  TextWatermark,
  Watermark,
} from "../bindings";
import type { ColorSpace, ExportFormat } from "../types";

/**
 * Everything the export dialog asks, remembered between exports and across
 * restarts. It was the dialog's own state, so every Develop visit and
 * every launch started again from JPEG at full size with no watermark — and
 * forgetting to pick the logo again once sent unwatermarked proofs to a client.
 */
export interface ExportSettings {
  format: ExportFormat;
  /** sRGB unless asked: what the web and most screens assume. */
  space: ColorSpace;
  /** Relative colorimetric unless asked: exact for every colour the space holds. */
  intent: RenderingIntent;
  quality: number;
  /** Eight unless asked: sixteen is for a file headed into another editor. */
  depth: BitDepth;
  /** The long edge in pixels; null is full size. */
  size: number | null;
  sharpen: SharpenLevel;
  /** Strip everything but the pixels. */
  strip: boolean;
  /** A {name} rename pattern, for a batch. */
  rename: string;
  /** 001-, 002-… before each name, in the order shown. */
  numbered: boolean;
  subfolder: string;
  wmMode: "none" | "image" | "text";
  wmImage: Watermark;
  wmText: TextWatermark;
}

export const DEFAULT_EXPORT: ExportSettings = {
  format: "jpeg",
  space: "srgb",
  intent: "relative-colorimetric",
  quality: 92,
  depth: "eight",
  size: null,
  sharpen: "off",
  strip: false,
  rename: "",
  numbered: false,
  subfolder: "",
  wmMode: "none",
  wmImage: { imagePath: "", anchor: "bottom-right", opacity: 0.6, scale: 0.2 },
  wmText: { text: "© ", fontPath: "", anchor: "bottom-right", opacity: 0.7, size: 0.04 },
};

/** Settings saved under a name — "Web proofs", "Print" — with the metadata
 *  choices, which are the dialog's too. */
export interface ExportPreset {
  name: string;
  settings: ExportSettings;
  metadata: Required<MetadataChoices>;
}

/** The long edges offered as buttons; any other is typed. */
export const SIZES = [4096, 2048, 1200] as const;

/** What the backend is asked for, from the settings and metadata choices. */
export function exportOptions(s: ExportSettings, metadata: Required<MetadataChoices>): ExportOptions {
  return {
    format: s.format,
    quality: s.quality,
    maxEdge: s.size,
    sharpen: s.sharpen,
    colorSpace: s.space,
    intent: s.intent,
    // Only an image actually chosen, or text with a font, is sent.
    watermark: s.wmMode === "image" && s.wmImage.imagePath ? s.wmImage : null,
    textWatermark: s.wmMode === "text" && s.wmText.text.trim() && s.wmText.fontPath ? s.wmText : null,
    stripMetadata: s.strip,
    metadata,
    bitDepth: s.depth,
  };
}

/** The rename pattern a batch sends, numbering included; null for none. */
export function renamePattern(s: ExportSettings): string | null {
  const rename = s.rename.trim();
  return s.numbered ? `{seq}-${rename || "{name}"}` : rename || null;
}

/** A remembered value laid over the defaults, so a field added later starts
 *  at its default rather than missing. */
export function withDefaults(saved: Partial<ExportSettings> | undefined): ExportSettings {
  return {
    ...DEFAULT_EXPORT,
    ...saved,
    wmImage: { ...DEFAULT_EXPORT.wmImage, ...saved?.wmImage },
    wmText: { ...DEFAULT_EXPORT.wmText, ...saved?.wmText },
  };
}

/** Whether the dialog's settings are exactly a preset's, to show its name. */
export function isPreset(p: ExportPreset, s: ExportSettings, metadata: Required<MetadataChoices>): boolean {
  return stable(p.settings) === stable(s) && stable(p.metadata) === stable(metadata);
}

/** JSON with the keys in order, so the same settings compare equal however
 *  they were built. */
function stable(v: unknown): string {
  if (Array.isArray(v)) return `[${v.map(stable).join(",")}]`;
  if (v && typeof v === "object") {
    const o = v as Record<string, unknown>;
    return `{${Object.keys(o)
      .filter((k) => o[k] !== undefined)
      .sort()
      .map((k) => `${JSON.stringify(k)}:${stable(o[k])}`)
      .join(",")}}`;
  }
  return JSON.stringify(v);
}
