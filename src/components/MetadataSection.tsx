import { useEffect, useState } from "react";
import { api } from "../lib/api";
import { useStore } from "../store";
import { prompt, toast } from "../lib/dialogs";
import type { IptcField, IptcFields, IptcSelection, IptcTemplate, SharedField } from "../bindings";
import { t, type Key } from "../i18n";

/** The fields, in the order the panel shows them, with what each is for. */
export const IPTC_FIELDS: { id: IptcField; key: keyof IptcFields; label: Key; hint: Key; long?: boolean }[] = [
  { id: "title", key: "title", label: "library.iptc.title", hint: "library.iptc.titleHint" },
  { id: "caption", key: "caption", label: "library.iptc.caption", hint: "library.iptc.captionHint", long: true },
  { id: "creator", key: "creator", label: "library.iptc.creator", hint: "library.iptc.creatorHint" },
  { id: "copyright", key: "copyright", label: "library.iptc.copyright", hint: "library.iptc.copyrightHint" },
  { id: "credit", key: "credit", label: "library.iptc.credit", hint: "library.iptc.creditHint" },
  { id: "usageTerms", key: "usageTerms", label: "library.iptc.usageTerms", hint: "library.iptc.usageTermsHint" },
];

function Field({
  label,
  hint,
  long,
  shared,
  onSave,
}: {
  label: string;
  hint: string;
  long?: boolean;
  shared: SharedField;
  onSave: (value: string) => Promise<void>;
}) {
  const [draft, setDraft] = useState(shared.value ?? "");
  // A new selection, or a save elsewhere, shows what is there now.
  useEffect(() => setDraft(shared.value ?? ""), [shared.value, shared.mixed]);
  const save = () => {
    const next = draft.trim();
    // Unchanged saves nothing; a mixed field left empty leaves each photo's own.
    if (shared.mixed ? next === "" : next === (shared.value ?? "").trim()) return;
    void onSave(draft);
  };
  const props = {
    className: "input",
    value: draft,
    placeholder: shared.mixed ? t("library.iptc.mixedHint") : hint,
    "aria-label": label,
    onChange: (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => setDraft(e.target.value),
    onBlur: save,
    onKeyDown: (e: React.KeyboardEvent) => {
      if (e.key === "Enter" && (!long || e.ctrlKey || e.metaKey)) {
        e.preventDefault();
        (e.target as HTMLElement).blur();
      } else if (e.key === "Escape") {
        setDraft(shared.value ?? "");
      }
    },
  };
  return (
    <label className="block">
      <span className="mb-1 flex items-center gap-1.5 text-label text-faint">
        {label}
        {shared.mixed && (
          <span className="rounded-sm bg-raised px-1 py-px text-label font-medium text-faint">{t("library.iptc.mixed")}</span>
        )}
      </span>
      {long ? <textarea rows={3} {...props} className="input resize-y leading-snug" /> : <input {...props} />}
    </label>
  );
}

/**
 * A photo's own title, caption and rights, for the selection: each
 * field saved as it is left, across every selected photo. Fields that differ
 * say so; typing into one sets them all. A template fills in the fields it
 * has; the current values can be kept as one.
 */
export default function MetadataSection({ ids }: { ids: number[] }) {
  const [sel, setSel] = useState<IptcSelection | null>(null);
  const [templates, setTemplates] = useState<IptcTemplate[]>([]);
  const [open, setOpen] = useState(false);

  const bump = useStore((s) => s.bumpIptc);
  const load = () =>
    api
      .iptcSelection(ids)
      .then(setSel)
      .catch(() => setSel(null));
  const changed = async () => {
    bump();
    await load();
  };
  useEffect(() => {
    if (ids.length === 0) return;
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ids]);
  useEffect(() => {
    if (open) void api.listIptcTemplates().then(setTemplates).catch(() => setTemplates([]));
  }, [open]);

  const many = ids.length > 1;
  const set = async (field: IptcField, value: string) => {
    try {
      await api.setIptcField(ids, field, value.trim() === "" ? null : value);
      await changed();
    } catch (e) {
      toast(String(e).replace(/^Error: /, ""), "error");
    }
  };
  const apply = async (tpl: IptcTemplate) => {
    try {
      const n = await api.applyIptcTemplate(ids, null, tpl.id);
      toast(t("library.iptc.applied", { name: tpl.name, count: n }), "success");
      await changed();
    } catch (e) {
      toast(String(e).replace(/^Error: /, ""), "error");
    }
  };
  const keep = async () => {
    if (!sel) return;
    const name = await prompt({
      title: t("library.iptc.keepTitle"),
      message: t("library.iptc.keepMessage"),
      placeholder: t("library.iptc.keepPlaceholder"),
      confirmLabel: t("library.iptc.keepConfirm"),
    });
    if (!name?.trim()) return;
    const fields = Object.fromEntries(
      IPTC_FIELDS.map((f) => {
        const s = sel[f.key as keyof IptcSelection];
        return [f.key, s.mixed ? null : s.value];
      }),
    ) as IptcFields;
    try {
      await api.saveIptcTemplate(null, name, fields);
      toast(t("library.iptc.kept", { name: name.trim() }), "success");
      setTemplates(await api.listIptcTemplates());
    } catch (e) {
      toast(String(e).replace(/^Error: /, ""), "error");
    }
  };

  const filled = sel ? IPTC_FIELDS.filter((f) => sel[f.key as keyof IptcSelection].value).length : 0;

  return (
    <div className="border-b border-line/60 px-4 py-3.5">
      <button
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
        className="section-label flex w-full items-center justify-between px-0 pb-2 text-left"
      >
        <span>{t("library.iptc.section")}</span>
        <span className="text-label font-normal normal-case tracking-normal text-faint">
          {open
            ? t("library.iptc.hide")
            : filled > 0
              ? t("library.iptc.set", { count: filled })
              : many
                ? t("library.iptc.forPhotos", { count: ids.length })
                : t("library.iptc.add")}
        </span>
      </button>
      {open && sel && (
        <div className="flex flex-col gap-2.5">
          {IPTC_FIELDS.map((f) => (
            <Field
              key={f.id}
              label={t(f.label)}
              hint={t(f.hint)}
              long={f.long}
              shared={sel[f.key as keyof IptcSelection]}
              onSave={(v) => set(f.id, v)}
            />
          ))}
          <div className="flex flex-wrap items-center gap-2 pt-0.5">
            {templates.length > 0 && (
              <select
                className="input w-auto flex-1"
                value=""
                aria-label={t("library.iptc.apply")}
                onChange={(e) => {
                  const tpl = templates.find((x) => String(x.id) === e.target.value);
                  if (tpl) void apply(tpl);
                }}
              >
                <option value="">{t("library.iptc.applyMenu")}</option>
                {templates.map((tpl) => (
                  <option key={tpl.id} value={tpl.id}>
                    {tpl.name}
                  </option>
                ))}
              </select>
            )}
            <button onClick={() => void keep()} className="text-caption text-accent hover:underline">
              {t("library.iptc.keep")}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
