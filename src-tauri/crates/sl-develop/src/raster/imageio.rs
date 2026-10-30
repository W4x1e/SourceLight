//! HEIC and HEIF through macOS's ImageIO.
//!
//! The Mac counterpart of `wic`. Every Mac the app supports (macOS 14 and
//! later) reads HEIF out of the box, so unlike Windows there is nothing to
//! install and no "missing codec" case: a file ImageIO cannot open is damaged.
//!
//! **Orientation.** As on Windows, the pixels come back upright and the EXIF
//! tag is ignored: ImageIO reports the container's rotation (`irot`) as the
//! image's orientation, and the thumbnail call below applies it as it decodes.
//! `a_rotated_heic_comes_back_upright` pins it with a file written the way a
//! phone writes a portrait shot.
//!
//! **Colour.** The pixels come back in the photo's own colour space, with its
//! ICC profile, as `wic` returns them: an iPhone's Display P3 stays P3, and
//! `raster` converts it for the grid and keeps its primaries for Develop. Only
//! an image that is not RGB (a greyscale HEIF) is drawn into sRGB instead.
//!
//! Plain C calls into CoreFoundation, CoreGraphics and ImageIO, as
//! `hardware::metered` calls Network.framework: no bindings crate for four
//! frameworks' worth of API when a dozen functions are all this needs.

use anyhow::{anyhow, bail, Result};
use image::ImageBuffer;
use std::ffi::c_void;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

type CFTypeRef = *const c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFBooleanTrue: CFTypeRef;
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;
    fn CFRelease(cf: CFTypeRef);
    fn CFURLCreateFromFileSystemRepresentation(
        allocator: CFTypeRef,
        buffer: *const u8,
        len: isize,
        is_directory: u8,
    ) -> CFTypeRef;
    fn CFDictionaryCreate(
        allocator: CFTypeRef,
        keys: *const CFTypeRef,
        values: *const CFTypeRef,
        count: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> CFTypeRef;
    fn CFDictionaryGetValue(dict: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    fn CFNumberCreate(allocator: CFTypeRef, kind: isize, value: *const c_void) -> CFTypeRef;
    fn CFNumberGetValue(number: CFTypeRef, kind: isize, value: *mut c_void) -> u8;
    fn CFDataGetLength(data: CFTypeRef) -> isize;
    fn CFDataGetBytePtr(data: CFTypeRef) -> *const u8;
}

/// `kCFNumberSInt64Type`.
const SINT64: isize = 4;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    static kCGColorSpaceSRGB: CFTypeRef;
    fn CGColorSpaceCreateWithName(name: CFTypeRef) -> CFTypeRef;
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: CFTypeRef,
        bitmap_info: u32,
    ) -> CFTypeRef;
    fn CGContextDrawImage(context: CFTypeRef, rect: CGRect, image: CFTypeRef);
    fn CGImageGetWidth(image: CFTypeRef) -> usize;
    fn CGImageGetHeight(image: CFTypeRef) -> usize;
    fn CGImageGetColorSpace(image: CFTypeRef) -> CFTypeRef;
    fn CGColorSpaceGetModel(space: CFTypeRef) -> i32;
    fn CGColorSpaceCopyICCData(space: CFTypeRef) -> CFTypeRef;
    #[cfg(any(test, feature = "test-helpers"))]
    fn CGBitmapContextCreateImage(context: CFTypeRef) -> CFTypeRef;
}

/// `kCGImageAlphaNoneSkipLast`: RGBX, one byte each.
const RGBX: u32 = 5;
/// `kCGColorSpaceModelRGB`.
const MODEL_RGB: i32 = 1;

