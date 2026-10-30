import { api } from "../lib/api";
import type { BatchOutcome, MetadataChoices } from "../bindings";
import { DEFAULT_EXPORT, withDefaults, type ExportPreset, type ExportSettings } from "../lib/exportSettings";
import type { BatchRequest } from "../lib/batchExport";
import type { Slice } from "./types";

/**
 * Exporting: the batch in progress, and the settings and presets both export
 * dialogs share.
 */

export interface ExportSlice {
  /** The running (or just-finished) batch export, shown in the queue panel.
   *  Lives in the global store so it survives the Develop dialog closing. */
  exportJob: {
    /** What was asked, kept so the failed ones can run again. */
    request: BatchRequest;
    done: number;
    message: string;
    running: boolean;
    summary: string | null;
    /** Each photo's result as it lands: null written, else why not. */
    results: Record<number, string | null>;
    /** The folder the files went into, once the batch says. */
    directory: string | null;
  } | null;
  startExportJob: (request: BatchRequest) => void;
  updateExportProgress: (done: number, total: number, message: string) => void;
  /** One photo written, or failed with its reason. */
  noteExportItem: (photoId: number, error: string | null) => void;
  finishExportJob: (summary: string, outcome?: BatchOutcome) => void;
  dismissExportJob: () => void;
  cancelExportJob: () => Promise<void>;
  /** Quick batch export of the current selection with sensible JPEG defaults —
   *  the selection panel's Export action. Picks a destination folder, exports
   *  each photo through its own saved edits, and drives the export queue. Deeper
   *  options (format, size, watermark) still live in Develop's export dialog. */
  exportSelection: () => Promise<void>;
  /** The export dialog for the selection, outside Develop. */
  showExport: boolean;
  setShowExport: (v: boolean) => void;
  /** What exports and sidecars say beyond the camera's record:
   *  remembered, with defaults right for a client delivery. */
  exportMetadata: Required<MetadataChoices>;
  setExportMetadata: (patch: Partial<MetadataChoices>) => void;
  /** The export dialog's settings, remembered between exports and across
   *  restarts, and shared by Develop's dialog and the library's. */
  exportSettings: ExportSettings;
  setExportSettings: (patch: Partial<ExportSettings>) => void;
  /** Named export settings — "Web proofs", "Print". */
  exportPresets: ExportPreset[];
  /** Saves the current settings under a name, replacing one of that name. */
  saveExportPreset: (name: string) => void;
  applyExportPreset: (name: string) => void;
  deleteExportPreset: (name: string) => void;
}

export const createExportSlice: Slice<ExportSlice> = (set, get) => ({
  exportJob: null,
  startExportJob: (request) =>
    set({
      exportJob: {
        request,
        done: 0,
        message: request.files[0] ?? "",
        running: true,
        summary: null,
        results: {},
        directory: null,
      },
    }),
  noteExportItem: (photoId, error) =>
    set((s) =>
      s.exportJob ? { exportJob: { ...s.exportJob, results: { ...s.exportJob.results, [photoId]: error } } } : {},
    ),
  updateExportProgress: (done, total, message) =>
    set((s) =>
      s.exportJob
        ? { exportJob: { ...s.exportJob, done, message, running: done < total } }
        : {},
    ),
  finishExportJob: (summary, outcome) =>
    set((s) => {
      if (!s.exportJob) return {};
      // The outcome's failures over the live results, in case an event
      // was missed: it is the record of what happened.
      const results = { ...s.exportJob.results };
      for (const f of outcome?.failures ?? []) results[f.photoId] = f.error;
      return {
        exportJob: {
          ...s.exportJob,
          running: false,
          message: "",
          summary,
          results,
          directory: outcome?.directory ?? s.exportJob.directory,
        },
      };
    }),
  dismissExportJob: () => set({ exportJob: null }),
  cancelExportJob: async () => {
    await api.cancelExport();
  },

  showExport: false,
  setShowExport: (v) => set({ showExport: v }),

  // The full dialog, as Develop has. This used to export at once
  // with its own fixed settings — JPEG 92, full size, Standard sharpening
  // — while the dialog defaulted sharpening to Off, so the same photos
  // came out differently depending on which button you pressed.
  exportSelection: async () => {
    const st = get();
    if (st.selected.size === 0) return;
    set({ showExport: true });
  },
  exportMetadata: { location: true, names: false, aiCaption: false, aiKeywords: false },
  setExportMetadata: (patch) => set({ exportMetadata: { ...get().exportMetadata, ...patch } }),
  exportSettings: DEFAULT_EXPORT,
  setExportSettings: (patch) => set({ exportSettings: { ...get().exportSettings, ...patch } }),
  exportPresets: [],
  saveExportPreset: (name) => {
    const { exportSettings, exportMetadata, exportPresets } = get();
    const preset = { name, settings: exportSettings, metadata: exportMetadata };
    const others = exportPresets.filter((p) => p.name !== name);
    set({ exportPresets: [...others, preset].sort((a, b) => a.name.localeCompare(b.name)) });
  },
  applyExportPreset: (name) => {
    const preset = get().exportPresets.find((p) => p.name === name);
    if (!preset) return;
    set({ exportSettings: withDefaults(preset.settings), exportMetadata: { ...get().exportMetadata, ...preset.metadata } });
  },
  deleteExportPreset: (name) => set({ exportPresets: get().exportPresets.filter((p) => p.name !== name) }),
});
