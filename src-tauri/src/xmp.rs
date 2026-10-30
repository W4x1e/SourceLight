//! XMP sidecar metadata: import from and export to Adobe-style `.xmp` files.
//!
//! The point of this module is **migration** — letting someone bring the work
//! they did in another editor (or take ours elsewhere) rather than
//! starting from scratch. That is the DAM metadata, and it is what portably
//! crosses between tools. What crosses, and where it goes:
//!
//! - rating and colour label: `xmp:Rating`, `xmp:Label`;
//! - keywords: `dc:subject`, flat — the subjects, and the genre and conditions
//!   too, since the flat list is all most tools read;
//! - genre and conditions again as `lr:hierarchicalSubject` paths under
//!   [`GENRE_ROOT`] and [`CONDITIONS_ROOT`] (`Genre|landscape`), so other editors
//!   file them under a parent and they come back in as what they were;
//! - the caption: `dc:description`;
//! - named people: `Iptc4xmpExt:PersonInImage`, and an MWG face region
//!   (`mwg-rs:Regions`) for each named face with a box — the form other
//!   editors and exiftool keep face names in;
//! - pick or reject: `sourcelight:Flag` ([`APP_NS`]), `pick` or `reject`.
//!   There is no standard field — other editors keep flags in their catalogues — so
//!   only this app reads it back. digiKam's `digiKam:PickLabel` is read too;
//! - where it was taken: written only, see below;
//! - a title, and the rights: `dc:title`, `dc:creator`, `dc:rights`,
//!   `photoshop:Credit` and `xmpRights:UsageTerms`.
//! - a capture time the user corrected: `exif:DateTimeOriginal` and
//!   `photoshop:DateCreated`, see below.
//!
//! **Develop settings are deliberately out of scope.** Adobe's `crs:` namespace
//! describes *Adobe's* pipeline — its tone curve, its masks, its colour model —
//! and none of it maps onto ours. Importing `crs:Exposure2012` as if it were our
//! exposure would produce a different picture and quietly lie about what the
//! edit was. The fields above are engine-independent facts about a photo;
//! develop settings are not.
//!
//! **Location is written, not read.** A photo's GPS and its town go into the
//! sidecar so other editors see where it was taken (see
//! [`crate::export::location`]); reading them back is the scanner's job, from
//! the file itself. Only the location fields a photo *has* are written, and
//! only those are replaced — a position someone set in another editor for a photo
//! we know nothing about is left exactly as it was.
//!
//! The parser tolerates the two forms Adobe actually emits — properties as
//! **attributes** on `rdf:Description` and as **child elements** — because
//! editors, file browsers, exiftool and camera bodies disagree about which to use,
//! and a migration tool that only handled one would silently drop half of it.
//! Fields are matched by namespace as well as name, so Microsoft's
//! `MPRI:Regions` is never mistaken for MWG's; a prefix the file never declares
//! falls back to the name alone, as it always has.
//!
//! **A corrected capture time is written only where it corrects something.**
//! An uncorrected photo's sidecar gets no date — the file already has it — and a
//! corrected one's replaces only a date this app wrote, or the camera's own time
//! (other editors copy that into every sidecar). A date someone set elsewhere is
//! theirs and is kept. Reverting takes ours back out, while it is still ours.
//! On import a sidecar's date is read but never applied by itself: the caller
//! offers it (see [`Sidecar::captured_is_ours`]).
//!
//! **Writing into someone else's sidecar never deletes their values**.
//! A rating, label or keyword this app did not put there is kept: a value the
//! catalogue *has* replaces it, but the catalogue having *nothing* does not
//! clear it. Export used to make the sidecar mirror the catalogue, so writing
//! sidecars before ever importing them wiped another editor's ratings, labels
//! and keywords. Clearing still reaches the sidecar for values this app wrote —
//! each packet records what it last wrote, under [`APP_NS`], and a value that
//! still matches that record is ours to take back out. One that no longer
//! matches was changed elsewhere since, and is left alone. The other lists —
//! hierarchical keywords, people — work the same way as keywords.
//!
//! **A title, the rights, and a caption the user wrote are theirs**, so
//! like a rating the catalogue's replaces the sidecar's, and one this app wrote
//! is taken back out only while it still says what was written.
//!
//! **A caption or face regions someone else wrote win outright.** Ours is a
//! vision model's description; theirs is a person's words. So ours is written
//! only where the sidecar has none, or still has the one this app last wrote.
//! Regions likewise: a set from another editor — which may hold unnamed
//! faces, pets or focus points this app knows nothing of — is left whole, and
//! our names still reach it through `PersonInImage`.

use anyhow::Result;
use quick_xml::escape::unescape;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::name::{LocalName, NamespaceResolver, ResolveResult};
use quick_xml::{NsReader, Writer};

const RDF_NS: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const XMP_NS: &str = "http://ns.adobe.com/xap/1.0/";
const DC_NS: &str = "http://purl.org/dc/elements/1.1/";
const LR_NS: &str = "http://ns.adobe.com/lightroom/1.0/";
const IPTC_EXT_NS: &str = "http://iptc.org/std/Iptc4xmpExt/2008-02-29/";
const MWG_RS_NS: &str = "http://www.metadataworkinggroup.com/schemas/regions/";
const ST_DIM_NS: &str = "http://ns.adobe.com/xap/1.0/sType/Dimensions#";
const ST_AREA_NS: &str = "http://ns.adobe.com/xmp/sType/Area#";
const DIGIKAM_NS: &str = "http://www.digikam.org/ns/1.0/";
const EXIF_NS: &str = "http://ns.adobe.com/exif/1.0/";
const PHOTOSHOP_NS: &str = "http://ns.adobe.com/photoshop/1.0/";
const XMP_RIGHTS_NS: &str = "http://ns.adobe.com/xap/1.0/rights/";

/// This app's own XMP namespace: the pick/reject flag, and where a packet
/// records what it last wrote so a later export can tell its values from
/// everyone else's. An identifier, never fetched.
const APP_NS: &str = "http://ns.sourcelight.app/xmp/1.0/";
const APP_PREFIX: &str = "sourcelight";
/// The namespace builds before the public release wrote. Read as [`APP_NS`],
/// so their flags and records still count, and replaced by it on the next write.
const LEGACY_APP_NS: &str = "http://ns.jamestully.com/photoorganiser/1.0/";
/// The local names written in [`APP_NS`]. The record's are deliberately not
/// `Rating`/`Label`/`subject`: a lenient match goes by local name, and these
/// must never be mistaken for the fields they describe.
const FLAG: &str = "Flag";
const SYNCED_RATING: &str = "SyncedRating";
const SYNCED_LABEL: &str = "SyncedLabel";
const SYNCED_DESCRIPTION: &str = "SyncedDescription";
const SYNCED_REGIONS: &str = "SyncedRegions";
const SYNCED_KEYWORDS: &str = "SyncedKeywords";
const SYNCED_HIERARCHY: &str = "SyncedHierarchy";
const SYNCED_PEOPLE: &str = "SyncedPeople";
const SYNCED_TITLE: &str = "SyncedTitle";
const SYNCED_CREATOR: &str = "SyncedCreator";
const SYNCED_RIGHTS: &str = "SyncedRights";
const SYNCED_CREDIT: &str = "SyncedCredit";
const SYNCED_USAGE: &str = "SyncedUsageTerms";
const SYNCED_CAPTURED: &str = "SyncedCaptured";
/// Every field this app writes in its namespace, each rewritten on every
/// export. A field some later version adds is not among them, so an older
/// build leaves it alone.
const APP_FIELDS: [&str; 14] = [
    FLAG,
    SYNCED_RATING,
    SYNCED_LABEL,
    SYNCED_DESCRIPTION,
    SYNCED_REGIONS,
    SYNCED_KEYWORDS,
    SYNCED_HIERARCHY,
    SYNCED_PEOPLE,
    SYNCED_TITLE,
    SYNCED_CREATOR,
    SYNCED_RIGHTS,
    SYNCED_CREDIT,
    SYNCED_USAGE,
    SYNCED_CAPTURED,
];

/// How a capture time is written: ISO 8601 with no zone, as the camera recorded
/// it — the catalogue keeps local times and knows no zone to add.
const DATE_FORMAT: &str = "%Y-%m-%dT%H:%M:%S";

/// A date field's value as a time, or `None` for one without a whole time
/// (`2025-06-14` alone). A zone or fraction after the seconds is ignored: the
/// catalogue's times are the camera's local ones.
fn parse_date(value: &str) -> Option<chrono::NaiveDateTime> {
    let v = value.trim();
    let head = v.get(..19)?;
    chrono::NaiveDateTime::parse_from_str(head, DATE_FORMAT)
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(head, "%Y:%m:%d %H:%M:%S"))
        .ok()
}

fn app(local: &str) -> String {
    format!("{APP_PREFIX}:{local}")
}

/// The parents genre and conditions are filed under in `lr:hierarchicalSubject`.
pub const GENRE_ROOT: &str = "Genre";
pub const CONDITIONS_ROOT: &str = "Conditions";
/// The parent digiKam files people under; a name there is read as a person.
pub const PEOPLE_ROOT: &str = "People";

/// The portable slice of a photo's metadata.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct XmpMeta {
    /// 0..=5, or `None` when the sidecar carries no rating.
    pub rating: Option<i32>,
    /// Our colour-label integer (0 none, 1..=5), resolved from Adobe's label
    /// *string*. `None` when absent or unrecognised.
    pub color_label: Option<i32>,
    /// `dc:subject` keywords, lower-cased and de-duplicated on the way in.
    pub keywords: Vec<String>,
    /// Which of `keywords` only the tagger gave, none of them the user's:
    /// left out of a file unless asked for. Never read from a
    /// sidecar; [`parse`] leaves it empty.
    pub ai_keywords: Vec<String>,
    /// `lr:hierarchicalSubject` paths, `Parent|Child`, as spelled.
    pub hierarchy: Vec<String>,
    /// 1 pick, -1 reject; `None` for neither.
    pub flag: Option<i32>,
    /// `dc:description`, its default-language text.
    pub description: Option<String>,
    /// `Iptc4xmpExt:PersonInImage`: who is in the photo, as spelled.
    pub people: Vec<String>,
    /// Named face regions.
    pub regions: Vec<Region>,
    /// The size in pixels, of the image as stored, the regions were set on
    /// (`mwg-rs:AppliedToDimensions`). Written only alongside regions.
    pub region_dims: Option<(u32, u32)>,
    /// Where it was taken. Written only; [`parse`] leaves it `None`.
    pub location: Option<crate::export::location::PhotoLocation>,
    /// Whether `description` is one the user wrote, which replaces the
    /// sidecar's like a rating does, rather than the model's, which yields.
    pub description_is_own: bool,
    /// The title and the rights.
    pub text: XmpText,
    /// When it was taken. Written only when the user corrected it;
    /// read from `exif:DateTimeOriginal`, else `photoshop:DateCreated`.
    pub captured: Option<chrono::NaiveDateTime>,
    /// The file's own time, beside a corrected `captured`: a sidecar date that
    /// still says this is the camera's, not someone's correction, so it may be
    /// replaced. Written only; [`parse`] leaves it `None`.
    pub captured_original: Option<chrono::NaiveDateTime>,
}

/// What a written file says beyond the camera's own record, each a
/// separate choice, remembered: a client delivery, a stock upload and the
/// photographer's own archive want different ones. Exports and sidecars
/// apply the same choices. "Strip" in the export dialog is a separate switch
/// that leaves out everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct MetadataChoices {
    /// Where it was taken: GPS, town and country.
    pub location: bool,
    /// Who is in it, by name, with their face regions. Off unless asked:
    /// the names come from automatic grouping, which can give a stranger
    /// someone's name.
    pub names: bool,
    /// The vision model's description, where the user wrote none. Off unless
    /// asked: in a caption field it reads as the photographer's own words.
    pub ai_caption: bool,
    /// Keywords only the tagger gave.
    pub ai_keywords: bool,
}

impl Default for MetadataChoices {
    /// Right for a client delivery.
    fn default() -> Self {
        Self {
            location: true,
            names: false,
            ai_caption: false,
            ai_keywords: false,
        }
    }
}

impl MetadataChoices {
    /// Everything: what a sidecar wrote before the choices existed.
    pub const ALL: Self = Self {
        location: true,
        names: true,
        ai_caption: true,
        ai_keywords: true,
    };

    /// Takes out of `meta` what these choices leave out.
    pub fn apply(&self, meta: &mut XmpMeta) {
        if !self.names {
            meta.people.clear();
            meta.regions.clear();
            meta.region_dims = None;
        }
        if !self.location {
            meta.location = None;
        }
        // The model's description names the setting and copies out signs,
        // so it goes with the location too.
        if !meta.description_is_own && !(self.ai_caption && self.location) {
            meta.description = None;
        }
        if !self.ai_keywords {
            let ai = std::mem::take(&mut meta.ai_keywords);
            meta.keywords.retain(|k| !ai.contains(k));
            meta.hierarchy
                .retain(|h| !ai.iter().any(|k| h == k || h.ends_with(&format!("|{k}"))));
        }
    }
}

/// What `photoshop:CaptionWriter` says of a caption the on-device model wrote,
/// so a tool reading the file can tell it from the photographer's.
pub const AI_CAPTION_WRITER: &str = "AI (on-device model)";

/// A photo's title and rights as XMP carries them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct XmpText {
    /// `dc:title`.
    pub title: Option<String>,
    /// `dc:creator`; several are joined with "; ".
    pub creator: Option<String>,
    /// `dc:rights`, the copyright line.
    pub rights: Option<String>,
    /// `photoshop:Credit`.
    pub credit: Option<String>,
    /// `xmpRights:UsageTerms`.
    pub usage_terms: Option<String>,
}

impl XmpText {
    /// Each field with its record's name, for the rules they share.
    fn fields(&self) -> [(&'static str, Option<&String>); 5] {
        [
            (SYNCED_TITLE, self.title.as_ref()),
            (SYNCED_CREATOR, self.creator.as_ref()),
            (SYNCED_RIGHTS, self.rights.as_ref()),
            (SYNCED_CREDIT, self.credit.as_ref()),
            (SYNCED_USAGE, self.usage_terms.as_ref()),
        ]
    }

    fn slot(&mut self, record: &str) -> &mut Option<String> {
        match record {
            SYNCED_TITLE => &mut self.title,
            SYNCED_CREATOR => &mut self.creator,
            SYNCED_RIGHTS => &mut self.rights,
            SYNCED_CREDIT => &mut self.credit,
            _ => &mut self.usage_terms,
        }
    }
}

impl XmpMeta {
    /// The location properties to write, (qualified name, unescaped value).
    fn location_fields(&self) -> Vec<(&'static str, String)> {
        self.location
            .as_ref()
            .map(crate::export::location::PhotoLocation::xmp_fields)
            .unwrap_or_default()
    }

