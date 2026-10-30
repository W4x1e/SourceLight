//! Types shared with the frontend.
//!
//! Integers here are deliberately 32-bit rather than `i64`. Everything crossing
//! this boundary is a SQLite rowid or a small count, and JavaScript has no
//! integer type — `JSON.parse` silently truncates anything past 2^53, so an
//! `i64` in a UI-facing struct was always a promise we could not keep. specta
//! refuses to export 64-bit integers for exactly that reason. SQLite still
//! stores them as 64-bit; rusqlite narrows on read and errors if a value ever
//! genuinely overflows, which beats truncating silently in the browser.

use serde::{Deserialize, Serialize};

/// Which grid load a request is: a number the page picked when it
/// loaded, and the load's place in that page's count.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ViewRequest {
    pub page: u32,
    pub seq: u32,
}

/// A single indexed photo, flattened with its genre/subjects/place so the UI can
/// render a grid cell without any follow-up queries.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Photo {
    pub id: i32,
    pub path: String,
    pub filename: String,
    pub capture_date: Option<String>,
    pub camera: Option<String>,
    pub lens: Option<String>,
    pub iso: Option<i32>,
    pub aperture: Option<f64>,
    pub shutter: Option<String>,
    pub focal_length: Option<f64>,
    pub width: i32,
    pub height: i32,
    pub orientation: i32,
    pub thumb_path: Option<String>,
    pub preview_path: Option<String>,
    pub rating: i32,
    /// Pick/reject flag: -1 reject, 0 none, 1 pick.
    pub flag: i32,
    /// Colour label: 0 none, 1..=5 (red, yellow, green, blue, purple).
    pub color_label: i32,
    pub tagged: bool,
    pub genre: Option<String>,
    pub subjects: Vec<String>,
    pub place: Option<String>,
    pub place_id: Option<i32>,
    /// VLM-written description, indexed for full-text/OCR search. `None` until
    /// the captioning job has run over this photo.
    pub caption: Option<String>,
    /// Focus/sharpness score (variance of the Laplacian; higher = sharper), a
    /// relative metric for ranking a burst and flagging soft frames. `None` until
    /// the photo has been scored (at scan time, or by the backfill job).
    pub sharpness: Option<f32>,
    /// Mean brightness of the camera's preview, 0 black … 1 white — absolute,
    /// unlike focus. The cull only reads its extremes, to set aside frames that
    /// failed outright. `None` until scored, like `sharpness`.
    pub brightness: Option<f32>,
    /// Faces detected in the photo. `None` until the face pass has analysed it
    /// (needs the face models); `Some(0)` means analysed and none found.
    pub face_count: Option<i32>,
    /// Focus of the primary (largest) face — the relevant sharpness when the
    /// subject is a person. `None` when unanalysed or no face was found.
    pub face_sharpness: Option<f32>,
    /// Lowest eye-open score across every face (0 shut … 1 wide) — so a single
    /// blink in a group reads low. `None` when unanalysed or no face was found.
    pub eyes_open: Option<f32>,
    /// The user's verdict on the eye-open reading, overruling the model:
    /// `None` no opinion, `Some(0)` "not closed", `Some(1)` "yes, closed".
    /// A downcast gaze reads as a blink to the classifier, and the cull ranks
    /// on that signal — so without this a good frame stays demoted forever.
    pub eyes_override: Option<i32>,
    /// 0 for the photo the scanner found; 1, 2, … for a virtual copy of it.
    /// A copy shares the raw file and every EXIF fact about it, and owns its
    /// own edits, rating, flag, label and tags.
    pub variant: i32,
    /// What the user called this copy, if anything.
    pub variant_name: Option<String>,
    /// Whether this row carries develop edits at all — what the grid's badge
    /// reads, since the edit stack itself never crosses to the frontend.
    pub edited: bool,
    /// The photo rendered as edited, preview-sized, for the grid and loupe;
    /// none until it has been rendered, and none again once its edits change.
    pub edited_path: Option<String>,
    /// The last scan of its folder did not find the file: moved outside the
    /// library, deleted, or on a drive that is not connected. The row and
    /// all its work are kept; the file comes back when it is found again.
    pub missing: bool,
    /// Its capture time is one the user set, not the file's.
    pub capture_edited: bool,
    /// Its flag is a suggestion accepted in bulk, not one the person
    /// gave it; cleared the moment the flag changes any other way.
    pub flag_suggested: bool,
}

