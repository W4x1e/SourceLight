//! A minimal sRGB ICC profile, built rather than bundled.
//!
//! Exports have always *been* sRGB; they were simply untagged. Untagged sRGB is
//! the near-universal default so files looked right in practice, but anything
//! colour-managed — a print shop, a browser on a wide-gamut display, Photoshop
//! with a non-sRGB working space — has to guess, and guessing wrong shifts
//! every colour.
//!
//! **Constructed here instead of shipping a profile file.** The contents are
//! the sRGB standard's own numbers, so there is nothing to licence, nothing to
//! attribute, and no binary blob in the repository whose provenance someone has
//! to check before the app can be released. It is also about two kilobytes.
//!
//! Written as ICC v2 rather than v4: v2 is understood everywhere, and the one
//! v4 feature that would help here — `parametricCurveType`, which states the
//! sRGB transfer function in seven numbers — is not worth the compatibility
//! risk when a sampled curve says the same thing.

/// The sRGB transfer function's inverse: an encoded value to linear light.
///
/// This direction because an ICC tone curve (`rTRC` and friends) maps the
/// file's device values *to* the linear connection space. It first sampled the
/// encoding direction instead, so every colour-managed reader (browsers,
/// photo editors, and the app's own full-resolution zoom) brightened
/// each pixel by the sRGB curve a second time, and tagged exports looked
/// milky and washed out.
///
/// The piecewise form, not the 2.2 approximation. The linear segment near black
/// is the whole reason the piecewise version exists, and a profile claiming
/// pure gamma would misreport every shadow.
fn srgb_decode(encoded: f64) -> f64 {
    if encoded <= 0.04045 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    }
}

/// ICC's `s15Fixed16Number`: signed, 16 fractional bits.
fn s15_fixed16(v: f64) -> [u8; 4] {
    ((v * 65536.0).round() as i32).to_be_bytes()
}

fn tag_xyz(x: f64, y: f64, z: f64) -> Vec<u8> {
    let mut out = b"XYZ \0\0\0\0".to_vec();
    out.extend_from_slice(&s15_fixed16(x));
    out.extend_from_slice(&s15_fixed16(y));
    out.extend_from_slice(&s15_fixed16(z));
    out
}

/// A sampled tone curve, device value in, linear out. One table serves all
/// three channels, which the tag table is free to point at three times.
fn tag_curve(points: usize) -> Vec<u8> {
    let mut out = b"curv\0\0\0\0".to_vec();
    out.extend_from_slice(&(points as u32).to_be_bytes());
    for i in 0..points {
        let encoded = i as f64 / (points - 1) as f64;
        let linear = (srgb_decode(encoded) * 65535.0).round().clamp(0.0, 65535.0);
        out.extend_from_slice(&(linear as u16).to_be_bytes());
    }
    out
}

/// v2's `textDescriptionType`, which carries three encodings of the same string
/// and is fussy about all of them being present even when empty.
fn tag_description(text: &str) -> Vec<u8> {
    let ascii = format!("{text}\0");
    let mut out = b"desc\0\0\0\0".to_vec();
    out.extend_from_slice(&(ascii.len() as u32).to_be_bytes());
    out.extend_from_slice(ascii.as_bytes());
    out.extend_from_slice(&[0; 8]); // Unicode language code and count.
    out.extend_from_slice(&[0; 2]); // ScriptCode code.
    out.push(0); // ScriptCode count.
    out.extend_from_slice(&[0; 67]); // ScriptCode description, fixed length.
    out
}

fn tag_text(text: &str) -> Vec<u8> {
    let mut out = b"text\0\0\0\0".to_vec();
    out.extend_from_slice(text.as_bytes());
    out.push(0);
    out
}

