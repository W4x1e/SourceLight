import { api } from "../lib/api";
import { toast } from "../lib/dialogs";
import { t } from "../i18n";
import { orderGroupsBySharpness } from "../lib/survey";
import { EXTRA_SECTIONS } from "./library";
import { DUPLICATE_THRESHOLD, duplicateScope } from "./compare";
import type { Slice } from "./types";

/**
 * Browse's view state: the grid, the filter rail, the timeline and burst stacks.
 */

export interface BrowseSlice {
  cellSize: number;

  /**
   * Browse's view state.
   *
   * `App` routes modes with a ternary, so `BrowseWorkspace` and everything under
   * it unmounts the moment you visit Cull, Places or People. Held in component
   * state, all of this was thrown away on every mode switch — and the Cull →
   * "Review N picks" → Browse round trip is a core loop, so you landed back at
   * the top of the grid with the rail re-collapsed every time.
   */
  /** The Browse filter rail folded down to a strip, giving the grid its width. */
  railCollapsed: boolean;
  /** The timeline strip over the grid, folded away; remembered. */
  timelineCollapsed: boolean;
  setTimelineCollapsed: (v: boolean) => void;
  /** The selection panel's "Nothing selected" slot closed: the panel then
   *  shows only with a selection, as on a narrow window. Remembered. */
  emptyPanelHidden: boolean;
  setEmptyPanelHidden: (v: boolean) => void;
  /** A period the grid should scroll to, set by the timeline strip. `seq`
   *  makes the same period asked twice a new request. */
  gridJump: { period: string; seq: number } | null;
  jumpGridTo: (period: string) => void;
  setRailCollapsed: (collapsed: boolean) => void;
  /** Rail sections the user has explicitly opened or closed, by label. Absent
   *  means "still at the section's own default". */
  railSections: Record<string, boolean>;
  setRailSection: (label: string, open: boolean) => void;
  /** Which facet lists have been expanded past their "show more" cut, by id. */
  railExpanded: Record<string, boolean>;
  setRailExpanded: (id: string, expanded: boolean) => void;
  /** Grid scroll offset. In memory only — across a restart the library may have
   *  changed underneath it, so a stale offset would be a worse guess than 0. */
  gridScrollTop: number;
  setGridScrollTop: (n: number) => void;

  /** Break the library grid into months and days. On by default: a photo
   *  library is organised on time before anything else, and without it the
   *  grid is one undifferentiated wall of tiles. Off for anyone who wants the
   *  denser contact sheet. */
  groupByDate: boolean;
  setGroupByDate: (v: boolean) => void;

  /** Burst stacking: collapse near-duplicate groups to one grid cell each. */
  stacking: boolean;
  /** Cached near-duplicate groups (ids) the grid stacks by; null until fetched. */
  stackGroups: number[][] | null;
  /** Which stacks are expanded, by representative photo id. */
  expandedStacks: Set<number>;
  /** True while the stack grouping is being fetched. */
  stackingBusy: boolean;
  setCellSize: (n: number) => void;

  /** Turn burst stacking on (fetching the grouping if needed) or off. */
  toggleStacking: () => Promise<void>;
  /** Expand or collapse one stack, by its representative id. */
  toggleStack: (id: number) => void;
}

export const createBrowseSlice: Slice<BrowseSlice> = (set, get) => ({
  cellSize: 168,

  groupByDate: true,
  setGroupByDate: (v) => set({ groupByDate: v }),

  stacking: false,
  stackGroups: null,
  expandedStacks: new Set(),
  stackingBusy: false,

  setCellSize: (n) => set({ cellSize: n }),

  railCollapsed: false,
  timelineCollapsed: false,
  setTimelineCollapsed: (v) => set({ timelineCollapsed: v }),
  emptyPanelHidden: false,
  setEmptyPanelHidden: (v) => set({ emptyPanelHidden: v }),
  gridJump: null,
  jumpGridTo: (period) => set((s) => ({ gridJump: { period, seq: (s.gridJump?.seq ?? 0) + 1 } })),
  setRailCollapsed: (railCollapsed) => set({ railCollapsed }),
  railSections: {},
  setRailSection: (label, open) => {
    set((s) => ({ railSections: { ...s.railSections, [label]: open } }));
    // Their counts are only kept while they are open: opening one
    // counts it now rather than showing the last filter's numbers.
    if (open && (Object.values(EXTRA_SECTIONS) as string[]).includes(label)) void get().refreshExtraFacets();
  },
  railExpanded: {},
  setRailExpanded: (id, expanded) =>
    set((s) => ({ railExpanded: { ...s.railExpanded, [id]: expanded } })),
  gridScrollTop: 0,
  setGridScrollTop: (n) => set({ gridScrollTop: n }),

  toggleStacking: async () => {
    if (get().stacking) {
      set({ stacking: false });
      return;
    }
    // Reuse the grouping if we already have it; otherwise fetch it once. It
    // is the same near-duplicate grouping the duplicate review uses.
    if (get().stackGroups) {
      set({ stacking: true });
      return;
    }
    set({ stackingBusy: true, error: null });
    try {
      const groups = await api.duplicateGroups(DUPLICATE_THRESHOLD, duplicateScope(get()));
      if (groups.length === 0) {
        toast(t("store.stacking.none"), "info");
        return;
      }
      // Sharpest first, so a collapsed stack's cover is its best frame.
      const ordered = orderGroupsBySharpness(groups, get().searchResults ?? get().photos);
      set({ stacking: true, stackGroups: ordered, expandedStacks: new Set() });
    } catch (e) {
      console.error("stacking failed", e);
      set({ error: String(e) });
    } finally {
      set({ stackingBusy: false });
    }
  },

  toggleStack: (id) => {
    const next = new Set(get().expandedStacks);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    set({ expandedStacks: next });
  },
});