    /// The keywords sorted into what the catalogue keeps them as, by their
    /// place in the hierarchy. A flat keyword that is also a hierarchical
    /// one's leaf — other editors write both — goes where the hierarchy
    /// says, and a person's name is not a subject.
    ///
    /// A flat keyword that is only some path's *ancestor* is not a subject
    /// either. Other editors list a keyword's parents in `dc:subject` too
    /// on export, so `Places|Europe|France` arrives with `europe`
    /// and `places` beside `france`: tagging the photo with all three would
    /// file it under every continent it is in. A parent the photo really has
    /// comes with a path of its own, and is kept by that.
    pub fn sorted_keywords(&self) -> Keywords {
        let mut out = Keywords::default();
        // Lower-cased leaves already placed, so the flat list skips them.
        let mut placed: Vec<String> = self
            .people
            .iter()
            .chain(self.regions.iter().map(|r| &r.name))
            .map(|n| n.to_lowercase())
            .collect();
        // Lower-cased parents of every path: the flat list skips them too,
        // but a path naming one outright ("Animals") still counts.
        let mut implied: Vec<String> = Vec::new();
        for path in &self.hierarchy {
            let parts: Vec<&str> = path
                .split('|')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect();
            let (Some(root), Some(&leaf)) = (parts.first(), parts.last()) else {
                continue;
            };
            let lower = leaf.to_lowercase();
            let under = |name: &str| parts.len() >= 2 && root.eq_ignore_ascii_case(name);
            if under(GENRE_ROOT) {
                out.genre.get_or_insert(lower.clone());
            } else if under(CONDITIONS_ROOT) {
                push_unique(&mut out.conditions, lower.clone());
            } else if under(PEOPLE_ROOT) {
                if !out.people.iter().any(|p| p.eq_ignore_ascii_case(leaf)) {
                    out.people.push(leaf.to_string());
                }
            } else if !placed.contains(&lower) {
                // Anything else is a subject by its leaf: "Places|Europe|France"
                // is about France. The flat list almost always has it too.
                push_unique(&mut out.subjects, lower.clone());
                if parts.len() >= 2 {
                    let path: Vec<String> = parts.iter().map(|s| s.to_lowercase()).collect();
                    if !out.subject_paths.contains(&path) {
                        out.subject_paths.push(path);
                    }
                }
            }
            implied.extend(parts[..parts.len() - 1].iter().map(|s| s.to_lowercase()));
            placed.push(lower);
        }
        for kw in &self.keywords {
            if !placed.contains(kw) && !implied.contains(kw) {
                push_unique(&mut out.subjects, kw.clone());
            }
        }
        out
    }
}

/// A sidecar's keywords as the catalogue keeps them — see
/// [`XmpMeta::sorted_keywords`]. All lower-cased but people's names.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Keywords {
    pub genre: Option<String>,
    pub conditions: Vec<String>,
    pub subjects: Vec<String>,
    /// Where the subjects that came with a hierarchy sit in it, root first:
    /// `["places", "europe", "france"]`. The keyword tree is built from
    /// these; a subject from the flat list alone has none.
    pub subject_paths: Vec<Vec<String>>,
    /// Names filed under [`PEOPLE_ROOT`].
    pub people: Vec<String>,
}

fn push_unique(list: &mut Vec<String>, item: String) {
    if !list.contains(&item) {
        list.push(item);
    }
}

/// A named face region, as MWG defines one: centre and size as fractions of
/// the image **as stored** — before its EXIF orientation is applied. The
/// catalogue measures faces on the image as displayed, so crossing between
/// the two goes through [`Region::from_displayed`] and
/// [`Region::displayed_box`].
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    pub name: String,
    /// Centre x, centre y, width, height.
    pub area: [f64; 4],
}

impl Region {
    /// Values are kept to four places — what is written — so a region read
    /// back compares equal to the one written.
    pub fn new(name: &str, area: [f64; 4]) -> Self {
        Region {
            name: name.trim().to_string(),
            area: area.map(|v| (v * 10_000.0).round() / 10_000.0),
        }
    }

    /// From a box `[x, y, w, h]` — its top-left corner and size, as fractions
    /// — on the image as displayed, which is how the catalogue keeps faces.
    pub fn from_displayed(name: &str, b: [f32; 4], orientation: i32) -> Self {
        let [x, y, w, h] = b.map(f64::from);
        let (u, v) = displayed_to_stored(orientation, x + w / 2.0, y + h / 2.0);
        let (w, h) = if swaps(orientation) { (h, w) } else { (w, h) };
        Region::new(name, [u, v, w, h])
    }

    /// This region as a box `[x, y, w, h]` on the image as displayed.
    pub fn displayed_box(&self, orientation: i32) -> [f32; 4] {
        let [u, v, w, h] = self.area;
        let (cx, cy) = stored_to_displayed(orientation, u, v);
        let (w, h) = if swaps(orientation) { (h, w) } else { (w, h) };
        [cx - w / 2.0, cy - h / 2.0, w, h].map(|v| v as f32)
    }
}

/// Whether an EXIF orientation turns the image a quarter, swapping its edges.
fn swaps(orientation: i32) -> bool {
    (5..=8).contains(&orientation)
}

/// Where a point of the stored image lands once `orientation` is applied — the
/// same transforms `raw::apply_orientation` makes the previews with.
fn stored_to_displayed(orientation: i32, u: f64, v: f64) -> (f64, f64) {
    match orientation {
        2 => (1.0 - u, v),
        3 => (1.0 - u, 1.0 - v),
        4 => (u, 1.0 - v),
        5 => (v, u),
        6 => (1.0 - v, u),
        7 => (1.0 - v, 1.0 - u),
        8 => (v, 1.0 - u),
        _ => (u, v),
    }
}

/// The inverse of [`stored_to_displayed`].
fn displayed_to_stored(orientation: i32, p: f64, q: f64) -> (f64, f64) {
    match orientation {
        2 => (1.0 - p, q),
        3 => (1.0 - p, 1.0 - q),
        4 => (p, 1.0 - q),
        5 => (q, p),
        6 => (q, 1.0 - p),
        7 => (1.0 - q, 1.0 - p),
        8 => (1.0 - q, p),
        _ => (p, q),
    }
}

/// The image's size as stored, for `AppliedToDimensions`: its two edges in
/// either order, and whether it is displayed landscape.
pub fn stored_dims(edges: (u32, u32), displayed_landscape: bool, orientation: i32) -> (u32, u32) {
    let (long, short) = (edges.0.max(edges.1), edges.0.min(edges.1));
    if displayed_landscape != swaps(orientation) {
        (long, short)
    } else {
        (short, long)
    }
}

/// A sidecar as import reads it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sidecar {
    pub meta: XmpMeta,
    /// Whether `meta.description` is the caption this app last wrote there,
    /// rather than someone's own words.
    pub description_is_ours: bool,
    /// The title and rights someone else wrote there — those this app did
    /// not, or that have changed since it did.
    pub text_from_elsewhere: XmpText,
    /// Whether `meta.captured` is the time this app last wrote there. One that
    /// isn't came from elsewhere — another editor's capture-time fix, say — and
    /// is for the user to accept, never applied by itself.
    pub captured_is_ours: bool,
    /// The keywords this app last wrote there, lower-cased: read back,
    /// they are already the catalogue's, with whatever provenance it gave
    /// them, and not the user's own words to adopt.
    pub keywords_ours: Vec<String>,
}

/// Reads a sidecar for import — see [`parse`].
pub fn read(xml: &str) -> Result<Sidecar> {
    let found = scan(xml)?;
    let description_is_ours = match (&found.meta.description, &found.synced.description) {
        (Some(d), Some(wrote)) => fingerprint(std::slice::from_ref(d)) == *wrote,
        _ => false,
    };
    let mut text_from_elsewhere = XmpText::default();
    for (record, value) in found.meta.text.fields() {
        let ours = value.is_some_and(|v| {
            found.synced.text.get(record) == Some(&fingerprint(std::slice::from_ref(v)))
        });
        if !ours {
            *text_from_elsewhere.slot(record) = value.cloned();
        }
    }
    let captured_is_ours =
        found.meta.captured.is_some() && found.meta.captured == found.synced.captured;
    Ok(Sidecar {
        keywords_ours: found.synced.keywords.clone(),
        meta: found.meta,
        description_is_ours,
        text_from_elsewhere,
        captured_is_ours,
    })
}

/// What this app last wrote into a sidecar, read back from [`APP_NS`]. Empty for
/// a sidecar it never wrote — or wrote before the record existed, which is
/// treated the same way: as someone else's.
#[derive(Debug, Clone, Default, PartialEq)]
struct Synced {
    rating: Option<i32>,
    label: Option<i32>,
    /// The lists, lower-cased — each is matched without regard to case.
    keywords: Vec<String>,
    hierarchy: Vec<String>,
    people: Vec<String>,
    /// Fingerprints of the caption and of the regions ([`fingerprint`]): only
    /// whether the sidecar still has ours matters, not what it said.
    description: Option<String>,
    regions: Option<String>,
    /// Fingerprints of the title and rights fields written, by record.
    text: std::collections::BTreeMap<&'static str, String>,
    /// The capture time written, when it was a correction.
    captured: Option<chrono::NaiveDateTime>,
}

impl Synced {
    fn is_empty(&self) -> bool {
        *self == Synced::default()
    }

    /// The record's single values as attributes, each prefixed with `sep`.
    fn attributes(&self, sep: &str) -> String {
        let mut out = String::new();
        if let Some(r) = self.rating {
            out.push_str(&format!("{sep}{}=\"{r}\"", app(SYNCED_RATING)));
        }
        if let Some(l) = self.label.and_then(label_to_string) {
            out.push_str(&format!("{sep}{}=\"{l}\"", app(SYNCED_LABEL)));
        }
        if let Some(d) = &self.description {
            out.push_str(&format!("{sep}{}=\"{d}\"", app(SYNCED_DESCRIPTION)));
        }
        if let Some(r) = &self.regions {
            out.push_str(&format!("{sep}{}=\"{r}\"", app(SYNCED_REGIONS)));
        }
        for (record, f) in &self.text {
            out.push_str(&format!("{sep}{}=\"{f}\"", app(record)));
        }
        if let Some(t) = &self.captured {
            out.push_str(&format!(
                "{sep}{}=\"{}\"",
                app(SYNCED_CAPTURED),
                t.format(DATE_FORMAT)
            ));
        }
        out
    }

    /// The record's lists, as the element each is written in.
    fn lists(&self) -> [(&'static str, &Vec<String>); 3] {
        [
            (SYNCED_KEYWORDS, &self.keywords),
            (SYNCED_HIERARCHY, &self.hierarchy),
            (SYNCED_PEOPLE, &self.people),
        ]
    }
}

/// A short, stable fingerprint of some strings, in any order: FNV-1a over them
/// sorted. Stable across builds and platforms, which a record in a file must be.
fn fingerprint(items: &[String]) -> String {
    let mut sorted: Vec<&str> = items.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in sorted.join("\n").bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// One region, whatever it is, as it goes into a fingerprint.
fn region_key(name: &str, kind: &str, area: [Option<f64>; 4]) -> String {
    let area = area.map(|v| v.map_or_else(|| "-".to_string(), |v| format!("{v:.4}")));
    format!("{name}\u{1f}{kind}\u{1f}{}", area.join("\u{1f}"))
}

/// The fingerprint of the regions this app writes, or `None` for none.
fn our_regions(regions: &[Region]) -> Option<String> {
    let keys: Vec<String> = regions
        .iter()
        .map(|r| region_key(&r.name, "Face", r.area.map(Some)))
        .collect();
    (!keys.is_empty()).then(|| fingerprint(&keys))
}

/// The five Adobe default colour-label strings, in our 1..=5 order. Adobe lets a
/// user rename these per-catalog, but the defaults are near-universal and are
/// what a fresh catalogue writes — matching more than that would need the
/// catalog we do not have.
const LABELS: [&str; 5] = ["Red", "Yellow", "Green", "Blue", "Purple"];

/// Maps our colour-label integer to Adobe's string. 0 (or out of range) → none.
pub fn label_to_string(label: i32) -> Option<&'static str> {
    LABELS.get((label - 1) as usize).copied()
}

/// Maps an Adobe label string to our integer, case-insensitively. Unknown
/// strings (a renamed label) return `None` rather than guessing.
pub fn label_from_string(s: &str) -> Option<i32> {
    LABELS
        .iter()
        .position(|l| l.eq_ignore_ascii_case(s.trim()))
        .map(|i| i as i32 + 1)
}

fn flag_to_string(flag: i32) -> Option<&'static str> {
    match flag {
        1 => Some("pick"),
        -1 => Some("reject"),
        _ => None,
    }
}

fn flag_from_string(s: &str) -> Option<i32> {
    match s.trim().to_ascii_lowercase().as_str() {
        "pick" => Some(1),
        "reject" => Some(-1),
        _ => None,
    }
}

/// An element or attribute name: its namespace, when the file declared the
/// prefix, and its local name.
#[derive(Debug)]
struct Name {
    ns: Option<Vec<u8>>,
    local: Vec<u8>,
}

impl Name {
    fn of((ns, local): (ResolveResult, LocalName)) -> Self {
        Name {
            ns: match ns {
                ResolveResult::Bound(n) if n.0 == LEGACY_APP_NS.as_bytes() => {
                    Some(APP_NS.as_bytes().to_vec())
                }
                ResolveResult::Bound(n) => Some(n.0.to_vec()),
                _ => None,
            },
            local: local.as_ref().to_vec(),
        }
    }

    /// Whether this is `local` in namespace `ns` — or `local` under a prefix
    /// the file never declared, read by its name alone as it always was.
    fn is(&self, ns: &str, local: &str) -> bool {
        self.local == local.as_bytes() && self.ns.as_deref().is_none_or(|n| n == ns.as_bytes())
    }
}

