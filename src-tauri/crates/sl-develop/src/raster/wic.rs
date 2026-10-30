//! HEIC and HEIF through the Windows Imaging Component.
//!
//! iPhones have shot HEIC by default since 2017, so without it a phone library
//! is mostly missing. libheif would read it but is LGPL; Windows already has a
//! HEIF decoder once two free Microsoft Store packages are installed (HEIF
//! Image Extensions for the container, HEVC Video Extensions for the codec), so
//! this asks the operating system. When they are missing the error says so,
//! and says how to get them.
//!
//! **Orientation.** HEIF can carry a rotation in the container (`irot`) as well
//! as the EXIF tag, and the two must not both be applied. WIC applies the
//! container's rotation as it decodes. Measured, not assumed: a file written
//! with a 90° `irot` decodes at its rotated size and reports orientation 1. So
//! the pixels here are upright and the EXIF tag is ignored, as the HEIF
//! specification says. `a_rotated_heic_comes_back_upright` pins it.

use anyhow::{anyhow, Context, Result};
use image::ImageBuffer;
use std::path::Path;
use windows::core::HSTRING;
use windows::Win32::Foundation::{GENERIC_READ, WINCODEC_ERR_COMPONENTNOTFOUND};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatHeif, GUID_WICPixelFormat24bppRGB,
    IWICBitmapFrameDecode, IWICColorContext, IWICImagingFactory, WICColorContextProfile,
    WICConvertBitmapSource, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};

/// What the user sees when Windows cannot read HEIC.
pub const MISSING_CODEC: &str = "Reading HEIC needs two free Microsoft Store apps: \
     HEIF Image Extensions and HEVC Video Extensions";

/// COM for the length of one call, on whichever thread the scan's pool runs
/// it. Joining an apartment the thread is already in is fine; only a
/// successful join is balanced with an uninitialise.
struct Com(bool);

impl Com {
    fn enter() -> Self {
        // S_FALSE ("already initialised") is still a join to balance;
        // RPC_E_CHANGED_MODE (an STA thread) is not, and WIC works there too.
        // SAFETY: no pointers are passed, and `drop` balances a successful join.
        Com(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok())
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: balances the successful `CoInitializeEx` of `enter`, on
            // the same thread, since `Com` lives on one stack frame.
            unsafe { CoUninitialize() };
        }
    }
}

fn factory() -> Result<IWICImagingFactory> {
    // SAFETY: every caller holds a `Com` for the call, and the class ID is a
    // static.
    unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }
        .context("cannot start the Windows Imaging Component")
}

fn first_frame(factory: &IWICImagingFactory, path: &Path) -> Result<IWICBitmapFrameDecode> {
    // SAFETY: a live factory, and the file name is an owned string that
    // outlives the call.
    let decoder = unsafe {
        factory.CreateDecoderFromFilename(
            &HSTRING::from(path.as_os_str()),
            None,
            GENERIC_READ,
            WICDecodeMetadataCacheOnDemand,
        )
    }
    .map_err(|e| {
        // "No component" means no decoder recognised the content, which is
        // also what a damaged file gets. Only blame the codec when there
        // truly is no HEIF decoder.
        if e.code() == WINCODEC_ERR_COMPONENTNOTFOUND && !has_heif_decoder(factory) {
            anyhow!("{MISSING_CODEC}")
        } else {
            anyhow!(e).context(format!("Windows cannot read {}", path.display()))
        }
    })?;
    // SAFETY: a live decoder; a missing frame comes back as an error.
    unsafe { decoder.GetFrame(0) }.with_context(|| format!("no image in {}", path.display()))
}

/// Whether Windows has a HEIF container decoder (the HEIF Image Extensions).
fn has_heif_decoder(factory: &IWICImagingFactory) -> bool {
    // SAFETY: a live factory and a static GUID; a null vendor means any.
    unsafe { factory.CreateDecoder(&GUID_ContainerFormatHeif, std::ptr::null()) }.is_ok()
}

/// The stored size (already upright, see the module notes) without decoding
/// pixels.
pub fn size(path: &Path) -> Result<(u32, u32)> {
    let _com = Com::enter();
    let frame = first_frame(&factory()?, path)?;
    let (mut w, mut h) = (0, 0);
    // SAFETY: a live frame, writing to two local integers.
    unsafe { frame.GetSize(&mut w, &mut h) }?;
    Ok((w, h))
}

