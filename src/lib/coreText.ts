import { en } from "../i18n/en";
import { t, type Key } from "../i18n";

/**
 * Wording for the text the Rust core sends.
 *
 * The core hands the interface finished English sentences — command errors, the
 * AI status detail, job messages, what a model is for. Rather than change every
 * command to send a code, the interface recognises the core's known sentences
 * and shows its own translation of them; anything it does not recognise is
 * shown as it came, so a new or unexpected message is never lost.
 *
 * The table is the `core.msg.*` and `core.progress.*` entries of the English
 * catalogue: each value is the core's exact wording, with `{name}` where the
 * core formats a value in. A pattern is built from it, and the captured values
 * become the parameters of the translated message. **Keep those values in step
 * with the Rust source** — `coreText.test.ts` reads the Rust files and fails when
 * a mapped message no longer occurs there, so a reworded core message is caught
 * instead of silently going untranslated.
 */

type Matcher = { key: Key; re: RegExp; names: string[]; literal: number };

const cache = new Map<string, Matcher[]>();

/** Runs of whitespace are one space, so a string the source wraps still matches. */
const squash = (s: string) => s.replace(/\s+/g, " ").trim();

function matchers(prefix: string): Matcher[] {
  let list = cache.get(prefix);
  if (list) return list;
  list = [];
  for (const [key, value] of Object.entries(en)) {
    if (!key.startsWith(prefix) || typeof value !== "string") continue;
    const names: string[] = [];
    let literal = 0;
    const body = squash(value)
      .split(/(\{\w+\})/)
      .map((part) => {
        const name = /^\{(\w+)\}$/.exec(part)?.[1];
        if (name) {
          names.push(name);
          return "(.+?)";
        }
        literal += part.length;
        return part.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
      })
      .join("");
    list.push({ key: key as Key, re: new RegExp(`^${body}$`, "s"), names, literal });
  }
  cache.set(prefix, list);
  return list;
}

type Hit = { key: Key; params: Record<string, string> };

/** The best entry that matches the whole of `text` — the one with the most fixed
 *  wording, so "Ready · built-in, runs locally" beats "Ready · {model}". */
function matchWhole(text: string, prefix: string): Hit | null {
  const flat = squash(text);
  let best: { m: Matcher; groups: RegExpExecArray } | null = null;
  for (const m of matchers(prefix)) {
    const groups = m.re.exec(flat);
    if (groups && (!best || m.literal > best.m.literal)) best = { m, groups };
  }
  if (!best) return null;
  const params: Record<string, string> = {};
  best.m.names.forEach((name, i) => (params[name] = best!.groups[i + 1]!));
  return { key: best.m.key, params };
}

const say = (key: Key, params: Record<string, string>): string =>
  (t as (k: Key, p?: Record<string, string>) => string)(key, Object.keys(params).length ? params : undefined);

/** A matched sentence in the active language. The values the core formatted in
 *  are themselves translated when they are a known phrase ("tagging", "the
 *  import"); paths, names and numbers pass through. */
function render(hit: Hit): string {
  const params: Record<string, string> = {};
  for (const [name, value] of Object.entries(hit.params)) {
    const inner = matchWhole(value, "core.msg.");
    params[name] = inner && Object.keys(inner.params).length === 0 ? say(inner.key, {}) : value;
  }
  return say(hit.key, params);
}

/**
 * A message from the core, in the active language when it is one the interface
 * knows. An error that arrives as a chain — `"outer: inner"`, how `anyhow`
 * joins its context — is translated at the segment that is known and the rest
 * kept, so a file name in front or an operating-system detail behind survives.
 * Anything else comes back unchanged.
 */
export function translateCoreError(message: string): string {
  if (!message) return message;
  const classified = parseCoreError(message);
  if (classified) return describeClassified(classified);
  const whole = matchWhole(message, "core.msg.");
  if (whole) return render(whole);
  const parts = message.split(": ");
  if (parts.length < 2) return message;
  const head = matchWhole(parts[0]!, "core.msg.");
  if (head) return `${render(head)}: ${parts.slice(1).join(": ")}`;
  for (let i = 1; i < parts.length; i++) {
    const tail = matchWhole(parts.slice(i).join(": "), "core.msg.");
    if (tail) return `${parts.slice(0, i).join(": ")}: ${render(tail)}`;
  }
  return message;
}

/** For the status sentences (`Readiness.detail`, `blocked`, a requirement's
 *  label, `ProviderStatus.detail`): the same table, under a name that says what
 *  it is used for. */
export const translateCoreDetail = translateCoreError;

