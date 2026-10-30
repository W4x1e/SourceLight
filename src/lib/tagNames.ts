import { api } from "./api";
import { tableNamesCondition } from "./content";
import { contentLanguage, refreshText } from "../i18n";
import { setLearnedTagNames, tableNamesTag } from "../i18n/tags";
import type { Facets } from "../bindings";

/**
 * Names for the tags no table covers (docs/I18N.md).
 *
 * The built-in tagger's words are translated by fixed tables; a vision model's
 * tags are free text, so the local model names them itself, once per
 * language, and the core keeps the answers (`translate_tags`). This asks for
 * the ones the library has that are not named yet, and shows what is kept.
 *
 * Opportunistic: with no vision model running the core answers with what it
 * already has, and a tag without a name reads in English. So it runs where a
 * model is likeliest to be loaded — straight after a tagging or describing
 * pass (`force`) — as well as once per language on its own.
 */
let learnedFor: string | null = null;
let running: Promise<void> | null = null;

export function learnTagNames(facets: Facets | null, force = false): Promise<void> {
  const language = contentLanguage();
  if (!language) {
    learnedFor = null;
    return Promise.resolve();
  }
  if (!force && learnedFor === language) return running ?? Promise.resolve();
  learnedFor = language;
  running = learn(language, facets).finally(() => (running = null));
  return running;
}

async function learn(language: string, facets: Facets | null): Promise<void> {
  const kept = await api.tagTranslations(language).catch(() => ({}) as Record<string, string>);
  if (contentLanguage() !== language) return;
  setLearnedTagNames(language, kept);
  refreshText();

  const free = [
    ...(facets?.subjects ?? []).map((f) => f.value).filter((v) => !tableNamesTag(v)),
    ...(facets?.conditions ?? []).map((f) => f.value).filter((v) => !tableNamesCondition(v)),
  ].filter((v) => !(v in kept));
  if (free.length === 0) return;
  const named = await api.translateTags(language, free).catch(() => ({}) as Record<string, string>);
  if (contentLanguage() !== language || Object.keys(named).length === 0) return;
  setLearnedTagNames(language, { ...kept, ...named });
  refreshText();
}
