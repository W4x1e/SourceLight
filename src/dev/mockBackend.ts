/**
 * A stand-in Tauri backend for looking at the UI in a plain browser.
 *
 * The app's surfaces could only ever be seen in the running Tauri window, which
 * an agent — and CI — cannot drive. This answers the IPC calls with a small,
 * fixed, fake library, so `npm run dev` in a browser renders Browse, Cull, the
 * Lightbox and Settings with realistic content: enough to check layout, type
 * and clipping at any window size. It is **not** the product: nothing renders
 * through the GPU, Develop's canvas is a placeholder, and nothing is saved.
 *
 * Loaded only by `main.tsx` under `import.meta.env.DEV` *and* when no real Tauri
 * runtime is present, so it is never in a production build and never shadows
 * the real backend in `tauri dev`.
 *
 * An unknown command logs a warning and answers `null`: the console then says
 * exactly which fixture a new surface needs. Any fixture can be overridden per
 * browser with `localStorage["mock:<command>"] = JSON.stringify(value)`, and
 * made to answer late with `localStorage["mock:delay:<command>"] = "3000"`, to
 * look at what a slow command shows while it runs (a raw opening in Develop).
 */
import { mockIPC } from "@tauri-apps/api/mocks";
import type { EditStack, Photo } from "../bindings";
import { DEFAULT_STACK } from "../types";
import { clipAudit } from "./clipAudit";

const CAMERAS = ["Sony ILCE-6400", "Nikon Z 6", "Canon EOS R6", "Fujifilm X-T3"];
// The long values are deliberate: dense rows have to survive real-world text.
const LENSES = [
  "E 18-135mm F3.5-5.6 OSS",
  "NIKKOR Z 24-70mm f/4 S",
  "FE 200-600mm F5.6-6.3 G OSS + 1.4x Teleconverter",
];
const GENRES = ["landscape", "wildlife", "portrait", "street", "macro", "architecture"];
const SUBJECTS = ["mountain", "coast", "long-tailed tit on a frosted hawthorn", "tree", "city", "flower", "dog", "sky"];
const PLACES = ["Karoo", "Cape Point", "Llanfairpwllgwyngyll railway station car park", "Edinburgh"];
/** Photos not yet checked for a GPS position; read_locations clears it. */
let MOCK_LOCATIONS_UNREAD = 1204;
/** Trips: one mixing a phone's GPS shots with camera frames, one named by
 *  hand, and one placed entirely by GPS — the three states a card can be in. */
function mockTrip(ids: number[], start: string, end: string, existingPlace: string | null, places: { name: string; count: number }[], located: number) {
  return {
    photoIds: ids,
    startDate: start,
    endDate: end,
    count: ids.length,
    existingPlace,
    places,
    located,
    thumbPath: `mock/thumb/${ids[0]}`,
    thumbPaths: ids.slice(0, 12).map((id) => `mock/thumb/${id}`),
    cameras: located > 0 && located < ids.length ? ["Sony ILCE-6400", "iPhone 15 Pro"] : ["Sony ILCE-6400"],
  };
}
const range = (a: number, b: number) => Array.from({ length: b - a + 1 }, (_, i) => a + i);
const MOCK_TRIPS = [
  mockTrip(range(1, 14), "2025-06-12 09:00:00", "2025-06-15 19:00:00", null, [{ name: "Edinburgh, United Kingdom", count: 3 }], 3),
  mockTrip(range(15, 26), "2025-03-02 08:00:00", "2025-03-04 17:00:00", "Karoo", [{ name: "Karoo", count: 12 }], 0),
  mockTrip(range(27, 31), "2024-11-20 10:00:00", "2024-11-20 16:00:00", null, [{ name: "Cape Town, South Africa", count: 5 }], 5),
];

/** Named places; the two named from trips have no position until pinned. */
const MOCK_PLACES: { id: number; name: string; lat: number | null; lon: number | null; count: number; country: string | null }[] = [
  { id: 1, name: "Edinburgh, United Kingdom", lat: 55.95, lon: -3.19, count: 40, country: "United Kingdom" },
  { id: 2, name: "Cape Town, South Africa", lat: -33.92, lon: 18.42, count: 12, country: "South Africa" },
  { id: 3, name: "Stellenbosch, South Africa", lat: -33.93, lon: 18.86, count: 5, country: "South Africa" },
  { id: 4, name: "Karoo", lat: null, lon: null, count: 12, country: null },
  { id: 5, name: "Isle of Skye", lat: null, lon: null, count: 31, country: null },
];

/** A soft two-tone gradient as the "photo" — distinct per id, so a grid of
 *  them reads as a grid of different frames. */
function placeholder(seed: number, w: number, h: number): string {
  const hue = (seed * 47) % 360;
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}">` +
    `<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1">` +
    `<stop offset="0" stop-color="hsl(${hue} 45% 55%)"/>` +
    `<stop offset="1" stop-color="hsl(${(hue + 60) % 360} 35% 22%)"/>` +
    `</linearGradient></defs><rect width="100%" height="100%" fill="url(#g)"/>` +
    `<circle cx="${w * 0.62}" cy="${h * 0.4}" r="${Math.min(w, h) * 0.16}" fill="hsl(${hue} 60% 75%)" opacity="0.7"/>` +
    `</svg>`;
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
}

/** A burst of unjudged frames with two people in, for Cull and Compare. */
const BURST = [12, 18, 24];

