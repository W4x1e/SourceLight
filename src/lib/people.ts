import { api } from "./api";
import { ensureAi } from "./aiReady";
import type { FacesStatus, FacesSummary } from "../bindings";
import { intlLocale, t } from "../i18n";

/**
 * Finding people, as one action.
 *
 * It used to be three: set up the models, run the face pass, then regroup.
 * Each surface wired them together itself, and the Settings row stopped after
 * the second, so its faces never became people until someone opened the page
 * and pressed Regroup. Every caller now goes through `findPeople`.
 */

/**
 * Photos an automatic pass would work on now. It excludes faces waiting for the
 * recogniser while the recogniser isn't installed: without it they would be
 * picked up and left unrecognised on every pass.
 */
export function facesBacklog(s: FacesStatus | null | undefined): number {
  if (!s) return 0;
  return s.toDetect + (s.recogniserInstalled ? s.toRecognise : 0);
}

/** Photos `findPeople` would work on. It sets up the recogniser itself, so everything counts. */
export function peopleBacklog(s: FacesStatus | null | undefined): number {
  return s ? s.toDetect + s.toRecognise : 0;
}

export type PeopleStage = "setup" | "faces" | "grouping";

export interface PeopleRun {
  summary: FacesSummary;
  /** People after grouping; null when the pass was stopped before it. */
  people: number | null;
}

/**
 * Sets up the People models if needed, finds and recognises faces, then groups them.
 *
 * Returns null when setup was declined or could not happen; `ensureAi` has
 * already said why. Throws if the pass or grouping fails, including when it is
 * cancelled from Activity (see `isCancelled`).
 */
export async function findPeople(onStage?: (stage: PeopleStage) => void): Promise<PeopleRun | null> {
  onStage?.("setup");
  // Recognising who anyone is waits for People's yes: without it the
  // pass finds faces for culling and fetches no recogniser.
  const recognise = await api.peopleEnabled().catch(() => true);
  if (!(await ensureAi(recognise ? "people" : "faces"))) return null;
  onStage?.("faces");
  const summary = await api.analyzeFaces();
  if (summary.stopped || !recognise) return { summary, people: null };
  // Grouping reads every embedded face, not only this run's. It runs even when
  // nothing new was embedded, so an earlier run that stopped part-way still
  // gets grouped.
  onStage?.("grouping");
  const people = await api.groupPeople();
  return { summary, people };
}

/**
 * What a run did, in one sentence, and how serious it is. A run that found
 * faces but recognised none of them is an error, not a success with nothing to
 * show.
 */
export function describePeopleRun(run: PeopleRun): {
  text: string;
  tone: "success" | "info" | "error";
} {
  const { summary: s, people } = run;
  if (s.stopped) return { text: t("people.run.stopped"), tone: "info" };
  const grouped = people === null ? null : t("people.count.people", { count: people });
  if (s.embedFailed > 0 && s.embedded === 0) {
    return {
      text: s.firstError
        ? t("people.run.recogniseFailedWhy", { count: s.faces, error: s.firstError })
        : t("people.run.recogniseFailed", { count: s.faces }),
      tone: "error",
    };
  }
  if (s.photos === 0) {
    return {
      text: grouped === null ? t("people.run.nothingNew") : t("people.run.nothingNewGrouped", { people: grouped }),
      tone: "info",
    };
  }
  const photos = t("common.photos", { count: s.photos });
  if (s.faces === 0) return { text: t("people.run.noFaces", { photos }), tone: "info" };
  const faces = t("people.count.faces", { count: s.faces });
  let text: string;
  if (grouped === null) {
    text =
      s.embedFailed > 0
        ? t("people.run.foundMissed", { faces, photos, missed: s.embedFailed })
        : t("people.run.found", { faces, photos });
  } else {
    text =
      s.embedFailed > 0
        ? t("people.run.foundGroupedMissed", { faces, photos, people: grouped, missed: s.embedFailed })
        : t("people.run.foundGrouped", { faces, photos, people: grouped });
  }
  return { text, tone: "success" };
}

/** How the People page orders everyone. */
export type PeopleOrder = "photos" | "name";

/**
 * The People page's people, found and ordered. A search matches a
 * name anywhere in it, case aside; "unnamed" finds the people still to name.
 * By name, the named come first, A to Z, and the unnamed after, most
 * photographed first — they are the ones worth naming next.
 */
export function arrangePeople<T extends { name: string | null; count: number; id: number }>(
  people: readonly T[],
  query: string,
  order: PeopleOrder,
): T[] {
  const q = query.trim().toLowerCase();
  const found = !q
    ? [...people]
    : people.filter((p) => (p.name ? p.name.toLowerCase().includes(q) : t("people.unnamed").toLowerCase().startsWith(q)));
  const byCount = (a: T, b: T) => b.count - a.count || a.id - b.id;
  if (order === "photos") return found.sort(byCount);
  return found.sort((a, b) => {
    if (a.name && b.name) return a.name.localeCompare(b.name, intlLocale(), { sensitivity: "base" }) || byCount(a, b);
    if (a.name) return -1;
    if (b.name) return 1;
    return byCount(a, b);
  });
}
