import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import type { EditStack, PresetGroupInfo } from "../../src/bindings";

export type { PresetGroupInfo };

/** The phone app's Rust commands (mobile/src-tauri/src/link.rs). */

export type LinkStatus = {
  /** When the phone last synced with the computer, ms since the epoch. */
  lastSyncedMs: number | null;
  desktop: string | null;
  connected: boolean;
  relayed: boolean;
  platform: "android" | "ios" | "other";
};
export type Connected = { desktopName: string; relayed: boolean; millis: number };
/** What is waiting here for the next sync. */
export type Waiting = { decisions: number; edits: number; adds: number; tracks: number };
export type Pinged = { millis: number; relayed: boolean };
export type SyncSummary = {
  total: number;
  changed: number;
  /** Phone photos added to the library. */
  added: number;
  /** Edits to library photos the computer took, and those it put on copies
   *  because the photo had been edited there too. */
  editsSent: number;
  editsCopied: number;
  /** Photos sent from the computer and saved to the gallery. */
  received: number;
  /** Why the gallery would not take one. */
  receiveFailed: string | null;
  millis: number;
};
export type TrackStatus = {
  recording: boolean;
  file: string | null;
  points: number;
  /** UTC milliseconds of the newest fix; 0 for none yet. */
  lastFixMs: number;
  /** Metres. */
  lastAccuracy: number;
};
export type Track = { id: number; name: string; startedMs: number; endedMs: number; points: number; sent: boolean };
export type Tracks = { status: TrackStatus; tracks: Track[] };
/** The GPU test's findings (mobile/gpu-test/src/bench.rs). */
export type GpuPhotoTimes = {
  label: string;
  width: number;
  height: number;
  decodeMs: number;
  uploadMs: number;
  frameWidth: number;
  frameHeight: number;
  firstFrameMs: number;
  frameMedianMs: number;
  frameWorstMs: number;
  jpegMs: number;
  fullSizeMs: number;
};
export type GpuReport = {
  adapter: string;
  backend: string;
  driver: string;
  deviceType: string;
  shortOf: string[];
  fallback: string | null;
  deviceMs: number;
  pipelineMs: number;
  photos: GpuPhotoTimes[];
  error: string | null;
};
/** One photo in the grid: id, flag (-1 rejected, 0, 1 picked), rating,
 *  whether it is edited, and where it can be edited (`EDIT_*`). */
export type Tile = [
  id: number,
  flag: number,
  rating: number,
  edited: boolean,
  editOn: number,
  editTag: string | null,
  /** When it was taken, as the catalogue writes it: `YYYY-MM-DD HH:MM:SS`. */
  captured: string | null,
];
/** Where a library photo can be edited (mobile/src-tauri/src/replica.rs):
 *  not said (a computer from before editing on the phone), here too, a raw
 *  (on the computer), or on the computer only. */
export const EDIT_UNKNOWN = 0;
export const EDIT_PHONE = 1;
export const EDIT_RAW = 2;
export const EDIT_COMPUTER = 3;

/** Whether the app may see the phone's own photos: all, only those chosen
 *  (Android 14), or none. */
export type DeviceAccess = "all" | "some" | "none";
export type DeviceAlbum = { id: string; name: string; count: number };
/** One of the phone's own photos: id, taken (ms), modified, upright width and
 *  height, and 1 for a raw (DNG). */
export type DevicePhoto = [id: number, takenMs: number, modified: number, width: number, height: number, raw: number];

/** A phone photo's thumbnail or screen-sized copy (mobile/src-tauri/src/device.rs).
 *  Named with its modified time, so an edited photo is made again. */
export const deviceThumbUrl = (p: DevicePhoto) => convertFileSrc(`${p[0]}_${p[2]}`, "devthumb");
export const devicePreviewUrl = (p: DevicePhoto) => convertFileSrc(`${p[0]}_${p[2]}`, "devpreview");

/** A phone photo as Add to library knows it: waiting (no `libraryId`, no
 *  `error`), added, or not added. */
export type DeviceAdd = { photo: number; libraryId: number | null; error: string | null };

/** Editing a phone photo (mobile/src-tauri/src/edit.rs). `width` and
 *  `height` are the photo upright, before any crop or turn. */
/** An opening for editing. `token` goes back with every change to it: one
 *  from an editor whose photo has gone is refused with `closed`. */
export type EditOpened = {
  token: number;
  stack: EditStack;
  width: number;
  height: number;
  canUndo: boolean;
  canRedo: boolean;
};
export type EditStep = { stack: EditStack; canUndo: boolean; canRedo: boolean };
/** One of the desktop's built-in film looks: the edit's `lut`, by `file`. */
export type LookInfo = { file: string; title: string; family: string; blurb: string };

/** A frame of the photo being edited with `stack` applied, fitted inside
 *  `width` × `height` device pixels. The edit travels in the name. */
export const editFrameUrl = (
  width: number,
  height: number,
  quality: number,
  stack: object,
  /** Zoomed in: `k` times the size that fits, around (`cx`, `cy`), each 0
   *  to 1 across the framed photo. */
  view?: { k: number; cx: number; cy: number },
) =>
  convertFileSrc(`${width}x${height}.${quality}.${base64url(JSON.stringify(stack))}`, "editframe") +
  (view ? `?k=${view.k.toFixed(4)}&cx=${view.cx.toFixed(5)}&cy=${view.cy.toFixed(5)}` : "");

