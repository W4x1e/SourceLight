import type { StateCreator } from "zustand";
import type { LibrarySlice } from "./library";
import type { SearchSlice } from "./search";
import type { BrowseSlice } from "./browse";
import type { ViewerSlice } from "./viewer";
import type { SelectionSlice } from "./selection";
import type { CompareSlice } from "./compare";
import type { CullSlice } from "./cull";
import type { DevelopSlice } from "./develop";
import type { AiSlice } from "./ai";
import type { KeywordsSlice } from "./keywords";
import type { CollectionsSlice } from "./collections";
import type { ShootsSlice } from "./shoots";
import type { ExportSlice } from "./export";
import type { MetadataSlice } from "./metadata";
import type { PlacesSlice } from "./places";
import type { AppSlice } from "./app";
import type { SettingsSlice } from "./settings";

/**
 * The store's shape, shared by its slices: the whole state, and what one slice
 * of it is.
 */

/** The whole store: every slice's state and actions. */
export type State = LibrarySlice &
  SearchSlice &
  BrowseSlice &
  ViewerSlice &
  SelectionSlice &
  CompareSlice &
  CullSlice &
  DevelopSlice &
  AiSlice &
  KeywordsSlice &
  CollectionsSlice &
  ShootsSlice &
  ExportSlice &
  MetadataSlice &
  PlacesSlice &
  AppSlice &
  SettingsSlice;

/** One slice of the store. It is given the whole store's `set`, `get` and API,
 *  so its actions can reach every other slice's. */
export type Slice<T> = StateCreator<State, [["zustand/persist", unknown]], [], T>;