/// How to change a selection's capture times.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CaptureTimeChange {
    /// Every photo moved by this many seconds, keeping their spacing: a
    /// camera clock an hour out, or left on home time.
    Shift { seconds: i32 },
    /// The earliest photo set to `time`, and the rest moved by as much.
    FirstTo { time: String },
    /// Every photo set to `time` — for scans, which have none of their own.
    AllTo { time: String },
}

/// What pasting a location did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PastedLocation {
    /// Photos given the place.
    pub placed: u32,
    /// Photos given the position, having none of their own.
    pub positioned: u32,
}

/// One bar of an insight: what it counts, how many photos, and how many
/// of them are keepers — picked, or rated three stars or more.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct InsightCount {
    /// A year, month or day; a camera or lens as EXIF names it; a range's key
    /// (`db::FOCAL_RANGES` and the like); a focal length in mm; a keyword.
    pub key: String,
    pub count: u32,
    pub keepers: u32,
}

/// The focal lengths one lens was used at, by whole mm, shortest
/// first — one entry for a prime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LensInsight {
    pub lens: String,
    pub focals: Vec<InsightCount>,
}

/// What one lens took: its most common subjects and genres. Counted
/// apart from [`Insights`], since it reads every keyword of the lens's photos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LensKeywords {
    pub lens: String,
    pub subjects: Vec<InsightCount>,
    pub genres: Vec<InsightCount>,
}

/// What the photos in view say about how they were made. All counts,
/// over the current filter.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Insights {
    pub total: u32,
    pub keepers: u32,
    pub years: Vec<InsightCount>,
    pub months: Vec<InsightCount>,
    /// Most used first.
    pub cameras: Vec<InsightCount>,
    pub lenses: Vec<InsightCount>,
    /// By the rail's ranges, in their order.
    pub focal: Vec<InsightCount>,
    pub iso: Vec<InsightCount>,
    pub aperture: Vec<InsightCount>,
    /// As EXIF presents it ("1/250"), fastest first.
    pub shutter: Vec<InsightCount>,
    /// The most used lenses, each in more detail.
    pub lens_detail: Vec<LensInsight>,
    /// The ten busiest days, busiest first.
    pub busiest_days: Vec<InsightCount>,
}

/// One bar of the timeline strip: a month (`2024-05`) or a day
/// (`2024-05-14`), and how many photos in view were taken in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TimelineBar {
    pub period: String,
    pub count: u32,
}

/// A photo's own title, caption and rights — the IPTC Core fields a
/// person writes, apart from the vision model's caption. `None` when unset.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct IptcFields {
    pub title: Option<String>,
    /// The caption someone wrote, which wins over the model's everywhere.
    pub caption: Option<String>,
    pub creator: Option<String>,
    pub copyright: Option<String>,
    /// The credit line, `photoshop:Credit`.
    pub credit: Option<String>,
    /// How it may be used, `xmpRights:UsageTerms`.
    pub usage_terms: Option<String>,
}

/// One of [`IptcFields`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum IptcField {
    Title,
    Caption,
    Creator,
    Copyright,
    Credit,
    UsageTerms,
}

/// One field across a selection: the value every photo shares (`None`
/// when none has one), or that they differ.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SharedField {
    pub value: Option<String>,
    pub mixed: bool,
}

/// [`IptcFields`] across a selection, for editing them all at once.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct IptcSelection {
    pub title: SharedField,
    pub caption: SharedField,
    pub creator: SharedField,
    pub copyright: SharedField,
    pub credit: SharedField,
    pub usage_terms: SharedField,
}

/// A named set of [`IptcFields`] to put on photos at import or on a selection.
/// A field left blank leaves the photo's own alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct IptcTemplate {
    pub id: i32,
    pub name: String,
    pub fields: IptcFields,
}