/// Decodes to 8-bit RGB, upright, with the frame's ICC profile if it has one.
pub fn decode(path: &Path) -> Result<crate::raster::WithProfile> {
    let _com = Com::enter();
    let factory = factory()?;
    let frame = first_frame(&factory, path)?;
    let icc = profile(&factory, &frame);
    // SAFETY: a live frame and a static pixel format GUID.
    let rgb = unsafe { WICConvertBitmapSource(&GUID_WICPixelFormat24bppRGB, &frame) }
        .with_context(|| format!("cannot convert {}", path.display()))?;
    let (mut w, mut h) = (0, 0);
    // SAFETY: a live bitmap, writing to two local integers.
    unsafe { rgb.GetSize(&mut w, &mut h) }?;
    crate::raster::check_decode_size(path, u64::from(w) * u64::from(h) * 3)?;
    // Checked: a frame with no rows passes the size check at any width.
    let stride = w.checked_mul(3).context("the frame is too wide")?;
    let mut pixels = vec![0u8; (w as usize) * (h as usize) * 3];
    // The container opened, so a failure here is most often the other half
    // of the codec: HEVC Video Extensions.
    // SAFETY: a null rectangle means the whole frame, which is `h` rows of
    // `stride` bytes: exactly the length of `pixels`, which WIC also checks.
    unsafe { rgb.CopyPixels(std::ptr::null(), stride, &mut pixels) }.with_context(|| {
        format!(
            "cannot decode {} (is HEVC Video Extensions installed?)",
            path.display()
        )
    })?;
    let image = ImageBuffer::from_raw(w, h, pixels).ok_or_else(|| anyhow!("pixel buffer size"))?;
    Ok((image, icc))
}

/// The first ICC profile among the frame's colour contexts. An iPhone's HEIC
/// carries Display P3 this way.
fn profile(factory: &IWICImagingFactory, frame: &IWICBitmapFrameDecode) -> Option<Vec<u8>> {
    // WIC reports the count only for a null array, which a slice cannot be,
    // so offer a few contexts up front; a frame has one or two at most.
    const SLOTS: usize = 4;
    // SAFETY: live COM objects throughout, and every buffer is a slice whose
    // length WIC is given with it.
    unsafe {
        let mut contexts: Vec<Option<IWICColorContext>> = (0..SLOTS)
            .map(|_| factory.CreateColorContext().ok())
            .collect();
        let mut count = 0u32;
        frame.GetColorContexts(&mut contexts, &mut count).ok()?;
        contexts
            .into_iter()
            .take(count as usize)
            .flatten()
            .find_map(|ctx| {
                (ctx.GetType().ok()? == WICColorContextProfile).then_some(())?;
                // Asked with no buffer, WIC answers "insufficient" but
                // still says how long the profile is.
                let mut len = 0u32;
                let _ = ctx.GetProfileBytes(&mut [], &mut len);
                (len > 0).then_some(())?;
                let mut bytes = vec![0u8; len as usize];
                ctx.GetProfileBytes(&mut bytes, &mut len).ok()?;
                Some(bytes)
            })
    }
}

