import PhotoDescription from "./PhotoDescription";
import type { Photo } from "../types";
import { t } from "../i18n";
import { tagName } from "../i18n/tags";
import { sentenceCase } from "../lib/format";
import { localPlaceName } from "../lib/countries";

/**
 * Read-only capture metadata for one photo — the EXIF/classification block
 * shared by the Lightbox sidebar and the library selection panel, so the two
 * describe a photo the same way and a change lands in one place.
 *
 * Interactive state (rating, flags, colour label, tags) stays with each surface,
 * since those differ (a selection acts on many photos, the lightbox on one).
 */

function Row({ label, value }: { label: string; value: React.ReactNode }) {
  if (value === null || value === undefined || value === "") return null;
  return (
    <div className="flex justify-between gap-3 py-[3px]">
      <span className="shrink-0 text-faint">{label}</span>
      <span className="truncate text-right text-muted" title={String(value)}>
        {value}
      </span>
    </div>
  );
}

export default function PhotoMeta({ photo, showFile = false }: { photo: Photo; showFile?: boolean }) {
  return (
    <div className="text-caption">
      <div className="section-label px-0">{t("library.meta.capture")}</div>
      <Row label={t("library.meta.date")} value={photo.captureDate?.replace(" ", " · ")} />
      <Row label={t("library.meta.camera")} value={photo.camera} />
      <Row label={t("library.meta.lens")} value={photo.lens} />
      <Row label={t("library.meta.iso")} value={photo.iso} />
      <Row label={t("library.meta.aperture")} value={photo.aperture ? t("library.meta.apertureValue", { value: photo.aperture }) : null} />
      <Row label={t("library.meta.shutter")} value={photo.shutter ? t("library.meta.shutterValue", { value: photo.shutter }) : null} />
      <Row label={t("library.meta.focal")} value={photo.focalLength ? t("library.meta.focalValue", { value: photo.focalLength }) : null} />
      <Row label={t("library.meta.size")} value={t("library.meta.sizeValue", { width: String(photo.width), height: String(photo.height) })} />

      {(photo.genre || photo.place) && (
        <>
          <div className="section-label mt-4 px-0">{t("library.meta.classification")}</div>
          <Row label={t("library.meta.genre")} value={photo.genre && sentenceCase(tagName(photo.genre))} />
          <Row label={t("library.meta.place")} value={photo.place ? localPlaceName(photo.place) : photo.place} />
        </>
      )}

      <PhotoDescription photo={photo} />

      {showFile && (
        <>
          <div className="section-label mt-4 px-0">{t("library.meta.file")}</div>
          <div className="break-all text-label text-faint">{photo.path}</div>
        </>
      )}
    </div>
  );
}