/// What a capture-time change did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CaptureTimeOutcome {
    /// Photos whose time changed.
    pub changed: u32,
    /// Photos left alone for having no time to move.
    pub undated: u32,
}

/// Filters combine with AND *across* facets and OR *within* a facet, which is the
/// behaviour people expect from a faceted filter rail.
/// No `#[serde(default)]`: it makes every field optional in the generated
/// TypeScript, which forces `undefined` checks through the whole filter rail
/// for a case that cannot happen — the store's persist `merge` rebuilds the
/// filter over `EMPTY_FILTER` on load, so the frontend always sends a complete
/// filter. `Default` is still derived for Rust-side construction.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PhotoFilter {
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub genres: Vec<String>,
    pub subjects: Vec<String>,
    pub conditions: Vec<String>,
    /// The chosen subjects must **all** be on a photo ("beach and dog"),
    /// rather than any one of them. Each still takes in its branch.
    #[serde(default)]
    pub subjects_all: bool,
    /// Keywords a photo must not have — nor anything filed beneath them —
    /// whatever else it matches: "sunset, not clouds".
    #[serde(default)]
    pub exclude_subjects: Vec<String>,
    /// Genres left out, likewise.
    #[serde(default)]
    pub exclude_genres: Vec<String>,
    /// Conditions left out, likewise.
    #[serde(default)]
    pub exclude_conditions: Vec<String>,
    /// Only keywords a person gave or confirmed — or took from their folder
    /// names — count: with "Show the tagger's keywords" off, its
    /// unconfirmed suggestions neither match a keyword nor count towards one. A view setting rather than a choice —
    /// the interface adds it to every query, and a saved view never keeps it.
    #[serde(default)]
    pub own_keywords: bool,
    pub place_ids: Vec<i32>,
    /// At least this many stars. Kept so smart collections saved with it still
    /// load and match; the filter rail sets [`Self::ratings`] instead.
    pub min_rating: Option<i32>,
    /// Exact star ratings to include (OR within), `0` meaning unrated. Clicking
    /// three stars means "the threes", not "three and up", which is what
    /// `min_rating` gave. Empty means every rating. `#[serde(default)]` for
    /// the smart collections saved before it existed.
    #[serde(default)]
    pub ratings: Vec<i32>,
    /// Pick/reject flags to include (OR within), e.g. `[1]` for picks only or
    /// `[-1]` for rejects. Empty means every flag.
    pub flags: Vec<i32>,
    /// Colour labels to include (OR within), 1..=5. Empty means every label.
    pub color_labels: Vec<i32>,
    /// Restrict to a single collection's members.
    pub collection_id: Option<i32>,
    /// Restrict to photos a given grouped person appears in. Kept so a saved
    /// view from before [`Self::person_ids`] still loads and matches; the rail
    /// sets that instead.
    #[serde(default)]
    pub person_id: Option<i32>,
    /// People who must **all** be in the photo — "Ada and Bo together".
    /// AND, unlike the facets' OR, because "Ada or Bo" is rarely the question.
    #[serde(default)]
    pub person_ids: Vec<i32>,
    /// Camera models to include (OR within), as EXIF names them.
    #[serde(default)]
    pub cameras: Vec<String>,
    /// Lenses to include (OR within), as EXIF names them.
    #[serde(default)]
    pub lenses: Vec<String>,
    /// Focal-length ranges to include (OR within), by key — see
    /// `db::FOCAL_RANGES`. Ranges rather than exact values: a zoom at 70 and
    /// at 72 mm is the same question.
    #[serde(default)]
    pub focal_ranges: Vec<String>,
    /// ISO ranges, by key — see `db::ISO_RANGES`.
    #[serde(default)]
    pub iso_ranges: Vec<String>,
    /// Aperture ranges, by key — see `db::APERTURE_RANGES`.
    #[serde(default)]
    pub aperture_ranges: Vec<String>,
    /// File types to include, by key — see `db::FILE_KINDS`.
    #[serde(default)]
    pub file_kinds: Vec<String>,
    /// Shapes to include, as shown — see `db::SHAPES`.
    #[serde(default)]
    pub shapes: Vec<String>,
    /// How many faces, by key — see `db::FACE_COUNTS`.
    #[serde(default)]
    pub face_counts: Vec<String>,
    /// Named colours that cover a photo, by `palette::SWATCHES` name;
    /// any of them.
    #[serde(default)]
    pub palette: Vec<String>,
    /// Status toggles. Each narrows on its own, and they combine with
    /// AND, like `untagged_only` and `missing_only` beside them.
    #[serde(default)]
    pub edited_only: bool,
    /// Only virtual copies.
    #[serde(default)]
    pub copies_only: bool,
    /// Only photos the face pass found a face in.
    #[serde(default)]
    pub faces_only: bool,
    /// Only photos that know where they were taken (GPS, or a pin by hand).
    #[serde(default)]
    pub located_only: bool,
    /// Only photos with no place named.
    #[serde(default)]
    pub unplaced_only: bool,
    /// How the grid orders what the filter matches. Not a filter, but it
    /// travels with one: persisted with it, and kept by a saved view.
    #[serde(default)]
    pub sort: PhotoSort,
    pub untagged_only: bool,
    /// Library folders to confine to, by id. OR within, AND across, like
    /// every facet. `#[serde(default)]` on this and `missing_only` alone: they
    /// arrived after smart collections began storing filters as JSON, and a
    /// saved filter without them must still load rather than be skipped.
    #[serde(default)]
    pub folder_ids: Vec<i32>,
    /// Only photos whose file the last scan did not find.
    #[serde(default)]
    pub missing_only: bool,
    /// Only the photos a card import brought in.
    #[serde(default)]
    pub shoot_id: Option<i32>,
    /// Literal text match over filename / tag / place names (a plain `LIKE`).
    pub search: Option<String>,
    /// Natural-language content search: CLIP "looks like" fused with caption/OCR
    /// "mentions the words". Unlike `search`, this isn't a SQL clause — the
    /// command layer resolves it to a ranked id set and restricts the query to
    /// it, so results come back in relevance order and still compose with every
    /// other facet here. Saved verbatim in a smart collection, so a natural-
    /// language album re-runs against the live library.
    #[serde(default)]
    pub semantic: Option<String>,
}