/// Parses an XMP packet. Never errors on unrecognised content — a sidecar full
/// of `crs:` develop settings simply yields whatever of the fields it also
/// happens to carry. Genuinely malformed XML is the only failure.
pub fn parse(xml: &str) -> Result<XmpMeta> {
    scan(xml).map(|s| s.meta)
}

/// Everything [`merge`] and [`read`] need to know about an existing packet.
#[derive(Debug, Default, PartialEq)]
struct Scan {
    meta: XmpMeta,
    /// The `dc:subject` keywords as the file spells them — case kept, so a
    /// rewrite hands back "Holiday" rather than the "holiday" `meta` holds.
    keywords: Vec<String>,
    synced: Synced,
    /// A fingerprint of every region the packet has — named or not, faces or
    /// not, which `meta.regions` leaves out — or `None` when it has none.
    regions: Option<String>,
    /// Every capture-date field's value, `None` for one that isn't a whole
    /// time: a merge replaces them only if each is one it may.
    dates: Vec<Option<chrono::NaiveDateTime>>,
}

/// A list property: which one an `rdf:li` belongs to.
#[derive(Debug, Clone, Copy, PartialEq)]
enum List {
    Keywords,
    Hierarchy,
    People,
    Description,
    Title,
    Creator,
    Rights,
    UsageTerms,
    SyncedKeywords,
    SyncedHierarchy,
    SyncedPeople,
}

const LISTS: [(List, &str, &str); 11] = [
    (List::Keywords, DC_NS, "subject"),
    (List::Hierarchy, LR_NS, "hierarchicalSubject"),
    (List::People, IPTC_EXT_NS, "PersonInImage"),
    (List::Description, DC_NS, "description"),
    (List::Title, DC_NS, "title"),
    (List::Creator, DC_NS, "creator"),
    (List::Rights, DC_NS, "rights"),
    (List::UsageTerms, XMP_RIGHTS_NS, "UsageTerms"),
    (List::SyncedKeywords, APP_NS, SYNCED_KEYWORDS),
    (List::SyncedHierarchy, APP_NS, SYNCED_HIERARCHY),
    (List::SyncedPeople, APP_NS, SYNCED_PEOPLE),
];

/// Simple properties read wherever they appear.
const SIMPLE: [(&str, &str); 17] = [
    (XMP_NS, "Rating"),
    (XMP_NS, "Label"),
    (APP_NS, FLAG),
    (DIGIKAM_NS, "PickLabel"),
    (APP_NS, SYNCED_RATING),
    (APP_NS, SYNCED_LABEL),
    (APP_NS, SYNCED_DESCRIPTION),
    (APP_NS, SYNCED_REGIONS),
    (PHOTOSHOP_NS, "Credit"),
    (APP_NS, SYNCED_TITLE),
    (APP_NS, SYNCED_CREATOR),
    (APP_NS, SYNCED_RIGHTS),
    (APP_NS, SYNCED_CREDIT),
    (APP_NS, SYNCED_USAGE),
    (APP_NS, SYNCED_CAPTURED),
    (EXIF_NS, "DateTimeOriginal"),
    (PHOTOSHOP_NS, "DateCreated"),
];
/// A region's own properties, read inside one.
const REGION_FIELDS: [(&str, &str); 7] = [
    (MWG_RS_NS, "Name"),
    (MWG_RS_NS, "Type"),
    (ST_AREA_NS, "x"),
    (ST_AREA_NS, "y"),
    (ST_AREA_NS, "w"),
    (ST_AREA_NS, "h"),
    (ST_AREA_NS, "unit"),
];

/// What the text of the element being read is.
#[derive(Debug)]
enum Capture {
    /// An item of a list, with its `xml:lang`.
    Item(List, Option<String>),
    Simple(Name),
}

/// A region being read.
#[derive(Debug, Default)]
struct Draft {
    name: Option<String>,
    kind: Option<String>,
    area: [Option<f64>; 4],
    unit: Option<String>,
}

#[derive(Default)]
struct Scanner {
    out: Scan,
    /// Our flag and digiKam's pick label, kept apart so ours wins.
    flag: Option<i32>,
    pick_label: Option<i32>,
    rated_reject: bool,
    /// `dc:description`'s alternatives, (language, text); likewise the
    /// title's, the rights' and the usage terms'. The creators, in order.
    descriptions: Vec<(String, String)>,
    titles: Vec<(String, String)>,
    rights: Vec<(String, String)>,
    usage: Vec<(String, String)>,
    creators: Vec<String>,
    /// Every region seen, keyed for the fingerprint.
    all_regions: Vec<String>,
    region: Option<Draft>,
    in_regions: bool,
    dims: (Option<u32>, Option<u32>),
    /// `exif:DateTimeOriginal` and `photoshop:DateCreated`, kept apart so the
    /// first wins.
    date_original: Option<chrono::NaiveDateTime>,
    date_created: Option<chrono::NaiveDateTime>,
}

impl Scanner {
    /// Applies one simple property, from either form.
    fn simple(&mut self, n: &Name, value: &str) {
        let v = value.trim();
        let meta = &mut self.out.meta;
        let synced = &mut self.out.synced;
        if n.is(XMP_NS, "Rating") {
            // Some browsers and culling tools rate a reject -1: a verdict, not a
            // rating to clear.
            match v.parse::<f32>() {
                Ok(x) if x < 0.0 => self.rated_reject = true,
                Ok(x) => meta.rating = Some((x.round() as i32).clamp(0, 5)),
                Err(_) => {}
            }
        } else if n.is(XMP_NS, "Label") {
            if let Some(l) = label_from_string(v) {
                meta.color_label = Some(l);
            }
        } else if n.is(APP_NS, FLAG) {
            self.flag = flag_from_string(v);
        } else if n.is(DIGIKAM_NS, "PickLabel") {
            // 0 none, 1 rejected, 2 pending, 3 accepted.
            self.pick_label = match v {
                "1" => Some(-1),
                "3" => Some(1),
                _ => None,
            };
        } else if n.is(APP_NS, SYNCED_RATING) {
            synced.rating = v.parse().ok();
        } else if n.is(APP_NS, SYNCED_LABEL) {
            synced.label = label_from_string(v);
        } else if n.is(APP_NS, SYNCED_DESCRIPTION) {
            synced.description = Some(v.to_string()).filter(|s| !s.is_empty());
        } else if n.is(APP_NS, SYNCED_REGIONS) {
            synced.regions = Some(v.to_string()).filter(|s| !s.is_empty());
        } else if n.is(PHOTOSHOP_NS, "Credit") {
            meta.text.credit = Some(v.to_string()).filter(|s| !s.is_empty());
        } else if let Some(record) = [
            SYNCED_TITLE,
            SYNCED_CREATOR,
            SYNCED_RIGHTS,
            SYNCED_CREDIT,
            SYNCED_USAGE,
        ]
        .into_iter()
        .find(|r| n.is(APP_NS, r))
        {
            if !v.is_empty() {
                synced.text.insert(record, v.to_string());
            }
        } else if n.is(APP_NS, SYNCED_CAPTURED) {
            synced.captured = parse_date(v);
        } else if n.is(EXIF_NS, "DateTimeOriginal") {
            self.date_original = parse_date(v);
            self.out.dates.push(self.date_original);
        } else if n.is(PHOTOSHOP_NS, "DateCreated") {
            self.date_created = parse_date(v);
            self.out.dates.push(self.date_created);
        } else if let Some(d) = self.region.as_mut() {
            if n.is(MWG_RS_NS, "Name") {
                d.name = Some(v.to_string());
            } else if n.is(MWG_RS_NS, "Type") {
                d.kind = Some(v.to_string());
            } else if n.is(ST_AREA_NS, "unit") {
                d.unit = Some(v.to_string());
            } else if let Some(i) = ["x", "y", "w", "h"]
                .iter()
                .position(|k| n.is(ST_AREA_NS, k))
            {
                d.area[i] = v.parse().ok();
            }
        } else if self.in_regions {
            if n.is(ST_DIM_NS, "w") {
                self.dims.0 = v.parse::<f64>().ok().map(|x| x as u32);
            } else if n.is(ST_DIM_NS, "h") {
                self.dims.1 = v.parse::<f64>().ok().map(|x| x as u32);
            }
        }
    }

    /// Whether an element's text is a simple property to read.
    fn wants_text(&self, n: &Name) -> bool {
        SIMPLE.iter().any(|(ns, l)| n.is(ns, l))
            || (self.region.is_some() && REGION_FIELDS.iter().any(|(ns, l)| n.is(ns, l)))
            || (self.in_regions && (n.is(ST_DIM_NS, "w") || n.is(ST_DIM_NS, "h")))
    }

    fn item(&mut self, list: List, text: String, lang: Option<String>) {
        let meta = &mut self.out.meta;
        let synced = &mut self.out.synced;
        let push_folded = |into: &mut Vec<String>, text: String| {
            if !into.iter().any(|x| x.eq_ignore_ascii_case(&text)) {
                into.push(text);
            }
        };
        match list {
            List::Keywords => {
                let kw = text.to_lowercase();
                if !meta.keywords.contains(&kw) {
                    meta.keywords.push(kw);
                    self.out.keywords.push(text);
                }
            }
            List::Hierarchy => push_folded(&mut meta.hierarchy, text),
            List::People => push_folded(&mut meta.people, text),
            List::Description => self.descriptions.push((lang.unwrap_or_default(), text)),
            List::Title => self.titles.push((lang.unwrap_or_default(), text)),
            List::Rights => self.rights.push((lang.unwrap_or_default(), text)),
            List::UsageTerms => self.usage.push((lang.unwrap_or_default(), text)),
            List::Creator => self.creators.push(text),
            List::SyncedKeywords => push_unique(&mut synced.keywords, text.to_lowercase()),
            List::SyncedHierarchy => push_unique(&mut synced.hierarchy, text.to_lowercase()),
            List::SyncedPeople => push_unique(&mut synced.people, text.to_lowercase()),
        }
    }

    /// Closes the region being read: every one counts toward the fingerprint,
    /// and a named face with a whole area is kept.
    fn finish_region(&mut self) {
        let Some(d) = self.region.take() else {
            return;
        };
        let name = d.name.as_deref().map(str::trim).unwrap_or_default();
        self.all_regions.push(region_key(
            name,
            d.kind.as_deref().unwrap_or_default(),
            d.area,
        ));
        let face = d
            .kind
            .as_deref()
            .is_none_or(|k| k.eq_ignore_ascii_case("Face"));
        let normalized = d
            .unit
            .as_deref()
            .is_none_or(|u| u.eq_ignore_ascii_case("normalized"));
        if let (true, true, false, [Some(x), Some(y), Some(w), Some(h)]) =
            (face, normalized, name.is_empty(), d.area)
        {
            self.out.meta.regions.push(Region::new(name, [x, y, w, h]));
        }
    }

    fn finish(mut self) -> Scan {
        let default = |d: &[(String, String)]| {
            d.iter()
                .find(|(lang, _)| lang.eq_ignore_ascii_case("x-default"))
                .or(d.first())
                .map(|(_, text)| text.clone())
        };
        self.out.meta.description = default(&self.descriptions);
        self.out.meta.text.title = default(&self.titles);
        self.out.meta.text.rights = default(&self.rights);
        self.out.meta.text.usage_terms = default(&self.usage);
        self.out.meta.text.creator = (!self.creators.is_empty()).then(|| self.creators.join("; "));
        self.out.meta.flag = self
            .flag
            .or(self.pick_label)
            .or(self.rated_reject.then_some(-1));
        if let (Some(w), Some(h)) = self.dims {
            self.out.meta.region_dims = Some((w, h));
        }
        self.out.meta.captured = self.date_original.or(self.date_created);
        self.out.regions = (!self.all_regions.is_empty()).then(|| fingerprint(&self.all_regions));
        self.out
    }
}

/// An attribute value, unescaped. XMP is always UTF-8, so a lossy decode plus
/// entity unescape is enough and dodges the encoding-aware attribute API's
/// churn between quick-xml versions.
fn decode(raw: &[u8]) -> String {
    let raw = String::from_utf8_lossy(raw);
    unescape(&raw)
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| raw.into_owned())
}

