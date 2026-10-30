//! Magic erase's records in an edit: what was erased and which fills replaced
//! it, laid in in order. The model, the fills themselves and laying them in
//! are the desktop's (`develop::erase` there, behind `--features ai`); a
//! stack holds only these, so it stays small and travels anywhere.

use serde::{Deserialize, Serialize};

/// One object erased: what it was, and the fills that replaced it, laid in
/// in order.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct Erase {
    /// What was erased, for the list of erases.
    pub label: String,
    pub pieces: Vec<Piece>,
}

/// One window the model filled.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename = "ErasePiece", rename_all = "camelCase", default)]
pub struct Piece {
    /// The fill's name: its file's SHA-256, in hex (the desktop's `FillStore`).
    pub fill: String,
    /// The window's top-left corner and side, in pixels of the stored image
    /// (sensor order, before the camera's orientation). It can reach past
    /// the image's edge on its short side.
    pub x: i32,
    pub y: i32,
    pub size: u32,
}

impl Piece {
    pub fn rect(&self) -> Rect {
        Rect {
            x: self.x,
            y: self.y,
            w: self.size,
            h: self.size,
        }
    }

    /// The seed its grain is drawn from.
    pub fn seed(&self) -> u64 {
        u64::from_str_radix(self.fill.get(..16).unwrap_or("0"), 16).unwrap_or(0)
    }
}

/// Every fill an edit's erases use, each once.
pub fn fill_names(erases: &[Erase]) -> Vec<&str> {
    let mut names: Vec<&str> = erases
        .iter()
        .flat_map(|e| e.pieces.iter().map(|p| p.fill.as_str()))
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// A rectangle of pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub fn right(&self) -> i32 {
        self.x + self.w as i32
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h as i32
    }

    pub fn from_edges(x0: i32, y0: i32, x1: i32, y1: i32) -> Option<Rect> {
        (x1 > x0 && y1 > y0).then(|| Rect {
            x: x0,
            y: y0,
            w: (x1 - x0) as u32,
            h: (y1 - y0) as u32,
        })
    }

    pub fn intersect(&self, o: &Rect) -> Option<Rect> {
        Self::from_edges(
            self.x.max(o.x),
            self.y.max(o.y),
            self.right().min(o.right()),
            self.bottom().min(o.bottom()),
        )
    }

    pub fn union(&self, o: &Rect) -> Rect {
        Rect {
            x: self.x.min(o.x),
            y: self.y.min(o.y),
            w: (self.right().max(o.right()) - self.x.min(o.x)) as u32,
            h: (self.bottom().max(o.bottom()) - self.y.min(o.y)) as u32,
        }
    }

    /// The part of this inside an image of `(w, h)`.
    pub fn within(&self, (w, h): (u32, u32)) -> Option<Rect> {
        self.intersect(&Rect { x: 0, y: 0, w, h })
    }

    /// As the GPU's region calls take it. Only for a rectangle inside the
    /// image ([`Self::within`]).
    pub fn as_region(&self) -> (u32, u32, u32, u32) {
        (self.x.max(0) as u32, self.y.max(0) as u32, self.w, self.h)
    }
}

/// The rectangles of the image to lay fills into again, after the erases
/// have changed from `before` to `after`, within an image of `size`: every
/// window that differs, grown to take in every window of `after` it
/// touches. Laying in a fill reads its whole window (its grain is measured
/// there), so a fill is only ever laid in whole, onto pixels that hold
/// exactly the base and the fills before it.
pub fn dirty_regions(before: &[Erase], after: &[Erase], size: (u32, u32)) -> Vec<Rect> {
    let pieces =
        |e: &[Erase]| -> Vec<Piece> { e.iter().flat_map(|e| e.pieces.iter().cloned()).collect() };
    let (old, new) = (pieces(before), pieces(after));
    let same = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let mut regions: Vec<Rect> = old[same..]
        .iter()
        .chain(&new[same..])
        .filter_map(|p| p.rect().within(size))
        .collect();
    let windows: Vec<Rect> = new.iter().filter_map(|p| p.rect().within(size)).collect();
    loop {
        let mut grown = false;
        for r in regions.iter_mut() {
            for w in &windows {
                if r.intersect(w).is_some() && r.union(w) != *r {
                    *r = r.union(w);
                    grown = true;
                }
            }
        }
        // Merge regions that now overlap.
        let mut i = 0;
        while i < regions.len() {
            let mut j = i + 1;
            while j < regions.len() {
                if regions[i].intersect(&regions[j]).is_some() {
                    let other = regions.remove(j);
                    regions[i] = regions[i].union(&other);
                    grown = true;
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
        if !grown {
            return regions;
        }
    }
}

// ------------------------------------------------------------------------
// Fills on disk.
// ------------------------------------------------------------------------
