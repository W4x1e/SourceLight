//! Raw files. The decode contract and everything after it is shared with the
//! phone app (`sl_develop::raw`, re-exported here whole); LibRaw, the one
//! decoder, is the desktop's.

pub use sl_develop::raw::*;

#[cfg(feature = "libraw")]
pub mod libraw;