fn scan(xml: &str) -> Result<Scan> {
    // Text is buffered and trimmed per field at the closing tag, so per-event
    // trimming must stay off — it would eat the spaces around an entity
    // reference (`a &amp; b` arrives as "a ", "&", " b") and glue the words.
    let mut reader = NsReader::from_str(xml);
    let mut s = Scanner::default();
    // How deep the element being read is, counting the root as 1.
    let mut depth = 0usize;
    // The list being read, and the depth of its element; likewise the regions
    // and the region inside them.
    let mut list: Option<(List, usize)> = None;
    let mut regions_at: Option<usize> = None;
    let mut region_at: Option<usize> = None;
    // The element whose text is being read, and its depth. Text accumulates
    // in `buf`: quick-xml emits each entity (`&amp;`) as its own event,
    // splitting an element's text into pieces, so a keyword like "a & b"
    // arrives as three events and can only be reassembled by buffering until
    // the closing tag.
    let mut capture: Option<(Capture, usize)> = None;
    let mut buf = String::new();

    loop {
        let ev = reader.read_event()?;
        match ev {
            Event::Eof => break,
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(ev, Event::Empty(_));
                let at = depth + 1;
                if !empty {
                    depth = at;
                }
                let name = Name::of(reader.resolver().resolve_element(e.name()));
                let is_li = name.is(RDF_NS, "li");

                // An item of the region list is a region. It opens before its
                // attributes are read, since they may be its fields.
                if is_li && regions_at.is_some() && region_at.is_none() && list.is_none() {
                    s.region = Some(Draft::default());
                    region_at = Some(at);
                }
                if !empty && regions_at.is_none() && name.is(MWG_RS_NS, "Regions") {
                    regions_at = Some(at);
                    s.in_regions = true;
                }

                // Attribute form: xmp:Rating="4" xmp:Label="Red" on any
                // element (Adobe puts them on rdf:Description).
                let mut lang = None;
                for attr in e.attributes().flatten() {
                    if attr.key.as_ref() == b"xml:lang" {
                        lang = Some(decode(&attr.value));
                        continue;
                    }
                    let n = Name::of(reader.resolver().resolve_attribute(attr.key));
                    s.simple(&n, &decode(&attr.value));
                }

                if empty {
                    if region_at == Some(at) {
                        s.finish_region();
                        region_at = None;
                    }
                    continue;
                }
                if list.is_none() {
                    if let Some(&(l, _, _)) = LISTS.iter().find(|(_, ns, local)| name.is(ns, local))
                    {
                        list = Some((l, at));
                    }
                }
                if capture.is_none() {
                    // Only list items inside a list are read — a stray
                    // `rdf:li` elsewhere is not a keyword.
                    if is_li {
                        if let Some((l, _)) = list {
                            capture = Some((Capture::Item(l, lang), at));
                            buf.clear();
                        }
                    } else if s.wants_text(&name) {
                        capture = Some((Capture::Simple(name), at));
                        buf.clear();
                    }
                }
            }
            Event::Text(e) => {
                if capture.is_some() {
                    if let Ok(decoded) = e.decode() {
                        buf.push_str(&decoded);
                    }
                }
            }
            // An entity reference (`&amp;`, `&#169;`) inside the captured text.
            Event::GeneralRef(r) => {
                if capture.is_some() {
                    if let Ok(name) = r.decode() {
                        if let Some(s) = resolve_entity(&name, &r) {
                            buf.push_str(&s);
                        }
                    }
                }
            }
            Event::End(_) => {
                if capture.as_ref().is_some_and(|(_, at)| *at == depth) {
                    if let Some((c, _)) = capture.take() {
                        let text = buf.trim().to_string();
                        if !text.is_empty() {
                            match c {
                                Capture::Item(l, lang) => s.item(l, text, lang),
                                Capture::Simple(n) => s.simple(&n, &text),
                            }
                        }
                    }
                    buf.clear();
                }
                if region_at == Some(depth) {
                    s.finish_region();
                    region_at = None;
                }
                if list.is_some_and(|(_, at)| at == depth) {
                    list = None;
                }
                if regions_at == Some(depth) {
                    regions_at = None;
                    s.in_regions = false;
                }
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
    }

    Ok(s.finish())
}

/// A fresh packet: one empty `rdf:Description` for [`merge`] to fill.
const EMPTY_PACKET: &str = r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about="">
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>
"#;

/// Serialises metadata to a standalone XMP packet Adobe products and exiftool
/// both read. Simple values go on `rdf:Description` as attributes (the compact
/// form other editors write); lists become bags.
pub fn write(meta: &XmpMeta) -> String {
    merge(EMPTY_PACKET, meta).expect("the empty packet is well-formed")
}

/// Updates an existing XMP packet in place, **preserving everything it does not
/// own** — Adobe `crs:` develop settings, unknown namespaces, comments,
/// formatting, and any value this app did not write:
///
/// - a rating or label the catalogue has replaces the sidecar's;
/// - when the catalogue has none, the sidecar's is cleared only if it is still
///   the value this app last wrote there — so clearing a rating here still
///   reaches the sidecar — and is otherwise left alone;
/// - keywords, hierarchical keywords and people are added to the sidecar's,
///   and one is taken out only when this app wrote it and the catalogue no
///   longer has it;
/// - a caption or regions the sidecar has from elsewhere are kept as they are;
/// - the flag is this app's alone, and always matches the catalogue.
///
/// Falls back to a fresh packet when the existing one has no `rdf:Description`
/// to update. Fails when it is not well-formed XML, so the caller leaves the
/// file alone: rewriting a file this cannot read would mean replacing it.
/// Whether writing `packet` over `existing` would change only its layout:
/// everything this app reads from a sidecar, and its marks of what it wrote,
/// the same. [`merge`] lays out what it owns afresh each time, so the
/// bytes of a sidecar already up to date never match; this does.
pub fn same_content(existing: &str, packet: &str) -> bool {
    matches!((scan(existing), scan(packet)), (Ok(a), Ok(b)) if a == b)
}

pub fn merge(existing: &str, meta: &XmpMeta) -> Result<String> {
    let found = scan(existing)?;
    let plan = Plan::new(&found, meta);
    match rewrite(existing, &plan)? {
        Some(packet) => Ok(packet),
        None => merge(EMPTY_PACKET, meta),
    }
}

/// What a merge does with one of the sidecar's fields.
#[derive(Debug, Clone, PartialEq)]
enum Change<T> {
    /// Leave the sidecar's value exactly as it is.
    Keep,
    /// Take the sidecar's value out wherever it is and write this once in its
    /// place; for a single value, `None` writes nothing, clearing it.
    Replace(T),
}

impl<T> Change<T> {
    fn replaces(&self) -> bool {
        matches!(self, Change::Replace(_))
    }
}

/// How [`merge`] updates one packet: decided up front from the packet and the
/// catalogue, then carried out by [`rewrite`].
struct Plan<'a> {
    meta: &'a XmpMeta,
    rating: Change<Option<i32>>,
    label: Change<Option<i32>>,
    /// The whole `dc:subject` list to write, or `Keep` to leave the bag alone;
    /// likewise the other lists.
    keywords: Change<Vec<String>>,
    hierarchy: Change<Vec<String>>,
    people: Change<Vec<String>>,
    description: Change<Option<String>>,
    regions: Change<Vec<Region>>,
    /// The title and rights fields, by their record's name.
    text: std::collections::BTreeMap<&'static str, Change<Option<String>>>,
    /// Both date fields together, `exif:DateTimeOriginal` and
    /// `photoshop:DateCreated`.
    captured: Change<Option<chrono::NaiveDateTime>>,
    /// The record of what this write leaves in the sidecar from us.
    synced: Synced,
}

impl<'a> Plan<'a> {
    fn new(found: &Scan, meta: &'a XmpMeta) -> Self {
        let rating = meta.rating.map(|r| r.clamp(0, 5));
        let label = meta.color_label.filter(|&l| label_to_string(l).is_some());
        let caption = meta
            .description
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(str::to_string);
        let description = if meta.description_is_own {
            owned_text(
                caption,
                found.meta.description.as_ref(),
                found.synced.description.as_deref(),
            )
        } else {
            yielding_caption(
                caption,
                found.meta.description.as_ref(),
                found.synced.description.as_deref(),
            )
        };
        let mut text = std::collections::BTreeMap::new();
        for ((record, ours), (_, theirs)) in
            meta.text.fields().into_iter().zip(found.meta.text.fields())
        {
            let ours = ours.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
            text.insert(
                record,
                owned_text(
                    ours,
                    theirs,
                    found.synced.text.get(record).map(String::as_str),
                ),
            );
        }
        let ours = our_regions(&meta.regions);
        let regions = match &found.regions {
            None if meta.regions.is_empty() => Change::Keep,
            None => Change::Replace(meta.regions.clone()),
            Some(theirs)
                if Some(theirs) == found.synced.regions.as_ref()
                    || Some(theirs) == ours.as_ref() =>
            {
                Change::Replace(meta.regions.clone())
            }
            Some(_) => Change::Keep,
        };
        let captured = dated_change(
            meta.captured,
            meta.captured_original,
            &found.dates,
            found.synced.captured,
        );
        let lower = |list: &[String]| {
            let mut out = Vec::new();
            for item in list {
                push_unique(&mut out, item.to_lowercase());
            }
            out
        };
        let synced = Synced {
            rating,
            label,
            keywords: lower(&meta.keywords),
            hierarchy: lower(&meta.hierarchy),
            people: lower(&meta.people),
            description: match &description {
                Change::Replace(Some(d)) => Some(fingerprint(std::slice::from_ref(d))),
                _ => None,
            },
            regions: match &regions {
                Change::Replace(r) => our_regions(r),
                Change::Keep => None,
            },
            text: text
                .iter()
                .filter_map(|(record, c)| match c {
                    Change::Replace(Some(v)) => {
                        Some((*record, fingerprint(std::slice::from_ref(v))))
                    }
                    _ => None,
                })
                .collect(),
            captured: match captured {
                Change::Replace(t) => t,
                Change::Keep => None,
            },
        };
        Plan {
            meta,
            rating: owned_change(rating, found.meta.rating, found.synced.rating),
            label: owned_change(label, found.meta.color_label, found.synced.label),
            keywords: merged_list(&found.keywords, &found.synced.keywords, &meta.keywords),
            hierarchy: merged_list(
                &found.meta.hierarchy,
                &found.synced.hierarchy,
                &meta.hierarchy,
            ),
            people: merged_list(&found.meta.people, &found.synced.people, &meta.people),
            description,
            regions,
            text,
            captured,
            synced,
        }
    }

    /// Whether this write puts the model's description in.
    fn ai_caption(&self) -> bool {
        matches!(self.description, Change::Replace(Some(_))) && !self.meta.description_is_own
    }

    fn replaces_text(&self, record: &str) -> bool {
        self.text.get(record).is_some_and(Change::replaces)
    }

    fn text_value(&self, record: &str) -> Option<&String> {
        match self.text.get(record) {
            Some(Change::Replace(Some(v))) => Some(v),
            _ => None,
        }
    }

    /// Whether an element or attribute is one this write takes out: a field it
    /// replaces, or our own, which is always rewritten.
    fn drops(&self, n: &Name) -> bool {
        (n.is(XMP_NS, "Rating") && self.rating.replaces())
            || (n.is(XMP_NS, "Label") && self.label.replaces())
            || (n.is(DC_NS, "subject") && self.keywords.replaces())
            || (n.is(DC_NS, "description") && self.description.replaces())
            || (n.is(PHOTOSHOP_NS, "CaptionWriter") && self.description.replaces())
            || (n.is(LR_NS, "hierarchicalSubject") && self.hierarchy.replaces())
            || (n.is(IPTC_EXT_NS, "PersonInImage") && self.people.replaces())
            || (n.is(MWG_RS_NS, "Regions") && self.regions.replaces())
            || (n.is(DC_NS, "title") && self.replaces_text(SYNCED_TITLE))
            || (n.is(DC_NS, "creator") && self.replaces_text(SYNCED_CREATOR))
            || (n.is(DC_NS, "rights") && self.replaces_text(SYNCED_RIGHTS))
            || (n.is(PHOTOSHOP_NS, "Credit") && self.replaces_text(SYNCED_CREDIT))
            || (n.is(XMP_RIGHTS_NS, "UsageTerms") && self.replaces_text(SYNCED_USAGE))
            || ((n.is(EXIF_NS, "DateTimeOriginal") || n.is(PHOTOSHOP_NS, "DateCreated"))
                && self.captured.replaces())
            || APP_FIELDS.iter().any(|f| n.is(APP_NS, f))
    }

    /// The namespaces what this write adds needs declared, (prefix, URI).
    fn namespaces(&self) -> Vec<(&'static str, &'static str)> {
        let written = |c: &Change<Vec<String>>| matches!(c, Change::Replace(v) if !v.is_empty());
        let mut out = Vec::new();
        if matches!(self.rating, Change::Replace(Some(_)))
            || matches!(self.label, Change::Replace(Some(l)) if label_to_string(l).is_some())
        {
            out.push(("xmp", XMP_NS));
        }
        if written(&self.keywords)
            || matches!(self.description, Change::Replace(Some(_)))
            || [SYNCED_TITLE, SYNCED_CREATOR, SYNCED_RIGHTS]
                .iter()
                .any(|r| self.text_value(r).is_some())
        {
            out.push(("dc", DC_NS));
        }
        if self.text_value(SYNCED_CREDIT).is_some() || self.ai_caption() {
            out.push(("photoshop", PHOTOSHOP_NS));
        }
        if self.text_value(SYNCED_USAGE).is_some() {
            out.push(("xmpRights", XMP_RIGHTS_NS));
        }
        if written(&self.hierarchy) {
            out.push(("lr", LR_NS));
        }
        if written(&self.people) {
            out.push(("Iptc4xmpExt", IPTC_EXT_NS));
        }
        if matches!(&self.regions, Change::Replace(r) if !r.is_empty()) {
            out.extend([
                ("mwg-rs", MWG_RS_NS),
                ("stDim", ST_DIM_NS),
                ("stArea", ST_AREA_NS),
            ]);
        }
        if matches!(self.captured, Change::Replace(Some(_))) {
            out.extend([("exif", EXIF_NS), ("photoshop", PHOTOSHOP_NS)]);
        }
        if !self.synced.is_empty() || self.meta.flag.and_then(flag_to_string).is_some() {
            out.push((APP_PREFIX, APP_NS));
        }
        out
    }
}

/// A single-valued field: ours wins when we have one. When we don't, the
/// sidecar's is cleared only if it is still what we last wrote — anything else
/// was put there by someone else, or changed by them since.
fn owned_change(ours: Option<i32>, theirs: Option<i32>, wrote: Option<i32>) -> Change<Option<i32>> {
    match ours {
        Some(v) => Change::Replace(Some(v)),
        None if theirs.is_some() && theirs == wrote => Change::Replace(None),
        None => Change::Keep,
    }
}

/// A single text the user owns: ours replaces the sidecar's whenever we
/// have one; without one, theirs is taken out only while it is still what we
/// last wrote (`wrote` is its fingerprint).
fn owned_text(
    ours: Option<String>,
    theirs: Option<&String>,
    wrote: Option<&str>,
) -> Change<Option<String>> {
    match (ours, theirs) {
        (Some(v), _) => Change::Replace(Some(v)),
        (None, Some(t)) if wrote == Some(fingerprint(std::slice::from_ref(t)).as_str()) => {
            Change::Replace(None)
        }
        (None, _) => Change::Keep,
    }
}

/// The capture time. A correction replaces the sidecar's dates only
/// when each is one we may: the one we last wrote, the camera's own time, or
/// this very one — never a date someone set elsewhere, or one we can't read.
/// With no correction, dates are taken out only while they are still ours.
fn dated_change(
    ours: Option<chrono::NaiveDateTime>,
    original: Option<chrono::NaiveDateTime>,
    theirs: &[Option<chrono::NaiveDateTime>],
    wrote: Option<chrono::NaiveDateTime>,
) -> Change<Option<chrono::NaiveDateTime>> {
    match ours {
        Some(t) => {
            let ours_to_replace = |d: &Option<chrono::NaiveDateTime>| {
                d.is_some() && [wrote, original, ours].contains(d)
            };
            if theirs.iter().all(ours_to_replace) {
                Change::Replace(Some(t))
            } else {
                Change::Keep
            }
        }
        None if !theirs.is_empty() && wrote.is_some() && theirs.iter().all(|d| *d == wrote) => {
            Change::Replace(None)
        }
        None => Change::Keep,
    }
}

