import { useEffect, useState } from "react";
import type { PhotoFace } from "../bindings";
import { api, fileUrl } from "../lib/api";
import { prompt, toast } from "../lib/dialogs";
import { useStore } from "../store";
import { t } from "../i18n";
import type { Photo } from "../types";
import FaceCrop from "./FaceCrop";

/**
 * Who is in the photo, face by face, in the Lightbox — and where a
 * face is named. People could only be seen as chips in the selection panel,
 * with no way to point at a face and say who it is; and a face grouping left
 * out (too small or turned to vote) could not be seen or named anywhere.
 * Every detected face is here, grouped or not.
 *
 * - An unnamed face: **Name…** — see `db::name_face` for what a name does.
 * - A named face: **Not them** takes it out of that person, after which it
 *   can be named for who it really is.
 */
export default function LightboxFaces({ photo, onChanged }: { photo: Photo; onChanged?: () => void }) {
  const [faces, setFaces] = useState<PhotoFace[] | null>(null);
  // Bumped after a change, to read the faces again.
  const [version, setVersion] = useState(0);

  useEffect(() => {
    let live = true;
    api
      .photoFaces([photo.id])
      .then((found) => live && setFaces(found))
      .catch(() => live && setFaces([]));
    return () => {
      live = false;
    };
  }, [photo.id, version]);

  if (!faces || faces.length === 0) return null;

  const changed = () => {
    setVersion((v) => v + 1);
    // The Lightbox's own, which keeps it on this photo.
    if (onChanged) onChanged();
    else void useStore.getState().reload();
  };
  const name = async (f: PhotoFace) => {
    const given = await prompt({
      title: t("people.lightbox.nameTitle"),
      message: f.personId ? t("people.lightbox.nameMessageGrouped") : t("people.lightbox.nameMessageUngrouped"),
      placeholder: t("people.lightbox.namePlaceholder"),
      confirmLabel: t("people.lightbox.nameConfirm"),
    });
    if (!given?.trim()) return;
    try {
      await api.nameFace(f.faceId, given.trim());
      changed();
    } catch (e) {
      toast(String(e), "error");
    }
  };
  const notThem = async (f: PhotoFace) => {
    try {
      await api.detachFace(f.faceId);
      changed();
    } catch (e) {
      toast(String(e), "error");
    }
  };

  return (
    <>
      <div className="section-label mt-4 px-0">{t("people.lightbox.title")}</div>
      <div className="grid grid-cols-3 gap-2 pb-1">
        {faces.map((f) => {
          const label = f.personName ?? (f.personId == null ? t("people.lightbox.notGrouped") : t("people.unnamed"));
          return (
            <div key={f.faceId} className="min-w-0">
              <div className="overflow-hidden rounded-sm">
                <FaceCrop
                  src={fileUrl(photo.thumbPath ?? photo.previewPath)}
                  box={f.boxFrac}
                  portrait={f.portraitPath}
                />
              </div>
              <div
                className={`truncate pt-1 text-label ${f.personName ? "text-muted" : "text-faint"}`}
                title={
                  f.personId == null
                    ? t("people.lightbox.notGroupedHint")
                    : label
                }
              >
                {label}
              </div>
              {f.personName ? (
                <button
                  onClick={() => void notThem(f)}
                  className="text-label text-faint hover:text-danger-ink"
                  title={t("people.lightbox.notThemHint", { name: f.personName })}
                >
                  {t("people.lightbox.notThem")}
                </button>
              ) : (
                <button onClick={() => void name(f)} className="text-label text-accent hover:underline">
                  {t("people.lightbox.nameLink")}
                </button>
              )}
            </div>
          );
        })}
      </div>
    </>
  );
}