function photo(id: number): Photo {
  const portrait = id % 5 === 0;
  const day = 1 + (id % 27);
  const month = 1 + (Math.floor(id / 9) % 12);
  const year = 2025 - (Math.floor(id / 2000) % 6);
  // Every eleventh is a phone JPEG.
  const name =
    id % 8 === 0
      ? `2025-06-14_wedding_ceremony_second-shooter_${1000 + id}.ARW`
      : id % 11 === 0
        ? `IMG_${1000 + id}.JPG`
        : `DSC0${1000 + id}.ARW`;
  return {
    id,
    path: `C:/Photos/2025/${name}`,
    filename: name,
    captureDate: `${year}-${String(month).padStart(2, "0")}-${String(day).padStart(2, "0")} 10:${String(id % 60).padStart(2, "0")}:00`,
    camera: CAMERAS[id % CAMERAS.length]!,
    lens: LENSES[id % LENSES.length]!,
    iso: [100, 200, 400, 800, 3200][id % 5]!,
    aperture: [2.8, 4, 5.6, 8, 11][id % 5]!,
    shutter: ["1/250", "1/1000", "1/60", "1/2000", "2"][id % 5]!,
    focalLength: [24, 35, 70, 135, 400][id % 5]!,
    width: portrait ? 4000 : 6000,
    height: portrait ? 6000 : 4000,
    orientation: 1,
    thumbPath: `mock/thumb/${id}`,
    previewPath: `mock/preview/${id}`,
    rating: id % 6,
    flag: id % 7 === 0 ? 1 : id % 11 === 0 ? -1 : 0,
    colorLabel: id % 9 === 0 ? 2 : 0,
    tagged: id % 4 !== 0,
    genre: id % 4 === 0 ? null : GENRES[id % GENRES.length]!,
    subjects: [SUBJECTS[id % SUBJECTS.length]!, SUBJECTS[(id * 3) % SUBJECTS.length]!],
    place: PLACES[id % PLACES.length]!,
    placeId: (id % PLACES.length) + 1,
    caption: id % 3 === 0 ? "A long valley under low cloud, the road running out of frame" : null,
    sharpness: 0.2 + ((id * 37) % 100) / 100,
    // One black frame among the unjudged ones, for a suggested cull to set aside.
    brightness: id === 36 ? 0.01 : 0.3 + ((id * 13) % 40) / 100,
    // The burst `photo_faces` puts two people in; 18 is the frame with the blink.
    faceCount: BURST.includes(id) ? 2 : GENRES[id % GENRES.length] === "portrait" ? 1 : 0,
    faceSharpness: null,
    eyesOpen: BURST.includes(id)
      ? id === 18 ? 0.08 : 0.93
      : GENRES[id % GENRES.length] === "portrait" ? (id % 3 === 0 ? 0.1 : 0.95) : null,
    eyesOverride: null,
    variant: 0,
    variantName: null,
    edited: id % 5 === 1,
    editedPath: null,
    // One ghost, so the missing-file badge has something to show.
    missing: id === 7,
    // One corrected by hand, so "back to the file's time" shows.
    captureEdited: id === 2,
    flagSuggested: false,
  };
}

/** 48 photos by default; `localStorage["mock:photoCount"] = "20000"` for a
 *  library at the app's page cap, to see how the UI holds up at scale. */
function photoCount(): number {
  try {
    const n = Number(localStorage.getItem("mock:photoCount"));
    return Number.isInteger(n) && n > 0 ? Math.min(n, 50_000) : 48;
  } catch {
    return 48;
  }
}

const PHOTOS: Photo[] = Array.from({ length: photoCount() }, (_, i) => photo(i + 1));

function counts(values: (string | null)[]) {
  const m = new Map<string, number>();
  for (const v of values) if (v) m.set(v, (m.get(v) ?? 0) + 1);
  return [...m].map(([value, count]) => ({ value, count })).sort((a, b) => b.count - a.count);
}

const FACETS = {
  genres: counts(PHOTOS.map((p) => p.genre)),
  subjects: counts(PHOTOS.flatMap((p) => p.subjects)),
  conditions: counts(PHOTOS.map((_, i) => (i % 3 === 0 ? "golden hour" : i % 3 === 1 ? "overcast" : null))),
  places: PLACES.map((name, i) => ({ id: i + 1, name, count: PHOTOS.filter((p) => p.placeId === i + 1).length })),
  flags: counts(PHOTOS.map((p) => String(p.flag))),
  colorLabels: counts(PHOTOS.map((p) => String(p.colorLabel))),
  ratings: counts(PHOTOS.map((p) => String(p.rating))),
  total: PHOTOS.length,
  untagged: PHOTOS.filter((p) => !p.tagged).length,
  dateMin: PHOTOS.reduce((m, p) => (p.captureDate! < m ? p.captureDate! : m), "9999"),
  dateMax: "2025-12-27 10:00:00",
};

/** The keyword tree: a branch someone built, holding some of the
 *  tagger's words, and a keyword made by hand that no photo has yet. Mutable,
 *  so filing, renaming and deleting can be tried in the browser. */
const MOCK_KEYWORDS: { name: string; parent: string | null; kept: boolean }[] = [
  { name: "nature", parent: null, kept: true },
  { name: "mountain", parent: "nature", kept: false },
  { name: "coast", parent: "nature", kept: false },
  { name: "tree", parent: "nature", kept: false },
  { name: "animals", parent: "nature", kept: true },
  { name: "dog", parent: "animals", kept: false },
  { name: "birds", parent: "animals", kept: true },
  { name: "long-tailed tit on a frosted hawthorn", parent: "birds", kept: false },
  { name: "to print", parent: null, kept: true },
];

/** The subject facet with each branch counted once per photo, as the backend
 *  counts it. */
function keywordCounts() {
  const parent = new Map(MOCK_KEYWORDS.filter((k) => k.parent).map((k) => [k.name, k.parent!]));
  const m = new Map<string, number>();
  for (const p of PHOTOS) {
    const hit = new Set<string>();
    for (let s of p.subjects) {
      for (let guard = 0; s && guard < 32; guard++) {
        hit.add(s);
        s = parent.get(s) ?? "";
      }
    }
    for (const k of hit) m.set(k, (m.get(k) ?? 0) + 1);
  }
  return [...m].map(([value, count]) => ({ value, count })).sort((a, b) => b.count - a.count);
}

function keywordError(message: string): never {
  throw message;
}

/** Photos whose tagger keywords have been confirmed. */
const MOCK_CONFIRMED = new Set<number>();

let openId = 1;
let frame = 0;
// A short history, for the History menu.
const HISTORY = ["Open", "Exposure", "Contrast", "White balance"];
let historyAt = HISTORY.length - 1;
// Whether AI noise reduction has "run" on the open photo.
let denoiseReady = false;
/** The edit last handed back, which an erase is added to or taken from. */
let lastStack = DEFAULT_STACK as unknown as EditStack;
function developInfo(photoId: number, stack: EditStack) {
  if (photoId !== openId) denoiseReady = false;
  openId = photoId;
  lastStack = stack;
  const p = PHOTOS.find((x) => x.id === photoId);
  return {
    photoId,
    width: p?.width ?? 6000,
    height: p?.height ?? 4000,
    stack,
    frame: ++frame,
    canUndo: historyAt > 0,
    canRedo: historyAt < HISTORY.length - 1,
    history: HISTORY,
    historyAt,
    fromRaw: !/\.(jpe?g|png|tiff?|webp)$/i.test(p?.filename ?? ""),
    denoiseReady,
  };
}