/// The caption: theirs wins. Ours is written where the sidecar has none, where
/// it has the one we last wrote (so a changed or cleared caption follows), or
/// where it already says exactly what ours does.
fn yielding_caption(
    ours: Option<String>,
    theirs: Option<&String>,
    wrote: Option<&str>,
) -> Change<Option<String>> {
    match theirs {
        None if ours.is_none() => Change::Keep,
        None => Change::Replace(ours),
        Some(t)
            if wrote == Some(fingerprint(std::slice::from_ref(t)).as_str())
                || Some(t) == ours.as_ref() =>
        {
            Change::Replace(ours)
        }
        Some(_) => Change::Keep,
    }
}

/// A list as the sidecar spells it, less any items we wrote that the catalogue
/// has since dropped, plus ours it lacks — matched without regard to case.
/// `Keep` when that changes nothing, so an untouched bag keeps its formatting
/// (and an `rdf:Seq` stays one).
fn merged_list(theirs: &[String], wrote: &[String], ours: &[String]) -> Change<Vec<String>> {
    let wanted: Vec<String> = ours.iter().map(|k| k.to_lowercase()).collect();
    let mut out: Vec<String> = theirs
        .iter()
        .filter(|k| {
            let k = k.to_lowercase();
            wanted.contains(&k) || !wrote.contains(&k)
        })
        .cloned()
        .collect();
    for item in ours {
        let lower = item.to_lowercase();
        if !out.iter().any(|o| o.to_lowercase() == lower) {
            out.push(item.clone());
        }
    }
    if out == theirs {
        Change::Keep
    } else {
        Change::Replace(out)
    }
}

/// Streams the packet through, copying every event verbatim except what the
/// plan takes out — stripped wherever it appears (attribute *or* element form,
/// in any `rdf:Description`) and re-emitted once into the first Description.
/// `Ok(None)` means "no Description found — write a fresh packet".
fn rewrite(existing: &str, plan: &Plan) -> Result<Option<String>> {
    let mut reader = NsReader::from_str(existing);
    let mut writer = Writer::new(Vec::new());
    let mut applied = false;
    // >0 while inside an element we are dropping.
    let mut skip: usize = 0;
    // Location properties being written, matched by qualified name: `City` alone
    // would also catch the one nested inside an IPTC Extension location struct.
    let located = plan.meta.location_fields();
    // Whitespace waiting to see whether the element after it is dropped: it
    // goes with the element, or every rewrite would leave another blank line.
    let mut indent: Option<Event> = None;

    loop {
        let ev = reader.read_event()?;
        if skip > 0 {
            match ev {
                Event::Start(_) => skip += 1,
                Event::End(_) => skip -= 1,
                Event::Eof => break,
                _ => {}
            }
            continue;
        }
        if let Event::Text(t) = &ev {
            if t.iter().all(u8::is_ascii_whitespace) {
                if let Some(held) = indent.replace(ev) {
                    writer.write_event(held)?;
                }
                continue;
            }
        }
        let empty = matches!(ev, Event::Empty(_));
        if let Event::Start(e) | Event::Empty(e) = &ev {
            let name = Name::of(reader.resolver().resolve_element(e.name()));
            // Drop the old value's subtree; the field is re-emitted centrally.
            if plan.drops(&name)
                || located
                    .iter()
                    .any(|(n, _)| n.as_bytes() == e.name().as_ref())
            {
                indent = None;
                if !empty {
                    skip = 1;
                }
                continue;
            }
        }
        if let Some(held) = indent.take() {
            writer.write_event(held)?;
        }
        match ev {
            Event::Eof => break,
            Event::Start(e) | Event::Empty(e) => {
                let name = Name::of(reader.resolver().resolve_element(e.name()));
                let qname = e.name();
                if !name.is(RDF_NS, "Description") {
                    writer.write_event(if empty {
                        Event::Empty(e)
                    } else {
                        Event::Start(e)
                    })?;
                    continue;
                }
                let add = !applied;
                let tag = String::from_utf8_lossy(qname.as_ref()).into_owned();
                writer.write_event(Event::Start(rewrite_description(
                    &e,
                    reader.resolver(),
                    plan,
                    add,
                )))?;
                if add {
                    writer.write_event(Event::Text(BytesText::from_escaped(fields_xml(plan))))?;
                    applied = true;
                }
                // An empty Description is re-opened so our fields can become
                // its children, then closed here — there is no End to copy.
                if empty {
                    writer.write_event(Event::End(BytesEnd::new(tag)))?;
                }
            }
            other => writer.write_event(other)?,
        }
    }

    if !applied {
        return Ok(None);
    }
    Ok(Some(String::from_utf8(writer.into_inner())?))
}

/// How each attribute this app adds is separated from the one before.
const ATTR_SEP: &str = "\n    ";

/// Rebuilds an `rdf:Description` start tag: copies every attribute the plan
/// does not take out, and (when `add`) writes ours — the rating and label being
/// replaced, the flag, the location, our record — plus any namespace
/// declarations the tag lacks. Copied attribute values are written as they sat
/// in the file — already escaped — so `crs:` and other foreign attributes
/// survive untouched, except that a `"` a single-quoted value held becomes
/// `&quot;`, since every value is written between double quotes.
fn rewrite_description(
    e: &BytesStart,
    resolver: &NamespaceResolver,
    plan: &Plan,
    add: bool,
) -> BytesStart<'static> {
    let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
    let mut content = name.clone();
    let located = plan.meta.location_fields();
    let mut declared: Vec<Vec<u8>> = Vec::new();
    for attr in e.attributes().flatten() {
        let key = attr.key.as_ref();
        if plan.drops(&Name::of(resolver.resolve_attribute(attr.key))) {
            continue;
        }
        // Replaced by ours below — but only on the Description ours go on.
        if add && located.iter().any(|(n, _)| n.as_bytes() == key) {
            continue;
        }
        if key.starts_with(b"xmlns:") {
            declared.push(key.to_vec());
        }
        content.push(' ');
        content.push_str(&String::from_utf8_lossy(key));
        content.push_str("=\"");
        content.push_str(&String::from_utf8_lossy(attr.value.as_ref()).replace('"', "&quot;"));
        content.push('"');
    }
    if add {
        for (prefix, uri) in plan.namespaces() {
            let decl = format!("xmlns:{prefix}");
            if !declared.iter().any(|d| d == decl.as_bytes()) {
                content.push_str(&format!("{ATTR_SEP}{decl}=\"{uri}\""));
                // The location's namespaces overlap these (exif, photoshop).
                declared.push(decl.into_bytes());
            }
        }
        if let Change::Replace(Some(r)) = &plan.rating {
            content.push_str(&format!("{ATTR_SEP}xmp:Rating=\"{}\"", (*r).clamp(0, 5)));
        }
        if let Change::Replace(Some(l)) = &plan.label {
            if let Some(l) = label_to_string(*l) {
                content.push_str(&format!("{ATTR_SEP}xmp:Label=\"{l}\""));
            }
        }
        if let Some(f) = plan.meta.flag.and_then(flag_to_string) {
            content.push_str(&format!("{ATTR_SEP}{}=\"{f}\"", app(FLAG)));
        }
        if let Some(c) = plan.text_value(SYNCED_CREDIT) {
            content.push_str(&format!("{ATTR_SEP}photoshop:Credit=\"{}\"", escape(c)));
        }
        if plan.ai_caption() {
            content.push_str(&format!(
                "{ATTR_SEP}photoshop:CaptionWriter=\"{}\"",
                escape(AI_CAPTION_WRITER)
            ));
        }
        if let Change::Replace(Some(t)) = &plan.captured {
            let t = t.format(DATE_FORMAT);
            content.push_str(&format!("{ATTR_SEP}exif:DateTimeOriginal=\"{t}\""));
            content.push_str(&format!("{ATTR_SEP}photoshop:DateCreated=\"{t}\""));
        }
        if !located.is_empty() {
            content.push_str(&location_namespaces(ATTR_SEP, &declared));
            for (field, value) in &located {
                content.push_str(&format!("{ATTR_SEP}{field}=\"{}\"", escape(value)));
            }
        }
        content.push_str(&plan.synced.attributes(ATTR_SEP));
    }
    BytesStart::from_content(content, name.len())
}

/// `xmlns:` declarations for the location namespaces, each prefixed with
/// `sep`, leaving out any the tag already declares.
fn location_namespaces(sep: &str, declared: &[Vec<u8>]) -> String {
    crate::export::location::XMP_NAMESPACES
        .iter()
        .filter(|(prefix, _)| {
            !declared
                .iter()
                .any(|d| d == format!("xmlns:{prefix}").as_bytes())
        })
        .map(|(prefix, uri)| format!("{sep}xmlns:{prefix}=\"{uri}\""))
        .collect()
}

/// The elements the first Description gets: each list and struct being
/// replaced, and our record of the lists we wrote.
fn fields_xml(plan: &Plan) -> String {
    let mut out = String::new();
    if let Change::Replace(k) = &plan.keywords {
        bag(&mut out, "dc:subject", k);
    }
    if let Change::Replace(h) = &plan.hierarchy {
        bag(&mut out, "lr:hierarchicalSubject", h);
    }
    let alt = |out: &mut String, element: &str, text: &str| {
        out.push_str(&format!(
            "\n   <{element}>\n    <rdf:Alt>\n     <rdf:li xml:lang=\"x-default\">{}</rdf:li>\n    </rdf:Alt>\n   </{element}>",
            escape(text)
        ));
    };
    if let Some(t) = plan.text_value(SYNCED_TITLE) {
        alt(&mut out, "dc:title", t);
    }
    if let Some(c) = plan.text_value(SYNCED_CREATOR) {
        let names: Vec<String> = c
            .split(';')
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
            .collect();
        out.push_str("\n   <dc:creator>\n    <rdf:Seq>");
        for n in names {
            out.push_str(&format!("\n     <rdf:li>{}</rdf:li>", escape(&n)));
        }
        out.push_str("\n    </rdf:Seq>\n   </dc:creator>");
    }
    if let Some(r) = plan.text_value(SYNCED_RIGHTS) {
        alt(&mut out, "dc:rights", r);
    }
    if let Some(u) = plan.text_value(SYNCED_USAGE) {
        alt(&mut out, "xmpRights:UsageTerms", u);
    }
    if let Change::Replace(Some(d)) = &plan.description {
        out.push_str(&format!(
            "\n   <dc:description>\n    <rdf:Alt>\n     <rdf:li xml:lang=\"x-default\">{}</rdf:li>\n    </rdf:Alt>\n   </dc:description>",
            escape(d)
        ));
    }
    if let Change::Replace(p) = &plan.people {
        bag(&mut out, "Iptc4xmpExt:PersonInImage", p);
    }
    if let Change::Replace(r) = &plan.regions {
        regions_xml(&mut out, r, plan.meta.region_dims);
    }
    for (element, items) in plan.synced.lists() {
        bag(&mut out, &app(element), items);
    }
    out
}

/// `element` holding an `rdf:Bag` of `items`; nothing when there are none.
fn bag(out: &mut String, element: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    out.push_str(&format!("\n   <{element}>\n    <rdf:Bag>"));
    for item in items {
        out.push_str(&format!("\n     <rdf:li>{}</rdf:li>", escape(item)));
    }
    out.push_str(&format!("\n    </rdf:Bag>\n   </{element}>"));
}

/// The regions in the form other editors write them.
fn regions_xml(out: &mut String, regions: &[Region], dims: Option<(u32, u32)>) {
    if regions.is_empty() {
        return;
    }
    out.push_str("\n   <mwg-rs:Regions rdf:parseType=\"Resource\">");
    if let Some((w, h)) = dims {
        out.push_str(&format!(
            "\n    <mwg-rs:AppliedToDimensions stDim:w=\"{w}\" stDim:h=\"{h}\" stDim:unit=\"pixel\"/>"
        ));
    }
    out.push_str("\n    <mwg-rs:RegionList>\n     <rdf:Bag>");
    for r in regions {
        let [x, y, w, h] = r.area;
        out.push_str(&format!(
            "\n      <rdf:li>\n       <rdf:Description mwg-rs:Name=\"{}\" mwg-rs:Type=\"Face\">\n        <mwg-rs:Area stArea:x=\"{x:.4}\" stArea:y=\"{y:.4}\" stArea:w=\"{w:.4}\" stArea:h=\"{h:.4}\" stArea:unit=\"normalized\"/>\n       </rdf:Description>\n      </rdf:li>",
            escape(&r.name)
        ));
    }
    out.push_str("\n     </rdf:Bag>\n    </mwg-rs:RegionList>\n   </mwg-rs:Regions>");
}

/// Resolves an entity reference to its text. Handles the five predefined named
/// entities and numeric character references (`&#169;` / `&#xA9;`); an unknown
/// named entity yields nothing rather than corrupting the value.
fn resolve_entity(name: &str, bytes: &quick_xml::events::BytesRef) -> Option<String> {
    match name {
        "amp" => Some("&".into()),
        "lt" => Some("<".into()),
        "gt" => Some(">".into()),
        "quot" => Some("\"".into()),
        "apos" => Some("'".into()),
        _ => bytes
            .resolve_char_ref()
            .ok()
            .flatten()
            .map(|c| c.to_string()),
    }
}