/// The grid's order. A natural-language search ignores it: its results
/// come back by relevance.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum PhotoSort {
    /// Capture date, newest first — how the grid has always been.
    #[default]
    Newest,
    Oldest,
    /// When it was added to the library, newest first.
    Added,
    /// Rating, highest first; newest first within a rating.
    Rating,
    /// File name, A to Z.
    Name,
    /// Focus score, sharpest first; unscored photos last.
    Sharpest,
    /// A collection's own order, as arranged by hand. Only while a
    /// collection is shown; anywhere else it is the newest first.
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FacetCount {
    pub value: String,
    pub count: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PlaceFacet {
    pub id: i32,
    pub name: String,
    pub count: i32,
}

/// A clustered identity for the People workspace: how many photos it appears in,
/// the user-given name (None until named), and a cover face to show on the tile.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub id: i32,
    pub name: Option<String>,
    pub count: i32,
    /// Thumbnail path of the cover face's photo.
    pub cover_path: Option<String>,
    /// The cover face's box `[x, y, w, h]` as **fractions of the image**, 0..1,
    /// so the tile can crop to the face rather than showing whichever square of
    /// the photo the face happens to sit in. `None` when the face was analysed
    /// before its image size was recorded — see `PersonFace::box_frac`.
    pub cover_box: Option<Vec<f32>>,
    /// A cached portrait of the cover face, cut from the full preview (see
    /// `faces::portrait`); shown in place of cropping `cover_path` when set.
    pub portrait_path: Option<String>,
    /// Put out of the way by the user: not someone they care about.
    pub hidden: bool,
    /// The face on the tile, so a face review can mark it.
    pub cover_face_id: Option<i32>,
}

/// What turning folder names into keywords did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FolderKeywords {
    /// Photos that gained at least one keyword.
    pub photos: u32,
    /// Different keywords put on them.
    pub keywords: u32,
}

/// The tagger's unconfirmed keywords on one photo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AiKeywords {
    pub photo_id: i32,
    pub names: Vec<String>,
}