/** Answers per command. Anything not here gets `null` and a console warning. */
// Mutable, so hiding someone in a preview sticks until the page reloads.
const mockCovers = new Map<number, number>();
const MOCK_PEOPLE = [
  { id: 1, name: null, count: 6, coverPath: "C:/Photos/DSC00087.ARW", coverBox: [0.4, 0.25, 0.12, 0.18], portraitPath: null, hidden: false },
  { id: 2, name: null, count: 3, coverPath: "C:/Photos/DSC00095.ARW", coverBox: [0.45, 0.2, 0.12, 0.18], portraitPath: null, hidden: false },
  { id: 3, name: "Beth", count: 3, coverPath: "C:/Photos/DSC00145.ARW", coverBox: [0.5, 0.2, 0.14, 0.2], portraitPath: null, hidden: false },
  // A second Beth, split off in one photo: the "same person as…" case.
  { id: 4, name: "Beth", count: 1, coverPath: "C:/Photos/DSC00150.ARW", coverBox: [0.3, 0.3, 0.1, 0.15], portraitPath: null, hidden: false },
  { id: 5, name: null, count: 1, coverPath: "C:/Photos/DSC00151.ARW", coverBox: [0.6, 0.3, 0.1, 0.15], portraitPath: null, hidden: true },
];

const BLANK_IPTC = { title: null, caption: null, creator: null, copyright: null, credit: null, usageTerms: null };
const iptc = new Map<number, Record<string, string | null>>();
const iptcTemplates: { id: number; name: string; fields: unknown }[] = [
  {
    id: 1,
    name: "Agency delivery",
    fields: { ...BLANK_IPTC, creator: "Jane Doe", copyright: "© 2025 Jane Doe", credit: "Jane Doe / Agency" },
  },
];

const groups: { id: number; name: string }[] = [{ id: 1, name: "Photo books" }];

const MOCK_COLLECTIONS: { id: number; name: string; count: number; groupId: number | null }[] = [
  { id: 1, name: "Portfolio", count: 12, groupId: null },
  { id: 2, name: "Print shortlist", count: 5, groupId: null },
];

