import { useEffect, useRef, useState } from "react";
import { api, fileUrl } from "../lib/api";
import { useEscape } from "../lib/useEscape";
import { Icon } from "../lib/icons";
import { toast } from "../lib/dialogs";
import FaceCrop from "./FaceCrop";
import type { Person, PersonFace } from "../bindings";
import { intlLocale, t } from "../i18n";

/**
 * Faces grouping could not place: too small, too turned away or too
 * faint to vote, so no person holds them and, until this, nothing showed them.
 * Largest first. Pick one, type who it is and press Enter, or say it isn't a
 * face at all (the rock, the poster). Named faces join that person.
 */
export default function UnplacedFaces({
  people,
  onChanged,
  onClose,
}: {
  /** Everyone with a name, for the suggestions. */
  people: Person[];
  /** Something was named: the People page reloads. */
  onChanged: () => void;
  onClose: () => void;
}) {
  const [faces, setFaces] = useState<PersonFace[] | null>(null);
  const [total, setTotal] = useState(0);
  const [picked, setPicked] = useState<number | null>(null);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  useEscape(true, onClose);

  useEffect(() => {
    void api
      .unplacedFaces()
      .then(([list, count]) => {
        setFaces(list);
        setTotal(count);
        setPicked(list[0]?.faceId ?? null);
      })
      .catch((e) => {
        toast(t("people.unplaced.couldNotList", { error: String(e) }), "error");
        setFaces([]);
      });
  }, []);

  useEffect(() => {
    if (picked !== null) input.current?.focus();
  }, [picked]);

  /** Takes a face off the list and moves on to the next. */
  const done = (faceId: number) => {
    const list = faces ?? [];
    const at = list.findIndex((f) => f.faceId === faceId);
    const rest = list.filter((f) => f.faceId !== faceId);
    setFaces(rest);
    setPicked(rest[Math.min(Math.max(at, 0), rest.length - 1)]?.faceId ?? null);
    setTotal((n) => Math.max(0, n - 1));
    setName("");
  };

  const nameIt = async () => {
    const who = name.trim();
    if (picked === null || !who || busy) return;
    setBusy(true);
    try {
      await api.nameFace(picked, who);
      done(picked);
      onChanged();
      // What it did: a typo made a new person, and looked the same.
      toast(
        joins ? t("people.unplaced.joined", { name: joins }) : t("people.unplaced.created", { name: who }),
        "success",
      );
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setBusy(false);
    }
  };

  const notAFace = async (faceId: number) => {
    setBusy(true);
    try {
      await api.dismissFace(faceId);
      done(faceId);
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setBusy(false);
    }
  };

  const names = [...new Set(people.map((p) => p.name).filter((n): n is string => !!n))].sort((a, b) =>
    a.localeCompare(b, intlLocale()),
  );
  // The person the typed name joins, as Enter will (case aside), or none: a
  // new person.
  const joins = names.find((n) => n.toLocaleLowerCase() === name.trim().toLocaleLowerCase()) ?? null;

  return (
    <div
      className="anim-scrim fixed inset-0 z-50 grid place-items-center bg-scrim p-8"
      onClick={(e) => e.target === e.currentTarget && onClose()}
    >
      <div
        role="dialog"
        aria-labelledby="unplaced-title"
        className="anim-card flex max-h-[min(760px,calc(100vh-4rem))] w-full max-w-2xl flex-col overflow-hidden rounded-lg border border-line bg-panel shadow-2xl"
      >
        <div className="flex items-center gap-3 border-b border-line px-4 py-3">
          <div className="min-w-0">
            <h2 id="unplaced-title" className="font-display text-display text-ink">
              {t("people.unplaced.title")}
            </h2>
            <p className="text-caption text-faint">
              {faces === null
                ? t("people.unplaced.looking")
                : total === 0
                  ? t("people.unplaced.none")
                  : t("people.unplaced.summary", { count: total })}
            </p>
          </div>
          <button onClick={onClose} className="icon-chip ml-auto shrink-0" aria-label={t("common.close")}>
            <Icon name="close" size={15} />
          </button>
        </div>

        <div className="flex min-h-0 shrink flex-wrap content-start gap-2 overflow-y-auto p-4">
          {(faces ?? []).map((f) => (
            <button
              key={f.faceId}
              onClick={() => setPicked(f.faceId)}
              aria-pressed={picked === f.faceId}
              className={`overflow-hidden rounded-md border-2 transition-colors ${
                picked === f.faceId ? "border-accent" : "border-transparent hover:border-line"
              }`}
              title={t("people.unplaced.nameThisFace")}
            >
              <FaceCrop
                src={f.thumbPath ? fileUrl(f.thumbPath) : null}
                box={f.boxFrac}
                portrait={f.portraitPath}
                size={84}
              />
            </button>
          ))}
        </div>

        {picked !== null && (
          <form
            className="flex items-center gap-2 border-t border-line px-4 py-3"
            onSubmit={(e) => {
              e.preventDefault();
              void nameIt();
            }}
          >
            <input
              ref={input}
              className="input h-8 flex-1"
              placeholder={t("people.unplaced.whoIsThis")}
              list="unplaced-names"
              value={name}
              onChange={(e) => setName(e.target.value)}
              aria-label={t("people.unplaced.nameAria")}
              autoComplete="off"
              spellCheck={false}
            />
            {name.trim() && (
              <span className="shrink-0 text-label text-faint" aria-live="polite">
                {joins
                  ? t("people.unplaced.willJoin", { name: joins })
                  : t("people.unplaced.willCreate", { name: name.trim() })}
              </span>
            )}
            <datalist id="unplaced-names">
              {names.map((n) => (
                <option key={n} value={n} />
              ))}
            </datalist>
            <button type="submit" className="btn-accent" disabled={busy || !name.trim()}>
              {t("people.unplaced.name")}
            </button>
            <button
              type="button"
              className="btn-ghost"
              disabled={busy}
              onClick={() => void notAFace(picked)}
              title={t("people.unplaced.notAFaceHint")}
            >
              {t("people.unplaced.notAFace")}
            </button>
          </form>
        )}
      </div>
    </div>
  );
}