/// One keyword of the keyword tree. Counts come with the facets, which
/// follow the filter; this is only the tree's shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct KeywordNode {
    pub name: String,
    /// The keyword it is filed under; `None` at the top.
    pub parent: Option<String>,
    /// Made or filed by the user, so shown even while no photo has it.
    pub kept: bool,
}

/// A named, user-curated group of photos, with its current member count.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Collection {
    pub id: i32,
    pub name: String,
    pub count: i32,
    /// The group it is filed in, or none: at the top level.
    pub group_id: Option<i32>,
}

/// A group of collections and saved views, one level deep.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CollectionGroup {
    pub id: i32,
    pub name: String,
}

/// A named, saved filter. Selecting it replaces the live filter, so it stays
/// live — it re-runs against the current library rather than freezing a set of
/// photos the way a [`Collection`] does.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SmartCollection {
    pub id: i32,
    pub name: String,
    pub filter: PhotoFilter,
    /// The group it is filed in, or none.
    #[serde(default)]
    pub group_id: Option<i32>,
}

/// One photo's rating, flag and colour label — the verdicts a keypress sets,
/// kept so a mistaken one can be undone. Named as on [`Photo`], so the UI
/// can patch a photo with it directly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Verdict {
    pub id: i32,
    pub rating: i32,
    pub flag: i32,
    pub color_label: i32,
}

/// A card import, and how far along it is: imported, judged, picked,
/// the picks edited, the picks exported.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Shoot {
    pub id: i32,
    pub name: String,
    pub created_at: String,
    pub photos: u32,
    /// Rated or flagged: a verdict given.
    pub judged: u32,
    pub picks: u32,
    pub edited_picks: u32,
    pub exported_picks: u32,
    /// Photos its import could not copy into the library.
    pub not_copied: u32,
    /// Photos in the library its import could not copy to the backup.
    pub not_backed_up: u32,
}

/// The names the search box recognises in a query.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchVocabulary {
    /// Named people, most photographed first — a name two share goes to the
    /// first.
    pub people: Vec<NamedPerson>,
    pub places: Vec<Place>,
    /// The genres and conditions photos are tagged with.
    pub genres: Vec<String>,
    pub conditions: Vec<String>,
    /// Cameras and lenses as EXIF names them.
    pub cameras: Vec<String>,
    pub lenses: Vec<String>,
    /// The keywords photos have. Read only after "not" — "sunset, not
    /// clouds" — since a keyword said plainly is better left to content search.
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct NamedPerson {
    pub id: i32,
    pub name: String,
}

/// A capture time a sidecar gives that the catalogue doesn't have:
/// offered to the user, never applied by itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SidecarTime {
    pub photo_id: i32,
    /// As the catalogue keeps it, `2025-06-14 10:30:00`.
    pub time: String,
}

/// What reading sidecars found.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct XmpImport {
    /// Photos with a sidecar that was read.
    pub read: u32,
    /// Names put on people, faces or photos.
    pub people: u32,
    /// Names left off a face because People already calls it someone else.
    pub kept: u32,
    /// Capture times from elsewhere (another editor's capture-time fix) that
    /// differ from the catalogue's, for the user to accept or not.
    pub times: Vec<SidecarTime>,
    /// Photos whose rating, flag or colour label the sidecars changed — one
    /// step of Undo puts them all back.
    pub verdicts_changed: u32,
}

/// One photo's new pick/reject flag, for setting many different flags at once.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FlagChange {
    pub id: i32,
    /// -1 reject, 0 none, 1 pick.
    pub flag: i32,
}

/// What an undo put back: which change it was, and each photo's restored
/// verdicts, for the UI to show again.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UndoneVerdicts {
    /// E.g. "Rating on 12 photos".
    pub label: String,
    pub restored: Vec<Verdict>,
    /// Photos whose develop settings were put back.
    pub settings: Vec<i32>,
    /// The step was one Undo can't take back — keywords, a place — so
    /// nothing was undone, and the next Undo goes on from before it.
    pub beyond: bool,
}