const HANDLERS: Record<string, (args: Record<string, unknown>) => unknown> = {
  locations_status: () => MOCK_LOCATIONS_UNREAD,
  read_locations: () => {
    MOCK_LOCATIONS_UNREAD = 0;
    return { read: 1204, found: 57, placed: 57 };
  },
  list_places: () => MOCK_PLACES.map((p) => ({ ...p })),
  rename_place: (a) => {
    const name = (a.name as string).trim();
    const clash = MOCK_PLACES.find((q) => q.id !== a.placeId && q.name.toLowerCase() === name.toLowerCase());
    if (clash) return clash.id;
    const p = MOCK_PLACES.find((q) => q.id === a.placeId);
    if (p) p.name = name;
    return null;
  },
  merge_places: (a) => {
    const into = MOCK_PLACES.find((q) => q.id === a.intoId);
    for (const id of a.fromIds as number[]) {
      const i = MOCK_PLACES.findIndex((q) => q.id === id && id !== a.intoId);
      if (i < 0 || !into) continue;
      const [from] = MOCK_PLACES.splice(i, 1);
      into.count += from!.count;
      if (into.lat == null) Object.assign(into, { lat: from!.lat, lon: from!.lon });
    }
    return null;
  },
  place_thumbs: (a) => {
    const id = a.placeId as number;
    return [0, 1, 2, 3].map((i) => `mock/thumb/${id * 7 + i}`);
  },
  delete_place: (a) => {
    const i = MOCK_PLACES.findIndex((q) => q.id === a.placeId);
    if (i >= 0) MOCK_PLACES.splice(i, 1);
    return null;
  },
  set_place_position: (a) => {
    const p = MOCK_PLACES.find((q) => q.id === a.placeId);
    if (p) Object.assign(p, { lat: a.lat as number, lon: a.lon as number });
    return null;
  },
  "plugin:dialog|open": (a) =>
    (a.options as { directory?: boolean } | undefined)?.directory ? "D:/Photos" : ["C:/Users/me/Tracks/skye-walk.gpx"],
  gpx_preview: (a) => {
    const offset = (a.offsetMinutes as number | null) ?? 60;
    const hit = offset === 60 ? 1 : Math.abs(offset - 60) <= 30 ? 0.4 : 0;
    return {
      points: 4812,
      candidates: 9340,
      offsetMinutes: offset,
      matched: Math.round(212 * hit),
      days: hit
        ? [
            { date: "2025-06-13", count: Math.round(148 * hit) },
            { date: "2025-06-14", count: Math.round(64 * hit) },
          ]
        : [],
    };
  },
  gpx_apply: () => ({ matched: 212, placed: 212 }),
  // Each person's cover is their first face unless one was chosen.
  list_people: () => MOCK_PEOPLE.map((p) => ({ ...p, coverFaceId: mockCovers.get(p.id) ?? p.id * 100 })),
  set_cover_face: (a) => {
    const face = Number(a.faceId);
    mockCovers.set(Math.floor(face / 100), face);
    return null;
  },
  unplaced_faces: () => [
    Array.from({ length: 9 }, (_, i) => ({
      faceId: 9000 + i,
      photoId: 5 + i * 3,
      thumbPath: `mock/thumb/${5 + i * 3}`,
      boxFrac: [0.2 + (i % 3) * 0.2, 0.3, 0.08, 0.11],
      portraitPath: null,
      ungrouped: true,
      manual: false,
    })),
    9,
  ],
  dismiss_face: () => null,
  // Highlights: the first dozen of what was asked, as a stand-in.
  highlights: (a) => (a.ids as number[]).slice(0, Math.min(12, Number(a.count))),
  set_people_hidden: (args) => {
    for (const p of MOCK_PEOPLE) if ((args.ids as number[]).includes(p.id)) p.hidden = Boolean(args.hidden);
    return null;
  },
  add_photos_to_person: (args) => (args.photoIds as number[]).length,
  add_photos_to_new_person: () => 9,
  remove_photos_from_person: () => null,
  // Two people in the unjudged burst [12, 18, 24]: Ada, grouped, on the
  // left — blinking in 18 — and someone never grouped on the right, sharpest in
  // 24. Everyone else with a face has one, of nobody in particular.
  photo_faces: (a) =>
    (a.photoIds as number[]).flatMap((id) => {
      const p = PHOTOS.find((x) => x.id === id);
      const face = (n: number, box: number[], over: Record<string, unknown>) => ({
        faceId: id * 10 + n,
        photoId: id,
        personId: null,
        personName: null,
        boxFrac: box,
        sharpness: 0.5,
        eyeOpen: 0.93,
        portraitPath: null,
        ...over,
      });
      if (BURST.includes(id)) {
        return [
          face(1, [0.3, 0.3, 0.14, 0.2], { personId: 1, personName: "Ada", eyeOpen: id === 18 ? 0.08 : 0.95, sharpness: id === 12 ? 0.2 : 0.7 }),
          face(2, [0.56, 0.32, 0.12, 0.18], { sharpness: id === 24 ? 0.9 : 0.4 }),
        ];
      }
      return p && (p.faceCount ?? 0) > 0 ? [face(1, [0.4, 0.3, 0.2, 0.3], {})] : [];
    }),
  photo_people: () => [
    { id: 3, manual: false },
    { id: 2, manual: true },
  ],
  group_people: () => 3,
  // The mock library uses People already.
  people_enabled: () => true,
  set_people_enabled: () => null,
  erase_face_data: () => ({ scrubbed: 2, kept: 1 }),
  merge_people: () => 2,
  people_suggestions: () => [{ a: 1, b: 2, score: 0.31 }],
  person_faces: (args) =>
    Array.from({ length: MOCK_PEOPLE.find((p) => p.id === args.personId)?.count ?? 3 }, (_, i) => ({
      faceId: Number(args.personId) * 100 + i,
      photoId: 3 + i * 4,
      thumbPath: `mock/thumb/${3 + i * 4}`,
      boxFrac: [0.3 + i * 0.05, 0.2, 0.14, 0.2],
      portraitPath: null,
      ungrouped: args.personId === 4,
      manual: i === 2,
    })),
  keep_people_apart: () => null,
  // Anything under a DCIM folder is on a card.
  folder_is_card: (args) => /[\\/]DCIM([\\/]|$)/i.test(String(args.path)),
  // A backup path of "X:" is a drive that is not connected.
  folder_exists: (a) => !String((a as { path?: string }).path ?? "").startsWith("X:"),
  import_sources: () => [{ path: "E:\\DCIM", label: "E:" }],
  import_preview: () => ({
    files: 412,
    already: 12,
    bytes: 10_850_000_000,
    days: [
      { date: "2025-06-13", files: 12, newFiles: 0 },
      { date: "2025-06-14", files: 400, newFiles: 400 },
    ],
    unreadable: [],
    leftBehind: { videos: 3, audio: 1, other: 0, examples: ["C0001.MP4", "C0002.MP4", "C0003.MP4", "DSC00412.WAV"] },
  }),
  // An import that lost frames to a flaky reader and could not finish the
  // backup: the result panel's worst case.
  import_run: () => ({
    copied: 397,
    skipped: 12,
    failed: [
      { path: "E:\\DCIM\\100MSDCF\\DSC00213.ARW", reason: "E:\\DCIM\\100MSDCF\\DSC00213.ARW read differently twice — the card or its reader may be failing" },
      { path: "E:\\DCIM\\100MSDCF\\DSC00214.ARW", reason: "reading E:\\DCIM\\100MSDCF\\DSC00214.ARW: The request could not be performed because of an I/O device error." },
    ],
    notBackedUp: [{ path: "E:\\DCIM\\100MSDCF\\DSC00398.ARW", reason: "writing F:\\Backup\\DSC00398.ARW.part: There is not enough space on the disk." }],
    unreadable: [],
    leftBehind: { videos: 3, audio: 1, other: 0, examples: ["C0001.MP4", "C0002.MP4", "C0003.MP4", "DSC00412.WAV"] },
    stopped: false,
    notReached: [],
    folders: ["C:/Photos/2025/2025-06-14"],
    shootId: 1,
  }),
  finish_shoot_backup: () => ({ backedUp: 397, failed: [] }),
  list_library_folders: () => ({
    folders: [
      { id: 1, path: "C:/Photos", photos: 40, missing: 1, online: true },
      { id: 2, path: "E:/Archive/2019", photos: 8, missing: 8, online: false },
    ],
    unfiled: 3,
  }),
  // Copies, as a real call's JSON always is: the same array back every time
  // would hide a tag change from anything that compares by identity.
  get_photos: () => PHOTOS.map((p) => ({ ...p })),
  get_photo: (a) => PHOTOS.find((p) => p.id === a.id) ?? null,
  get_photos_by_ids: (a) => PHOTOS.filter((p) => (a.ids as number[]).includes(p.id)),
  get_facets: () => ({ ...FACETS, subjects: keywordCounts() }),
  // The rail's optional counts, from the fake photos. The filter is
  // ignored, as `get_facets`'s fixture ignores it.
  get_extra_facets: () => {
    const tally = (values: (string | null)[]) => {
      const m = new Map<string, number>();
      for (const v of values) if (v) m.set(v, (m.get(v) ?? 0) + 1);
      return [...m].map(([value, count]) => ({ value, count })).sort((a, b) => b.count - a.count);
    };
    const bucket = (v: number | null, edges: [string, number][]) =>
      v == null ? null : (edges.find(([, hi]) => v <= hi)?.[0] ?? null);
    return {
      cameras: tally(PHOTOS.map((p) => p.camera)),
      lenses: tally(PHOTOS.map((p) => p.lens)),
      focalRanges: tally(PHOTOS.map((p) => bucket(p.focalLength, [["0-24", 24], ["25-50", 50], ["51-105", 105], ["106-200", 200], ["201-", Infinity]]))),
      isoRanges: tally(PHOTOS.map((p) => bucket(p.iso, [["0-400", 400], ["401-1600", 1600], ["1601-6400", 6400], ["6401-", Infinity]]))),
      apertureRanges: tally(PHOTOS.map((p) => bucket(p.aperture, [["0-2.8", 2.8], ["2.9-5.6", 5.6], ["5.7-", Infinity]]))),
      // From the fake photos: all raws, mostly landscape, a few faces.
      fileKinds: tally(PHOTOS.map((p) => (/\.(jpe?g)$/i.test(p.path) ? "jpeg" : "raw"))),
      shapes: tally(PHOTOS.map((p) => (p.width > p.height ? "landscape" : p.width < p.height ? "portrait" : "square"))),
      faceCounts: tally(PHOTOS.map((p) => (p.faceCount == null ? null : p.faceCount >= 6 ? "6-" : p.faceCount >= 3 ? "3-5" : String(p.faceCount)))),
      // The fake photos' colours, from their placeholder's hue.
      palette: tally(PHOTOS.flatMap((p) => [["orange", "green", "teal", "blue", "purple"][p.id % 5]!, p.id % 3 ? "grey" : "black"])),
      people: MOCK_PEOPLE.filter((p) => p.name && !p.hidden).map((p) => ({ id: p.id, name: p.name!, count: p.count })),
      edited: PHOTOS.filter((p) => p.edited).length,
      copies: PHOTOS.filter((p) => p.variant > 0).length,
      withFaces: PHOTOS.filter((p) => (p.faceCount ?? 0) > 0).length,
      located: 0,
      unplaced: PHOTOS.filter((p) => p.placeId == null).length,
    };
  },
  get_genres: () => GENRES,
  list_collections: () => MOCK_COLLECTIONS.map((c) => ({ ...c })),
  duplicate_groups: () => [
    [1, 2, 3],
    [10, 11],
    // A burst nobody has judged yet, for Cull's suggestions to choose from.
    [12, 18, 24],
  ],
  set_flags: () => null,
  clear_place: (a) => (a.photoIds as number[]).length,
  list_shoots: () => [
    { id: 1, name: "Smith wedding", createdAt: "2026-09-20T10:00:00Z", photos: 120, judged: 72, picks: 18, editedPicks: 4, exportedPicks: 0, notCopied: 2, notBackedUp: 1 },
    { id: 2, name: "Skye, day two", createdAt: "2026-09-12T10:00:00Z", photos: 40, judged: 40, picks: 9, editedPicks: 9, exportedPicks: 9, notCopied: 0, notBackedUp: 0 },
  ],
  delete_shoot: () => null,
  // AI noise reduction: a pause standing in for the model, then the Amount.
  develop_denoise: (a) =>
    new Promise((resolve) =>
      window.setTimeout(() => {
        denoiseReady = true;
        const detail = { ...DEFAULT_STACK.detail, aiDenoise: (a.amount as number | null) ?? 1 };
        resolve(developInfo(openId, { ...DEFAULT_STACK, detail } as unknown as EditStack));
      }, 1500),
    ),
  // The eyedropper: a fixed cooling, as if the picked point were warm.
  pick_wb_develop: () =>
    developInfo(openId, { ...DEFAULT_STACK, warmth: -0.32, tint: 0.08 } as unknown as EditStack),
  name_face: () => null,
  detach_face: () => 1,
  rename_collection: () => null,
  add_tag: (a) => {
    const path = String(a.name).split(/[|>]/).map((x) => x.trim().toLowerCase()).filter(Boolean);
    path.forEach((name, i) => {
      if (i > 0 && !MOCK_KEYWORDS.some((k) => k.name === name)) MOCK_KEYWORDS.push({ name, parent: path[i - 1]!, kept: true });
    });
    const leaf = path[path.length - 1];
    if (leaf)
      for (const p of PHOTOS) if ((a.photoIds as number[]).includes(p.id) && !p.subjects.includes(leaf)) p.subjects = [...p.subjects, leaf];
    return null;
  },
  remove_tag: (a) => {
    for (const p of PHOTOS) if ((a.photoIds as number[]).includes(p.id)) p.subjects = p.subjects.filter((s) => s !== a.name);
    return null;
  },
  // The tagger's keywords on a photo: its second subject, until confirmed.
  ai_keywords: (a) =>
    (a.photoIds as number[])
      .map((id) => PHOTOS.find((p) => p.id === id))
      .filter((p): p is Photo => !!p && !MOCK_CONFIRMED.has(p.id) && p.subjects.length > 1)
      .map((p) => ({ photoId: p.id, names: [p.subjects[1]!] })),
  confirm_keywords: (a) => {
    for (const id of a.photoIds as number[]) MOCK_CONFIRMED.add(id);
    return (a.photoIds as number[]).length;
  },
  add_folder_keywords: () => ({ photos: 12, keywords: 3 }),
  remove_folder_keywords: () => 12,
  set_capture_time: (a) => ({ changed: (a.photoIds as number[]).length, undated: 0 }),
  revert_capture_time: (a) => (a.photoIds as number[]).length,
  paste_location: (a) => ({ placed: (a.photoIds as number[]).length, positioned: 1 }),
  paste_keywords: () => 2,
  keyword_tree: () => {
    const used = new Set(PHOTOS.flatMap((p) => p.subjects));
    const names = new Set([...MOCK_KEYWORDS.map((k) => k.name), ...used]);
    return [...names].sort().map((name) => {
      const k = MOCK_KEYWORDS.find((x) => x.name === name);
      return { name, parent: k?.parent ?? null, kept: k?.kept ?? false };
    });
  },
  create_keyword: (a) => {
    const name = String(a.name).trim().toLowerCase();
    if (MOCK_KEYWORDS.some((k) => k.name === name)) keywordError(`“${name}” already exists`);
    MOCK_KEYWORDS.push({ name, parent: (a.parent as string | null) ?? null, kept: true });
    return null;
  },
  move_keyword: (a) => {
    const k = MOCK_KEYWORDS.find((x) => x.name === a.name);
    if (k) k.parent = (a.parent as string | null) ?? null;
    else MOCK_KEYWORDS.push({ name: String(a.name), parent: (a.parent as string | null) ?? null, kept: true });
    return null;
  },
  rename_keyword: (a) => {
    const to = String(a.newName).trim().toLowerCase();
    for (const k of MOCK_KEYWORDS) {
      if (k.name === a.name) k.name = to;
      if (k.parent === a.name) k.parent = to;
    }
    for (const p of PHOTOS) p.subjects = [...new Set(p.subjects.map((s) => (s === a.name ? to : s)))];
    return to;
  },
  delete_keyword: (a) => {
    const gone = MOCK_KEYWORDS.find((k) => k.name === a.name);
    for (const k of MOCK_KEYWORDS) if (k.parent === a.name) k.parent = gone?.parent ?? null;
    MOCK_KEYWORDS.splice(MOCK_KEYWORDS.findIndex((k) => k.name === a.name) >>> 0, gone ? 1 : 0);
    let n = 0;
    for (const p of PHOTOS) {
      if (p.subjects.includes(String(a.name))) n++;
      p.subjects = p.subjects.filter((s) => s !== a.name);
    }
    return n;
  },
  remove_from_collection: () => null,
  search_vocabulary: () => ({
    people: MOCK_PEOPLE.filter((p) => p.name).map((p) => ({ id: p.id, name: p.name })),
    places: MOCK_PLACES.map((p) => ({ ...p })),
    genres: FACETS.genres.map((g) => g.value),
    conditions: ["golden hour", "overcast"],
    cameras: [...new Set(PHOTOS.map((p) => p.camera))],
    lenses: [...new Set(PHOTOS.map((p) => p.lens))],
    keywords: [...new Set(PHOTOS.flatMap((p) => p.subjects))].sort(),
  }),
  import_xmp: (a) => ({
    read: (a.photoIds as number[]).length > 0 ? 1 : 0,
    people: 0,
    kept: 0,
    // A sidecar dated by another editor, to exercise the offer.
    times: (a.photoIds as number[]).slice(0, 1).map((photoId) => ({ photoId, time: "2025-06-14 09:30:00" })),
    verdictsChanged: (a.photoIds as number[]).length > 0 ? 1 : 0,
  }),
  // Sidecars beside a dozen photos, for "Import all" to ask about.
  count_sidecars: () => 12,
  use_sidecar_times: (a) => ({ changed: (a.times as unknown[]).length, undated: 0 }),
  export_xmp: (a) => ({ written: Math.max(1, (a.photoIds as number[]).length), failed: 0, errors: [], cancelled: false }),
  quality_status: () => 0,
  faces_status: () => ({ toDetect: 0, toRecognise: 0, recogniserInstalled: true }),
  analyze_faces: () => ({ photos: 0, faces: 0, embedded: 0, embedFailed: 0, firstError: null, stopped: false }),
  // No model here: "names" each free-text tag visibly, so the preview shows
  // where learned tag names land (lib/tagNames).
  tag_translations: () => ({}),
  translate_tags: (args) =>
    Object.fromEntries((args.names as string[]).map((n) => [n, `${n} · ${String(args.language)}`])),
  get_trip_clusters: () => MOCK_TRIPS.map((t) => ({ ...t, places: t.places.map((p) => ({ ...p })) })),
  name_trip: (a) => {
    const t = MOCK_TRIPS.find((x) => x.photoIds.join() === (a.photoIds as number[]).join());
    if (!t) return { placed: 0, kept: 0 };
    const placed = t.count - t.located;
    if (placed > 0) {
      const name = a.name as string;
      t.places = [{ name, count: placed }, ...t.places.filter((p) => p.name !== t.existingPlace && p.name !== name)];
      t.existingPlace = name;
    }
    return { placed, kept: t.located };
  },
  licence_text: () => "Mozilla Public License Version 2.0 (mock)",
  get_export_rights: () => ({ creator: "", copyright: "" }),
  list_smart_collections: () => [],
  scan_library: () => ({
    added: 0,
    skipped: PHOTOS.length,
    failed: 0,
    relinked: 0,
    missing: 1,
    firstFailure: null,
  }),
  caption_status: () => 0,
  search_index_status: () => 0,
  backup_catalogue_if_due: () => null,
  on_battery: () => false,
  list_catalogue_backups: () => [
    {
      path: "C:/Users/me/AppData/Local/app.sourcelight/backups/library-2025-09-01.db",
      filename: "library-2025-09-01.db",
      bytes: 4_812_800,
      modified: 1756720800,
      automatic: true,
    },
  ],
  catalogue_restore_pending: () => false,
  catalogue_recovery: () => null,
  unclean_exit: () => null,
  // AI settings round-trip so the AI tab's switches can be exercised.
  get_ai_settings: () => ({ provider: mockAi.provider, model: "", defaultModel: "" }),
  set_ai_settings: (a) => {
    mockAi.provider = a.provider as string;
    return null;
  },
  // AI noise reduction is "installed", so its panel can be driven here.
  ensure_ai_ready: (a) =>
    // Switched off refuses every purpose, as the core does.
    mockAi.provider === "none"
      ? {
          ready: false,
          provider: "off",
          model: "",
          detail: "",
          needs: [],
          askFirst: false,
          metered: false,
          blocked: "On-device AI is switched off — turn it back on in Settings → AI",
        }
      : {
          ready: a.purpose === "denoise" || a.purpose === "erase",
          provider: mockAi.provider,
          model: "",
          detail: "",
          needs: [],
          askFirst: false,
          metered: false,
          blocked: "Nothing is installed in the mock backend",
        },
  vlm_catalogue: () => VLMS,
  local_vlm_storage: () => ({ runtimeInstalled: false, runtimeMb: 91, models: [] }),
  remove_local_vlm: () => null,
  ai_available_models: () => [
    {
      id: "tagger",
      name: "Auto-Tagging",
      purpose: "Keyword & subject tagging, no server needed (CLIP)",
      license: "MIT",
      approxMb: 581,
      installed: true,
    },
    {
      id: "faces",
      name: "Face tools",
      purpose: "Find faces, and flag closed eyes and soft faces in a cull",
      license: "MIT",
      approxMb: 1,
      installed: true,
    },
    {
      id: "people",
      name: "People",
      purpose: "Recognise the same person across photos, to group them in People",
      license: "Apache-2.0",
      approxMb: 261,
      installed: false,
    },
    {
      id: "subject",
      name: "Select Subject",
      purpose: "One-click subject / background cut-out",
      license: "MIT",
      approxMb: 224,
      installed: false,
    },
  ],
  hardware_report: () => ({
    gpuName: "NVIDIA GeForce RTX 5070 Ti",
    gpuKind: "discrete",
    backend: "Vulkan",
    cpuThreads: 16,
    vramMb: 16303,
    ramMb: 32768,
    tier: "full",
    advice: "This machine runs the full models comfortably.",
  }),
  // Develop: the stack round-trips so sliders move; the canvas is a placeholder.
  open_develop: (a) => developInfo(a.photoId as number, DEFAULT_STACK as unknown as EditStack),
  set_develop_stack: (a) => developInfo(openId, a.stack as EditStack),
  reset_develop: () => developInfo(openId, DEFAULT_STACK as unknown as EditStack),
  // Magic erase: a pause standing in for the model, then one more erase.
  develop_erase: (a) =>
    new Promise((resolve) =>
      window.setTimeout(() => {
        const piece = { fill: "0".repeat(64), x: 0, y: 0, size: 512 };
        const erases = [...(lastStack.erases ?? []), { label: a.label as string, pieces: [piece] }];
        resolve(developInfo(openId, { ...lastStack, erases }));
      }, 1500),
    ),
  develop_prepare_erase: () => null,
  develop_remove_erase: (a) =>
    developInfo(openId, {
      ...lastStack,
      erases: (lastStack.erases ?? []).filter((_, i) => i !== (a.index as number)),
    }),
  undo_develop: () => developInfo(openId, DEFAULT_STACK as unknown as EditStack),
  redo_develop: () => developInfo(openId, DEFAULT_STACK as unknown as EditStack),
  close_develop: () => null,
  list_luts: () => [
    { name: "PortraitNegative.cube", title: "Portrait Negative", family: "film", blurb: "Warm skin tones and muted greens.", grouped: false, imported: false },
    { name: "Cinescope.cube", title: "Cinescope", family: "cinema", blurb: "Teal shadows and orange highlights.", grouped: false, imported: false },
    { name: "my-grade.cube", title: "My grade", family: "Imported", blurb: "", grouped: false, imported: true },
  ],
  list_presets: () => [],
  develop_go_to: (a) => {
    historyAt = a.index as number;
    return developInfo(openId, DEFAULT_STACK as unknown as EditStack);
  },
  paste_settings_from: () => developInfo(openId, DEFAULT_STACK as unknown as EditStack),
  copy_settings_to: (a) => (a.photoIds as number[]).length,
  // Colours.
  palette_status: () => 0,
  // Photos whose colours are still to be read: none, like the palette.
  colours_status: () => 0,
  read_palettes: () => 0,
  similar_colours: (a) => PHOTOS.filter((p) => p.id !== a.photoId && p.id % 5 === (a.photoId as number) % 5).map((p) => ({ ...p })),
  // Collection groups.
  list_collection_groups: () => [...groups],
  create_collection_group: (a) => {
    const id = groups.length + 1;
    groups.push({ id, name: (a.name as string).trim() });
    return id;
  },
  rename_collection_group: (a) => {
    const g = groups.find((x) => x.id === a.id);
    if (g) g.name = (a.name as string).trim();
    return null;
  },
  delete_collection_group: (a) => {
    const at = groups.findIndex((x) => x.id === a.id);
    if (at >= 0) groups.splice(at, 1);
    return null;
  },
  set_collection_group: (a) => {
    const c = MOCK_COLLECTIONS.find((x) => x.id === a.id);
    if (c && !a.smart) c.groupId = a.groupId as number | null;
    return null;
  },
  move_in_collection: () => null,
  // Library insights, counted from the fake library.
  insights: () => {
    const keep = (p: (typeof PHOTOS)[number]) => p.flag === 1 || p.rating >= 3;
    const count = (key: (p: (typeof PHOTOS)[number]) => string | null | undefined) => {
      const m = new Map<string, { count: number; keepers: number }>();
      for (const p of PHOTOS) {
        const k = key(p);
        if (!k) continue;
        const e = m.get(k) ?? { count: 0, keepers: 0 };
        e.count++;
        if (keep(p)) e.keepers++;
        m.set(k, e);
      }
      return [...m].map(([k, v]) => ({ key: k, ...v }));
    };
    const byCount = <T extends { count: number }>(xs: T[]) => xs.sort((a, b) => b.count - a.count);
    const byKey = <T extends { key: string }>(xs: T[]) => xs.sort((a, b) => a.key.localeCompare(b.key));
    // In the ranges' own order, as `db::insights` returns them: by key as
    // text put "106-200" before "25-50" and the harness drew it so.
    const inOrder = <T extends { key: string }>(xs: T[], keys: string[]) =>
      xs.sort((a, b) => keys.indexOf(a.key) - keys.indexOf(b.key));
    const focal = (mm: number | null) =>
      mm == null ? null : mm <= 24 ? "0-24" : mm <= 50 ? "25-50" : mm <= 105 ? "51-105" : mm <= 200 ? "106-200" : "201-";
    const lenses = byCount(count((p) => p.lens));
    return {
      total: PHOTOS.length,
      keepers: PHOTOS.filter(keep).length,
      years: byKey(count((p) => p.captureDate?.slice(0, 4))),
      months: byKey(count((p) => p.captureDate?.slice(0, 7))),
      cameras: byCount(count((p) => p.camera)),
      lenses,
      focal: inOrder(count((p) => focal(p.focalLength)), ["0-24", "25-50", "51-105", "106-200", "201-"]),
      iso: inOrder(
        count((p) => (p.iso == null ? null : p.iso <= 400 ? "0-400" : p.iso <= 1600 ? "401-1600" : "1601-6400")),
        ["0-400", "401-1600", "1601-6400", "6401-"],
      ),
      aperture: inOrder(
        count((p) => (p.aperture == null ? null : p.aperture <= 2.8 ? "0-2.8" : p.aperture <= 5.6 ? "2.9-5.6" : "5.7-")),
        ["0-2.8", "2.9-5.6", "5.7-"],
      ),
      shutter: count((p) => p.shutter),
      lensDetail: lenses.slice(0, 4).map((l) => ({
        lens: l.key,
        focals: byKey(count((p) => (p.lens === l.key && p.focalLength != null ? String(Math.round(p.focalLength)) : null))),
      })),
      busiestDays: byCount(count((p) => p.captureDate?.slice(0, 10))).slice(0, 10),
    };
  },
  lens_keywords: (a) =>
    (a.lenses as string[]).map((lens) => ({
      lens,
      genres: [{ key: "landscape", count: 3, keepers: 2 }],
      subjects: [
        { key: "coast", count: 5, keepers: 3 },
        { key: "tree", count: 2, keepers: 0 },
      ],
    })),
  // The timeline strip: the fake library by month, or by day.
  timeline: (a) => {
    const from = a.dayFrom as string | null;
    const to = a.dayTo as string | null;
    const counts = new Map<string, number>();
    for (const p of PHOTOS) {
      const d = p.captureDate;
      if (!d) continue;
      if (from && to && (d.slice(0, 10) < from || d.slice(0, 10) > to)) continue;
      const key = d.slice(0, from && to ? 10 : 7);
      counts.set(key, (counts.get(key) ?? 0) + 1);
    }
    return [...counts].sort(([x], [y]) => x.localeCompare(y)).map(([period, count]) => ({ period, count }));
  },
  // A photo's own title, caption and rights, and templates of them.
  photo_iptc: (a) => ({ ...BLANK_IPTC, ...(iptc.get(a.photoId as number) ?? {}) }),
  iptc_selection: (a) => {
    const each = (a.photoIds as number[]).map((id) => ({ ...BLANK_IPTC, ...(iptc.get(id) ?? {}) }));
    return Object.fromEntries(
      Object.keys(BLANK_IPTC).map((k) => {
        const values = each.map((f) => f[k as keyof typeof BLANK_IPTC]);
        const mixed = values.some((v) => v !== values[0]);
        return [k, { value: mixed ? null : (values[0] ?? null), mixed }];
      }),
    );
  },
  set_iptc_field: (a) => {
    const ids = a.photoIds as number[];
    const value = (a.value as string | null)?.trim() || null;
    for (const id of ids) iptc.set(id, { ...(iptc.get(id) ?? {}), [a.field as string]: value });
    return ids.length;
  },
  list_iptc_templates: () => [...iptcTemplates],
  save_iptc_template: (a) => {
    const id = (a.id as number | null) ?? iptcTemplates.length + 1;
    const t = { id, name: (a.name as string).trim(), fields: a.fields };
    const at = iptcTemplates.findIndex((x) => x.id === id);
    if (at >= 0) iptcTemplates[at] = t;
    else iptcTemplates.push(t);
    return id;
  },
  delete_iptc_template: (a) => {
    const at = iptcTemplates.findIndex((x) => x.id === a.id);
    if (at >= 0) iptcTemplates.splice(at, 1);
    return null;
  },
  apply_iptc_template: (a) => {
    const t = iptcTemplates.find((x) => x.id === a.templateId);
    const ids = a.photoIds as number[];
    if (!t) return 0;
    const set = Object.fromEntries(Object.entries(t.fields as object).filter(([, v]) => v));
    for (const id of ids) iptc.set(id, { ...(iptc.get(id) ?? {}), ...set });
    return ids.length;
  },
  // Pretend the first photo of any set was placed from a track.
  count_track_placed: (a) => Math.min((a.photoIds as number[]).length, 1),
  gpx_replace: (a) => ({ moved: Math.min((a.photoIds as number[]).length, 1), outside: 0, missing: 0 }),
  render_for_clipboard: (a) => `mock/clip/${a.photoId as number}`,
  save_preset: () => 1,
  list_styles: () => [],
  lut_previews: () => [
    { id: "PortraitNegative.cube", preview: placeholder(11, 300, 200) },
    { id: "Cinescope.cube", preview: placeholder(13, 300, 200) },
  ],
  preset_previews: () => [],
  style_previews: () => [],
  // Develop's Presets panel and histogram, so they render with content
  // rather than empty (or, for the preset groups, not at all).
  list_preset_groups: () => [
    {
      id: "film",
      name: "Film",
      description: "Colour film looks, from soft negatives to slide film.",
      kind: "style",
      presets: [
        { id: "film-warm", name: "Warm negative", description: "Soft highlights and warm skin." },
        { id: "film-slide", name: "Slide", description: "Deep blacks and saturated colour." },
        { id: "film-faded", name: "Faded print", description: "Pale colour and lifted shadows." },
      ],
    },
    {
      id: "landscape",
      name: "Landscape",
      description: "Deeper colour and darker skies.",
      kind: "subject",
      presets: [
        { id: "landscape-vivid", name: "Vivid", description: "Strong colour and crisp detail." },
        { id: "landscape-moody", name: "Moody", description: "Darker midtones and restrained colour." },
      ],
    },
  ],
  builtin_preset_previews: (a) => (a.ids as string[]).map((id, i) => ({ id, preview: placeholder(i + 3, 300, 200) })),
  suggest_edits: () => [
    { kind: "auto", id: "auto", name: "Auto", preview: placeholder(2, 300, 200) },
    { kind: "preset", id: "film-warm", name: "Warm negative", preview: placeholder(5, 300, 200) },
    { kind: "preset", id: "landscape-vivid", name: "Vivid", preview: placeholder(8, 300, 200) },
  ],
  preset_stack: (a) => ({ ...(a.stack as EditStack), preset: { id: a.id as string, amount: a.amount as number } }),
  develop_histogram: () => {
    const bell = (c: number, w: number) => Array.from({ length: 64 }, (_, i) => Math.round(1000 * Math.exp(-(((i - c) / w) ** 2))));
    return { r: bell(38, 12), g: bell(32, 14), b: bell(26, 10), shadowClip: 0.2, highlightClip: 1.4 };
  },
  check_for_update: () => null,
  provider_status: () => ({
    provider: mockAi.provider,
    model: "",
    ready: mockAi.provider === "builtin",
    detail:
      mockAi.provider === "none"
        ? "AI is off — tags are yours to write, or turn it back on in Settings → AI"
        : "Ready · built-in, runs locally",
    models: [],
    configured: true,
  }),
};

