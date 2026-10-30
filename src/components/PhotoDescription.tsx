import { useEffect, useState } from "react";
import { api } from "../lib/api";
import { useStore } from "../store";
import type { IptcFields } from "../bindings";
import type { Photo } from "../types";
import { t } from "../i18n";

/**
 * A photo's title and caption: the caption someone wrote in place of
 * the vision model's, which shows only when there is none. Read for the one
 * photo on show, the vision model's too: the grid's own query carries
 * neither. Read again when the photo's row is (a reload after captioning).
 */
export default function PhotoDescription({ photo }: { photo: Photo }) {
  const version = useStore((s) => s.iptcVersion);
  const [own, setOwn] = useState<IptcFields | null>(null);
  const [described, setDescribed] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    api
      .photoIptc(photo.id)
      .then((f) => live && setOwn(f))
      .catch(() => live && setOwn(null));
    api
      .getPhoto(photo.id)
      .then((p) => live && setDescribed(p?.caption ?? null))
      .catch(() => live && setDescribed(null));
    return () => {
      live = false;
    };
  }, [photo, version]);

  const title = own?.title;
  const caption = own?.caption ?? described ?? photo.caption;
  if (!title && !caption) return null;
  return (
    <>
      {/* Whose words they are, on the page and not only in a tooltip. */}
      <div className="section-label mt-4 px-0">
        {own?.caption || !caption ? t("library.description.title") : t("library.description.ai")}
      </div>
      {title && <p className="text-caption font-medium text-ink">{title}</p>}
      {caption && (
        <p
          className="text-caption leading-relaxed text-muted"
          title={own?.caption ? undefined : t("library.description.aiHint")}
        >
          {caption}
        </p>
      )}
    </>
  );
}