/// Counts shown next to each filter option. Each facet is counted with every
/// *other* filter applied but not its own, so options never collapse to zero
/// the moment you pick one.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Facets {
    pub genres: Vec<FacetCount>,
    pub subjects: Vec<FacetCount>,
    pub conditions: Vec<FacetCount>,
    pub places: Vec<PlaceFacet>,
    /// Photos per flag value (`"1"` pick, `"0"` unflagged, `"-1"` reject),
    /// counted like every other facet: against all the *other* filters.
    pub flags: Vec<FacetCount>,
    /// Photos per colour label value (`"0"` none, `"1"`–`"5"`), likewise.
    pub color_labels: Vec<FacetCount>,
    /// Photos per star rating (`"0"` unrated, `"1"`–`"5"`), likewise.
    pub ratings: Vec<FacetCount>,
    /// Photos per library folder id, against the other filters.
    pub folders: Vec<FacetCount>,
    /// Photos whose file is missing, against the other filters.
    pub missing: i32,
    pub total: i32,
    pub untagged: i32,
    pub date_min: Option<String>,
    pub date_max: Option<String>,
}

/// Which of the rail's optional counts to compute. Each costs a pass
/// over the photos that the base counts don't, so they are only asked for while
/// the rail section that shows them is open.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FacetGroups {
    /// Camera, lens, focal length, aperture and ISO.
    pub gear: bool,
    /// Named people.
    pub people: bool,
    /// The status toggles: edited, copies, faces, located, unplaced.
    pub status: bool,
    /// File type, shape and number of faces.
    #[serde(default)]
    pub kind: bool,
    /// The named colours that cover photos.
    #[serde(default)]
    pub palette: bool,
}

/// The rail's optional counts, each against the other filters like
/// every facet. Groups not asked for come back empty.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ExtraFacets {
    /// Photos per camera and per lens, most used first.
    pub cameras: Vec<FacetCount>,
    pub lenses: Vec<FacetCount>,
    /// Photos per focal-length, ISO and aperture range, by range key, in the
    /// ranges' own order.
    pub focal_ranges: Vec<FacetCount>,
    pub iso_ranges: Vec<FacetCount>,
    pub aperture_ranges: Vec<FacetCount>,
    /// Photos per file type, shape and number of faces, by key, in the
    /// keys' own order.
    pub file_kinds: Vec<FacetCount>,
    pub shapes: Vec<FacetCount>,
    pub face_counts: Vec<FacetCount>,
    /// Photos each named colour covers, in `palette::SWATCHES` order.
    pub palette: Vec<FacetCount>,
    /// Named people, and how many photos each would leave. Counted under the
    /// whole filter, the chosen people included: people combine with AND, so a
    /// count answers "with everyone chosen so far, and them".
    pub people: Vec<PersonFacet>,
    /// How many photos each status toggle would leave.
    pub edited: i32,
    pub copies: i32,
    pub with_faces: i32,
    pub located: i32,
    pub unplaced: i32,
}

/// A named person as a filter option.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PersonFacet {
    pub id: i32,
    pub name: String,
    pub count: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    pub id: i32,
    pub name: String,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    /// Photos in this place, across the whole library — not the active filter.
    /// See [`crate::db::list_places`] for why that distinction matters.
    pub count: i32,
    /// The country it is in, for grouping the list: from a GPS-style name
    /// ("Edinburgh, United Kingdom"), or else from where its pin is. `None`
    /// for a hand-named place with no pin.
    pub country: Option<String>,
}

/// One place a trip's photos are in, and how many of them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TripPlace {
    pub name: String,
    pub count: i32,
}

/// What naming a trip did: photos placed, and photos left in the place their
/// own GPS gave them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TripNamed {
    pub placed: i32,
    pub kept: i32,
    /// Photos left in a place someone set by hand.
    pub kept_placed: i32,
}