/// Linear sRGB to XYZ, D65 — the IEC 61966-2-1 matrix. Only the tests need
/// it: they check [`d50_primaries`] against sRGB's published D50 values.
#[cfg(test)]
const SRGB_TO_XYZ_D65: [[f64; 3]; 3] = [
    [0.4124564, 0.3575761, 0.1804375],
    [0.2126729, 0.7151522, 0.0721750],
    [0.0193339, 0.1191920, 0.9503041],
];

/// Linear Adobe RGB (1998) to XYZ, D65.
const ADOBE_RGB_TO_XYZ_D65: [[f64; 3]; 3] = [
    [0.5767309, 0.1855540, 0.1881852],
    [0.2973769, 0.6273491, 0.0752741],
    [0.0270343, 0.0706872, 0.9911085],
];

/// Linear Display P3 to XYZ, D65: DCI-P3 primaries on a D65 white.
const DISPLAY_P3_TO_XYZ_D65: [[f64; 3]; 3] = [
    [0.4865709, 0.2656677, 0.1982173],
    [0.2289746, 0.6917385, 0.0792869],
    [0.0000000, 0.0451134, 1.0439444],
];

/// A space's red, green and blue primaries as D50 XYZ — what an ICC matrix
/// profile's `rXYZ`/`gXYZ`/`bXYZ` tags hold. The connection space is always
/// D50, so the D65 columns are **Bradford-adapted**; writing D65 numbers
/// directly is the classic mistake and warms everything slightly.
fn d50_primaries(rgb_to_xyz_d65: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    const BRADFORD: [[f64; 3]; 3] = [
        [0.8951, 0.2664, -0.1614],
        [-0.7502, 1.7135, 0.0367],
        [0.0389, -0.0685, 1.0296],
    ];
    const BRADFORD_INV: [[f64; 3]; 3] = [
        [0.9869929, -0.1470543, 0.1599627],
        [0.4323053, 0.5183603, 0.0492912],
        [-0.0085287, 0.0400428, 0.9684867],
    ];
    const D65: [f64; 3] = [0.95047, 1.0, 1.08883];
    const D50: [f64; 3] = [0.96422, 1.0, 0.82521];
    let mul = |m: &[[f64; 3]; 3], v: [f64; 3]| -> [f64; 3] {
        std::array::from_fn(|r| (0..3).map(|k| m[r][k] * v[k]).sum())
    };
    let (src, dst) = (mul(&BRADFORD, D65), mul(&BRADFORD, D50));
    let adapt = |xyz: [f64; 3]| {
        let cone = mul(&BRADFORD, xyz);
        mul(
            &BRADFORD_INV,
            std::array::from_fn(|i| cone[i] * dst[i] / src[i]),
        )
    };
    // Columns of the matrix are the primaries.
    std::array::from_fn(|c| adapt(std::array::from_fn(|r| rgb_to_xyz_d65[r][c])))
}

/// The sRGB profile every export carried before the colour space could be chosen, and
/// still the default.
///
/// Its primaries are the published D50 values rather than [`d50_primaries`]'
/// output, so the bytes of every sRGB export stay exactly what they were; a
/// test holds the two within rounding of each other.
pub fn srgb_profile() -> Vec<u8> {
    rgb_profile(
        "sRGB IEC61966-2.1",
        [
            [0.43607, 0.22249, 0.01392],
            [0.38515, 0.71687, 0.09708],
            [0.14307, 0.06061, 0.71410],
        ],
        tag_curve(1024),
    )
}

/// Display P3, for a wide-gamut export. Same transfer curve as sRGB —
/// P3 uses it — so only the primaries differ.
pub fn display_p3_profile() -> Vec<u8> {
    rgb_profile(
        "Display P3",
        d50_primaries(DISPLAY_P3_TO_XYZ_D65),
        tag_curve(1024),
    )
}

/// Adobe RGB (1998). Its curve is a pure gamma, which ICC states exactly as a
/// one-entry `curv` rather than a sampled table.
pub fn adobe_rgb_profile() -> Vec<u8> {
    rgb_profile(
        "Adobe RGB (1998)",
        d50_primaries(ADOBE_RGB_TO_XYZ_D65),
        tag_gamma(563.0 / 256.0),
    )
}