/// The five XML characters that must be escaped inside element text.
fn escape(s: &str) -> String {
    xml_chars(s)
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// `s` without the characters XML 1.0 forbids — the C0 controls other than
/// tab, newline and return, and U+FFFE/U+FFFF — which no escape can carry. A
/// caption the vision model read off a photo can hold one, and a strict
/// reader then rejects the whole sidecar.
pub(crate) fn xml_chars(s: &str) -> String {
    s.chars()
        .filter(|&c| {
            !matches!(c, '\u{0}'..='\u{8}' | '\u{B}' | '\u{C}' | '\u{E}'..='\u{1F}' | '\u{FFFE}' | '\u{FFFF}')
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The attribute form editors write most often.
    #[test]
    fn parses_attribute_form() {
        let xml = r#"
            <x:xmpmeta xmlns:x="adobe:ns:meta/">
             <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
              <rdf:Description rdf:about=""
                xmlns:xmp="http://ns.adobe.com/xap/1.0/"
                xmp:Rating="4" xmp:Label="Green"/>
             </rdf:RDF>
            </x:xmpmeta>"#;
        let m = parse(xml).unwrap();
        assert_eq!(m.rating, Some(4));
        assert_eq!(m.color_label, Some(3)); // Green -> 3
    }

    /// The element form Bridge and some cameras write.
    #[test]
    fn parses_element_form_and_keywords() {
        let xml = r#"
            <x:xmpmeta xmlns:x="adobe:ns:meta/">
             <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
              <rdf:Description rdf:about=""
                xmlns:xmp="http://ns.adobe.com/xap/1.0/"
                xmlns:dc="http://purl.org/dc/elements/1.1/">
               <xmp:Rating>2</xmp:Rating>
               <xmp:Label>Blue</xmp:Label>
               <dc:subject>
                <rdf:Bag>
                 <rdf:li>Cat</rdf:li>
                 <rdf:li>Kitten</rdf:li>
                </rdf:Bag>
               </dc:subject>
              </rdf:Description>
             </rdf:RDF>
            </x:xmpmeta>"#;
        let m = parse(xml).unwrap();
        assert_eq!(m.rating, Some(2));
        assert_eq!(m.color_label, Some(4)); // Blue -> 4
        assert_eq!(m.keywords, vec!["cat".to_string(), "kitten".to_string()]);
    }

    /// A `crs:` develop-heavy sidecar with no DAM fields yields an empty meta,
    /// not an error — the common case where only edits were stored.
    #[test]
    fn ignores_develop_settings() {
        let xml = r#"
            <x:xmpmeta xmlns:x="adobe:ns:meta/">
             <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
              <rdf:Description rdf:about=""
                xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
                crs:Exposure2012="+0.50" crs:Contrast2012="25"/>
             </rdf:RDF>
            </x:xmpmeta>"#;
        assert_eq!(parse(xml).unwrap(), XmpMeta::default());
    }

    /// An unrecognised (renamed) label is dropped rather than guessed.
    #[test]
    fn unknown_label_is_ignored() {
        let xml = r#"<rdf:Description xmp:Label="Approved" xmlns:rdf="x" xmlns:xmp="y"/>"#;
        assert_eq!(parse(xml).unwrap().color_label, None);
    }

    /// A `dc:subject` list item outside any subject bag is not read as a keyword.
    #[test]
    fn li_outside_subject_is_not_a_keyword() {
        let xml = r#"
            <rdf:Description xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
             <rdf:Bag><rdf:li>notakeyword</rdf:li></rdf:Bag>
            </rdf:Description>"#;
        assert!(parse(xml).unwrap().keywords.is_empty());
    }

    /// What we write, we read back identically — the round trip a migration
    /// depends on.
    #[test]
    fn write_then_parse_round_trips() {
        let meta = XmpMeta {
            rating: Some(5),
            color_label: Some(1), // Red
            keywords: vec!["sunset".into(), "beach".into()],
            ..Default::default()
        };
        let parsed = parse(&write(&meta)).unwrap();
        assert_eq!(parsed, meta);
    }

    /// Writing the same metadata again changes nothing but layout:
    /// what lets a second "Write all sidecars" leave every unchanged file as
    /// it was, its date with it, instead of rewriting the library's sidecars.
    /// Not byte for byte: `merge` re-lays out what it owns each time.
    #[test]
    fn merging_what_a_sidecar_already_says_changes_nothing() {
        let meta = XmpMeta {
            rating: Some(4),
            color_label: Some(2),
            keywords: vec!["harbour".into(), "boats".into()],
            ..Default::default()
        };
        let ours = write(&meta);
        assert!(same_content(&ours, &merge(&ours, &meta).unwrap()));
        // And twice over, once merged into someone else's.
        let foreign = r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
    xmp:Rating="2" crs:Exposure2012="+0.50"/>
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>"#;
        let once = merge(foreign, &meta).unwrap();
        assert!(same_content(&once, &merge(&once, &meta).unwrap()));
        // Someone else's file isn't ours until we've written to it.
        assert!(!same_content(foreign, &once));
        // And a change is a change.
        let rerated = XmpMeta {
            rating: Some(1),
            ..meta.clone()
        };
        assert!(!same_content(&once, &merge(&once, &rerated).unwrap()));
    }

    /// Keywords with XML-significant characters survive the round trip.
    #[test]
    fn special_characters_in_keywords_survive() {
        let meta = XmpMeta {
            rating: None,
            color_label: None,
            keywords: vec!["a & b < c".into()],
            ..Default::default()
        };
        assert_eq!(parse(&write(&meta)).unwrap().keywords, meta.keywords);
    }

    /// An empty meta writes a valid packet that parses back to empty — exporting
    /// an untouched photo must not crash or invent metadata.
    #[test]
    fn empty_meta_round_trips() {
        assert_eq!(
            parse(&write(&XmpMeta::default())).unwrap(),
            XmpMeta::default()
        );
    }

    #[test]
    fn label_mapping_is_symmetric() {
        for i in 1..=5 {
            assert_eq!(label_from_string(label_to_string(i).unwrap()), Some(i));
        }
        assert_eq!(label_to_string(0), None);
        assert_eq!(label_from_string("green"), Some(3)); // case-insensitive
    }

    /// The whole point of `merge`: exporting our three fields must not destroy a
    /// sidecar's Adobe develop settings (or any other foreign metadata).
    #[test]
    fn merge_preserves_foreign_develop_settings() {
        let existing = r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
    xmp:Rating="2" crs:Exposure2012="+0.50" crs:Contrast2012="25"/>
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>"#;
        let meta = XmpMeta {
            rating: Some(5),
            color_label: Some(1),
            keywords: vec!["sunset".into()],
            ..Default::default()
        };
        let merged = merge(existing, &meta).unwrap();

        // Foreign develop settings survive untouched.
        assert!(merged.contains(r#"crs:Exposure2012="+0.50""#), "{merged}");
        assert!(merged.contains(r#"crs:Contrast2012="25""#), "{merged}");
        // Our field is updated (2 -> 5), not duplicated.
        assert_eq!(merged.matches("xmp:Rating").count(), 1, "{merged}");
        // And the packet reads back to the new values.
        let parsed = parse(&merged).unwrap();
        assert_eq!(parsed.rating, Some(5));
        assert_eq!(parsed.color_label, Some(1));
        assert_eq!(parsed.keywords, vec!["sunset".to_string()]);
    }

    fn karoo() -> crate::export::location::PhotoLocation {
        crate::export::location::PhotoLocation {
            position: Some((-32.3, 22.5)),
            approximate: true,
            name: Some("Karoo".into()),
            country: Some("South Africa".into()),
            ..Default::default()
        }
    }

    #[test]
    fn a_fresh_sidecar_carries_the_location() {
        let meta = XmpMeta {
            rating: Some(3),
            location: Some(karoo()),
            ..Default::default()
        };
        let packet = write(&meta);
        assert!(
            packet.contains(r#"exif:GPSLatitude="32,18.000000S""#),
            "{packet}"
        );
        assert!(
            packet.contains(r#"exif:GPSHPositioningError="5000/1""#),
            "{packet}"
        );
        assert!(
            packet.contains(r#"photoshop:Country="South Africa""#),
            "{packet}"
        );
        assert!(
            packet.contains(r#"Iptc4xmpCore:Location="Karoo""#),
            "{packet}"
        );
        assert!(packet.contains(r#"xmlns:photoshop="http://ns.adobe.com/photoshop/1.0/""#));
        // Still a packet this module (and so other editors) reads.
        assert_eq!(parse(&packet).unwrap().rating, Some(3));
    }

    #[test]
    fn merge_replaces_a_location_in_either_form_and_keeps_the_rest() {
        let existing = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:exif="http://ns.adobe.com/exif/1.0/"
    xmlns:photoshop="http://ns.adobe.com/photoshop/1.0/"
    xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
    exif:GPSLatitude="10,0.0N" crs:Exposure2012="+0.50">
   <photoshop:Country>Nowhere</photoshop:Country>
   <photoshop:State>Western Cape</photoshop:State>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;
        let meta = XmpMeta {
            location: Some(karoo()),
            ..Default::default()
        };
        let merged = merge(existing, &meta).unwrap();
        assert_eq!(merged.matches("GPSLatitude").count(), 1, "{merged}");
        assert!(
            merged.contains(r#"exif:GPSLatitude="32,18.000000S""#),
            "{merged}"
        );
        assert!(
            !merged.contains("Nowhere"),
            "the old element-form country stays: {merged}"
        );
        assert!(
            merged.contains(r#"photoshop:Country="South Africa""#),
            "{merged}"
        );
        // Not ours to touch: a field we don't write, and develop settings.
        assert!(
            merged.contains("<photoshop:State>Western Cape</photoshop:State>"),
            "{merged}"
        );
        assert!(merged.contains(r#"crs:Exposure2012="+0.50""#), "{merged}");
        // Namespaces already declared are not declared twice.
        assert_eq!(merged.matches("xmlns:exif=").count(), 1, "{merged}");
        assert_eq!(merged.matches("xmlns:Iptc4xmpCore=").count(), 1, "{merged}");
    }

    #[test]
    fn a_photo_with_no_location_leaves_lightrooms_alone() {
        let existing = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about="" xmlns:exif="http://ns.adobe.com/exif/1.0/"
    exif:GPSLatitude="51,30.0N" exif:GPSLongitude="0,7.0W"/>
 </rdf:RDF>
</x:xmpmeta>"#;
        let merged = merge(
            existing,
            &XmpMeta {
                rating: Some(4),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            merged.contains(r#"exif:GPSLatitude="51,30.0N""#),
            "{merged}"
        );
        assert!(merged.contains(r#"exif:GPSLongitude="0,7.0W""#), "{merged}");
    }

    /// An element-form Rating is replaced, not left as a stale duplicate; a
    /// keyword someone else wrote stays beside ours.
    #[test]
    fn merge_rewrites_element_form_and_adds_to_foreign_keywords() {
        let existing = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:dc="http://purl.org/dc/elements/1.1/">
   <xmp:Rating>1</xmp:Rating>
   <dc:subject><rdf:Bag><rdf:li>old</rdf:li></rdf:Bag></dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;
        let meta = XmpMeta {
            rating: Some(4),
            color_label: None,
            keywords: vec!["new".into()],
            ..Default::default()
        };
        let merged = merge(existing, &meta).unwrap();
        let parsed = parse(&merged).unwrap();
        assert_eq!(parsed.rating, Some(4));
        assert_eq!(
            merged.matches("Rating>").count(),
            0,
            "stale element form: {merged}"
        );
        assert_eq!(parsed.keywords, vec!["old".to_string(), "new".to_string()]);
    }

    /// A Description missing the xmp/dc namespaces still gets valid, readable output.
    #[test]
    fn merge_adds_namespaces_when_absent() {
        let existing = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about=""/></rdf:RDF>"#;
        let meta = XmpMeta {
            rating: Some(3),
            color_label: None,
            keywords: vec![],
            ..Default::default()
        };
        let merged = merge(existing, &meta).unwrap();
        assert!(
            merged.contains(r#"xmlns:xmp="http://ns.adobe.com/xap/1.0/""#),
            "{merged}"
        );
        assert_eq!(parse(&merged).unwrap().rating, Some(3));
    }

    /// Clearing a rating, label or keyword this app wrote takes it back out of
    /// the sidecar rather than leaving the old value — and the record with it.
    #[test]
    fn merge_clears_what_we_wrote_and_the_catalogue_dropped() {
        let existing = write(&XmpMeta {
            rating: Some(5),
            color_label: Some(2),
            keywords: vec!["a".into(), "b".into()],
            ..Default::default()
        });
        let merged = merge(
            &existing,
            &XmpMeta {
                keywords: vec!["a".into()],
                ..Default::default()
            },
        )
        .unwrap();
        let parsed = parse(&merged).unwrap();
        assert_eq!(parsed.rating, None, "{merged}");
        assert_eq!(parsed.color_label, None, "{merged}");
        assert_eq!(parsed.keywords, vec!["a".to_string()]);
        assert!(
            !merged.contains("Rating"),
            "rating or its record left: {merged}"
        );
        assert!(
            !merged.contains("Label"),
            "label or its record left: {merged}"
        );
    }

    /// Writing sidecars before ever importing them used to wipe another
    /// editor's ratings, labels and keywords, because the catalogue having
    /// nothing was written out as the sidecar having nothing.
    #[test]
    fn merge_keeps_a_rating_label_and_keywords_it_did_not_write() {
        let existing = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmp:Rating="4" xmp:Label="Red">
   <dc:subject><rdf:Bag><rdf:li>Holiday</rdf:li></rdf:Bag></dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;
        let meta = XmpMeta {
            rating: None,
            color_label: None,
            keywords: vec!["tree".into()],
            ..Default::default()
        };
        let merged = merge(existing, &meta).unwrap();
        let parsed = parse(&merged).unwrap();
        assert_eq!(parsed.rating, Some(4), "{merged}");
        assert_eq!(parsed.color_label, Some(1), "{merged}");
        assert_eq!(
            parsed.keywords,
            vec!["holiday".to_string(), "tree".to_string()]
        );
        // Spelled as the sidecar had it, not as the catalogue stores it.
        assert!(merged.contains("<rdf:li>Holiday</rdf:li>"), "{merged}");
    }

    /// A value we wrote that someone changed since is theirs now: the catalogue
    /// having none doesn't clear it.
    #[test]
    fn merge_keeps_a_value_changed_elsewhere_since_we_wrote_it() {
        let ours = write(&XmpMeta {
            rating: Some(4),
            ..Default::default()
        });
        let changed = ours.replace(r#"xmp:Rating="4""#, r#"xmp:Rating="5""#);
        assert_ne!(ours, changed, "the fixture must change the rating");
        let merged = merge(&changed, &XmpMeta::default()).unwrap();
        assert_eq!(parse(&merged).unwrap().rating, Some(5), "{merged}");
    }

    /// Only the keywords we wrote are ours to take out; one added elsewhere
    /// survives our removals, as it was spelled.
    #[test]
    fn merge_removes_only_keywords_it_wrote() {
        let existing = format!(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:{APP_PREFIX}="{APP_NS}">
   <dc:subject><rdf:Bag><rdf:li>a</rdf:li><rdf:li>b</rdf:li><rdf:li>Cat</rdf:li></rdf:Bag></dc:subject>
   <{APP_PREFIX}:{SYNCED_KEYWORDS}><rdf:Bag><rdf:li>a</rdf:li><rdf:li>b</rdf:li></rdf:Bag></{APP_PREFIX}:{SYNCED_KEYWORDS}>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#
        );
        let meta = XmpMeta {
            keywords: vec!["a".into()],
            ..Default::default()
        };
        let merged = merge(&existing, &meta).unwrap();
        assert_eq!(
            parse(&merged).unwrap().keywords,
            vec!["a".to_string(), "cat".to_string()]
        );
        assert!(merged.contains("<rdf:li>Cat</rdf:li>"), "{merged}");
        // The record now says only "a" is ours.
        assert_eq!(
            scan(&merged).unwrap().synced.keywords,
            vec!["a".to_string()]
        );
    }

    /// When a merge changes no keyword, the bag is left exactly as it was — an
    /// `rdf:Seq` is not rewritten as a Bag.
    #[test]
    fn merge_leaves_an_unchanged_keyword_list_as_it_was() {
        let existing = r#"<rdf:Description xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:subject><rdf:Seq><rdf:li>Fern</rdf:li></rdf:Seq></dc:subject></rdf:Description>"#;
        let meta = XmpMeta {
            keywords: vec!["fern".into()],
            ..Default::default()
        };
        let merged = merge(existing, &meta).unwrap();
        assert!(
            merged.contains("<dc:subject><rdf:Seq><rdf:li>Fern</rdf:li></rdf:Seq></dc:subject>"),
            "{merged}"
        );
    }

    /// Writing the same catalogue values again changes nothing: no field or
    /// record is duplicated, and the packet settles rather than drifting. (The
    /// first rewrite may reorder attributes; after that it is a fixed point.)
    #[test]
    fn merging_again_is_stable() {
        let meta = XmpMeta {
            rating: Some(3),
            color_label: Some(4),
            keywords: vec!["fern".into(), "moss".into()],
            location: Some(karoo()),
            ..Default::default()
        };
        let once = merge(&write(&XmpMeta::default()), &meta).unwrap();
        let twice = merge(&once, &meta).unwrap();
        let thrice = merge(&twice, &meta).unwrap();
        assert_eq!(twice, thrice);
        let found = scan(&thrice).unwrap();
        assert_eq!(found.meta.rating, Some(3));
        assert_eq!(found.meta.color_label, Some(4));
        assert_eq!(found.meta.keywords, meta.keywords);
        assert_eq!(
            found.synced,
            Synced {
                rating: Some(3),
                label: Some(4),
                keywords: vec!["fern".into(), "moss".into()],
                ..Default::default()
            }
        );
        for once_only in [
            "xmp:Rating".to_string(),
            app(SYNCED_RATING),
            app(SYNCED_KEYWORDS),
            "<dc:subject>".to_string(),
            "GPSLatitude".to_string(),
        ] {
            let expected = if once_only == app(SYNCED_KEYWORDS) {
                2
            } else {
                1
            }; // open + close tag
            assert_eq!(
                thrice.matches(once_only.as_str()).count(),
                expected,
                "{once_only}: {thrice}"
            );
        }
    }

    /// What this app wrote is recorded, and the record never reads back as the
    /// fields it describes.
    #[test]
    fn write_records_what_it_wrote() {
        let meta = XmpMeta {
            rating: Some(2),
            color_label: Some(5),
            keywords: vec!["Fern".into()],
            ..Default::default()
        };
        let packet = write(&meta);
        let found = scan(&packet).unwrap();
        assert_eq!(
            found.synced,
            Synced {
                rating: Some(2),
                label: Some(5),
                keywords: vec!["fern".into()],
                ..Default::default()
            }
        );
        assert_eq!(
            found.meta.keywords,
            vec!["fern".to_string()],
            "the record isn't a keyword"
        );
        assert!(
            packet.contains(&format!(r#"xmlns:{APP_PREFIX}="{APP_NS}""#)),
            "{packet}"
        );
    }

    /// A sidecar that isn't well-formed XML is refused rather than replaced:
    /// the caller then leaves the file alone.
    #[test]
    fn merge_refuses_malformed_xml() {
        let broken =
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:Description xmp:Rating="4"></x:xmpmeta>"#;
        assert!(merge(broken, &XmpMeta::default()).is_err());
    }

    /// Anything without an rdf:Description falls back to a fresh packet.
    #[test]
    fn merge_falls_back_to_a_fresh_packet_without_a_description() {
        let meta = XmpMeta {
            rating: Some(2),
            color_label: None,
            keywords: vec![],
            ..Default::default()
        };
        let merged = merge("<html><body>not xmp</body></html>", &meta).unwrap();
        assert_eq!(parse(&merged).unwrap().rating, Some(2));
    }

    /// A sidecar as another editor writes one with a named face: the region,
    /// and the person again as a keyword, the way such editors keep them.
    pub(crate) const LIGHTROOM: &str = include_str!("../tests/fixtures/xmp/lightroom_faces.xmp");

    fn ada_region() -> Region {
        Region::new("Ada", [0.3, 0.3, 0.1, 0.1])
    }

    /// Every field reads back as it was written.
    #[test]
    fn every_field_round_trips() {
        let meta = XmpMeta {
            ai_keywords: Vec::new(),
            rating: Some(4),
            color_label: Some(3),
            keywords: vec!["sunset".into(), "landscape".into()],
            hierarchy: vec!["Genre|landscape".into(), "Conditions|golden hour".into()],
            flag: Some(-1),
            description: Some("A lighthouse at dusk & a gull".into()),
            people: vec!["Ada Lovelace".into(), "Bo".into()],
            regions: vec![Region::new("Ada Lovelace", [0.5, 0.4, 0.1, 0.15])],
            region_dims: Some((6000, 4000)),
            location: None,
            description_is_own: false,
            text: XmpText {
                title: Some("Dusk at the Point".into()),
                creator: Some("Ada Lovelace; Bo".into()),
                rights: Some("© 2025 Ada Lovelace".into()),
                credit: Some("Ada Lovelace / Northern Light".into()),
                usage_terms: Some("Editorial use only".into()),
            },
            captured: Some(at("2025-06-14T11:05:00")),
            captured_original: None,
        };
        let packet = write(&meta);
        assert_eq!(parse(&packet).unwrap(), meta, "{packet}");
        assert!(packet.contains(r#"sourcelight:Flag="reject""#), "{packet}");
        assert!(
            packet.contains(
                r#"<rdf:li xml:lang="x-default">A lighthouse at dusk &amp; a gull</rdf:li>"#
            ),
            "{packet}"
        );
        assert!(
            packet.contains("<rdf:li>Genre|landscape</rdf:li>"),
            "{packet}"
        );
    }

    #[test]
    fn a_pick_reads_back_and_other_tools_rejects_are_understood() {
        let pick = write(&XmpMeta {
            flag: Some(1),
            ..Default::default()
        });
        assert_eq!(parse(&pick).unwrap().flag, Some(1));
        let bridge = parse(r#"<rdf:Description xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:Rating="-1"/>"#).unwrap();
        assert_eq!(
            (bridge.flag, bridge.rating),
            (Some(-1), None),
            "Bridge's reject"
        );
        let digikam = r#"<rdf:Description xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:digiKam="http://www.digikam.org/ns/1.0/" digiKam:PickLabel="1"/>"#;
        assert_eq!(parse(digikam).unwrap().flag, Some(-1));
        assert_eq!(
            parse(&digikam.replace("\"1\"", "\"2\"")).unwrap().flag,
            None,
            "pending is no verdict"
        );
    }

    /// Sidecars from builds before the public release use the old namespace.
    #[test]
    fn the_old_namespace_still_reads_and_is_replaced_on_write() {
        let old = write(&XmpMeta {
            flag: Some(1),
            ..Default::default()
        })
        .replace(APP_NS, LEGACY_APP_NS)
        .replace(&format!("{APP_PREFIX}:"), "photoorganiser:")
        .replace(&format!("xmlns:{APP_PREFIX}="), "xmlns:photoorganiser=");
        assert!(
            old.contains(LEGACY_APP_NS) && !old.contains(APP_NS),
            "{old}"
        );
        assert_eq!(parse(&old).unwrap().flag, Some(1));

        let rejected = merge(
            &old,
            &XmpMeta {
                flag: Some(-1),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(parse(&rejected).unwrap().flag, Some(-1));
        assert!(!rejected.contains("photoorganiser:Flag"), "{rejected}");
        assert!(
            rejected.contains(&format!("{APP_PREFIX}:Flag")),
            "{rejected}"
        );
    }

    /// The flag is this app's alone, so clearing it takes it out.
    #[test]
    fn a_cleared_flag_leaves_the_sidecar() {
        let picked = write(&XmpMeta {
            flag: Some(1),
            ..Default::default()
        });
        let merged = merge(&picked, &XmpMeta::default()).unwrap();
        assert!(!merged.contains("Flag"), "{merged}");
    }

    #[test]
    fn reads_lightroom_face_regions() {
        let m = parse(LIGHTROOM).unwrap();
        assert_eq!(
            m.regions,
            vec![Region::new("Ada Lovelace", [0.5, 0.4, 0.1, 0.15])]
        );
        assert_eq!(m.region_dims, Some((6000, 4000)));
        assert_eq!(m.rating, Some(3));
        // Her name is a keyword too, but it says who, not what.
        let kw = m.sorted_keywords();
        assert_eq!(kw.subjects, vec!["beach".to_string()]);
        assert_eq!(kw.genre, None);
    }

    #[test]
    fn keywords_sort_into_genre_conditions_subjects_and_people() {
        let m = XmpMeta {
            keywords: ["landscape", "golden hour", "fern", "ada", "france"]
                .map(String::from)
                .to_vec(),
            hierarchy: [
                "Genre|Landscape",
                "Conditions|Golden hour",
                "People|Ada",
                "Places|Europe|France",
            ]
            .map(String::from)
            .to_vec(),
            ..Default::default()
        };
        let k = m.sorted_keywords();
        assert_eq!(k.genre.as_deref(), Some("landscape"));
        assert_eq!(k.conditions, vec!["golden hour".to_string()]);
        assert_eq!(k.people, vec!["Ada".to_string()]);
        assert_eq!(k.subjects, vec!["france".to_string(), "fern".to_string()]);
        assert_eq!(k.subject_paths, vec![vec!["places", "europe", "france"]]);
    }

    /// Other editors write a keyword's parents into the flat list as well.
    /// They say where it is filed, not what else the photo is of.
    #[test]
    fn a_keywords_parents_in_the_flat_list_are_not_subjects() {
        let m = XmpMeta {
            keywords: ["france", "europe", "places", "dog", "animals"]
                .map(String::from)
                .to_vec(),
            hierarchy: ["Places|Europe|France", "Animals|Dog", "Animals"]
                .map(String::from)
                .to_vec(),
            ..Default::default()
        };
        let k = m.sorted_keywords();
        // `animals` has a path of its own, so the photo has it; `europe` and
        // `places` are only where France is.
        assert_eq!(k.subjects, vec!["france", "dog", "animals"]);
        assert_eq!(
            k.subject_paths,
            vec![vec!["places", "europe", "france"], vec!["animals", "dog"]]
        );
    }

    #[test]
    fn a_region_crosses_every_orientation_and_back() {
        let b = [0.1f32, 0.2, 0.3, 0.25];
        for o in 1..=8 {
            let back = Region::from_displayed("Ada", b, o).displayed_box(o);
            for (got, want) in back.iter().zip(b) {
                assert!((got - want).abs() < 1e-4, "orientation {o}: {back:?}");
            }
        }
    }

    /// MWG measures regions on the image as stored. A frame shown upright by
    /// a quarter turn clockwise (orientation 6) with a face at its top middle
    /// has it at the middle of the stored frame's left edge.
    #[test]
    fn a_region_is_written_against_the_image_as_stored() {
        let r = Region::from_displayed("Ada", [0.4, 0.05, 0.2, 0.1], 6);
        assert_eq!(r.area, [0.1, 0.5, 0.1, 0.2]);
        assert_eq!(stored_dims((4000, 6000), false, 6), (6000, 4000));
        assert_eq!(stored_dims((6000, 4000), true, 1), (6000, 4000));
        assert_eq!(stored_dims((6000, 4000), false, 1), (4000, 6000));
    }

    /// A caption written elsewhere is a person's words, and ours never
    /// replaces it. Ours is ours to change and to clear.
    #[test]
    fn merge_keeps_someone_elses_caption_and_updates_ours() {
        let theirs = r#"<rdf:Description xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:description><rdf:Alt><rdf:li xml:lang="x-default">Granny's 80th</rdf:li></rdf:Alt></dc:description></rdf:Description>"#;
        let ours = XmpMeta {
            description: Some("An elderly woman and a cake".into()),
            ..Default::default()
        };
        let merged = merge(theirs, &ours).unwrap();
        let back = read(&merged).unwrap();
        assert_eq!(back.meta.description.as_deref(), Some("Granny's 80th"));
        assert!(!back.description_is_ours);

        let first = write(&ours);
        assert!(read(&first).unwrap().description_is_ours);
        let recaptioned = XmpMeta {
            description: Some("A woman and a cake".into()),
            ..Default::default()
        };
        let merged = merge(&first, &recaptioned).unwrap();
        assert_eq!(
            parse(&merged).unwrap().description.as_deref(),
            Some("A woman and a cake")
        );
        let cleared = merge(&merged, &XmpMeta::default()).unwrap();
        assert_eq!(parse(&cleared).unwrap().description, None, "{cleared}");
    }

    /// Another editor's regions are kept whole, and ours not added to them; the
    /// name still arrives through PersonInImage.
    #[test]
    fn merge_keeps_someone_elses_regions_whole() {
        let ours = XmpMeta {
            people: vec!["Ada".into()],
            regions: vec![ada_region()],
            ..Default::default()
        };
        let merged = merge(LIGHTROOM, &ours).unwrap();
        let m = parse(&merged).unwrap();
        assert_eq!(
            m.regions,
            vec![Region::new("Ada Lovelace", [0.5, 0.4, 0.1, 0.15])],
            "{merged}"
        );
        assert_eq!(m.people, vec!["Ada".to_string()]);
        assert!(merged.contains(r#"crs:Exposure2012="+0.50""#), "{merged}");
    }

    #[test]
    fn merge_moves_and_removes_its_own_regions() {
        let ours = XmpMeta {
            regions: vec![ada_region()],
            ..Default::default()
        };
        let first = write(&ours);
        let moved = XmpMeta {
            regions: vec![Region::new("Ada", [0.6, 0.3, 0.1, 0.1])],
            ..Default::default()
        };
        assert_eq!(
            parse(&merge(&first, &moved).unwrap()).unwrap().regions,
            moved.regions
        );
        let gone = merge(&first, &XmpMeta::default()).unwrap();
        assert!(!gone.contains("mwg-rs:Regions"), "{gone}");
    }

    /// A region someone added beside ours — even one with no name, or not a
    /// face — makes the set theirs, and none of it is rewritten.
    #[test]
    fn a_region_added_elsewhere_makes_the_set_theirs() {
        let ours = XmpMeta {
            regions: vec![ada_region()],
            ..Default::default()
        };
        let first = write(&ours);
        let close = "\n     </rdf:Bag>\n    </mwg-rs:RegionList>";
        let pet = r#"<rdf:li><rdf:Description mwg-rs:Type="Pet"><mwg-rs:Area stArea:x="0.7" stArea:y="0.7" stArea:w="0.1" stArea:h="0.1" stArea:unit="normalized"/></rdf:Description></rdf:li>"#;
        let added = first.replace(close, &format!("{pet}{close}"));
        assert_ne!(first, added, "the fixture must add a region");
        let merged = merge(&added, &XmpMeta::default()).unwrap();
        assert!(merged.contains("Pet"), "{merged}");
        assert_eq!(parse(&merged).unwrap().regions, ours.regions);
    }

    /// Microsoft's People regions share a local name with MWG's.
    #[test]
    fn microsoft_people_regions_are_left_alone() {
        let existing = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:MP="http://ns.microsoft.com/photo/1.2/" xmlns:MPRI="http://ns.microsoft.com/photo/1.2/t/RegionInfo#" xmlns:MPReg="http://ns.microsoft.com/photo/1.2/t/Region#"><MP:RegionInfo rdf:parseType="Resource"><MPRI:Regions><rdf:Bag><rdf:li MPReg:PersonDisplayName="Ada" MPReg:Rectangle="0.1, 0.1, 0.2, 0.2"/></rdf:Bag></MPRI:Regions></MP:RegionInfo></rdf:Description></rdf:RDF></x:xmpmeta>"#;
        let meta = XmpMeta {
            regions: vec![ada_region()],
            ..Default::default()
        };
        let merged = merge(existing, &meta).unwrap();
        assert!(merged.contains("<MPRI:Regions>"), "{merged}");
        assert!(merged.contains("<mwg-rs:Regions"), "{merged}");
        assert_eq!(parse(&merged).unwrap().regions, meta.regions);
    }

    #[test]
    fn merge_adds_people_and_takes_out_only_those_it_wrote() {
        let existing = r#"<rdf:Description xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:Iptc4xmpExt="http://iptc.org/std/Iptc4xmpExt/2008-02-29/"><Iptc4xmpExt:PersonInImage><rdf:Bag><rdf:li>Cy</rdf:li></rdf:Bag></Iptc4xmpExt:PersonInImage></rdf:Description>"#;
        let merged = merge(
            existing,
            &XmpMeta {
                people: vec!["Ada".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            parse(&merged).unwrap().people,
            vec!["Cy".to_string(), "Ada".to_string()]
        );
        let merged = merge(&merged, &XmpMeta::default()).unwrap();
        assert_eq!(parse(&merged).unwrap().people, vec!["Cy".to_string()]);
    }

    /// With every field set, writing again changes nothing and duplicates
    /// nothing.
    #[test]
    fn merging_every_field_again_is_stable() {
        let meta = XmpMeta {
            rating: Some(3),
            keywords: vec!["fern".into(), "landscape".into()],
            hierarchy: vec!["Genre|landscape".into()],
            flag: Some(1),
            description: Some("A fern".into()),
            people: vec!["Ada".into()],
            regions: vec![ada_region()],
            region_dims: Some((6000, 4000)),
            ..Default::default()
        };
        let twice = merge(&write(&meta), &meta).unwrap();
        let thrice = merge(&twice, &meta).unwrap();
        assert_eq!(twice, thrice);
        assert_eq!(parse(&thrice).unwrap(), meta);
        for once_only in [
            "<mwg-rs:Regions ",
            "<dc:description>",
            "<lr:hierarchicalSubject>",
            "<Iptc4xmpExt:PersonInImage>",
            "sourcelight:Flag=",
            "sourcelight:SyncedRegions=",
            "sourcelight:SyncedDescription=",
        ] {
            assert_eq!(
                thrice.matches(once_only).count(),
                1,
                "{once_only}: {thrice}"
            );
        }
    }

    // --- Title, caption and rights someone wrote -------------------------

    fn lightroom_titled() -> &'static str {
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:photoshop="http://ns.adobe.com/photoshop/1.0/"
    photoshop:Credit="Their credit">
   <dc:title><rdf:Alt><rdf:li xml:lang="x-default">Their title</rdf:li></rdf:Alt></dc:title>
   <dc:description><rdf:Alt><rdf:li xml:lang="x-default">Their caption</rdf:li></rdf:Alt></dc:description>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#
    }

    #[test]
    fn a_title_or_credit_set_elsewhere_is_read_and_offered_as_theirs() {
        let sidecar = read(lightroom_titled()).unwrap();
        assert_eq!(sidecar.meta.text.title.as_deref(), Some("Their title"));
        assert_eq!(
            sidecar.text_from_elsewhere.credit.as_deref(),
            Some("Their credit")
        );
    }

    #[test]
    fn our_own_title_and_caption_replace_theirs_and_the_models_does_not() {
        let own = XmpMeta {
            description: Some("Our caption".into()),
            description_is_own: true,
            text: XmpText {
                title: Some("Our title".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let merged = merge(lightroom_titled(), &own).unwrap();
        let back = parse(&merged).unwrap();
        assert_eq!(back.description.as_deref(), Some("Our caption"), "{merged}");
        assert_eq!(back.text.title.as_deref(), Some("Our title"), "{merged}");
        assert_eq!(
            back.text.credit.as_deref(),
            Some("Their credit"),
            "we have none: kept"
        );

        let models = XmpMeta {
            description: Some("A model's caption".into()),
            ..Default::default()
        };
        let merged = merge(lightroom_titled(), &models).unwrap();
        assert_eq!(
            parse(&merged).unwrap().description.as_deref(),
            Some("Their caption")
        );
        assert_eq!(
            parse(&merged).unwrap().text.title.as_deref(),
            Some("Their title")
        );
    }

    #[test]
    fn a_cleared_title_leaves_only_while_it_is_still_ours() {
        let titled = XmpMeta {
            text: XmpText {
                title: Some("Our title".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let ours = write(&titled);
        assert!(
            read(&ours).unwrap().text_from_elsewhere.title.is_none(),
            "ours to skip on import"
        );
        let cleared = merge(&ours, &XmpMeta::default()).unwrap();
        assert_eq!(parse(&cleared).unwrap().text.title, None, "{cleared}");

        let changed = ours.replace("Our title", "Their title now");
        let kept = merge(&changed, &XmpMeta::default()).unwrap();
        assert_eq!(
            parse(&kept).unwrap().text.title.as_deref(),
            Some("Their title now")
        );
    }

    // --- Corrected capture times ---------------------------------------

    fn at(s: &str) -> chrono::NaiveDateTime {
        chrono::NaiveDateTime::parse_from_str(s, DATE_FORMAT).unwrap()
    }

    /// A photo whose clock was an hour out: corrected to 11:00 from the
    /// camera's 10:00.
    fn corrected() -> XmpMeta {
        XmpMeta {
            rating: Some(3),
            captured: Some(at("2025-06-14T11:00:00")),
            captured_original: Some(at("2025-06-14T10:00:00")),
            ..Default::default()
        }
    }

    /// Another editor's sidecar dated `date` in both fields, as it writes them.
    fn dated(date: &str) -> String {
        format!(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:exif="http://ns.adobe.com/exif/1.0/"
    xmlns:photoshop="http://ns.adobe.com/photoshop/1.0/"
    xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
    exif:DateTimeOriginal="{date}"
    photoshop:DateCreated="{date}"
    crs:Exposure2012="+0.50"/>
 </rdf:RDF>
</x:xmpmeta>"#
        )
    }

    fn dates_in(packet: &str) -> Vec<Option<chrono::NaiveDateTime>> {
        scan(packet).unwrap().dates
    }

    #[test]
    fn a_corrected_time_is_written_and_an_uncorrected_one_is_not() {
        let packet = write(&corrected());
        assert!(
            packet.contains(r#"exif:DateTimeOriginal="2025-06-14T11:00:00""#),
            "{packet}"
        );
        assert!(
            packet.contains(r#"photoshop:DateCreated="2025-06-14T11:00:00""#),
            "{packet}"
        );
        assert!(read(&packet).unwrap().captured_is_ours, "{packet}");

        let plain = write(&XmpMeta {
            rating: Some(3),
            ..Default::default()
        });
        assert!(
            !plain.contains("DateTimeOriginal") && !plain.contains("DateCreated"),
            "{plain}"
        );
        assert!(!plain.contains(SYNCED_CAPTURED), "{plain}");
    }

    #[test]
    fn the_cameras_own_time_is_replaced_whatever_its_zone() {
        // Other editors copy the camera's time into the sidecar, with a zone and
        // hundredths: that is the time being corrected, not someone's own.
        let merged = merge(&dated("2025-06-14T10:00:00.00+01:00"), &corrected()).unwrap();
        assert_eq!(
            dates_in(&merged),
            vec![Some(at("2025-06-14T11:00:00")); 2],
            "{merged}"
        );
        assert!(merged.contains(r#"crs:Exposure2012="+0.50""#), "{merged}");
        assert_eq!(merged.matches("xmlns:exif=").count(), 1, "{merged}");
    }

    #[test]
    fn a_date_set_elsewhere_is_kept() {
        let theirs = dated("2025-06-14T09:30:00");
        let merged = merge(&theirs, &corrected()).unwrap();
        assert_eq!(
            dates_in(&merged),
            vec![Some(at("2025-06-14T09:30:00")); 2],
            "{merged}"
        );
        assert!(!merged.contains(SYNCED_CAPTURED), "{merged}");
        // Offered on import, never applied: it isn't ours.
        let sidecar = read(&merged).unwrap();
        assert_eq!(sidecar.meta.captured, Some(at("2025-06-14T09:30:00")));
        assert!(!sidecar.captured_is_ours);

        // Nor is a date that isn't a whole time, which can't be compared.
        let day = dated("2025-06-14");
        let merged = merge(&day, &corrected()).unwrap();
        assert!(
            merged.contains(r#"exif:DateTimeOriginal="2025-06-14""#),
            "{merged}"
        );
    }

    #[test]
    fn reverting_takes_ours_out_only_while_it_is_still_ours() {
        let ours = merge(&dated("2025-06-14T10:00:00"), &corrected()).unwrap();
        let reverted = XmpMeta {
            rating: Some(3),
            ..Default::default()
        };

        let back = merge(&ours, &reverted).unwrap();
        assert!(dates_in(&back).is_empty(), "{back}");
        assert!(!back.contains(SYNCED_CAPTURED), "{back}");
        assert!(back.contains(r#"crs:Exposure2012="+0.50""#), "{back}");

        // Changed in another editor since: theirs now, and kept.
        let changed = ours.replace(
            r#"exif:DateTimeOriginal="2025-06-14T11:00:00""#,
            r#"exif:DateTimeOriginal="2025-06-14T12:00:00""#,
        );
        let back = merge(&changed, &reverted).unwrap();
        assert!(
            back.contains(r#"exif:DateTimeOriginal="2025-06-14T12:00:00""#),
            "{back}"
        );
    }

    #[test]
    fn a_second_correction_replaces_the_first() {
        let first = merge(&dated("2025-06-14T10:00:00"), &corrected()).unwrap();
        let again = XmpMeta {
            captured: Some(at("2025-06-14T12:00:00")),
            ..corrected()
        };
        let merged = merge(&first, &again).unwrap();
        assert_eq!(
            dates_in(&merged),
            vec![Some(at("2025-06-14T12:00:00")); 2],
            "{merged}"
        );
    }

    #[test]
    fn a_date_beside_a_location_declares_each_namespace_once() {
        let meta = XmpMeta {
            location: Some(karoo()),
            ..corrected()
        };
        let packet = write(&meta);
        for ns in ["xmlns:exif=", "xmlns:photoshop="] {
            assert_eq!(packet.matches(ns).count(), 1, "{ns} in {packet}");
        }
        assert_eq!(parse(&packet).unwrap().captured, meta.captured, "{packet}");
    }

    /// A credit line and a corrected date both live in photoshop's namespace:
    /// declared once, and both read back.
    #[test]
    fn a_credit_and_a_corrected_date_share_one_namespace() {
        let meta = XmpMeta {
            text: XmpText {
                credit: Some("Northern Light".into()),
                ..Default::default()
            },
            captured: Some(at("2025-06-14T11:00:00")),
            location: Some(karoo()),
            ..Default::default()
        };
        let packet = write(&meta);
        assert_eq!(packet.matches("xmlns:photoshop=").count(), 1, "{packet}");
        let back = parse(&packet).unwrap();
        assert_eq!(back.text.credit.as_deref(), Some("Northern Light"));
        assert_eq!(back.captured, meta.captured);
    }

    /// A single-quoted foreign attribute holding a `"` comes back as valid
    /// XML, and a control character in our own text is dropped, not passed
    /// on.
    #[test]
    fn a_rewritten_sidecar_stays_well_formed() {
        let existing = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:Note='say "hi"'/></rdf:RDF></x:xmpmeta>"#;
        let meta = XmpMeta {
            rating: Some(3),
            description: Some("bell\u{7} rang".into()),
            ..Default::default()
        };
        let merged = merge(existing, &meta).unwrap();
        assert!(
            merged.contains(r#"crs:Note="say &quot;hi&quot;""#),
            "{merged}"
        );
        assert!(!merged.contains('\u{7}'));
        let back = read(&merged).expect("parses");
        assert_eq!(back.meta.rating, Some(3));
    }
}
