import { convertFileSrc } from "@tauri-apps/api/core";
import {
  commands,
  type CaptureTimeChange,
  type DirectFrame,
  type EditStack,
  type FacetGroups,
  type FlagChange,
  type IptcField,
  type IptcFields,
  type Group,
  type PhotoFilter,
  type MetadataChoices,
  type SidecarTime,
  type ViewRequest,
} from "../bindings";
import { coreErrorEnglish, translateCoreDetail, translateCoreError } from "./coreText";
import { contentLanguage } from "../i18n";

/**
 * Adapter over the generated `commands`.
 *
 * tauri-specta returns fallible commands as `{ status: "ok" | "error" }` rather
 * than rejecting. Unwrapping it back to a thrown error here keeps the promise
 * contract every component already relies on, so the generated bindings stay a
 * detail of this file rather than something the whole UI has to know about.
 * The core's error text is worded in the user's language here (`coreText.ts`),
 * so every toast and inline error that shows `String(e)` benefits.
 */
/**
 * A command's failure: its message in the user's language, and the English the
 * core sent, which is what the error log records (`errorLog.ts`) — a log read
 * by someone else, in any language, should say the same thing.
 */
export class CoreError extends Error {
  constructor(
    message: string,
    readonly english: string,
  ) {
    super(message);
  }

  /** The message alone. `String(e)` is how most callers word a failure —
   *  "That didn't work: {error}", a toast, a line under a button — and
   *  Error's own toString put "Error: " in front of every one, which reads
   *  as a crash in a sentence meant for a person. */
  override toString(): string {
    return this.message;
  }
}

async function unwrap<T, E>(
  call: Promise<{ status: "ok"; data: T } | { status: "error"; error: E }>,
): Promise<T> {
  const result = await call;
  if (result.status === "error") {
    const text = String(result.error);
    throw new CoreError(translateCoreError(text), coreErrorEnglish(text));
  }
  return result.data;
}