function base64url(text: string): string {
  let binary = "";
  for (const byte of new TextEncoder().encode(text)) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** A photo's thumbnail or preview, from the phone's cache or the desktop. */
export const thumbUrl = (id: number) => convertFileSrc(String(id), "thumb");
export const previewUrl = (id: number) => convertFileSrc(String(id), "preview");
/** A library photo's preview as edited on the computer, by its edits' tag. */
export const editedUrl = (id: number, tag: string) => convertFileSrc(`${id}_${tag}`, "edited");
/** A stand-in for the test grid (mobile/src-tauri/src/thumbs.rs). */
export const standinUrl = (i: number) => convertFileSrc(String(i), "standin");

export const api = {
  linkStatus: () => invoke<LinkStatus>("link_status"),
  pair: (code: string, deviceName: string) => invoke<string>("pair", { code, deviceName }),
  connect: () => invoke<Connected>("connect"),
  ping: () => invoke<Pinged>("ping"),
  sync: () => invoke<SyncSummary>("sync"),
  linkWaiting: () => invoke<Waiting>("link_waiting"),
  photos: () => invoke<Tile[]>("photos"),
  setFlag: (id: number, flag: number) => invoke<void>("set_flag", { id, flag }),
  setRating: (id: number, rating: number) => invoke<void>("set_rating", { id, rating }),
  /** Search, as the computer's search box reads and runs it. Both fail with
   *  a code: needsComputer, updateComputer or failed. */
  libraryVocabulary: () => invoke<string>("library_vocabulary"),
  libraryPhotoName: (id: number) => invoke<string | null>("library_photo_name", { id }),
  librarySearch: (filter: object) =>
    invoke<{ ids: number[]; ranked: boolean }>("library_search", { filter: JSON.stringify(filter) }),
  forget: () => invoke<void>("forget"),
  tracks: () => invoke<Tracks>("tracks"),
  trackStart: () => invoke<TrackStatus>("track_start"),
  trackStop: (name: string) => invoke<Tracks>("track_stop", { name }),
  /** Scans a QR code with the camera (mobile/plugins/qr-scan). Null when
   *  the person backs out. */
  scanQr: (prompt: string) => invoke<string | null>("scan_qr", { prompt }),
  /** The GPU test on photo `id`'s preview, or a test card (docs/MOBILE.md
   *  section 4, step 1). */
  /** On this phone (docs/MOBILE.md section 4). */
  deviceAccess: () => invoke<DeviceAccess>("device_access"),
  deviceRequestAccess: () => invoke<DeviceAccess>("device_request_access"),
  deviceAlbums: () => invoke<DeviceAlbum[]>("device_albums"),
  devicePhotos: (album: string | null) => invoke<DevicePhoto[]>("device_photos", { album }),
  /** Add to library (docs/MOBILE.md section 4, step 5): in line for the next sync. */
  deviceAdd: (photos: number[]) => invoke<number>("device_add", { photos }),
  deviceAdds: () => invoke<DeviceAdd[]>("device_adds"),
  /** Gets the GPU ready for editing ahead of time, so Edit opens quickly. */
  editWarm: () => invoke<void>("edit_warm"),
  /** Develop on the phone (docs/MOBILE.md section 4, step 4). */
  editOpen: (id: number, modified: number) => invoke<EditOpened>("edit_open", { id, modified }),
  /** A library photo (step 6). Fails with a code `libraryEditError` words. */
  editOpenLibrary: (id: number) => invoke<EditOpened>("edit_open_library", { id }),
  editCommit: (token: number, stack: EditStack) => invoke<EditStep>("edit_commit", { token, stack }),
  editUndo: (token: number) => invoke<EditStep>("edit_undo", { token }),
  editRedo: (token: number) => invoke<EditStep>("edit_redo", { token }),
  editPresets: () => invoke<PresetGroupInfo[]>("edit_presets"),
  editLooks: () => invoke<LookInfo[]>("edit_looks"),
  /** `stack` with built-in preset `id` at `amount`, or its preset taken off with "". */
  editPreset: (stack: EditStack, id: string, amount: number) => invoke<EditStack>("edit_preset", { stack, id, amount }),
  /** `onto` with a copied edit laid over it, less the crop. */
  editPaste: (copied: EditStack, onto: EditStack) => invoke<EditStack>("edit_paste", { copied, onto }),
  /** `stack` with Auto tone or Auto white balance worked out on the photo. */
  editAuto: (token: number, stack: EditStack, kind: "tone" | "wb") =>
    invoke<EditStack>("edit_auto", { token, stack, kind }),
  /** `stack` with the white balance that turns the point (0 to 1 in the
   *  framed photo) grey. */
  editPickWb: (token: number, stack: EditStack, x: number, y: number) =>
    invoke<EditStack>("edit_pick_wb", { token, stack, x, y }),
  editEdited: () => invoke<number[]>("edit_edited"),
  editClose: (token: number) => invoke<void>("edit_close", { token }),
  /** Done: keeps the edit (a phone photo's settings; a library photo's goes
   *  to the computer). Until then its steps are only the editor's. */
  editDone: (token: number) => invoke<void>("edit_done", { token }),
  /** Saves a copy, as edited, to the gallery; resolves its file name. */
  editSave: (token: number) => invoke<string>("edit_save", { token }),
  gpuTest: (id: number | null, width: number, height: number) =>
    invoke<GpuReport>("gpu_test", { id, width, height }),
};