/// A `curv` holding a single gamma, as `u8Fixed8Number` — 563/256 is exactly
/// representable, which is why the standard picked it.
fn tag_gamma(gamma: f64) -> Vec<u8> {
    let mut out = b"curv\0\0\0\0".to_vec();
    out.extend_from_slice(&1u32.to_be_bytes());
    out.extend_from_slice(&((gamma * 256.0).round() as u16).to_be_bytes());
    out
}

/// Builds a matrix/TRC display profile from D50 primaries (red, green, blue)
/// and one tone curve shared by all three channels.
fn rgb_profile(description: &str, primaries: [[f64; 3]; 3], curve: Vec<u8>) -> Vec<u8> {
    let [r, g, b] = primaries;
    // Each distinct block of tag data, stored once.
    let bodies = [
        tag_description(description),
        tag_xyz(0.96420, 1.0, 0.82491),
        tag_xyz(r[0], r[1], r[2]),
        tag_xyz(g[0], g[1], g[2]),
        tag_xyz(b[0], b[1], b[2]),
        curve,
        tag_text("Public Domain"),
    ];

    // Signature and which body it points at. The three tone curves name the
    // same one on purpose — the format allows it, and three copies of a 2KB
    // table would be most of the profile.
    const CURVE: usize = 5;
    let tags: [(&[u8; 4], usize); 9] = [
        (b"desc", 0),
        (b"wtpt", 1),
        (b"rXYZ", 2),
        (b"gXYZ", 3),
        (b"bXYZ", 4),
        (b"rTRC", CURVE),
        (b"gTRC", CURVE),
        (b"bTRC", CURVE),
        (b"cprt", 6),
    ];

    // Lay the bodies out first so the table can point at them. Every tag has to
    // start on a four-byte boundary.
    let mut offsets = [0usize; 7];
    let mut cursor = 128 + 4 + tags.len() * 12;
    for (offset, body) in offsets.iter_mut().zip(&bodies) {
        *offset = cursor;
        cursor += body.len().next_multiple_of(4);
    }

    let mut out = vec![0u8; 128];
    out.extend_from_slice(&(tags.len() as u32).to_be_bytes());
    for (sig, body) in tags {
        out.extend_from_slice(sig);
        out.extend_from_slice(&(offsets[body] as u32).to_be_bytes());
        out.extend_from_slice(&(bodies[body].len() as u32).to_be_bytes());
    }
    for body in &bodies {
        out.extend_from_slice(body);
        while !out.len().is_multiple_of(4) {
            out.push(0);
        }
    }

    // Header, now that the total size is known.
    let size = (out.len() as u32).to_be_bytes();
    out[0..4].copy_from_slice(&size);
    out[8..12].copy_from_slice(&0x0210_0000u32.to_be_bytes()); // v2.1
    out[12..16].copy_from_slice(b"mntr"); // display device
    out[16..20].copy_from_slice(b"RGB ");
    out[20..24].copy_from_slice(b"XYZ "); // profile connection space
    out[36..40].copy_from_slice(b"acsp"); // the file signature proper
    out[64..68].copy_from_slice(&0u32.to_be_bytes()); // perceptual intent
                                                      // PCS illuminant. Fixed at D50 by the specification, not a choice.
    out[68..72].copy_from_slice(&s15_fixed16(0.96420));
    out[72..76].copy_from_slice(&s15_fixed16(1.0));
    out[76..80].copy_from_slice(&s15_fixed16(0.82491));

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn be32(bytes: &[u8], at: usize) -> u32 {
        u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap())
    }

    /// The derivation the P3 profile relies on, checked against the one set
    /// of D50 primaries everyone agrees on: sRGB's.
    #[test]
    fn bradford_reproduces_the_published_srgb_primaries() {
        let got = d50_primaries(SRGB_TO_XYZ_D65);
        let published = [
            [0.43607, 0.22249, 0.01392],
            [0.38515, 0.71687, 0.09708],
            [0.14307, 0.06061, 0.71410],
        ];
        for (g, p) in got.iter().zip(published) {
            for i in 0..3 {
                assert!((g[i] - p[i]).abs() < 3e-4, "{got:?} vs {published:?}");
            }
        }
    }

    /// Adobe RGB against its own published D50 primaries, and its curve as the
    /// exact gamma the standard specifies.
    #[test]
    fn adobe_rgb_matches_its_published_primaries_and_gamma() {
        let got = d50_primaries(ADOBE_RGB_TO_XYZ_D65);
        let published = [
            [0.6097, 0.3111, 0.0195],
            [0.2053, 0.6257, 0.0609],
            [0.1492, 0.0632, 0.7446],
        ];
        for (g, p) in got.iter().zip(published) {
            for i in 0..3 {
                assert!((g[i] - p[i]).abs() < 1e-3, "{got:?} vs {published:?}");
            }
        }
        // 563/256 is 0x0233 as u8Fixed8.
        assert_eq!(tag_gamma(563.0 / 256.0)[12..14], [0x02, 0x33]);
        let p = adobe_rgb_profile();
        assert_eq!(
            u32::from_be_bytes(p[0..4].try_into().unwrap()) as usize,
            p.len()
        );
    }

    /// A matrix profile's primaries must sum to its white point, the D50 PCS
    /// illuminant — otherwise neutral greys come out tinted.
    #[test]
    fn display_p3_primaries_sum_to_d50_white() {
        let [r, g, b] = d50_primaries(DISPLAY_P3_TO_XYZ_D65);
        let white = [0.96422, 1.0, 0.82521];
        for i in 0..3 {
            assert!((r[i] + g[i] + b[i] - white[i]).abs() < 1e-3, "{i}");
        }
        // And P3 is not sRGB: its red sits further out.
        let [srgb_r, _, _] = d50_primaries(SRGB_TO_XYZ_D65);
        assert!(r[0] > srgb_r[0] + 0.05);
        let p = display_p3_profile();
        assert_eq!(
            u32::from_be_bytes(p[0..4].try_into().unwrap()) as usize,
            p.len()
        );
        assert!(String::from_utf8_lossy(&p).contains("Display P3"));
    }

    /// The header fields a reader checks before it will look at anything else.
    /// A profile that fails these is silently ignored, which looks exactly like
    /// not embedding one at all.
    #[test]
    fn the_header_identifies_a_display_rgb_profile() {
        let p = srgb_profile();
        assert_eq!(be32(&p, 0) as usize, p.len(), "size field must match");
        assert_eq!(&p[36..40], b"acsp", "missing the profile signature");
        assert_eq!(&p[12..16], b"mntr");
        assert_eq!(&p[16..20], b"RGB ");
        assert_eq!(&p[20..24], b"XYZ ");
        assert_eq!(be32(&p, 8), 0x0210_0000, "should declare ICC v2.1");
    }

    /// Every tag has to sit inside the file and start four-byte aligned. An
    /// offset past the end is the difference between a profile that loads and
    /// one that crashes a colour engine.
    #[test]
    fn every_tag_is_in_bounds_and_aligned() {
        let p = srgb_profile();
        let count = be32(&p, 128) as usize;
        assert_eq!(count, 9, "expected the nine required tags");

        let mut seen = Vec::new();
        for i in 0..count {
            let entry = 132 + i * 12;
            let sig = &p[entry..entry + 4];
            let offset = be32(&p, entry + 4) as usize;
            let len = be32(&p, entry + 8) as usize;

            assert_eq!(offset % 4, 0, "{sig:?} is not aligned");
            assert!(
                offset + len <= p.len(),
                "{sig:?} runs past the end: {offset}+{len} > {}",
                p.len()
            );
            seen.push(String::from_utf8_lossy(sig).into_owned());
        }

        for required in [
            "desc", "wtpt", "rXYZ", "gXYZ", "bXYZ", "rTRC", "gTRC", "bTRC", "cprt",
        ] {
            assert!(seen.contains(&required.to_string()), "missing {required}");
        }
    }

    /// The three channel curves must point at one table, or the profile carries
    /// three identical copies and triples in size for nothing.
    #[test]
    fn the_three_tone_curves_share_one_table() {
        let p = srgb_profile();
        let offset_of = |want: &str| {
            (0..9)
                .map(|i| 132 + i * 12)
                .find(|&e| &p[e..e + 4] == want.as_bytes())
                .map(|e| be32(&p, e + 4))
                .unwrap()
        };
        let r = offset_of("rTRC");
        assert_eq!(r, offset_of("gTRC"));
        assert_eq!(r, offset_of("bTRC"));
    }

    /// The curve has to be the sRGB transfer function, in the direction ICC
    /// reads it (encoded to linear): anchored at both ends, monotonic, and
    /// passing through the known midpoint. A profile describing the wrong
    /// curve is worse than none, because readers trust it. The encoding
    /// direction passed the first version of this test and washed out every
    /// tagged export.
    #[test]
    fn the_curve_is_the_srgb_transfer_function() {
        let p = srgb_profile();
        let entry = (0..9)
            .map(|i| 132 + i * 12)
            .find(|&e| &p[e..e + 4] == b"rTRC")
            .unwrap();
        let at = be32(&p, entry + 4) as usize;
        let count = be32(&p, at + 8) as usize;
        assert_eq!(count, 1024);

        let sample = |i: usize| {
            let o = at + 12 + i * 2;
            u16::from_be_bytes(p[o..o + 2].try_into().unwrap())
        };

        assert_eq!(sample(0), 0, "black must stay black");
        assert_eq!(sample(count - 1), 65535, "white must stay white");

        let mut prev = 0;
        for i in 0..count {
            let v = sample(i);
            assert!(v >= prev, "curve dips at {i}");
            prev = v;
        }

        // An encoded 0.5 is 0.2140 in linear light under sRGB: the value that
        // separates the real curve from a 2.2 gamma approximation (0.2176),
        // and from the encoding direction (0.7354) by a mile.
        let x = (count / 2) as f64 / (count - 1) as f64;
        let mid = sample(count / 2) as f64 / 65535.0;
        let expected = ((x + 0.055) / 1.055).powf(2.4);
        assert!(
            (mid - expected).abs() < 0.0005 && (mid - 0.2140).abs() < 0.0015,
            "midpoint is {mid:.4}, expected the piecewise sRGB decode {expected:.4}"
        );
        // Near black, the linear segment: device 10/1023 is 0.000756 linear.
        let dark = sample(10) as f64 / 65535.0;
        assert!((dark - (10.0 / 1023.0) / 12.92).abs() < 0.00005, "{dark}");
    }

    /// The primaries have to be the D50-adapted ones. Using the D65 values
    /// directly is the classic error and warms every exported file.
    #[test]
    fn the_white_point_is_d50() {
        let p = srgb_profile();
        let entry = (0..9)
            .map(|i| 132 + i * 12)
            .find(|&e| &p[e..e + 4] == b"wtpt")
            .unwrap();
        let at = be32(&p, entry + 4) as usize;
        let read = |i: usize| {
            i32::from_be_bytes(p[at + 8 + i * 4..at + 12 + i * 4].try_into().unwrap()) as f64
                / 65536.0
        };
        assert!((read(0) - 0.9642).abs() < 0.001, "X was {}", read(0));
        assert!((read(1) - 1.0).abs() < 0.001, "Y was {}", read(1));
        assert!((read(2) - 0.8249).abs() < 0.001, "Z was {}", read(2));
    }
}