#[link(name = "ImageIO", kind = "framework")]
extern "C" {
    static kCGImagePropertyPixelWidth: CFTypeRef;
    static kCGImagePropertyPixelHeight: CFTypeRef;
    static kCGImagePropertyOrientation: CFTypeRef;
    static kCGImageSourceCreateThumbnailFromImageAlways: CFTypeRef;
    static kCGImageSourceCreateThumbnailWithTransform: CFTypeRef;
    static kCGImageSourceThumbnailMaxPixelSize: CFTypeRef;
    fn CGImageSourceCreateWithURL(url: CFTypeRef, options: CFTypeRef) -> CFTypeRef;
    fn CGImageSourceGetCount(source: CFTypeRef) -> usize;
    fn CGImageSourceCopyPropertiesAtIndex(
        source: CFTypeRef,
        index: usize,
        options: CFTypeRef,
    ) -> CFTypeRef;
    fn CGImageSourceCreateThumbnailAtIndex(
        source: CFTypeRef,
        index: usize,
        options: CFTypeRef,
    ) -> CFTypeRef;
}

/// A CoreFoundation object this code created, released when dropped. Null
/// (a failed create) is held too, and checked by the caller.
struct Owned(CFTypeRef);

impl Owned {
    fn some(self, what: &str, path: &Path) -> Result<Self> {
        if self.0.is_null() {
            bail!("macOS cannot read {} ({what})", path.display());
        }
        Ok(self)
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: a +1 reference from a Create/Copy call, released once.
            unsafe { CFRelease(self.0) };
        }
    }
}

fn file_url(path: &Path) -> Owned {
    let bytes = path.as_os_str().as_bytes();
    // SAFETY: the buffer and its length describe one live byte slice.
    Owned(unsafe {
        CFURLCreateFromFileSystemRepresentation(
            std::ptr::null(),
            bytes.as_ptr(),
            bytes.len() as isize,
            0,
        )
    })
}

fn number(value: i64) -> Owned {
    // SAFETY: reads one i64 through the pointer, as `SINT64` says.
    Owned(unsafe { CFNumberCreate(std::ptr::null(), SINT64, (&value as *const i64).cast()) })
}

/// A CF dictionary of `pairs`; keys and values are retained by it.
fn dictionary(pairs: &[(CFTypeRef, CFTypeRef)]) -> Owned {
    let keys: Vec<CFTypeRef> = pairs.iter().map(|p| p.0).collect();
    let values: Vec<CFTypeRef> = pairs.iter().map(|p| p.1).collect();
    // SAFETY: two arrays of `pairs.len()` CF objects, and the standard
    // CFType callbacks, which retain what the dictionary holds.
    Owned(unsafe {
        CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            pairs.len() as isize,
            &kCFTypeDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks,
        )
    })
}

fn integer(dict: CFTypeRef, key: CFTypeRef) -> Option<i64> {
    // SAFETY: a Get (not retained) of a value the dictionary owns, read as
    // a number only if present.
    unsafe {
        let value = CFDictionaryGetValue(dict, key);
        if value.is_null() {
            return None;
        }
        let mut out: i64 = 0;
        (CFNumberGetValue(value, SINT64, (&mut out as *mut i64).cast()) != 0).then_some(out)
    }
}

/// The first image's stored size and its orientation (1–8).
fn open(path: &Path) -> Result<(Owned, (u32, u32), i64)> {
    let url = file_url(path).some("path", path)?;
    // SAFETY: a valid URL; no options.
    let source = Owned(unsafe { CGImageSourceCreateWithURL(url.0, std::ptr::null()) })
        .some("not an image", path)?;
    // SAFETY: `source` is live for both calls.
    if unsafe { CGImageSourceGetCount(source.0) } == 0 {
        bail!("no image in {}", path.display());
    }
    // SAFETY: as above.
    let props = Owned(unsafe { CGImageSourceCopyPropertiesAtIndex(source.0, 0, std::ptr::null()) })
        .some("no properties", path)?;
    // SAFETY: the keys are ImageIO's own constants.
    let (w, h, orientation) = unsafe {
        (
            integer(props.0, kCGImagePropertyPixelWidth),
            integer(props.0, kCGImagePropertyPixelHeight),
            integer(props.0, kCGImagePropertyOrientation).unwrap_or(1),
        )
    };
    match (w, h) {
        (Some(w), Some(h)) if w > 0 && h > 0 => Ok((source, (w as u32, h as u32), orientation)),
        _ => Err(anyhow!("{} has no pixel size", path.display())),
    }
}

