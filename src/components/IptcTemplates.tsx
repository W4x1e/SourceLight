import { useEffect, useState } from "react";
import { formatList, t } from "../i18n";
import { api } from "../lib/api";
import { confirm, toast } from "../lib/dialogs";
import type { IptcFields, IptcTemplate } from "../bindings";
import { IPTC_FIELDS } from "./MetadataSection";

const BLANK: IptcFields = {
  title: null,
  caption: null,
  creator: null,
  copyright: null,
  credit: null,
  usageTerms: null,
};

function Editor({
  template,
  onDone,
}: {
  template: IptcTemplate | null;
  onDone: (saved: boolean) => void;
}) {
  const [name, setName] = useState(template?.name ?? "");
  const [fields, setFields] = useState<IptcFields>(template?.fields ?? BLANK);
  const save = async () => {
    try {
      await api.saveIptcTemplate(template?.id ?? null, name, fields);
      onDone(true);
    } catch (e) {
      toast(String(e).replace(/^Error: /, ""), "error");
    }
  };
  return (
    <div className="flex flex-col gap-2 rounded-md bg-raised p-3">
      <input
        className="input"
        placeholder={t("library.iptc.templateName")}
        aria-label={t("library.iptc.templateName")}
        value={name}
        onChange={(e) => setName(e.target.value)}
        autoFocus
      />
      {IPTC_FIELDS.map((f) =>
        f.long ? (
          <textarea
            key={f.id}
            rows={2}
            className="input resize-y leading-snug"
            placeholder={t("library.iptc.fieldBlank", { field: t(f.label) })}
            aria-label={t(f.label)}
            value={fields[f.key] ?? ""}
            onChange={(e) => setFields({ ...fields, [f.key]: e.target.value || null })}
          />
        ) : (
          <input
            key={f.id}
            className="input"
            placeholder={t("library.iptc.fieldBlank", { field: t(f.label) })}
            aria-label={t(f.label)}
            value={fields[f.key] ?? ""}
            onChange={(e) => setFields({ ...fields, [f.key]: e.target.value || null })}
          />
        ),
      )}
      <div className="flex justify-end gap-2 pt-1">
        <button className="btn-ghost" onClick={() => onDone(false)}>
          {t("common.cancel")}
        </button>
        <button className="btn-accent" disabled={!name.trim()} onClick={() => void save()}>
          {t("common.save")}
        </button>
      </div>
    </div>
  );
}

/**
 * Templates of a photo's title, caption and rights: named sets of
 * fields to put on photos at import or on a selection. Beside the Settings
 * creator and copyright, which stay the default for photos with none.
 */
export default function IptcTemplates() {
  const [templates, setTemplates] = useState<IptcTemplate[]>([]);
  // The template being edited, "new" for a new one, or none.
  const [editing, setEditing] = useState<number | "new" | null>(null);
  const load = () => void api.listIptcTemplates().then(setTemplates).catch(() => setTemplates([]));
  useEffect(load, []);

  const done = (saved: boolean) => {
    setEditing(null);
    if (saved) load();
  };
  const remove = async (tpl: IptcTemplate) => {
    const ok = await confirm({
      title: t("library.iptc.deleteTitle", { name: tpl.name }),
      message: t("library.iptc.deleteMessage"),
      confirmLabel: t("common.delete"),
      danger: true,
    });
    if (!ok) return;
    await api.deleteIptcTemplate(tpl.id);
    load();
  };

  return (
    <div>
      <div className="section-label mb-1.5 mt-5 px-0">{t("library.iptc.templates")}</div>
      <p className="mb-2 text-label leading-relaxed text-faint">{t("library.iptc.templatesBlurb")}</p>
      <div className="flex flex-col gap-1.5">
        {templates.map((tpl) =>
          editing === tpl.id ? (
            <Editor key={tpl.id} template={tpl} onDone={done} />
          ) : (
            <div key={tpl.id} className="flex items-center gap-2 rounded-md bg-raised px-3 py-2">
              <div className="min-w-0 flex-1">
                <div className="truncate text-body text-ink">{tpl.name}</div>
                <div className="truncate text-label text-faint">
                  {formatList(IPTC_FIELDS.filter((f) => tpl.fields[f.key]).map((f) => t(f.label))) ||
                    t("library.iptc.noFields")}
                </div>
              </div>
              <button className="btn-ghost text-muted" onClick={() => setEditing(tpl.id)}>
                {t("common.edit")}
              </button>
              <button className="btn-ghost text-muted" onClick={() => void remove(tpl)}>
                {t("common.delete")}
              </button>
            </div>
          ),
        )}
        {editing === "new" ? (
          <Editor template={null} onDone={done} />
        ) : (
          <button className="btn-ghost self-start" onClick={() => setEditing("new")}>
            {t("library.iptc.newTemplate")}
          </button>
        )}
      </div>
    </div>
  );
}
