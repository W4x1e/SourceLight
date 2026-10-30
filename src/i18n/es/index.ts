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
import content from "./content";
import core from "./core";
import type { Catalogue } from "..";

/** Missing keys fall back to English (see ../index.ts). */
const catalogue: Catalogue = {
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
  ...content,
  ...core,
};

export default catalogue;
