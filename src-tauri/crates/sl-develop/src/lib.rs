//! SourceLight's develop core, shared by the desktop and the phone app
//! (docs/MOBILE.md section 4): the edit model (`develop`), the GPU pipeline
//! that renders it (`gpu`), decoding photos that are not raw (`raster`), the
//! contract between raw decoders and the pipeline (`raw`), and reading EXIF
//! (`exif_read`).
//!
//! What only the desktop does stays in the desktop crate, which adds to these
//! modules rather than wrapping them: LibRaw (`raw::libraw`), the AI masks,
//! noise reduction and magic erase's model (`develop::ai`, `sam2`, `erase`),
//! Develop's live state and the window it draws into (`develop::session`,
//! `present`). Nothing here names a window, a database or a model file.

#![warn(clippy::undocumented_unsafe_blocks)]

pub mod develop;
pub mod exif_read;
pub mod gpu;
pub mod raster;
pub mod raw;
pub mod test_support;
