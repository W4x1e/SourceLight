//! Non-destructive develop: the edit model and its persistence.
//!
//! Rendering lives in [`crate::gpu`]; this module only describes *what* to
//! render, never how.

pub mod auto;
pub mod curve;
pub mod detail;
pub mod effects;
pub mod erase;
pub mod eyedropper;
pub mod film;
pub mod geometry;
pub mod grading;
pub mod lens;
pub mod look;
pub mod lut;
pub mod lut_packs;
pub mod mask;
pub mod mixer;
pub mod oklab;
pub mod presets;
pub mod reframe;
pub mod spot;
pub mod stack;
pub mod tone;
pub mod white_balance;

pub use curve::{Curve, CurvePoint};
pub use detail::Detail;
pub use geometry::{CropRect, Geometry};
pub use grading::{ColorGrading, GradeRange};
pub use lens::LensCorrection;
pub use look::LookProfile;
pub use mask::{LocalAdjustments, Mask, MaskShape};
pub use mixer::ColorMixer;
pub use stack::{EditStack, History};
pub use tone::ToneParams;