/// A run of photos separated from its neighbours by a large time gap — the
/// proposal the user confirms and names in the trips flow.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TripCluster {
    pub photo_ids: Vec<i32>,
    pub start_date: String,
    pub end_date: String,
    pub count: i32,
    /// The name the trip was given: the commonest place among its photos with
    /// no position of their own. `None` until it is named. A phone photo's
    /// GPS town is not the trip's name — see [`Self::places`] for those.
    pub existing_place: Option<String>,
    /// Every place its photos are in, however they got there, most photos
    /// first. A trip that ran from Edinburgh to Skye is in both, and showing
    /// only the first placed photo's town said otherwise.
    pub places: Vec<TripPlace>,
    /// Photos that carry their own position. Naming the trip leaves these
    /// where their GPS put them.
    pub located: i32,
    pub thumb_path: Option<String>,
    /// Up to [`crate::trips::CONTACT_SHEET`] thumbnails spread across the trip,
    /// so a card can show what the trip *was* rather than one 56px square.
    pub thumb_paths: Vec<String>,
    /// Distinct camera bodies in the trip, most-used first. A trip shot on two
    /// bodies is usually two people, which is worth seeing before naming it.
    pub cameras: Vec<String>,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub phase: String,
    pub current: u32,
    pub total: u32,
    pub message: String,
}

/// Face work still to do, for the People page and the cull's Analyse button.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FacesStatus {
    /// Photos the face pass has never run on.
    pub to_detect: i32,
    /// Photos whose faces were found but have no identity yet: analysed
    /// before the People recogniser arrived, or an embed that failed.
    /// Counted whether or not the recogniser is installed, so People can offer
    /// to set it up rather than showing nothing to do.
    pub to_recognise: i32,
    pub recogniser_installed: bool,
}

/// What one run of the face pass did. The page reads this rather than
/// inferring success from an empty People grid, which is how a pass whose
/// every embed failed used to look exactly like a library with nobody in it.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FacesSummary {
    pub photos: u32,
    pub faces: u32,
    /// Faces given an identity embedding this run.
    pub embedded: u32,
    /// Faces the recogniser was present for but could not embed.
    pub embed_failed: u32,
    /// The first embed error, word for word, for the page to show.
    pub first_error: Option<String>,
    pub stopped: bool,
}

/// One folder of the library and what is in it.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LibraryFolder {
    pub id: i32,
    pub path: String,
    /// Photos (files, not virtual copies) the catalogue has from this folder.
    pub photos: i32,
    /// Of those, how many the last scan did not find.
    pub missing: i32,
    /// Whether the folder itself is there — false for an unplugged drive or a
    /// folder moved outside the app, which is when "Locate…" is offered.
    pub online: bool,
}

/// The library's folders, plus photos that belong to none of them — left
/// behind by the single-folder library, whose "switch" kept the old folder's
/// photos but stopped scanning them.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LibraryFolders {
    pub folders: Vec<LibraryFolder>,
    pub unfiled: i32,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub added: u32,
    pub skipped: u32,
    pub failed: u32,
    /// Files reunited with their old row after a folder was renamed or moved
    /// within the library.
    pub relinked: u32,
    /// Photos in the library folder whose file the scan did not find.
    pub missing: u32,
    /// Why the first unreadable photo failed, with its name.
    pub first_failure: Option<String>,
}

/// Structured output we constrain the vision model to, with a grammar built
/// from `tagger::schema`.
///
/// Field order matters: grammar-constrained generation emits properties in schema
/// order, so `main_subject` is answered *before* `genre`. Measured on real photos,
/// asking for the genre first made the model label almost everything "landscape"
/// (an ice-cream tub in a garden, a person filling the frame); committing to the
/// subject first fixed those.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct TagResult {
    #[serde(default)]
    pub main_subject: String,
    #[serde(default)]
    pub genre: String,
    #[serde(default)]
    pub subjects: Vec<String>,
    #[serde(default)]
    pub conditions: Vec<String>,
    #[serde(default)]
    pub confidence: f64,
}

/// The closed set of genres. Keeping this an enum in the JSON schema is what makes
/// the genre facet dependable — an open-vocabulary field would drift into dozens
/// of near-duplicate labels and be useless as a filter.
pub const GENRES: &[&str] = &[
    "landscape",
    "wildlife",
    "portrait",
    "street",
    "macro",
    "astro",
    "architecture",
    "sports",
    "event",
    "still-life",
    "abstract",
];