export const api = {
  checkForUpdate: () => unwrap(commands.checkForUpdate()),
  installUpdate: () => unwrap(commands.installUpdate()),
  quitApp: () => commands.quitApp(),
  /** Answers `quit-requested` with "stay open" (see `lib/quitGuard.ts`). */
  stayOpen: () => commands.stayOpen(),
  /** The day's catalogue backup, if one is due; the new file's path, or null. */
  backupCatalogueIfDue: () => unwrap(commands.backupCatalogueIfDue()),
  onBattery: () => commands.onBattery(),
  /** Folders dropped on the Dock icon since last asked (see `lib/folderDrop.ts`). */
  takeOpenedFolders: () => commands.takeOpenedFolders(),
  /** SourceLight's own licence text, compiled into the app. */
  licenceText: () => commands.licenceText(),
  thirdPartyLicences: () => unwrap(commands.thirdPartyLicences()),
  /** Takes back the last rating/flag/colour-label change; null if none is left. */
  undoVerdict: () => unwrap(commands.undoVerdict()),
  /** Writes an interface error beside the crash logs. */
  logUiError: (surface: string, message: string, stack: string) =>
    unwrap(commands.logUiError(surface, message, stack)),
  /** Version, OS, GPU, where things are, and recent logs — for a bug report. */
  diagnosticsSummary: () => commands.diagnosticsSummary(),
  openLogFolder: () => unwrap(commands.openLogFolder()),
  /** The editing server for AI apps (docs/AI-EDIT.md): Settings → AI. */
  editServerStatus: () => unwrap(commands.editServerStatus()),
  setEditServer: (enabled: boolean) => unwrap(commands.setEditServer(enabled)),
  /** The phone link (docs/MOBILE.md): Settings → Phone. */
  phoneLinkStatus: () => unwrap(commands.phoneLinkStatus()),
  setPhoneLink: (enabled: boolean, awayFromHome: boolean) =>
    unwrap(commands.setPhoneLink(enabled, awayFromHome)),
  startPhonePairing: () => unwrap(commands.startPhonePairing()),
  cancelPhonePairing: () => unwrap(commands.cancelPhonePairing()),
  answerPhonePairing: (phone: string, allow: boolean) => commands.answerPhonePairing(phone, allow),
  removePhone: (phone: string) => unwrap(commands.removePhone(phone)),
  /** Where photos added from a phone go; `null` sets the default back. */
  phoneImportFolder: () => unwrap(commands.phoneImportFolder()),
  setPhoneImportFolder: (path: string | null) => unwrap(commands.setPhoneImportFolder(path)),
  /** Send to phone: a folder in the outbox to export into, then queue it. */
  phoneOutboxStart: () => unwrap(commands.phoneOutboxStart()),
  phoneOutboxQueue: (directory: string) => unwrap(commands.phoneOutboxQueue(directory)),
  /** The library's folders with counts, and photos outside them. */
  listLibraryFolders: () => unwrap(commands.listLibraryFolders()),
  addLibraryFolder: (path: string) => unwrap(commands.addLibraryFolder(path)),
  /** Whether a folder is on a camera card, to offer importing it instead. */
  folderIsCard: (path: string) => commands.folderIsCard(path),
  removeLibraryFolder: (id: number) => unwrap(commands.removeLibraryFolder(id)),
  relocateLibraryFolder: (id: number, path: string) =>
    unwrap(commands.relocateLibraryFolder(id, path)),
  forgetUnfiledPhotos: () => unwrap(commands.forgetUnfiledPhotos()),
  /** How many photos have not been checked for a GPS position. */
  locationsStatus: () => unwrap(commands.locationsStatus()),
  /** Reads positions from those photos and places them; progress on
   *  `locations-progress`, cancellable as the `locations` job. */
  readLocations: () => unwrap(commands.readLocations()),
  /** How many photos' previews have not been checked against the colour
   *  space the camera recorded (Adobe RGB previews made before it was read). */
  coloursStatus: () => unwrap(commands.coloursStatus()),
  /** Remakes those previews in the right colours; progress on
   *  `colours-progress`, cancellable as the `colours` job. */
  readColours: () => unwrap(commands.readColours()),
  /** What placing photos from GPX files would do; with no offset,
   *  the camera clock's offset from UTC is guessed. Writes nothing. */
  gpxPreview: (paths: string[], offsetMinutes: number | null) =>
    unwrap(commands.gpxPreview(paths, offsetMinutes)),
  /** Places the photos a GPX track matches, at this camera offset. */
  gpxApply: (paths: string[], offsetMinutes: number) => unwrap(commands.gpxApply(paths, offsetMinutes)),
  /** Camera cards plugged in now (volumes with a DCIM folder). */
  importSources: () => commands.importSources(),
  /** What importing from a folder would copy, by day. */
  importPreview: (source: string) => unwrap(commands.importPreview(source)),
  /** Copies a card into the library; progress on `import-progress`. */
  importRun: (options: Parameters<typeof commands.importRun>[0]) => unwrap(commands.importRun(options)),
  /** Copies a shoot's photos from the library to its backup where the import could not. */
  finishShootBackup: (id: number) => unwrap(commands.finishShootBackup(id)),
  /** Opens the file manager with the photo's file selected. */
  showInFolder: (photoId: number) => unwrap(commands.showInFolder(photoId)),
  /** Whether a folder is there now: a remembered backup drive. */
  folderExists: (path: string) => commands.folderExists(path),
  /** Opens the file manager at a folder an export wrote into. */
  showFolder: (path: string) => unwrap(commands.showFolder(path)),
  /** Starts dragging these photos' files out of the app, always as copies. */
  dragPhotosOut: (ids: number[]) => unwrap(commands.dragPhotosOut(ids)),
  /** Moves rejected photos' files to the Recycle Bin and forgets them. Only
   *  after the confirmation in `store.binRejects`. */
  binRejects: (photoIds: number[]) => unwrap(commands.binRejects(photoIds)),
  /** Deletes photos whatever their flag — only ever after the confirmation in
   *  `store.deletePhotos`. */
  deletePhotos: (photoIds: number[]) => unwrap(commands.deletePhotos(photoIds)),
  scanLibrary: (path: string) => unwrap(commands.scanLibrary(path)),

  /** `request` marks a grid load, which the catalogue stops working on once a
   *  newer one is asked for. */
  getPhotos: (
    filter: Parameters<typeof commands.getPhotos>[0],
    limit: number,
    offset: number,
    request: ViewRequest | null = null,
  ) => unwrap(commands.getPhotos(filter, limit, offset, request)),
  getPhoto: (id: number) => unwrap(commands.getPhoto(id)),
  /** Specific photos by id, in the order asked for — how the compare view pulls
   *  a duplicate group that the current filter excludes. */
  getPhotosByIds: (ids: number[]) => unwrap(commands.getPhotosByIds(ids)),
  getFacets: (filter: Parameters<typeof commands.getFacets>[0], request: ViewRequest | null = null) =>
    unwrap(commands.getFacets(filter, request)),
  /** The rail's optional counts, for the groups whose section is open. */
  getExtraFacets: (filter: PhotoFilter, groups: FacetGroups) => unwrap(commands.getExtraFacets(filter, groups)),
  getGenres: () => commands.getGenres(),

  addTag: (photoIds: number[], name: string, category: string) =>
    unwrap(commands.addTag(photoIds, name, category)),
  removeTag: (photoIds: number[], name: string) => unwrap(commands.removeTag(photoIds, name)),
  /** Renders a photo with its edits as a PNG for the clipboard; its path. */
  renderForClipboard: (photoId: number, maxEdge: number | null = null) =>
    unwrap(commands.renderForClipboard(photoId, maxEdge)),
  /** Renders a photo as edited for the grid and loupe; its path, or null if unedited. */
  renderEditedPreview: (photoId: number) => unwrap(commands.renderEditedPreview(photoId)),
  /** One photo's place on others, and its position where they have none. */
  pasteLocation: (photoIds: number[], from: number) => unwrap(commands.pasteLocation(photoIds, from)),
  /** One photo's own keywords on others; how many were copied. */
  pasteKeywords: (photoIds: number[], from: number) => unwrap(commands.pasteKeywords(photoIds, from)),
  /** Shifts or sets capture times; copies move with their originals. */
  setCaptureTime: (photoIds: number[], change: CaptureTimeChange) =>
    unwrap(commands.setCaptureTime(photoIds, change)),
  /** Back to the files' own times; how many photos changed. */
  revertCaptureTime: (photoIds: number[]) => unwrap(commands.revertCaptureTime(photoIds)),
  /** Colours: how many are still to read, reading them, and ranking by likeness. */
  paletteStatus: () => unwrap(commands.paletteStatus()),
  readPalettes: () => unwrap(commands.readPalettes()),
  similarColours: (photoId: number, limit: number) => unwrap(commands.similarColours(photoId, limit)),
  /** Collection groups, and a collection's hand order. */
  listCollectionGroups: () => unwrap(commands.listCollectionGroups()),
  createCollectionGroup: (name: string) => unwrap(commands.createCollectionGroup(name)),
  renameCollectionGroup: (id: number, name: string) => unwrap(commands.renameCollectionGroup(id, name)),
  deleteCollectionGroup: (id: number) => unwrap(commands.deleteCollectionGroup(id)),
  setCollectionGroup: (id: number, smart: boolean, groupId: number | null) =>
    unwrap(commands.setCollectionGroup(id, smart, groupId)),
  moveInCollection: (collectionId: number, photoIds: number[], before: number | null) =>
    unwrap(commands.moveInCollection(collectionId, photoIds, before)),
  /** What the photos in view say about how they were made. */
  insights: (filter: PhotoFilter) => unwrap(commands.insights(filter)),
  lensKeywords: (filter: PhotoFilter, lenses: string[]) => unwrap(commands.lensKeywords(filter, lenses)),
  /** Photos in view by month, or by day between two dates. */
  timeline: (filter: PhotoFilter, dayFrom: string | null, dayTo: string | null) =>
    unwrap(commands.timeline(filter, dayFrom, dayTo)),
  /** A photo's own title, caption and rights. */
  photoIptc: (photoId: number) => unwrap(commands.photoIptc(photoId)),
  iptcSelection: (photoIds: number[]) => unwrap(commands.iptcSelection(photoIds)),
  setIptcField: (photoIds: number[], field: IptcField, value: string | null) =>
    unwrap(commands.setIptcField(photoIds, field, value)),
  listIptcTemplates: () => unwrap(commands.listIptcTemplates()),
  saveIptcTemplate: (id: number | null, name: string, fields: IptcFields) =>
    unwrap(commands.saveIptcTemplate(id, name, fields)),
  deleteIptcTemplate: (id: number) => unwrap(commands.deleteIptcTemplate(id)),
  /** On these photos, or on an import's shoot once scanned. */
  applyIptcTemplate: (photoIds: number[], shootId: number | null, templateId: number) =>
    unwrap(commands.applyIptcTemplate(photoIds, shootId, templateId)),
  /** How many of these photos a GPX track placed. */
  countTrackPlaced: (photoIds: number[]) => unwrap(commands.countTrackPlaced(photoIds)),
  /** Moves the track-placed ones along their track to their times now. */
  gpxReplace: (photoIds: number[]) => unwrap(commands.gpxReplace(photoIds)),
  /** Takes the capture times reading sidecars offered. */
  useSidecarTimes: (times: SidecarTime[]) => unwrap(commands.useSidecarTimes(times)),
  /** Folder names as keywords, for a library folder, a shoot, or all. */
  addFolderKeywords: (folderId: number | null, shootId: number | null) =>
    unwrap(commands.addFolderKeywords(folderId, shootId)),
  /** Takes them back off (all for null); how many photos lost one. */
  removeFolderKeywords: (folderId: number | null) => unwrap(commands.removeFolderKeywords(folderId)),
  /** The tagger's unconfirmed keywords on these photos. */
  aiKeywords: (photoIds: number[]) => unwrap(commands.aiKeywords(photoIds)),
  /** Makes the tagger's keywords the user's own; `names` empty for all. How many. */
  confirmKeywords: (photoIds: number[], names: string[]) => unwrap(commands.confirmKeywords(photoIds, names)),
  /** The keyword tree's shape; counts come with the facets. */
  keywordTree: () => unwrap(commands.keywordTree()),
  createKeyword: (name: string, parent: string | null) => unwrap(commands.createKeyword(name, parent)),
  /** Files a keyword and its branch under `parent`, or at the top for null. */
  moveKeyword: (name: string, parent: string | null) => unwrap(commands.moveKeyword(name, parent)),
  /** Renames a keyword — to another's name, merges them. The name it ends up with. */
  renameKeyword: (name: string, newName: string) => unwrap(commands.renameKeyword(name, newName)),
  /** Deletes a keyword from every photo; how many had it. */
  deleteKeyword: (name: string) => unwrap(commands.deleteKeyword(name)),
  setRating: (photoIds: number[], rating: number) => unwrap(commands.setRating(photoIds, rating)),
  setFlag: (photoIds: number[], flag: number) => unwrap(commands.setFlag(photoIds, flag)),
  /** A different flag per photo, as one undoable change. */
  /** Many flags as one undoable change; `what` names it for Undo. */
  /** `suggested`: accepted suggestions, remembered as the machine's. */
  setFlags: (flags: FlagChange[], what: string | null = null, suggested = false) =>
    unwrap(commands.setFlags(flags, what, suggested)),
  setColorLabel: (photoIds: number[], label: number) =>
    unwrap(commands.setColorLabel(photoIds, label)),

  listCollections: () => unwrap(commands.listCollections()),
  createCollection: (name: string) => unwrap(commands.createCollection(name)),
  renameCollection: (id: number, name: string) => unwrap(commands.renameCollection(id, name)),
  deleteCollection: (id: number) => unwrap(commands.deleteCollection(id)),
  addToCollection: (id: number, photoIds: number[]) =>
    unwrap(commands.addToCollection(id, photoIds)),
  removeFromCollection: (id: number, photoIds: number[]) =>
    unwrap(commands.removeFromCollection(id, photoIds)),

  listSmartCollections: () => unwrap(commands.listSmartCollections()),
  saveSmartCollection: (name: string, filter: Parameters<typeof commands.saveSmartCollection>[1]) =>
    unwrap(commands.saveSmartCollection(name, filter)),
  deleteSmartCollection: (id: number) => unwrap(commands.deleteSmartCollection(id)),

  importXmp: (photoIds: number[]) => unwrap(commands.importXmp(photoIds)),
  /** How many of these photos (none = all) have a sidecar to read. */
  countSidecars: (photoIds: number[]) => unwrap(commands.countSidecars(photoIds)),
  /** `choices`: what names, location and AI text go in. */
  exportXmp: (photoIds: number[], choices: MetadataChoices | null = null) =>
    unwrap(commands.exportXmp(photoIds, choices)),

  listPlaces: () => unwrap(commands.listPlaces()),
  searchVocabulary: () => unwrap(commands.searchVocabulary()),
  clearPlace: (photoIds: number[]) => unwrap(commands.clearPlace(photoIds)),
  listShoots: () => unwrap(commands.listShoots()),
  deleteShoot: (id: number) => unwrap(commands.deleteShoot(id)),
  assignPlace: (photoIds: number[], name: string, lat?: number, lon?: number) =>
    unwrap(commands.assignPlace(photoIds, name, lat ?? null, lon ?? null)),
  setPlacePosition: (placeId: number, lat: number, lon: number) =>
    unwrap(commands.setPlacePosition(placeId, lat, lon)),
  /** Renames a place; returns the id of a place that already has the name
   *  (nothing renamed) so the caller can offer to merge. */
  renamePlace: (placeId: number, name: string) => unwrap(commands.renamePlace(placeId, name)),
  mergePlaces: (fromIds: number[], intoId: number) => unwrap(commands.mergePlaces(fromIds, intoId)),
  deletePlace: (placeId: number) => unwrap(commands.deletePlace(placeId)),
  /** A few thumbnails from a place, for the map's hover card. */
  placeThumbs: (placeId: number) => unwrap(commands.placeThumbs(placeId)),
  /** Places a trip's photos that have no GPS of their own; the rest keep theirs. */
  nameTrip: (photoIds: number[], name: string) => unwrap(commands.nameTrip(photoIds, name)),
  getTripClusters: (gapHours?: number) => unwrap(commands.getTripClusters(gapHours ?? null)),

  providerStatus: () =>
    unwrap(commands.providerStatus()).then((st) => ({ ...st, detail: translateCoreDetail(st.detail) })),
  /** Sets local AI up for one purpose. See `src/lib/aiReady.ts` for the
   *  consent flow every caller should go through rather than calling this. */
  ensureAiReady: (purpose: Parameters<typeof commands.ensureAiReady>[0], consented: boolean) =>
    unwrap(commands.ensureAiReady(purpose, consented)).then((r) => ({
      ...r,
      detail: translateCoreDetail(r.detail),
      blocked: r.blocked === null ? null : translateCoreDetail(r.blocked),
      needs: r.needs.map((n) => ({ ...n, label: translateCoreDetail(n.label) })),
    })),
  getAiSettings: () => unwrap(commands.getAiSettings()),
  /** `provider` is "none" (AI off), "builtin" (built-in tags) or "vision"
   *  (detailed descriptions); `model` is a catalogue vision model or "" for
   *  this machine's pick. */
  setAiSettings: (provider: string, model: string) =>
    unwrap(commands.setAiSettings(provider, model)),
  /** On again as it was before it was switched off. */
  turnAiOn: () => unwrap(commands.turnAiOn()),

  /** Descriptions written while tagging are in the interface's language; tags stay English. */
  startTagging: () => unwrap(commands.startTagging(contentLanguage())),
  /** Tag names the local model has given, in `language`. */
  tagTranslations: (language: string) => unwrap(commands.tagTranslations(language)),
  /** Names `names` in `language` with the local model, where one is running. */
  translateTags: (language: string, names: string[]) => unwrap(commands.translateTags(language, names)),
  cancelTagging: () => commands.cancelTagging(),
  resetTagging: () => unwrap(commands.resetTagging()),

  openDevelop: (photoId: number) => unwrap(commands.openDevelop(photoId)),
  /** RGB histogram of the current edit, rendered Rust-side. */
  /** Of the current edit, or of `draft` — an edit mid-drag, not yet committed. */
  developHistogram: (draft?: EditStack | null) => unwrap(commands.developHistogram(draft ?? null)),
  /** Holds the edit being dragged for the preview to draw; its number for the URL. */
  setDevelopDraft: (draft: EditStack | null) => unwrap(commands.setDevelopDraft(draft)),
  /** Whether AI masking is available (feature compiled + model present). */
  aiStatus: () => unwrap(commands.aiStatus()),
  /** Runs subject segmentation on the current edit; returns a mask to add. */
  aiSelectSubject: () => unwrap(commands.aiSelectSubject()),
  developDenoise: (amount: number | null, focus: [number, number] | null) =>
    unwrap(commands.developDenoise(amount, focus)),
  /** Click-to-select: points in framed UV -> a mask. Cheap after the first call
   *  on a photo, since the image embedding is cached Rust-side. */
  aiSelectPoints: (points: Parameters<typeof commands.aiSelectPoints>[0]) =>
    unwrap(commands.aiSelectPoints(points)),
  /** The downloadable AI model catalogue with per-model installed status. */
  aiAvailableModels: () => unwrap(commands.aiAvailableModels()),
  /** Downloads a model; progress arrives on the `ai-download-progress` event. */
  downloadAiModel: (id: string) => unwrap(commands.downloadAiModel(id)),
  /** Removes a downloaded model's files. */
  deleteAiModel: (id: string) => unwrap(commands.deleteAiModel(id)),
  /** A quick GPU/CPU probe to advise which AI models suit this machine. */
  hardwareReport: () => unwrap(commands.hardwareReport()),
  /** Rank the library against a free-text query by CLIP visual similarity. */
  semanticSearch: (query: string, limit: number) =>
    unwrap(commands.semanticSearch(query, limit)),
  /** Photos visually similar to one photo (needs it indexed): ranked nearest
   *  first, keeping only those at or above `threshold` cosine similarity. */
  similarPhotos: (photoId: number, limit: number, threshold: number) =>
    unwrap(commands.similarPhotos(photoId, limit, threshold)),
  /** Full-text/OCR search over VLM captions (keyword + text-in-image); ranked
   *  best-first. Empty until photos have been captioned. */
  captionSearch: (query: string, limit: number) =>
    unwrap(commands.captionSearch(query, limit)),
  /** The vision-model catalogue graded against this machine's VRAM. */
  vlmCatalogue: () => unwrap(commands.vlmCatalogue()),
  /** What detailed descriptions have downloaded — the runtime and any vision
   *  models — read from disk, for the Storage list. */
  localVlmStorage: () => unwrap(commands.localVlmStorage()),
  /** Turns detailed descriptions off and deletes the runtime and its models. */
  removeLocalVlm: () => unwrap(commands.removeLocalVlm()),
  /** How many photos still need a caption (the captioning job's denominator). */
  captionStatus: () => unwrap(commands.captionStatus()),
  /** Captions every uncaptioned photo with the configured VLM; progress on the
   *  `caption-progress` event. Resolves with how many were captioned. */
  startCaptioning: (limit?: number) => unwrap(commands.startCaptioning(limit ?? null, contentLanguage())),
  cancelCaptioning: () => commands.cancelCaptioning(),
  /** Stops a running long job by the id its progress is tracked under. */
  cancelJob: (id: string) => commands.cancelJob(id),
  /** Groups of near-identical photos (bursts/duplicates) by CLIP similarity. */
  /** Near- and exact-duplicate groups among `within` (the whole library when null). */
  duplicateGroups: (threshold: number, within: number[] | null = null) =>
    unwrap(commands.duplicateGroups(threshold, within)),
  /** Embed every not-yet-indexed photo for semantic search (background job;
   *  progress on the `embed-progress` event). Returns how many were queued. */
  indexForSearch: () => unwrap(commands.indexForSearch()),
  /** How many photos still need a search embedding. */
  searchIndexStatus: () => unwrap(commands.searchIndexStatus()),
  /** Snapshot the catalogue now. Returns the path it was written to. */
  backupCatalogue: () => unwrap(commands.backupCatalogue()),
  /** Every catalogue backup on disk, newest first. */
  listCatalogueBackups: () => unwrap(commands.listCatalogueBackups()),
  /** Queue a backup to replace the live catalogue at the next launch. Nothing
   *  changes until the app restarts — the database is open right now. */
  restoreCatalogueBackup: (filename: string) => unwrap(commands.restoreCatalogueBackup(filename)),
  /** Discard a staged restore. True if there was one. */
  cancelCatalogueRestore: () => unwrap(commands.cancelCatalogueRestore()),
  /** Whether a restore is waiting for the next launch. */
  catalogueRestorePending: () => unwrap(commands.catalogueRestorePending()),
  /** What startup did about a damaged catalogue, once; null when nothing. */
  catalogueRecovery: () => commands.catalogueRecovery(),
  /** The kept log of a last session that never exited, once; null when it did. */
  uncleanExit: () => commands.uncleanExit(),
  /** Stage the newest backup that passes an integrity check. */
  restoreNewestBackup: () => unwrap(commands.restoreNewestBackup()),
  /** Restart the app, so a staged restore takes effect. */
  restartApp: () => commands.restartApp(),
  /** Score every not-yet-scored photo's focus/sharpness (background job; progress
   *  on the `quality-progress` event). Needs no model. Returns how many queued. */
  scoreQuality: () => unwrap(commands.scoreQuality()),
  /** How many photos still need a focus/sharpness score. */
  qualityStatus: () => unwrap(commands.qualityStatus()),
  /** Analyse every not-yet-analysed photo for faces (box + focus + eyes-open;
   *  progress on the `faces-progress` event). Needs the face models installed. */
  analyzeFaces: () => unwrap(commands.analyzeFaces()),
  /** How many photos still need the face pass. */
  facesStatus: () => unwrap(commands.facesStatus()),
  /** Cluster every embedded face into people; returns the number found. */
  /** `full` clusters every face again; otherwise new faces join the people
   *  already there when they can. */
  groupPeople: (full = false) => unwrap(commands.groupPeople(full)),
  /** The grouped people, most-photographed first. */
  listPeople: () => unwrap(commands.listPeople()),
  /** Rename a person (blank clears the name). */
  setPersonName: (id: number, name: string) => unwrap(commands.setPersonName(id, name)),
  personFaces: (personId: number) => unwrap(commands.personFaces(personId)),
  /** The detected faces in each photo, largest first, with portraits. */
  photoFaces: (photoIds: number[]) => unwrap(commands.photoFaces(photoIds)),
  /** Clears every detected face so the pass runs again from scratch. */
  resetFaces: () => unwrap(commands.resetFaces()),
  /** People's yes to recognising faces. */
  peopleEnabled: () => unwrap(commands.peopleEnabled()),
  setPeopleEnabled: (on: boolean) => unwrap(commands.setPeopleEnabled(on)),
  eraseFaceData: () => unwrap(commands.eraseFaceData()),
  mergePeople: (ids: number[]) => unwrap(commands.mergePeople(ids)),
  /** Takes back the last merge. */
  undoMerge: () => unwrap(commands.undoMerge()),
  detachFace: (faceId: number) => unwrap(commands.detachFace(faceId)),
  nameFace: (faceId: number, name: string) => unwrap(commands.nameFace(faceId, name)),
  setCoverFace: (faceId: number) => unwrap(commands.setCoverFace(faceId)),
  unplacedFaces: () => unwrap(commands.unplacedFaces()),
  dismissFace: (faceId: number) => unwrap(commands.dismissFace(faceId)),
  /** People who may be one person split in two, best first. */
  peopleSuggestions: () => unwrap(commands.peopleSuggestions()),
  /** "Not the same person": never suggest these two again. */
  keepPeopleApart: (a: number, b: number) => unwrap(commands.keepPeopleApart(a, b)),
  /** Hide people the user doesn't care about, or show them again. */
  setPeopleHidden: (ids: number[], hidden: boolean) => unwrap(commands.setPeopleHidden(ids, hidden)),
  /** Put a person in these photos by hand; returns how many they were added to. */
  addPhotosToPerson: (photoIds: number[], personId: number) =>
    unwrap(commands.addPhotosToPerson(photoIds, personId)),
  /** A new person made from these photos alone; returns their id. */
  addPhotosToNewPerson: (photoIds: number[], name: string) =>
    unwrap(commands.addPhotosToNewPerson(photoIds, name)),
  removePhotosFromPerson: (photoIds: number[], personId: number) =>
    unwrap(commands.removePhotosFromPerson(photoIds, personId)),
  /** Who is in a photo, and whether each was added by hand. */
  photoPeople: (photoId: number) => unwrap(commands.photoPeople(photoId)),
  /** `null` clears the override and defers to the eye-open model again. */
  setEyesOverride: (photoIds: number[], value: number | null) =>
    unwrap(commands.setEyesOverride(photoIds, value)),
  /** A framing change that keeps masks and spots on the content. */
  reframeDevelop: (label: string, stack: EditStack) => unwrap(commands.reframeDevelop(label, stack)),
  setDevelopStack: (label: string, stack: Parameters<typeof commands.setDevelopStack>[1]) =>
    unwrap(commands.setDevelopStack(label, stack)),
  /** Back to how a new photo opens, erases and all. */
  resetDevelop: () => unwrap(commands.resetDevelop()),
  /** Magic erase: takes out what `selection` covers and fills the gap. */
  eraseDevelop: (selection: Parameters<typeof commands.developErase>[0], label: string) =>
    unwrap(commands.developErase(selection, label)),
  /** Takes an erase back out, so its object shows again. */
  removeErase: (index: number) => unwrap(commands.developRemoveErase(index)),
  /** Gets click to select and the erase model ready for the photo, in the
   *  background, so the first click and the first erase don't wait. */
  prepareErase: () => unwrap(commands.developPrepareErase()),
  undoDevelop: () => unwrap(commands.undoDevelop()),
  redoDevelop: () => unwrap(commands.redoDevelop()),
  /** Full auto — tone + white balance — returning the new develop state. */
  autoAllDevelop: () => unwrap(commands.autoAllDevelop()),
  /** Auto-tone the open photo (heuristic), returning the new develop state. */
  autoToneDevelop: () => unwrap(commands.autoToneDevelop()),
  /** Auto white balance the open photo (gray-world), returning the new state. */
  autoWbDevelop: () => unwrap(commands.autoWbDevelop()),
  pickWbDevelop: (x: number, y: number) => unwrap(commands.pickWbDevelop(x, y)),
  highlights: (ids: number[], count: number) => unwrap(commands.highlights(ids, count)),
  /** Saves the current edits as a virtual copy, leaving the original alone.
   *  Returns the new photo's id. */
  saveDevelopAsCopy: (name: string | null) => unwrap(commands.saveDevelopAsCopy(name)),
  closeDevelop: () => unwrap(commands.closeDevelop()),
  /** Renders at full resolution and writes the file. Returns bytes written. */
  exportDevelop: (path: string, options: Parameters<typeof commands.exportDevelop>[1]) =>
    unwrap(commands.exportDevelop(path, options)),
  /** The creator and copyright lines every export's metadata carries. */
  getExportRights: () => unwrap(commands.getExportRights()),
  setExportRights: (rights: Parameters<typeof commands.setExportRights>[0]) =>
    unwrap(commands.setExportRights(rights)),

  /** Whether a batch export looks likely to fill its drive — null when it fits. */
  exportSpaceCheck: (
    photoIds: number[],
    directory: string,
    options: Parameters<typeof commands.exportSpaceCheck>[2],
  ) => unwrap(commands.exportSpaceCheck(photoIds, directory, options)),
  exportBatch: (
    photoIds: number[],
    directory: string,
    options: Parameters<typeof commands.exportBatch>[2],
    rename: Parameters<typeof commands.exportBatch>[3],
    subfolder: Parameters<typeof commands.exportBatch>[4],
  ) => unwrap(commands.exportBatch(photoIds, directory, options, rename, subfolder)),

  /** Stop the running batch export after the current file. */
  cancelExport: () => commands.cancelExport(),

  /** Turns Develop's direct GPU surface on or off; the images keep working either way. */
  setDirectRendering: (enabled: boolean) => unwrap(commands.setDirectRendering(enabled)),
  /** Fills the render surface with flat magenta; needs no photo open. */
  probeDirect: (x: number, y: number, width: number, height: number) =>
    unwrap(commands.probeDirect(x, y, width, height)),
  /** Draws the open photo into the direct surface, placed at the canvas. */
  renderDirect: (frame: DirectFrame) => unwrap(commands.renderDirect(frame)),

  listPresets: () => unwrap(commands.listPresets()),
  savePreset: (name: string, groups: Group[]) => unwrap(commands.savePreset(name, groups)),
  /** The installed creative LUTs (.cube files). */
  listLuts: () => unwrap(commands.listLuts()),
  /** Imports a .cube from a path into the LUTs folder; returns the stored entry.
   *  `group` files it under a heading of the user's naming as it lands. */
  importLut: (path: string, group?: string) =>
    unwrap(commands.importLut(path, group ?? null)),
  /** Files LUTs under a group name, or clears it when `group` is null.
   *  Returns the refreshed list. */
  setLutGroup: (names: string[], group: string | null) =>
    unwrap(commands.setLutGroup(names, group)),
  /** Deletes an installed LUT by filename. */
  /** How many photos' edits use a LUT. */
  lutUsage: (name: string) => unwrap(commands.lutUsage(name)),
  deleteLut: (name: string) => unwrap(commands.deleteLut(name)),
  /** The built-in presets, by group. */
  listPresetGroups: () => commands.listPresetGroups(),
  /** `stack` with built-in preset `id` at `amount` (1 as designed), or with its
   *  preset taken off when `id` is "". Pure: the caller applies the result. */
  presetStack: (stack: EditStack, id: string, amount: number) => unwrap(commands.presetStack(stack, id, amount)),
  /** Suggested edits (auto + genre-matched presets) with live previews of the
   *  open photo. */
  suggestEdits: () => unwrap(commands.suggestEdits()),
  /** Live thumbnails of the given built-in presets on the open photo. */
  builtinPresetPreviews: (ids: string[]) => unwrap(commands.builtinPresetPreviews(ids)),
  /** Live thumbnails of every saved Preset applied to the open photo. */
  presetPreviews: () => unwrap(commands.presetPreviews()),
  /** Live thumbnails of every installed LUT applied to the open photo. */
  lutPreviews: () => unwrap(commands.lutPreviews()),
  deletePreset: (id: number) => unwrap(commands.deletePreset(id)),
  applyPreset: (id: number) => unwrap(commands.applyPreset(id)),
  /** Copies the open photo's settings onto other photos. */
  copySettingsTo: (photoIds: number[], groups: Group[]) => unwrap(commands.copySettingsTo(photoIds, groups)),
  pasteSettingsFrom: (photoId: number, groups: Group[]) => unwrap(commands.pasteSettingsFrom(photoId, groups)),
  /** Pastes parts of a copied edit onto the open photo, as copied. */
  pasteSettings: (stack: EditStack, groups: Group[]) => unwrap(commands.pasteSettings(stack, groups)),
  developGoTo: (index: number) => unwrap(commands.developGoTo(index)),
};

