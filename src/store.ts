import { create } from "zustand";
import { persist, createJSONStorage } from "zustand/middleware";
import { changedOnlyStorage } from "./lib/changedOnlyStorage";
import { withDefaults } from "./lib/exportSettings";
import { migrateKeymap } from "./lib/keymap";
import { normaliseFilter, type PhotoFilter } from "./types";
import type { State } from "./store/types";
import { createAiSlice } from "./store/ai";
import { createAppSlice } from "./store/app";
import { createBrowseSlice } from "./store/browse";
import { createCollectionsSlice } from "./store/collections";
import { createCompareSlice } from "./store/compare";
import { createCullSlice } from "./store/cull";
import { createDevelopSlice } from "./store/develop";
import { createExportSlice } from "./store/export";
import { createKeywordsSlice } from "./store/keywords";
import { createLibrarySlice, viewFilterFor } from "./store/library";
import { createMetadataSlice } from "./store/metadata";
import { createPlacesSlice } from "./store/places";
import { createSearchSlice } from "./store/search";
import { createSelectionSlice } from "./store/selection";
import { createSettingsSlice } from "./store/settings";
import { createShootsSlice, DEFAULT_IMPORT } from "./store/shoots";
import { createViewerSlice } from "./store/viewer";

export { ANALYSIS_JOB, DESCRIBE_BATCH, POWER_POLL_MS } from "./store/ai";
export type { AnalysisStep, AutoStep, Job } from "./store/ai";
export type { AppMode } from "./store/app";
export { DUPLICATE_THRESHOLD } from "./store/compare";
export type { CompareSource, CompareState, PhotoPatch } from "./store/compare";
export type { CullSession } from "./store/cull";
export { AUTO_FIELDS, developShown } from "./store/develop";
export { EXTRA_SECTIONS, PAGE, extraGroups } from "./store/library";
export { SIMILAR_THRESHOLD } from "./store/search";
export type { SelectMode } from "./store/selection";
export type { SettingsTab } from "./store/settings";
export type { ImportSettings } from "./store/shoots";
export type { AidPlace, ViewAids } from "./store/viewer";

/**
 * Application state.
 *
 * Everything lives here rather than in `App.tsx` because the develop module
 * brings sixty-odd edit parameters, and threading those through the component
 * tree as props is not survivable. Components subscribe to the slices they
 * actually use, so a slider moving does not re-render the grid.
 *
 * Each area's state and actions are a slice in `store/`. This file puts them
 * together into the one store and persists the preferences among them.
 * Everything imports the store from here, not from the slices.
 */

/**
 * A filter as it goes to the backend: with the view settings that apply to
 * every query but belong to no filter. Today that is "Show the tagger's
 * keywords" — kept out of `filter` itself so Clear all can't flip it
 * and a saved view never carries it.
 */
export function viewFilter(f: PhotoFilter): PhotoFilter {
  return viewFilterFor(useStore.getState(), f);
}

export const useStore = create<State>()(
  persist(
    (...a) => ({
      ...createLibrarySlice(...a),
      ...createSearchSlice(...a),
      ...createBrowseSlice(...a),
      ...createViewerSlice(...a),
      ...createSelectionSlice(...a),
      ...createCompareSlice(...a),
      ...createCullSlice(...a),
      ...createDevelopSlice(...a),
      ...createAiSlice(...a),
      ...createKeywordsSlice(...a),
      ...createCollectionsSlice(...a),
      ...createShootsSlice(...a),
      ...createExportSlice(...a),
      ...createMetadataSlice(...a),
      ...createPlacesSlice(...a),
      ...createAppSlice(...a),
      ...createSettingsSlice(...a),
    }),
    {
      // One key replaces the old per-value `filter` / `cellSize` entries, so
      // existing installs start from defaults once. Preferences, not data.
      // The working name, kept: it is where saved settings live.
      name: "photo-organiser",
      // Written only when the saved part changed, not on every change.
      storage: createJSONStorage(() => changedOnlyStorage(localStorage)),
      partialize: (s) => ({
        filter: s.filter,
        cellSize: s.cellSize,
        groupByDate: s.groupByDate,
        keymap: s.keymap,
        cullAutoAdvance: s.cullAutoAdvance,
        cullKeepZoom: s.cullKeepZoom,
        viewAids: s.viewAids,
        peakColor: s.peakColor,
        peakSensitivity: s.peakSensitivity,
        autoAnalyse: s.autoAnalyse,
        autoSteps: s.autoSteps,
        checkUpdates: s.checkUpdates,
        exportMetadata: s.exportMetadata,
        exportSettings: s.exportSettings,
        exportPresets: s.exportPresets,
        importSettings: s.importSettings,
        autoBackup: s.autoBackup,
        language: s.language,
        showAiKeywords: s.showAiKeywords,
        holdOnBattery: s.holdOnBattery,
        directCanvas: s.directCanvas,
        cullSort: s.cullSort,
        cullSession: s.cullSession,
        cullSuggest: s.cullSuggest,
        railCollapsed: s.railCollapsed,
        slideshowOptions: s.slideshowOptions,
        timelineCollapsed: s.timelineCollapsed,
        emptyPanelHidden: s.emptyPanelHidden,
        railSections: s.railSections,
        railExpanded: s.railExpanded,
        tourSeen: s.tourSeen,
      }),
      // Merge onto the defaults so a schema change can't strand old state with
      // a missing field — the Rust `PhotoFilter` has no `#[serde(default)]`
      // precisely because this guarantees every field is present.
      merge: (persisted, current) => {
        const saved = (persisted ?? {}) as Partial<State>;
        const filter = normaliseFilter(saved.filter);
        // Merge onto defaults so a newly-added action gets its default binding
        // rather than being left unbound in an old install, and a changed
        // default reaches anyone who never rebound it.
        const keymap = migrateKeymap(saved.keymap);
        // Per key, so a pass added later starts on rather than missing.
        const autoSteps = { ...current.autoSteps, ...saved.autoSteps };
        const viewAids = { ...current.viewAids, ...saved.viewAids };
        // Per choice, so one added later starts at its default.
        const exportMetadata = { ...current.exportMetadata, ...saved.exportMetadata };
        // Per setting, for the same reason.
        const exportSettings = withDefaults(saved.exportSettings);
        const importSettings = { ...DEFAULT_IMPORT, ...saved.importSettings };
        return {
          ...current,
          ...saved,
          filter,
          committedFilter: filter,
          keymap,
          autoSteps,
          viewAids,
          exportMetadata,
          exportSettings,
          importSettings,
        };
      },
    },
  ),
);
