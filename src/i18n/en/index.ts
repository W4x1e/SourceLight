import common from "./common";
import shell from "./shell";
import library from "./library";
import filters from "./filters";
import cull from "./cull";
import develop from "./develop";
import tools from "./tools";
import people from "./people";
import places from "./places";
import transfer from "./transfer";
import settings from "./settings";
import store from "./store";
import viewer from "./viewer";
import core from "./core";
import content from "./content";
import mobile from "./mobile";

/**
 * Every area's strings, keyed `area.what`. Each area is its own file so the
 * files stay a size a translator can work through, and so two changes in
 * different corners of the app never touch the same file. Keys start with
 * their area's name, which is what keeps two areas from colliding — the
 * catalogue test checks that no key appears twice.
 */
export const AREAS = {
  common,
  shell,
  library,
  filters,
  cull,
  develop,
  tools,
  people,
  places,
  transfer,
  settings,
  store,
  viewer,
  core,
  content,
  mobile,
} as const;

export const en = {
  ...common,
  ...shell,
  ...library,
  ...filters,
  ...cull,
  ...develop,
  ...tools,
  ...people,
  ...places,
  ...transfer,
  ...settings,
  ...store,
  ...viewer,
  ...core,
  ...content,
  ...mobile,
};