/**
 * What a job's progress event says it is doing. `phase` is a stable token, and
 * for an export the message is the file being written — data, kept as it is.
 * Otherwise a known message is reworded and any other text (an abort reason)
 * goes through the error table.
 */
export function progressText(p: { phase: string; message: string }): string {
  if (p.phase === "export" || !p.message) return p.message;
  const hit = matchWhole(p.message, "core.progress.");
  return hit ? say(hit.key, {}) : translateCoreError(p.message);
}

/** An on-device model's name, by its stable id: the name of the feature it
 *  serves, in sentence case and the user's language. The core's own name
 *  ("Search & Auto-Tagging") when the id is a new one. */
export function modelName(id: string, fallback: string): string {
  const key = `core.model.${id}.name`;
  return key in en ? say(key as Key, {}) : fallback;
}

/** What an on-device model is for, by its stable id; the core's own text when
 *  the id is one this version of the interface has not heard of. */
export function modelPurpose(id: string, fallback: string): string {
  const key = `core.model.${id}.purpose`;
  return key in en ? say(key as Key, {}) : fallback;
}

// ---------------------------------------------------------------------------
// Classified failures

/**
 * A failure the core classified (`src-tauri/src/errors.rs`): the kind of its
 * root cause, the file it concerned when a context named one, an HTTP status,
 * and the English cause chain it would otherwise have sent.
 */
export interface CoreFailure {
  kind: string;
  file?: string;
  status?: number;
  text: string;
}

/** The classified failure in a command's error text, or null for a plain message. */
export function parseCoreError(message: string): CoreFailure | null {
  if (!message.startsWith('{"kind":')) return null;
  try {
    const v = JSON.parse(message) as Partial<CoreFailure>;
    return typeof v.kind === "string" && typeof v.text === "string" ? (v as CoreFailure) : null;
  } catch {
    return null;
  }
}

/** The English an error arrived in — what the log records, whatever the language. */
export function coreErrorEnglish(message: string): string {
  return parseCoreError(message)?.text ?? message;
}

/** Each kind's sentence, and the one that names the file when the core found it. */
const KINDS: Record<string, [Key, Key?]> = {
  notFound: ["core.sys.notFound", "core.sys.notFoundFile"],
  accessDenied: ["core.sys.accessDenied", "core.sys.accessDeniedFile"],
  fileInUse: ["core.sys.fileInUse", "core.sys.fileInUseFile"],
  readOnly: ["core.sys.readOnly", "core.sys.readOnlyFile"],
  alreadyExists: ["core.sys.alreadyExists", "core.sys.alreadyExistsFile"],
  fileDamaged: ["core.sys.fileDamaged", "core.sys.fileDamagedFile"],
  rawUnsupported: ["core.sys.rawUnsupported", "core.sys.rawUnsupportedFile"],
  imageUnsupported: ["core.sys.imageUnsupported", "core.sys.imageUnsupportedFile"],
  diskFull: ["core.sys.diskFull"],
  driveMissing: ["core.sys.driveMissing"],
  outOfMemory: ["core.sys.outOfMemory"],
  catalogueBusy: ["core.sys.catalogueBusy"],
  catalogueDamaged: ["core.sys.catalogueDamaged"],
  catalogueReadOnly: ["core.sys.catalogueReadOnly"],
  catalogueCannotOpen: ["core.sys.catalogueCannotOpen"],
  catalogue: ["core.sys.catalogue"],
  offline: ["core.sys.offline"],
  timedOut: ["core.sys.timedOut"],
  network: ["core.sys.network"],
  aiRuntime: ["core.sys.aiRuntime"],
  raw: ["core.sys.raw", "core.sys.rawFile"],
  image: ["core.sys.image", "core.sys.imageFile"],
  io: ["core.sys.io", "core.sys.ioFile"],
};

/**
 * A classified failure in the user's language: what went wrong, naming the
 * file where the core found one — after the core's own authored sentence for
 * what it was doing, when that is one the table knows ("Couldn't read …").
 * A kind this build does not know yet reads as the English it came with.
 */
function describeClassified(f: CoreFailure): string {
  const head = f.text.split(": ")[0] ?? "";
  const known = head && head !== f.text ? matchWhole(head, "core.msg.") : null;
  let sentence: string;
  if (f.kind === "httpStatus" && f.status) {
    sentence = t("core.sys.httpStatus", { status: String(f.status) });
  } else {
    const keys = KINDS[f.kind];
    if (!keys) return translateCoreError(f.text);
    const [plain, withFile] = keys;
    // A known context already names the path; the cause need not say it twice.
    sentence = f.file && withFile && !known ? say(withFile, { file: f.file }) : say(plain, {});
  }
  return known ? t("core.sys.withContext", { context: render(known), cause: sentence }) : sentence;
}
