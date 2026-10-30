import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { setLocale } from "../i18n";
import { en } from "../i18n/en";
import { coreErrorEnglish, modelName, modelPurpose, progressText, translateCoreError } from "./coreText";

afterEach(() => setLocale("en"));

describe("translateCoreError", () => {
  it("leaves English as it is", () => {
    expect(translateCoreError("On-device AI is switched off — turn it back on in Settings → AI")).toBe(
      en["core.msg.aiSwitchedOff"],
    );
    expect(translateCoreError("photos is an isn't a folder")).toBe("photos is an isn't a folder");
  });

  it("passes an unknown message through unchanged", () => {
    expect(translateCoreError("the disk is on fire")).toBe("the disk is on fire");
    expect(translateCoreError("")).toBe("");
    expect(translateCoreError("Cancelled")).toBe("Cancelled");
  });

  it("words a known message and fills in its parameters", async () => {
    await setLocale("de");
    expect(translateCoreError("Holiday isn't a folder — drop the folder your photos are in")).toBe(
      "Holiday ist kein Ordner – ziehen Sie den Ordner mit Ihren Fotos hierher",
    );
    expect(translateCoreError("a place needs a name")).toBe("Ein Ort braucht einen Namen");
    expect(translateCoreError("the llava vision model failed to load")).toBe(
      "Das Bildmodell llava konnte nicht geladen werden",
    );
  });

  it("translates the known segment of a chain and keeps the rest", async () => {
    await setLocale("de");
    // A file name in front, as a scan failure reads.
    expect(translateCoreError("IMG_0002.HEIC: HEIC is not read on this system yet: C:\\x.heic")).toContain(
      "IMG_0002.HEIC: HEIC wird auf diesem System noch nicht gelesen: C:\\x.heic",
    );
    // An operating-system detail behind.
    expect(translateCoreError("cannot write D:\\out\\a.jpg: Access is denied. (os error 5)")).toBe(
      "D:\\out\\a.jpg kann nicht geschrieben werden: Access is denied. (os error 5)",
    );
  });

  it("words a feature's name inside a sentence", async () => {
    await setLocale("de");
    expect(translateCoreError("Ready · tagging runs on this machine")).toBe(
      "Bereit · Verschlagwortung läuft auf diesem Gerät",
    );
    expect(translateCoreError("Ready · built-in, runs locally")).toBe("Bereit · integriert, läuft lokal");
    expect(translateCoreError("Ready · qwen3-vl:4b")).toBe("Bereit · qwen3-vl:4b");
    expect(translateCoreError("the Qwen3-VL 4B vision model")).toBe("das Bildmodell Qwen3-VL 4B");
    expect(translateCoreError("the Search & Auto-Tagging model")).toBe("das Modell Search & Auto-Tagging");
  });
});

describe("progressText", () => {
  it("words a known job message", async () => {
    expect(progressText({ phase: "import", message: "Importing" })).toBe("Importing");
    await setLocale("de");
    expect(progressText({ phase: "import", message: "Importing" })).toBe("Importieren");
    expect(progressText({ phase: "done", message: "Faces analysed" })).toBe("Gesichter analysiert");
  });

  it("keeps an export's file name", async () => {
    await setLocale("de");
    expect(progressText({ phase: "export", message: "Importing" })).toBe("Importing");
    expect(progressText({ phase: "export", message: "IMG_0001.jpg" })).toBe("IMG_0001.jpg");
  });

  it("shows an abort reason through the error table, and anything else as it came", async () => {
    await setLocale("de");
    expect(progressText({ phase: "done", message: "the model returned an empty caption" })).toBe(
      "Das Modell hat eine leere Beschreibung geliefert",
    );
    expect(progressText({ phase: "done", message: "" })).toBe("");
    expect(progressText({ phase: "scan", message: "something new" })).toBe("something new");
  });
});

describe("modelPurpose", () => {
  it("is keyed by the model's id, with the core's text for an unknown one", async () => {
    await setLocale("de");
    expect(modelPurpose("faces", "x")).toBe(
      "Findet Gesichter und markiert beim Sichten geschlossene Augen und unscharfe Gesichter",
    );
    expect(modelPurpose("brand-new", "What the core says")).toBe("What the core says");
  });
});

describe("modelName", () => {
  it("names a model as the feature it serves, in sentence case and the user's language", async () => {
    // The core's names are English title case: "Search & Auto-Tagging".
    expect(modelName("tagger", "Search & Auto-Tagging")).toBe("Search and tagging");
    expect(modelName("subject", "Select Subject")).toBe("Select subject");
    await setLocale("de");
    expect(modelName("subject", "Select Subject")).toBe("Motiv auswählen");
    expect(modelName("brand-new", "Brand New")).toBe("Brand New");
  });
});