/** The AI provider the mock is "set" to; the AI tab's switches write it. */
const mockAi = { provider: "builtin" };

/** The licence-clean vision catalogue, as `vlm_catalogue` returns it. */
const VLMS = [
  { id: "qwen3-vl-2b", name: "Qwen3-VL 2B", params: "2B", sizeGb: 1.6, minVramGb: 4, ocr: "good", license: "Apache-2.0", note: "", fit: "willRun", recommended: false, installed: false },
  { id: "qwen3-vl-4b", name: "Qwen3-VL 4B", params: "4B", sizeGb: 3.0, minVramGb: 6, ocr: "excellent", license: "Apache-2.0", note: "", fit: "willRun", recommended: false, installed: false },
  { id: "qwen3-vl-8b", name: "Qwen3-VL 8B", params: "8B", sizeGb: 5.8, minVramGb: 10, ocr: "excellent", license: "Apache-2.0", note: "", fit: "willRun", recommended: true, installed: false },
];

/** How long `cmd` should take to answer, in ms: `mock:delay:<command>`. */
function readDelay(cmd: string): number {
  try {
    return Math.max(0, Number(localStorage.getItem(`mock:delay:${cmd}`)) || 0);
  } catch {
    return 0;
  }
}

function readOverride(cmd: string): unknown {
  try {
    const raw = localStorage.getItem(`mock:${cmd}`);
    return raw === null ? undefined : JSON.parse(raw);
  } catch {
    return undefined;
  }
}

