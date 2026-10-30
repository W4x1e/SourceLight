//! Non-destructive develop. The edit model is shared with the phone app
//! (`sl_develop::develop`, re-exported here whole); what only the desktop
//! does is added beside it: the AI masks (`ai`, `sam2`), magic erase's model
//! and fills (`erase`), looks suggested from analysis (`suggest`), and
//! Develop's live state (`session`).

pub use sl_develop::develop::*;

pub mod ai;
pub mod coreml_cache;
pub mod erase;
pub mod sam2;
pub mod session;
pub mod suggest;

pub use session::{DevelopInfo, DevelopState, Histogram};
