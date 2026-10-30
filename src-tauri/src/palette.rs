//! A photo's colours: which named colours cover it, for the Palette
//! filter, and a signature of all its colours, for finding photos with similar
//! ones whatever they show.
//!
//! Both are read from a small copy of the photo in Oklab, where lightness,
//! chroma and hue are independent and hue angles fall where named colours do
//! (see `develop::oklab`). This module takes sRGB — thumbnails and small
//! renders — so it carries the sRGB form of the transform.
//!
//! The signature is a histogram: 12 hues × 4 lightnesses × 2 chromas, and 5
//! lightnesses of grey. It is compared after a slight blur around the hue
//! circle, so an orange a few degrees redder still counts as orange, by how
//! much the two overlap (the Bhattacharyya coefficient: 1 for the same colours
//! in the same proportions, 0 for none in common).

/// The named colours, in the order the rail shows them. Their bit in a
/// photo's mask is their index.
pub const SWATCHES: [&str; 12] = [
    "red", "orange", "yellow", "green", "teal", "blue", "purple", "pink", "brown", "white", "grey",
    "black",
];

/// A photo is that colour when the colour covers at least this share of it.
pub const SWATCH_SHARE: f32 = 0.12;

const HUES: usize = 12;
const LIGHTS: usize = 4;
const CHROMAS: usize = 2;
const GREYS: usize = 5;
/// Bins in a signature.
pub const BINS: usize = HUES * LIGHTS * CHROMAS + GREYS;

/// Below this chroma a colour is a grey.
const NEUTRAL: f32 = 0.035;
/// The chroma between the muted and the vivid bins.
const VIVID: f32 = 0.10;

fn to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// OkLCh — lightness 0..1, chroma, hue in degrees — of an sRGB colour.
pub fn lch(rgb: [u8; 3]) -> [f32; 3] {
    let [r, g, b] = rgb.map(to_linear);
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    let big_l = 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s;
    let a = 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s;
    let bb = 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s;
    [
        big_l,
        a.hypot(bb),
        bb.atan2(a).to_degrees().rem_euclid(360.0),
    ]
}

/// The named colour a pixel is, by index into [`SWATCHES`].
pub fn swatch_of(rgb: [u8; 3]) -> usize {
    let [l, c, h] = lch(rgb);
    let named = |n: &str| SWATCHES.iter().position(|s| *s == n).unwrap_or(0);
    if c < NEUTRAL {
        return named(if l < 0.25 {
            "black"
        } else if l > 0.88 {
            "white"
        } else {
            "grey"
        });
    }
    let warm = (15.0..100.0).contains(&h);
    if warm && ((l < 0.55 && c < 0.15) || c < 0.07) {
        return named("brown");
    }
    named(match h {
        h if !(15.0..345.0).contains(&h) => "pink",
        h if h < 45.0 => {
            if l > 0.78 {
                "pink"
            } else {
                "red"
            }
        }
        h if h < 85.0 => "orange",
        h if h < 125.0 => "yellow",
        h if h < 175.0 => "green",
        h if h < 230.0 => "teal",
        h if h < 290.0 => "blue",
        _ => "purple",
    })
}

fn bin_of(rgb: [u8; 3]) -> usize {
    let [l, c, h] = lch(rgb);
    let light = |n: usize| ((l.clamp(0.0, 0.999)) * n as f32) as usize;
    if c < NEUTRAL {
        return HUES * LIGHTS * CHROMAS + light(GREYS);
    }
    let hue = ((h / 360.0 * HUES as f32) as usize).min(HUES - 1);
    let chroma = usize::from(c >= VIVID);
    (hue * LIGHTS + light(LIGHTS)) * CHROMAS + chroma
}

/// What a photo's colours come to: which named colours cover it (a bit per
/// [`SWATCHES`] entry), and its signature, each bin a share of 65,535.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    pub swatches: i64,
    pub signature: Vec<u8>,
}

/// Reads a palette from an image's pixels, sRGB, row by row.
pub fn read(pixels: impl Iterator<Item = [u8; 3]>) -> Palette {
    let mut bins = [0u32; BINS];
    let mut named = [0u32; SWATCHES.len()];
    let mut n = 0u32;
    for px in pixels {
        bins[bin_of(px)] += 1;
        named[swatch_of(px)] += 1;
        n += 1;
    }
    let n = n.max(1) as f32;
    let swatches = named
        .iter()
        .enumerate()
        .filter(|(_, &k)| k as f32 / n >= SWATCH_SHARE)
        .fold(0i64, |m, (i, _)| m | (1 << i));
    let signature = bins
        .iter()
        .flat_map(|&k| (((k as f32 / n) * 65_535.0).round() as u16).to_le_bytes())
        .collect();
    Palette {
        swatches,
        signature,
    }
}

