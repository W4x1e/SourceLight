import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, fileUrl } from "../lib/api";
import { toast } from "../lib/dialogs";
import { Icon } from "../lib/icons";
import { usePersonPick } from "../lib/personPick";
import { useStore } from "../store";
import { intlLocale, t } from "../i18n";
import type { Person, PhotoPerson } from "../bindings";
import FaceCrop from "./FaceCrop";

const personName = (p: Person) => p.name ?? t("people.unnamed");

/** A person's face as a small round avatar — how an unnamed person is told apart. */
function Avatar({ person, size }: { person: Person; size: number }) {
  return (
    <span className="shrink-0 overflow-hidden rounded-full" style={{ width: size, height: size }}>
      <FaceCrop
        src={person.coverPath ? fileUrl(person.coverPath) : null}
        box={person.coverBox}
        portrait={person.portraitPath}
        size={size}
      />
    </span>
  );
}

/**
 * Who is in the selected photos, and a way to say who else is.
 *
 * The People page can only group the faces the detector found. This is the
 * other half: a person put in photos by hand — the back of a head, a face in
 * shadow, someone the detector never found at all — without the models being
 * asked. Chips show the focused photo's people, as Keywords shows its tags;
 * adding applies to the whole selection.
 */
export default function PeopleSection() {
  const selected = useStore((s) => s.selected);
  const focusId = useStore((s) => s.focusId);
  const filter = useStore((s) => s.filter);
  const reload = useStore((s) => s.reload);
  // Changes made elsewhere (the pick banner) show here too.
  const edits = usePersonPick((s) => s.edits);

  const focus = focusId ?? [...selected][0] ?? null;
  const [people, setPeople] = useState<Person[] | null>(null);
  const [inPhoto, setInPhoto] = useState<PhotoPerson[]>([]);
  const [adding, setAdding] = useState(false);
  const [query, setQuery] = useState("");
  const [working, setWorking] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  const refresh = useCallback(async () => {
    setPeople(await api.listPeople().catch(() => []));
    setInPhoto(focus === null ? [] : await api.photoPeople(focus).catch(() => []));
    // eslint-disable-next-line react-hooks/exhaustive-deps -- `edits` is the refresh signal
  }, [focus, edits]);
  useEffect(() => void refresh(), [refresh]);

  useEffect(() => {
    if (adding) inputRef.current?.focus();
  }, [adding]);

  const byId = useMemo(() => new Map((people ?? []).map((p) => [p.id, p])), [people]);

  // Named people who match first, then — only while nothing is typed, since an
  // unnamed person cannot match a name — the unnamed, most-photographed first.
  // Hidden people are left out: hiding them said they are not wanted.
  const matches = useMemo(() => {
    const q = query.trim().toLowerCase();
    const visible = (people ?? []).filter((p) => !p.hidden);
    const named = visible
      .filter((p) => p.name && (!q || p.name.toLowerCase().includes(q)))
      .sort((a, b) => a.name!.localeCompare(b.name!, intlLocale()));
    const unnamed = q ? [] : visible.filter((p) => !p.name).slice(0, 30);
    return [...named, ...unnamed];
  }, [people, query]);

  const ids = [...selected];
  const many = ids.length > 1;
  // After a change, anything showing a person's photos needs to hear of it.
  const changed = async () => {
    await refresh();
    if (filter.personId != null || (filter.personIds?.length ?? 0) > 0) await reload();
  };

  const addTo = async (person: Person) => {
    setWorking(true);
    try {
      const n = await api.addPhotosToPerson(ids, person.id);
      const who = personName(person);
      toast(
        n === 0
          ? t(many ? "people.section.alreadyInAll" : "people.section.alreadyInThis", { name: who })
          : t("people.toast.added", { name: who, count: n }),
        n === 0 ? "info" : "success",
      );
      setAdding(false);
      setQuery("");
      await changed();
    } catch (e) {
      toast(t("people.toast.couldNotAdd", { error: String(e) }), "error");
    } finally {
      setWorking(false);
    }
  };

  const addNew = async () => {
    const name = query.trim();
    if (!name) return;
    setWorking(true);
    try {
      await api.addPhotosToNewPerson(ids, name);
      toast(t("people.toast.added", { name, count: ids.length }), "success");
      setAdding(false);
      setQuery("");
      await changed();
    } catch (e) {
      toast(t("people.toast.couldNotAdd", { error: String(e) }), "error");
    } finally {
      setWorking(false);
    }
  };

  const remove = async (id: number) => {
    if (focus === null) return;
    setWorking(true);
    try {
      await api.removePhotosFromPerson([focus], id);
      await changed();
    } catch (e) {
      toast(t("people.section.couldNotRemove", { error: String(e) }), "error");
    } finally {
      setWorking(false);
    }
  };

  return (
    <div className="border-b border-line/60 px-4 py-3.5">
      <div className="section-label px-0 pb-2">{t("people.section.title")}</div>

      {inPhoto.length > 0 && (
        <div className="mb-2 flex flex-wrap gap-1">
          {inPhoto.map(({ id, manual }) => {
            const p = byId.get(id);
            if (!p) return null;
            return (
              <span
                key={id}
                className="chip pl-1"
                title={manual ? t("people.section.addedByYou") : t("people.section.foundByDetection")}
              >
                <Avatar person={p} size={18} />
                <span className={p.name ? "" : "text-faint"}>{personName(p)}</span>
                <button
                  onClick={() => void remove(id)}
                  disabled={working}
                  className="chip-x"
                  aria-label={t("people.section.notInPhotoAria", { name: personName(p) })}
                  title={t("people.section.notInPhoto")}
                >
                  <Icon name="close" size={11} strokeWidth={2.4} />
                </button>
              </span>
            );
          })}
        </div>
      )}

      {adding ? (
        <div>
          <input
            ref={inputRef}
            className="input"
            placeholder={t("people.section.findPlaceholder")}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.stopPropagation();
                setAdding(false);
                setQuery("");
              } else if (e.key === "Enter") {
                // An exact name is that person; anything else is someone new.
                const exact = matches.find((p) => p.name?.toLowerCase() === query.trim().toLowerCase());
                if (exact) void addTo(exact);
                else void addNew();
              }
            }}
          />
          <div className="mt-1 max-h-56 overflow-y-auto rounded-md bg-raised py-1">
            {matches.map((p) => (
              <button
                key={p.id}
                onClick={() => void addTo(p)}
                disabled={working}
                className="flex w-full items-center gap-2 px-2 py-1 text-left text-caption text-muted hover:bg-hover hover:text-ink"
              >
                <Avatar person={p} size={24} />
                <span className={`min-w-0 flex-1 truncate ${p.name ? "" : "text-faint"}`}>{personName(p)}</span>
                <span className="shrink-0 text-label tabular-nums text-faint">{p.count}</span>
              </button>
            ))}
            {query.trim() && !matches.some((p) => p.name?.toLowerCase() === query.trim().toLowerCase()) && (
              <button
                onClick={() => void addNew()}
                disabled={working}
                className="flex w-full items-center gap-2 px-2 py-1.5 text-left text-caption text-accent hover:bg-hover"
              >
                <Icon name="plus" size={14} />
                <span className="truncate">{t("people.section.newPerson", { name: query.trim() })}</span>
              </button>
            )}
            {matches.length === 0 && !query.trim() && (
              <div className="px-2 py-1.5 text-caption text-faint">
                {t("people.section.noPeopleYet")}
              </div>
            )}
          </div>
        </div>
      ) : (
        <button
          onClick={() => setAdding(true)}
          className="btn-solid gap-1.5 text-caption"
          title={t("people.section.addHint")}
        >
          <Icon name="plus" size={13} />
          {many ? t("people.section.addPersonMany", { count: ids.length }) : t("people.section.addPerson")}
        </button>
      )}
    </div>
  );
}
