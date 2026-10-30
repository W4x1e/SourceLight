//! Finishing effects applied to the developed, cropped frame: a post-crop
//! vignette and film grain.
//!
//! **Post-crop, not lens.** `lens::vignetting` corrects the *sensor's* falloff
//! about its optical centre, before tone. This vignette is a creative darkening
//! about the **cropped frame's** centre, applied at the very end — so it follows
//! a crop and reads the way the eye sees the finished image, which is why the
//! two are separate controls that look identical in isolation.
//!
//! Both are evaluated in framed-image UV (via the same drawn→framed mapping the
//! masks carry), so they stay pinned to the photo under a zoom and the grain is
//! the same scene density in the preview and the export.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct Effects {
    /// Post-crop vignette, -1..=1. Negative darkens the corners (the usual
    /// direction); positive lightens them.
    pub vignette: f32,
    /// 0..=1: how far in from the corners the vignette reaches. Higher pulls the
    /// darkening further toward the centre. Editors usually call this the midpoint.
    pub vignette_midpoint: f32,
    /// Film grain amount, 0..=1.
    pub grain: f32,
    /// Grain size, roughly 0.3..=3 as a multiple of the default. Larger is
    /// coarser, more like fast film.
    pub grain_size: f32,
    /// Halation, 0..=1: the warm bloom that spills out of a bright highlight.
    ///
    /// On real film, light that punches through the emulsion reflects off the
    /// base and re-exposes the surrounding grains — reddest first, because red
    /// penetrates furthest. It is the one thing on this list a digital sensor
    /// never does on its own, and its absence is a large part of why a graded
    /// digital frame still reads as digital.
    #[serde(default)]
    pub halation: f32,
    /// Luminance above which halation starts, 0..=1. Only genuine highlights
    /// should bloom; a low threshold fogs the whole frame.
    #[serde(default = "default_halation_threshold")]
    pub halation_threshold: f32,
    /// 0..=1: how far grain is faded out of the highlights.
    ///
    /// Uniform noise is the giveaway of fake grain. On film the highlights are
    /// where the emulsion is most exposed and reads smoothest, so grain lives
    /// in the mid-tones and shadows. 0 is the old uniform behaviour.
    #[serde(default = "default_grain_rolloff")]
    pub grain_highlight_rolloff: f32,
}

fn default_halation_threshold() -> f32 {
    0.72
}

fn default_grain_rolloff() -> f32 {
    0.7
}

impl Default for Effects {
    /// Everything off, but with a *usable* midpoint and grain size so the
    /// sliders act the moment their amount is raised (a zero size would describe
    /// no grain cell at all — the same trap `Detail::sharpen_radius` avoids).
    fn default() -> Self {
        Self {
            vignette: 0.0,
            vignette_midpoint: 0.5,
            grain: 0.0,
            grain_size: 1.0,
            halation: 0.0,
            halation_threshold: default_halation_threshold(),
            grain_highlight_rolloff: default_grain_rolloff(),
        }
    }
}

impl Effects {
    pub fn is_neutral(&self) -> bool {
        self.vignette == 0.0 && self.grain == 0.0 && self.halation == 0.0
    }

    /// `[vignette, midpoint, grain, grain size]` for the shader.
    pub fn to_shader(&self) -> [f32; 4] {
        [
            self.vignette,
            self.vignette_midpoint,
            self.grain,
            self.grain_size,
        ]
    }

    /// `[halation, halation threshold, grain highlight roll-off, unused]` — the
    /// second uniform slot, because the first is full.
    pub fn to_film_shader(&self) -> [f32; 4] {
        [
            self.halation,
            self.halation_threshold,
            self.grain_highlight_rolloff,
            0.0,
        ]
    }
}
