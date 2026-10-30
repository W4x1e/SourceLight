import { useEffect, useState } from "react";
import { t } from "../i18n";
import { api } from "../lib/api";
import IptcTemplates from "./IptcTemplates";

/**
 * The two metadata lines the catalogue cannot know.
 *
 * Everything else an export's EXIF carries — camera, lens, exposure, capture
 * date — comes from the raw. Who took the photo and who owns it has to come
 * from the photographer, once, and then rides along in every file until the
 * export dialog's Strip is chosen.
 *
 * Saves on blur, like the rest of Settings: there is no Save button to forget.
 */
export default function ExportRightsSection() {
  const [creator, setCreator] = useState("");
  const [copyright, setCopyright] = useState("");
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .getExportRights()
      .then((r) => {
        setCreator(r.creator);
        setCopyright(r.copyright);
        setLoaded(true);
      })
      .catch((e) => setError(String(e)));
  }, []);

  const persist = () => {
    // Before the first load, the fields are empty placeholders; writing them
    // would erase whatever was stored.
    if (!loaded) return;
    api
      .setExportRights({ creator, copyright })
      .then(() => setError(null))
      .catch((e) => setError(String(e)));
  };

  return (
    <div>
      <span className="section-label px-0">{t("settings.exportRights.title")}</span>
      <p className="mt-1 text-caption text-muted">
        {t("settings.exportRights.blurb")}
      </p>

      <label className="mt-3 block">
        <span className="text-label text-faint">{t("settings.exportRights.creator")}</span>
        <input
          className="input mt-1"
          value={creator}
          placeholder={t("settings.exportRights.creatorPlaceholder")}
          onChange={(e) => setCreator(e.target.value)}
          onBlur={persist}
        />
      </label>

      <label className="mt-2 block">
        <span className="text-label text-faint">{t("settings.exportRights.copyright")}</span>
        <input
          className="input mt-1"
          value={copyright}
          placeholder={t("settings.exportRights.copyrightPlaceholder", { year: String(new Date().getFullYear()) })}
          onChange={(e) => setCopyright(e.target.value)}
          onBlur={persist}
        />
      </label>

      <p className="mt-3 text-label text-faint">
        {t("settings.exportRights.tiffNote")}
      </p>
      {error && <p className="mt-2 text-label text-danger">{error}</p>}

      <IptcTemplates />
    </div>
  );
}