export function installMockBackend() {
  mockIPC((cmd, args) => {
    // The event plugin's listen/unlisten: answer with a handle and never fire.
    if (cmd.startsWith("plugin:event|")) return 0;
    const delay = readDelay(cmd);
    if (delay > 0) return new Promise((resolve) => setTimeout(() => resolve(answer(cmd, args)), delay));
    return answer(cmd, args);
  });

  function answer(cmd: string, args: unknown): unknown {
    // `localStorage["mock:<command>"] = JSON` overrides a fixture — how a
    // first-run state (an empty library, say) is looked at.
    const override = readOverride(cmd);
    if (override !== undefined) return override;
    const handler = HANDLERS[cmd];
    if (!handler) {
      console.warn(`[mock backend] no fixture for "${cmd}"`, args);
      return null;
    }
    return handler((args ?? {}) as Record<string, unknown>);
  }

  // `thumb://` and `develop://` URLs cannot load in a browser; hand back the
  // generated placeholders instead, at thumbnail or preview size.
  const internals = (window as unknown as { __TAURI_INTERNALS__: Record<string, unknown> })
    .__TAURI_INTERNALS__;
  internals.convertFileSrc = (path: string, protocol?: string) => {
    // `full://<id>` is the zoom's full-resolution source: sensor-sized, so the
    // swap from the 1600px preview actually happens.
    if (protocol === "full") return placeholder(Number(path) || 1, 6000, 4000);
    // Develop's preview: the open photo. `api.ts` appends the view as a query
    // string, which would land inside the SVG's text and break it; after a
    // `#` it is a fragment the image ignores.
    if (protocol === "develop") return `${placeholder(openId, 1600, 1067)}#`;
    // The clipboard takes only a PNG, so the copy gets a real one.
    const clip = /mock\/clip\/(\d+)/.exec(path);
    if (clip) {
      const canvas = document.createElement("canvas");
      [canvas.width, canvas.height] = [2048, 1365];
      const ctx = canvas.getContext("2d")!;
      ctx.fillStyle = `hsl(${(Number(clip[1]) * 47) % 360} 45% 45%)`;
      ctx.fillRect(0, 0, canvas.width, canvas.height);
      return canvas.toDataURL("image/png");
    }
    const match = /mock\/(thumb|preview)\/(\d+)/.exec(path);
    if (!match) return placeholder(7, 1600, 1067);
    const id = Number(match[2]);
    const p = PHOTOS.find((x) => x.id === id);
    const big = match[1] === "preview";
    const [w, h] = p && p.height > p.width ? [big ? 1067 : 341, big ? 1600 : 512] : [big ? 1600 : 512, big ? 1067 : 341];
    return placeholder(id, w, h);
  };
  console.info(`[mock backend] installed: a fake ${PHOTOS.length}-photo library, nothing is saved`);
}

(window as unknown as { __clipAudit: typeof clipAudit }).__clipAudit = clipAudit;
