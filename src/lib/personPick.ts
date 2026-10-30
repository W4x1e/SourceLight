import { create } from "zustand";

/**
 * "Add photos of Ada": the person Browse is picking photos for, if any.
 *
 * People can only offer who the detector found. The photos it could not help
 * with — the back of a head, a face in shadow, someone too small to see — are
 * found the way any photos are, in Browse, so the People page hands over to it
 * with this set and `PersonPickBanner` offers "Add to Ada" until it is done.
 *
 * Kept out of the main store because nothing else reads it, and a person id
 * only lasts until the next regroup (the banner ends the pick when one
 * happens). `edits` counts changes to who is in which photo, so the selection
 * panel's People section can show them wherever they were made.
 */
export const usePersonPick = create<{
  pick: { id: number; name: string } | null;
  edits: number;
  start: (id: number, name: string) => void;
  stop: () => void;
  edited: () => void;
}>((set) => ({
  pick: null,
  edits: 0,
  start: (id, name) => set({ pick: { id, name } }),
  stop: () => set({ pick: null }),
  edited: () => set((s) => ({ edits: s.edits + 1 })),
}));
