//! Keywords from folder names.
//!
//! A great many libraries are organised by nothing but folders —
//! `2024-05 Karoo trip/`, `Weddings/Smith & Jones/` — and that is knowledge the
//! catalogue already has, in every photo's path. This turns a photo's folders
//! into keywords, leaving out what says nothing about the photos: the names
//! cameras and phones give their folders (`DCIM`, `100MSDCF`), a workflow's
//! stages (`raw`, `export`, `selects`), and dates, which the capture date
//! already carries better.
//!
//! Pure and deterministic, so the rules can be read off the tests.

/// Folder names that describe the storage or the workflow, not the photos.
/// Compared lower-cased and with runs of `_`, `-` and spaces made one space.
const GENERIC: &[&str] = &[
    "dcim",
    "camera",
    "camera roll",
    "camera uploads",
    "photos",
    "photo",
    "pictures",
    "picture",
    "pics",
    "images",
    "image",
    "img",
    "my pictures",
    "my photos",
    "raw",
    "raws",
    "cr3",
    "cr2",
    "nef",
    "arw",
    "dng",
    "orf",
    "rw2",
    "raf",
    "jpg",
    "jpgs",
    "jpeg",
    "jpegs",
    "heic",
    "tif",
    "tiff",
    "png",
    "export",
    "exports",
    "exported",
    "output",
    "edit",
    "edits",
    "edited",
    "final",
    "finals",
    "selects",
    "select",
    "selected",
    "picks",
    "keepers",
    "rejects",
    "originals",
    "original",
    "masters",
    "import",
    "imports",
    "imported",
    "incoming",
    "inbox",
    "capture",
    "captures",
    "lightroom",
    "lightroom catalog",
    "capture one",
    "darktable",
    "untitled",
    "untitled folder",
    "new folder",
    "misc",
    "other",
    "various",
    "tmp",
    "temp",
    "backup",
    "backups",
    "archive",
    "archives",
    "sorted",
    "unsorted",
    "to sort",
    "web",
    "print",
    "prints",
    "hi res",
    "low res",
    "full res",
    "small",
    "large",
    "thumbnails",
    "thumbs",
    "previews",
    "sidecars",
    "xmp",
    "video",
    "videos",
    "movies",
    "users",
    "home",
    "desktop",
    "downloads",
    "documents",
    "onedrive",
    "dropbox",
    "google drive",
    "icloud photos",
];

/// Camera makers — one of which a folder named after its camera always holds.
const CAMERA_BRANDS: &[&str] = &[
    "sony",
    "canon",
    "nikon",
    "fujifilm",
    "fuji",
    "olympus",
    "panasonic",
    "lumix",
    "leica",
    "pentax",
    "ricoh",
    "hasselblad",
    "sigma",
    "gopro",
    "dji",
    "insta360",
    "iphone",
    "samsung",
    "pixel",
    "xiaomi",
    "huawei",
    "oneplus",
];

/// The words camera model names are built from, besides numbers.
const MODEL_WORDS: &[&str] = &[
    "eos", "alpha", "pro", "max", "mini", "plus", "ultra", "mark", "mk", "galaxy", "hero", "mavic",
    "osmo", "pocket", "action", "om", "system", "zv", "rx",
];

const MONTHS: &[&str] = &[
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
    "jan",
    "feb",
    "mar",
    "apr",
    "jun",
    "jul",
    "aug",
    "sep",
    "sept",
    "oct",
    "nov",
    "dec",
];

/// The keywords a photo's folders give, outermost first, each once. `dirs` are
/// folder names only — the library folder's own name first, then each folder
/// between it and the file.
pub fn keywords(dirs: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for dir in dirs {
        for k in from_name(dir) {
            if !out.contains(&k) {
                out.push(k);
            }
        }
    }
    out
}

/// What one folder's name says: none, one or (split on " - ") several
/// keywords.
pub fn from_name(name: &str) -> Vec<String> {
    let name = name.trim();
    // A camera's own folder: `100MSDCF`, `101_FUJI`, `100CANON`, `100APPLE`.
    if is_camera_folder(name) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for part in name.split(" - ") {
        let words = tidy(part);
        let words = strip_dates(&words);
        if is_camera_name(&words) {
            continue;
        }
        let k = words.join(" ");
        if k.chars().filter(|c| c.is_alphabetic()).count() < 2
            || GENERIC.contains(&k.as_str())
            || MONTHS.contains(&k.as_str())
            || k.contains('|')
        {
            continue;
        }
        if !out.contains(&k) {
            out.push(k);
        }
    }
    out
}

/// Lower-cased words, `_` and `.` taken as spaces, punctuation trimmed off
/// each end.
fn tidy(part: &str) -> Vec<String> {
    part.replace(['_', '.'], " ")
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric() && c != '&' && c != '\'')
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// Whether a word is part of a date: a year, a month by number or name, a
/// day, or a whole date run together (`2024-05-01`, `20240501`, `2024-05`).
fn is_date_word(w: &str) -> bool {
    if MONTHS.contains(&w) {
        return true;
    }
    let digits_and_seps = w.chars().all(|c| c.is_ascii_digit() || c == '-');
    if !digits_and_seps || !w.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }
    let digits: String = w.chars().filter(|c| c.is_ascii_digit()).collect();
    match digits.len() {
        1 | 2 => true,              // a day or a month
        4 => is_year(&digits),      // 2024
        6 => is_year(&digits[..4]), // 202405
        8 => is_year(&digits[..4]), // 20240501
        _ => false,
    }
}