/// Orientations 5–8 turn the frame a quarter, swapping its sides.
fn upright(size: (u32, u32), orientation: i64) -> (u32, u32) {
    if (5..=8).contains(&orientation) {
        (size.1, size.0)
    } else {
        size
    }
}

/// The size upright (see the module notes) without decoding pixels.
pub fn size(path: &Path) -> Result<(u32, u32)> {
    let (_, stored, orientation) = open(path)?;
    Ok(upright(stored, orientation))
}

/// The bytes of a `CFData` this code owns.
fn bytes(data: &Owned) -> Vec<u8> {
    // SAFETY: a live CFData; the pointer covers its whole length.
    unsafe {
        let len = CFDataGetLength(data.0).max(0) as usize;
        std::slice::from_raw_parts(CFDataGetBytePtr(data.0), len).to_vec()
    }
}

/// The colour space to draw `image` in, and the ICC profile to report: the
/// image's own when it is RGB and has one, otherwise sRGB and none. The
/// first element holds the sRGB space when that is the one used.
fn drawing_space(
    image: CFTypeRef,
    path: &Path,
) -> Result<(Option<Owned>, CFTypeRef, Option<Vec<u8>>)> {
    // SAFETY: a Get (not retained) of the image's own space, used while
    // `image` is alive; the ICC data is a Copy, released by `Owned`.
    unsafe {
        let own = CGImageGetColorSpace(image);
        if !own.is_null() && CGColorSpaceGetModel(own) == MODEL_RGB {
            let icc = Owned(CGColorSpaceCopyICCData(own));
            if !icc.0.is_null() {
                return Ok((None, own, Some(bytes(&icc))));
            }
        }
        let srgb = Owned(CGColorSpaceCreateWithName(kCGColorSpaceSRGB)).some("sRGB", path)?;
        let space = srgb.0;
        Ok((Some(srgb), space, None))
    }
}

/// Decodes to 8-bit RGB, upright, with the photo's ICC profile if it has one.
pub fn decode(path: &Path) -> Result<crate::raster::WithProfile> {
    let (source, (w, h), _) = open(path)?;
    // A "thumbnail" as large as the image itself, with the transform: the
    // one ImageIO call that decodes the whole frame *and* applies its
    // orientation.
    let longest = number(w.max(h) as i64);
    // SAFETY: ImageIO's option keys, and live CF values for each.
    let options = unsafe {
        dictionary(&[
            (kCGImageSourceCreateThumbnailFromImageAlways, kCFBooleanTrue),
            (kCGImageSourceCreateThumbnailWithTransform, kCFBooleanTrue),
            (kCGImageSourceThumbnailMaxPixelSize, longest.0),
        ])
    };
    // SAFETY: a live source and options dictionary, each released by `Owned`.
    let image = Owned(unsafe { CGImageSourceCreateThumbnailAtIndex(source.0, 0, options.0) })
        .some("cannot decode", path)?;
    // SAFETY: `image` is a live CGImage.
    let (w, h) = unsafe { (CGImageGetWidth(image.0), CGImageGetHeight(image.0)) };

    // Drawn in the image's own space, so CoreGraphics copies the values
    // rather than converting them.
    let (_srgb, space, icc) = drawing_space(image.0, path)?;
    crate::raster::check_decode_size(path, (w as u64) * (h as u64) * 4)?;
    let mut rgbx = vec![0u8; w * h * 4];
    // SAFETY: `rgbx` holds exactly `h` rows of `w * 4` bytes and outlives the
    // context, which draws into it once and is released before it is read.
    unsafe {
        let context = Owned(CGBitmapContextCreate(
            rgbx.as_mut_ptr().cast(),
            w,
            h,
            8,
            w * 4,
            space,
            RGBX,
        ))
        .some("bitmap", path)?;
        let rect = CGRect {
            x: 0.0,
            y: 0.0,
            width: w as f64,
            height: h as f64,
        };
        CGContextDrawImage(context.0, rect, image.0);
    }
    let rgb: Vec<u8> = rgbx
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    let img = ImageBuffer::from_raw(w as u32, h as u32, rgb)
        .ok_or_else(|| anyhow!("pixel buffer size"))?;
    Ok((img, icc))
}