/// Reads an encoded image (a thumbnail, a small render), shrunk first: a few
/// thousand pixels say as much about its colours as millions.
pub fn read_image(bytes: &[u8]) -> anyhow::Result<Palette> {
    let img = image::load_from_memory(bytes)?.thumbnail(96, 96).to_rgb8();
    Ok(read(img.pixels().map(|p| p.0)))
}

/// A signature as shares, each chromatic row blurred a little around the hue
/// circle so neighbouring hues overlap.
fn shares(signature: &[u8]) -> [f32; BINS] {
    let mut raw = [0f32; BINS];
    for (i, pair) in signature.chunks_exact(2).take(BINS).enumerate() {
        raw[i] = u16::from_le_bytes([pair[0], pair[1]]) as f32 / 65_535.0;
    }
    let mut out = raw;
    for light in 0..LIGHTS {
        for chroma in 0..CHROMAS {
            let at = |hue: usize| (hue * LIGHTS + light) * CHROMAS + chroma;
            for hue in 0..HUES {
                let prev = raw[at((hue + HUES - 1) % HUES)];
                let next = raw[at((hue + 1) % HUES)];
                out[at(hue)] = 0.5 * raw[at(hue)] + 0.25 * (prev + next);
            }
        }
    }
    out
}

/// How alike two signatures' colours are, 0 to 1.
pub fn likeness(a: &[u8], b: &[u8]) -> f32 {
    let (a, b) = (shares(a), shares(b));
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x * y).sqrt())
        .sum::<f32>()
        .min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(rgb: [u8; 3]) -> &'static str {
        SWATCHES[swatch_of(rgb)]
    }

    #[test]
    fn everyday_colours_get_their_names() {
        let cases = [
            ([220, 30, 30], "red"),
            ([255, 140, 0], "orange"),
            ([250, 220, 40], "yellow"),
            ([40, 150, 50], "green"),
            ([0, 140, 140], "teal"),
            ([30, 60, 220], "blue"),
            ([130, 40, 160], "purple"),
            ([255, 150, 190], "pink"),
            ([240, 20, 150], "pink"),
            ([120, 70, 30], "brown"),
            ([210, 185, 150], "brown"),
            ([250, 250, 250], "white"),
            ([128, 128, 128], "grey"),
            ([12, 12, 14], "black"),
        ];
        for (rgb, want) in cases {
            assert_eq!(named(rgb), want, "{rgb:?}");
        }
    }

    fn solid(parts: &[([u8; 3], usize)]) -> Palette {
        read(parts.iter().flat_map(|&(c, n)| std::iter::repeat_n(c, n)))
    }

    fn bit(name: &str) -> i64 {
        1 << SWATCHES.iter().position(|s| *s == name).unwrap()
    }

    #[test]
    fn a_colour_counts_once_it_covers_enough_of_the_frame() {
        let p = solid(&[
            ([30, 60, 220], 70),
            ([250, 220, 40], 20),
            ([220, 30, 30], 10),
        ]);
        assert_eq!(
            p.swatches,
            bit("blue") | bit("yellow"),
            "red is only a tenth"
        );
        assert_eq!(p.signature.len(), BINS * 2);
    }

    #[test]
    fn sunsets_are_alike_whatever_they_show_and_unlike_a_forest() {
        let sunset = solid(&[
            ([250, 120, 30], 50),
            ([110, 50, 140], 40),
            ([20, 15, 30], 10),
        ]);
        // Another sunset: a touch redder and darker, in other proportions.
        let other = solid(&[
            ([240, 100, 40], 35),
            ([100, 45, 130], 55),
            ([25, 20, 35], 10),
        ]);
        let forest = solid(&[([40, 110, 40], 70), ([90, 70, 40], 30)]);
        let same = likeness(&sunset.signature, &sunset.signature);
        assert!((same - 1.0).abs() < 1e-3, "{same}");
        let near = likeness(&sunset.signature, &other.signature);
        let far = likeness(&sunset.signature, &forest.signature);
        assert!(near > 0.7, "near {near}");
        assert!(far < 0.3, "far {far}");
    }
}