/**
 * URL for the current develop preview.
 *
 * `frame` is pure cache-busting — the handler always renders current state, but
 * without a changing URL the webview would serve the previous frame and the
 * sliders would appear dead. `before` renders the unedited stack for comparison.
 */
/** Zoom and pan. Absent means fit-to-window. */
export interface ViewPort {
  /** Output pixels per image pixel; 1 is one-to-one. */
  zoom: number;
  /** Centre of the visible region, normalised against the cropped frame. */
  cx: number;
  cy: number;
  /** The area being drawn into, in device pixels. */
  width: number;
  height: number;
}

export function developUrl(
  frame: number,
  before = false,
  edge?: number,
  uncropped = false,
  view?: ViewPort | null,
  softProof = false,
  draft: number | null = null,
  showMask: number | null = null,
): string {
  const params = new URLSearchParams({ v: String(frame) });
  // The mask whose coverage is tinted, while masking.
  if (showMask != null) params.set("mask", String(showMask));
  // The edit being dragged, held by `setDevelopDraft`.
  if (draft != null) params.set("draft", String(draft));
  if (before) params.set("before", "1");
  // Soft-proof (gamut check): the render marks colours the output profile can't
  // hold. A viewing aid, so it never touches the "before" or export renders.
  if (softProof) params.set("proof", "1");
  // A zoom carries its own target size — it is drawing a specific
  // magnification into a specific area, so `edge` does not apply.
  if (view) {
    params.set("zoom", view.zoom.toFixed(4));
    params.set("cx", view.cx.toFixed(5));
    params.set("cy", view.cy.toFixed(5));
    params.set("vw", String(Math.round(view.width)));
    params.set("vh", String(Math.round(view.height)));
  }
  // While the crop rectangle is being dragged the preview shows the whole
  // frame, so there is something to drag the edges back out into.
  if (uncropped) params.set("uncropped", "1");
  // Rendering more pixels than the canvas shows is pure cost: the frame is
  // JPEG-encoded on the CPU, which dominates the frame time, and it scales with
  // pixel count.
  if (edge) params.set("edge", String(Math.round(edge)));
  return `${convertFileSrc("preview", "develop")}?${params}`;
}

/**
 * Images are served over our own `thumb://` scheme rather than base64 — base64
 * inflates the payload ~33% and makes a few thousand thumbnails crawl.
 *
 * Not Tauri's built-in `asset:` protocol: its scope matching rejected every
 * request with 403 on Windows (see `serve_thumb` in src-tauri/src/commands/protocols.rs).
 * `convertFileSrc` still builds the platform-correct URL shape for us.
 */
export function fullUrl(photoId: number): string {
  // The raw's full-resolution pixels, for the zoom loupe. By photo id —
  // the backend looks the file up, so the page never names a path.
  return convertFileSrc(String(photoId), "full");
}

export function fileUrl(path: string | null): string | undefined {
  if (!path) return undefined;
  return convertFileSrc(path, "thumb");
}