/// Writes `image` as a HEIC with EXIF-style `orientation` (1–8), which ImageIO
/// stores as the container's rotation — the way a phone records a portrait
/// shot — tagged with `icc` (sRGB when `None`). For tests, as
/// `wic::encode_for_test` is: there are no HEIC fixtures to ship.
/// `Ok(false)` when this Mac cannot encode HEVC.
#[cfg(any(test, feature = "test-helpers"))]
pub fn encode_for_test(
    image: &ImageBuffer<image::Rgb<u8>, Vec<u8>>,
    orientation: i64,
    icc: Option<&[u8]>,
    path: &Path,
) -> Result<bool> {
    #[link(name = "ImageIO", kind = "framework")]
    extern "C" {
        fn CGImageDestinationCreateWithURL(
            url: CFTypeRef,
            kind: CFTypeRef,
            count: usize,
            options: CFTypeRef,
        ) -> CFTypeRef;
        fn CGImageDestinationAddImage(dest: CFTypeRef, image: CFTypeRef, props: CFTypeRef);
        fn CGImageDestinationFinalize(dest: CFTypeRef) -> u8;
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGColorSpaceCreateWithICCData(data: CFTypeRef) -> CFTypeRef;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFDataCreate(alloc: CFTypeRef, bytes: *const u8, len: isize) -> CFTypeRef;
        fn CFStringCreateWithCString(
            alloc: CFTypeRef,
            s: *const std::ffi::c_char,
            encoding: u32,
        ) -> CFTypeRef;
    }
    const UTF8: u32 = 0x0800_0100;

    let (w, h) = (image.width() as usize, image.height() as usize);
    let mut rgbx: Vec<u8> = image
        .pixels()
        .flat_map(|p| [p[0], p[1], p[2], 255])
        .collect();
    // SAFETY: each object is created here and released by `Owned`; `rgbx`
    // holds `h` rows of `w * 4` bytes and outlives the context drawing it.
    unsafe {
        let space = match icc {
            Some(icc) => {
                let data = Owned(CFDataCreate(
                    std::ptr::null(),
                    icc.as_ptr(),
                    icc.len() as isize,
                ));
                Owned(CGColorSpaceCreateWithICCData(data.0))
            }
            None => Owned(CGColorSpaceCreateWithName(kCGColorSpaceSRGB)),
        }
        .some("colour space", path)?;
        let context = Owned(CGBitmapContextCreate(
            rgbx.as_mut_ptr().cast(),
            w,
            h,
            8,
            w * 4,
            space.0,
            RGBX,
        ))
        .some("bitmap", path)?;
        let cg = Owned(CGBitmapContextCreateImage(context.0)).some("image", path)?;
        let url = file_url(path).some("path", path)?;
        let heic = Owned(CFStringCreateWithCString(
            std::ptr::null(),
            c"public.heic".as_ptr(),
            UTF8,
        ));
        let dest = Owned(CGImageDestinationCreateWithURL(
            url.0,
            heic.0,
            1,
            std::ptr::null(),
        ));
        if dest.0.is_null() {
            return Ok(false);
        }
        let turn = number(orientation);
        let props = dictionary(&[(kCGImagePropertyOrientation, turn.0)]);
        CGImageDestinationAddImage(dest.0, cg.0, props.0);
        Ok(CGImageDestinationFinalize(dest.0) != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    /// Left half red, right half blue: which way up it came back is visible
    /// in two pixels.
    fn halves() -> ImageBuffer<Rgb<u8>, Vec<u8>> {
        ImageBuffer::from_fn(64, 32, |x, _| {
            if x < 32 {
                Rgb([220, 30, 30])
            } else {
                Rgb([30, 30, 220])
            }
        })
    }

    #[test]
    fn a_rotated_heic_comes_back_upright() {
        let dir = crate::test_support::Scratch::new("imageio-heic");
        let (plain, turned) = (dir.join("plain.heic"), dir.join("turned.heic"));
        // 6: shown turned a quarter clockwise, the way a phone records a
        // portrait frame without re-encoding it.
        if !encode_for_test(&halves(), 1, None, &plain).unwrap()
            || !encode_for_test(&halves(), 6, None, &turned).unwrap()
        {
            eprintln!("skipping: no HEIC encoder on this Mac");
            return;
        }

        let (img, _) = decode(&plain).unwrap();
        assert_eq!(img.dimensions(), (64, 32));
        assert_eq!(size(&plain).unwrap(), (64, 32));
        let red = |p: &Rgb<u8>| p[0] > 150 && p[2] < 100;
        assert!(red(img.get_pixel(0, 0)) && !red(img.get_pixel(63, 0)));

        // Turned a quarter clockwise: the red left half is now the top.
        let (img, _) = decode(&turned).unwrap();
        assert_eq!(img.dimensions(), (32, 64), "ImageIO applied the rotation");
        assert_eq!(size(&turned).unwrap(), (32, 64));
        assert!(red(img.get_pixel(0, 0)) && red(img.get_pixel(31, 0)));
        assert!(!red(img.get_pixel(0, 63)));

        // And nothing rotates it again.
        let bytes = std::fs::read(&turned).unwrap();
        assert_eq!(
            crate::raster::probe(&turned, &bytes),
            (Some((32, 64)), Some(1))
        );
        assert_eq!(crate::raster::decode(&turned).unwrap().orientation, 1);
    }

    /// The system's own Display P3 profile, which is what an iPhone writes.
    fn display_p3() -> Vec<u8> {
        #[link(name = "CoreGraphics", kind = "framework")]
        extern "C" {
            static kCGColorSpaceDisplayP3: CFTypeRef;
        }
        // SAFETY: a system constant, and objects released by `Owned`.
        unsafe {
            let space = Owned(CGColorSpaceCreateWithName(kCGColorSpaceDisplayP3));
            bytes(&Owned(CGColorSpaceCopyICCData(space.0)))
        }
    }

    /// A HEIC's colour profile comes through with its values untouched: an
    /// iPhone's is Display P3, and `raster` needs it to show the photo right.
    #[test]
    fn a_heics_profile_comes_through() {
        use crate::raster::icc;
        let dir = crate::test_support::Scratch::new("imageio-icc");
        let path = dir.join("p3.heic");
        let p3 = display_p3();
        if !encode_for_test(&halves(), 1, Some(&p3), &path).unwrap() {
            eprintln!("skipping: no HEIC encoder on this Mac");
            return;
        }
        let (img, got) = decode(&path).unwrap();
        let got = icc::Profile::parse(&got.expect("the profile survives")).unwrap();
        assert!(!got.is_srgb());
        assert_eq!(Some(got), icc::Profile::parse(&p3));
        // Copied, not converted to sRGB (which would push the red out).
        let p = img.get_pixel(0, 0);
        assert!((p[0] as i32 - 220).abs() <= 3, "{p:?}");

        // An sRGB HEIC reports sRGB, which `raster` then leaves alone.
        let plain = dir.join("plain.heic");
        encode_for_test(&halves(), 1, None, &plain).unwrap();
        let (_, got) = decode(&plain).unwrap();
        assert!(icc::Profile::parse(&got.unwrap()).unwrap().is_srgb());
    }

    /// A file macOS cannot read fails with a reason rather than a panic or an
    /// empty image.
    #[test]
    fn a_broken_heic_fails_with_a_reason() {
        let dir = crate::test_support::Scratch::new("imageio-bad");
        let bad = dir.join("broken.heic");
        std::fs::write(&bad, b"not a heic").unwrap();
        let e = format!("{:#}", decode(&bad).unwrap_err());
        assert!(e.contains("broken.heic"), "{e}");
    }

    #[test]
    fn quarter_turns_swap_the_sides() {
        assert_eq!(upright((64, 32), 1), (64, 32));
        assert_eq!(upright((64, 32), 3), (64, 32));
        assert_eq!(upright((64, 32), 6), (32, 64));
        assert_eq!(upright((64, 32), 8), (32, 64));
    }
}