/// Writes `image` as a HEIC through WIC, rotated in the container by
/// `transform` (a `WICBitmapTransformOptions` value) the way a phone records
/// a portrait shot. For tests: there are no HEIC fixtures to ship, and this is
/// the same codec that reads them. `Ok(false)` when this machine cannot encode.
#[cfg(any(test, feature = "test-helpers"))]
pub fn encode_for_test(
    image: &ImageBuffer<image::Rgb<u8>, Vec<u8>>,
    transform: u8,
    icc: Option<&[u8]>,
    path: &Path,
) -> Result<bool> {
    use windows::core::BSTR;
    use windows::Win32::Foundation::GENERIC_WRITE;
    use windows::Win32::Graphics::Imaging::{
        GUID_ContainerFormatPng, IWICBitmapFrameEncode, WICBitmapEncoderNoCache,
    };
    use windows::Win32::System::Com::StructuredStorage::{IPropertyBag2, PROPBAG2};
    use windows::Win32::System::Variant::VARIANT;

    let _com = Com::enter();
    let factory = factory()?;
    let (w, h) = image.dimensions();
    // SAFETY: live COM objects throughout. The bitmap is copied from
    // `image`, `w * 3` bytes a row, and `name` outlives the property write.
    unsafe {
        let bitmap = factory.CreateBitmapFromMemory(
            w,
            h,
            &GUID_WICPixelFormat24bppRGB,
            w * 3,
            image.as_raw(),
        )?;
        // A `.png` path writes a PNG: WIC's HEIF encoder accepts a colour
        // profile but does not store it, and the PNG one does.
        let png = path.extension().is_some_and(|e| e == "png");
        let container = if png {
            GUID_ContainerFormatPng
        } else {
            GUID_ContainerFormatHeif
        };
        let Ok(encoder) = factory.CreateEncoder(&container, std::ptr::null()) else {
            return Ok(false);
        };
        let stream = factory.CreateStream()?;
        stream.InitializeFromFilename(&HSTRING::from(path.as_os_str()), GENERIC_WRITE.0)?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let (mut frame, mut options): (Option<IWICBitmapFrameEncode>, Option<IPropertyBag2>) =
            (None, None);
        encoder.CreateNewFrame(&mut frame, &mut options)?;
        let (frame, options) = (frame.context("no frame")?, options.context("no options")?);
        if transform != 0 {
            let name = BSTR::from("BitmapTransform");
            let option = PROPBAG2 {
                pstrName: windows::core::PWSTR(name.as_ptr() as *mut u16),
                ..Default::default()
            };
            options.Write(1, &option, &VARIANT::from(transform))?;
        }
        frame.Initialize(&options)?;
        if let Some(icc) = icc {
            let context = factory.CreateColorContext()?;
            context.InitializeFromMemory(icc)?;
            frame.SetColorContexts(&[Some(context)])?;
        }
        frame.WriteSource(&bitmap, std::ptr::null())?;
        if frame.Commit().is_err() || encoder.Commit().is_err() {
            // No HEVC encoder on this machine.
            return Ok(false);
        }
    }
    Ok(true)
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
        let dir = crate::test_support::Scratch::new("wic-heic");
        let (plain, turned) = (dir.join("plain.heic"), dir.join("turned.heic"));
        // 1 is WICBitmapTransformRotate90: a rotation in the container, the
        // way a phone records a portrait frame without re-encoding it.
        if !encode_for_test(&halves(), 0, None, &plain).unwrap()
            || !encode_for_test(&halves(), 1, None, &turned).unwrap()
        {
            eprintln!("skipping: no HEIC encoder on this machine");
            return;
        }

        let (img, _) = decode(&plain).unwrap();
        assert_eq!(img.dimensions(), (64, 32));
        assert_eq!(size(&plain).unwrap(), (64, 32));
        let red = |p: &Rgb<u8>| p[0] > 150 && p[2] < 100;
        assert!(red(img.get_pixel(0, 0)) && !red(img.get_pixel(63, 0)));

        // Turned a quarter clockwise: the red left half is now the top.
        let (img, _) = decode(&turned).unwrap();
        assert_eq!(img.dimensions(), (32, 64), "the codec applied the rotation");
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

    /// A frame's colour profile comes through: an iPhone HEIC's is Display
    /// P3. Written as a PNG, because WIC's HEIF encoder drops profiles; the
    /// reading side is the same call for every container.
    #[test]
    fn a_frames_profile_comes_through() {
        use crate::raster::icc;
        let dir = crate::test_support::Scratch::new("wic-icc");
        let path = dir.join("adobe.png");
        let profile = icc::build_for_test(icc::ADOBE_RGB_D50, &icc::gamma_curve_for_test(2.2));
        if !encode_for_test(&halves(), 0, Some(&profile), &path).unwrap() {
            eprintln!("skipping: no HEIC encoder on this machine");
            return;
        }
        let (_, got) = decode(&path).unwrap();
        let got = icc::Profile::parse(&got.expect("the profile survives"));
        assert_eq!(got, icc::Profile::parse(&profile));
    }

    /// A file Windows cannot read fails with a reason rather than a panic or
    /// an empty image.
    #[test]
    fn a_broken_heic_fails_with_a_reason() {
        let dir = crate::test_support::Scratch::new("wic-bad");
        let bad = dir.join("broken.heic");
        std::fs::write(&bad, b"not a heic").unwrap();
        let e = format!("{:#}", decode(&bad).unwrap_err());
        assert!(e.contains("broken.heic") || e.contains("HEIF"), "{e}");
    }
}