// --- The guard ----------------------------------------------------------
//
// `translateCoreError` and `progressText` recognise the core's sentences by
// their English wording, which lives in `core.msg.*` / `core.progress.*` in
// `src/i18n/en/core.ts`. If someone rewords a message in Rust, the sentence
// stops matching and silently shows untranslated. These read the Rust sources
// and fail instead.

const ROOT = join(__dirname, "..", "..");

function rustFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return name === "bin" ? [] : rustFiles(path);
    return name.endsWith(".rs") ? [path] : [];
  });
}

const squash = (s: string) => s.replace(/\s+/g, " ").trim();

/** Every string literal in the core, with line continuations joined the way
 *  the compiler does and whitespace runs squashed. */
function rustLiterals(): string[] {
  const out: string[] = [];
  // The app's own sources, and the develop core it shares with the phone app.
  const roots = [join(ROOT, "src-tauri", "src"), join(ROOT, "src-tauri", "crates", "sl-develop", "src")];
  for (const file of roots.flatMap(rustFiles)) {
    const source = readFileSync(file, "utf8").replace(/\\\r?\n\s*/g, "");
    for (const m of source.matchAll(/"((?:[^"\\\n]|\\.)*)"/g)) out.push(squash(m[1]!));
  }
  return out;
}

describe("the core still says what the table maps", () => {
  const literals = rustLiterals();
  const mapped = Object.entries(en).filter(
    ([key]) => key.startsWith("core.msg.") || key.startsWith("core.progress."),
  ) as [string, string][];

  it("finds the Rust sources", () => {
    expect(literals.length).toBeGreaterThan(1000);
  });

  for (const [key, english] of mapped) {
    it(`${key}`, () => {
      const text = squash(english);
      const parts = text.split(/\{\w+\}/);
      const found = literals.some((lit) => {
        // The fixed wording around the placeholders, in order, in one literal.
        let at = 0;
        for (const part of parts) {
          if (part === "") continue;
          const i = lit.indexOf(part, at);
          if (i < 0) return false;
          at = i + part.length;
        }
        // A literal that is the whole message starts and ends as the message does.
        if (parts[0] !== "" && !lit.startsWith(parts[0]!)) return false;
        const last = parts[parts.length - 1]!;
        return last === "" || lit.endsWith(last);
      });
      expect(found, `no Rust string literal says: ${english}`).toBe(true);
    });
  }
});

describe("classified failures (errors.rs)", () => {
  const failure = (v: object) => JSON.stringify(v);
  afterEach(async () => {
    await setLocale("en");
  });

  it("words the root cause, naming the file", async () => {
    const e = failure({ kind: "fileInUse", file: "IMG_2043.ARW", text: "writing /Users/anna/x/IMG_2043.ARW: The process cannot access the file (os error 32)" });
    expect(translateCoreError(e)).toBe("Another program is using IMG_2043.ARW. Close it there and try again.");
    await setLocale("de");
    expect(translateCoreError(e)).toContain("IMG_2043.ARW");
    expect(translateCoreError(e)).not.toContain("os error");
  });

  it("puts a known context in front, without naming the file twice", () => {
    const e = failure({ kind: "notFound", file: "b.gpx", text: "Couldn't read /tracks/b.gpx: No such file or directory (os error 2)" });
    expect(translateCoreError(e)).toBe(
      "Couldn't read /tracks/b.gpx: The file isn't there any more — it may have been moved, renamed or deleted.",
    );
  });

  it("says an HTTP status", () => {
    expect(translateCoreError(failure({ kind: "httpStatus", status: 503, text: "requesting https://x: 503" }))).toContain("503");
  });

  it("falls back to the English for a kind it does not know", () => {
    expect(translateCoreError(failure({ kind: "somethingNew", text: "it broke" }))).toBe("it broke");
  });

  it("keeps the English for the log", () => {
    const e = failure({ kind: "diskFull", text: "writing x: There is not enough space on the disk. (os error 112)" });
    expect(coreErrorEnglish(e)).toBe("writing x: There is not enough space on the disk. (os error 112)");
    expect(coreErrorEnglish("plain")).toBe("plain");
  });

  it("leaves a cancel as the plain word", () => {
    expect(translateCoreError("tagging: Cancelled")).toBe("tagging: Cancelled");
  });
});