fn is_year(d: &str) -> bool {
    d.parse::<u32>().is_ok_and(|y| (1900..=2099).contains(&y))
}

/// The words with a date taken off the front and the back: "2024 05 karoo
/// trip" is "karoo trip", "summer 2024" is "summer". A name that is all date
/// comes back empty.
fn strip_dates(words: &[String]) -> Vec<String> {
    let start = words.iter().take_while(|w| is_date_word(w)).count();
    let rest = &words[start..];
    let end = rest.len() - rest.iter().rev().take_while(|w| is_date_word(w)).count();
    rest[..end].to_vec()
}

/// A folder named after the camera that took its photos — "Sony A6400",
/// "Canon EOS R6", "iPhone 15 Pro" — which says what took them, not what they
/// are of: a maker's name, and nothing but model words, numbers and letters.
/// Found in a real library, where "Sony A6400" would have gone on
/// a thousand photos. "Pixel art" and "Canon walk" stay: a word that is
/// neither a model's nor a number makes it a name.
fn is_camera_name(words: &[String]) -> bool {
    words.iter().any(|w| CAMERA_BRANDS.contains(&w.as_str()))
        && words.iter().all(|w| {
            CAMERA_BRANDS.contains(&w.as_str())
                || MODEL_WORDS.contains(&w.as_str())
                || w.chars().any(|c| c.is_ascii_digit())
                || w.chars().count() <= 2
        })
}

fn is_camera_folder(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() >= 5
        && b.len() <= 12
        && b[..3].iter().all(u8::is_ascii_digit)
        && b[3..]
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || *c == b'_')
        && b[3..].iter().any(u8::is_ascii_alphabetic)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real folder names, and what each should give.
    const CASES: &[(&str, &[&str])] = &[
        // Camera cards and phones.
        ("DCIM", &[]),
        ("100MSDCF", &[]),
        ("101_FUJI", &[]),
        ("100CANON", &[]),
        ("100APPLE", &[]),
        ("100NIKON", &[]),
        ("100GOPRO", &[]),
        ("Camera Roll", &[]),
        ("Camera Uploads", &[]),
        // Named after the camera.
        ("Sony A6400", &[]),
        ("Canon EOS R6", &[]),
        ("Nikon Z 6", &[]),
        ("Fujifilm X-T3", &[]),
        ("iPhone 15 Pro", &[]),
        ("Canon 5D Mark IV", &[]),
        ("GoPro Hero 11", &[]),
        ("Sony Alpha", &[]),
        ("Pixel art", &["pixel art"]),
        ("Canon walk", &["canon walk"]),
        ("Apple picking", &["apple picking"]),
        ("2024-05 Sony A6400 - Karoo", &["karoo"]),
        // Date trees: an editor's import, and by hand.
        ("2024", &[]),
        ("05", &[]),
        ("2024-05-01", &[]),
        ("2024_05_01", &[]),
        ("20240501", &[]),
        ("2024-05", &[]),
        ("May", &[]),
        ("05 - May", &[]),
        ("2024 05 May", &[]),
        // Dates around a name.
        ("2024-05 Karoo trip", &["karoo trip"]),
        ("2024-05-01 Smith wedding", &["smith wedding"]),
        ("20240501_Smith_wedding", &["smith wedding"]),
        ("Summer 2024", &["summer"]),
        ("Iceland 2019", &["iceland"]),
        ("2019.08.14 Reykjavik", &["reykjavik"]),
        // Names kept as they are, lower-cased.
        ("Karoo", &["karoo"]),
        ("Smith & Jones", &["smith & jones"]),
        ("Mum's 60th", &["mum's 60th"]),
        ("Cape Point", &["cape point"]),
        ("St. Ives", &["st ives"]),
        // " - " divides two things; a hyphen inside a word does not.
        ("Karoo - Tankwa", &["karoo", "tankwa"]),
        ("2023 - Scotland - Skye", &["scotland", "skye"]),
        ("Long-tailed tits", &["long-tailed tits"]),
        // A workflow's stages and the storage itself.
        ("RAW", &[]),
        ("Exports", &[]),
        ("_Selects", &[]),
        ("Edited", &[]),
        ("JPG", &[]),
        ("New folder", &[]),
        ("Untitled", &[]),
        ("Lightroom", &[]),
        ("Pictures", &[]),
        ("OneDrive", &[]),
        // Too little to be a word.
        ("A", &[]),
        ("#1", &[]),
        ("x-2", &[]),
        // A pipe would split into two levels in a sidecar's hierarchy.
        ("A|B", &[]),
    ];

    #[test]
    fn each_folder_name_gives_what_it_says_about_the_photos() {
        let mut wrong = Vec::new();
        for (name, want) in CASES {
            let got = from_name(name);
            if got != want.iter().map(|s| s.to_string()).collect::<Vec<_>>() {
                wrong.push(format!("{name:?}: got {got:?}, want {want:?}"));
            }
        }
        assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
    }

    #[test]
    fn a_path_gives_each_keyword_once_outermost_first() {
        assert_eq!(
            keywords(&["Photos", "2024", "2024-05 Karoo trip", "RAW"]),
            vec!["karoo trip"]
        );
        assert_eq!(
            keywords(&["Weddings", "Smith wedding", "Smith wedding"]),
            vec!["weddings", "smith wedding"]
        );
        assert!(keywords(&[]).is_empty());
    }
}
