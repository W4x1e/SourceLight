//! SAM2 click-to-select: point at something, get a mask of it.
//!
//! The interactive counterpart to `ai.rs`'s one-shot Select Subject. SAM2 splits
//! into **two** models, and that split is the whole reason this feels live:
//!
//! - the **encoder** turns the image into an embedding. Expensive (comparable to
//!   a BiRefNet pass) but depends only on the photo, so it runs **once** and is
//!   cached.
//! - the **decoder** turns that embedding plus a few clicked points into a mask.
//!   Tiny — milliseconds — so every extra click re-decodes instantly.
//!
//! Same seam as everything else: the result becomes a
//! [`crate::develop::mask::MaskShape::Ai`] coverage bitmap, so it drops into the
//! component/combine system (AI selection **minus** a brushed patch, and so on).
//!
//! Points are given in **framed-output UV** (0..1), the space every mask uses.
//! The model sees the frame squished to a square, exactly as [`ai::preprocess`]
//! prepares it, so a UV point maps to model pixels by a plain multiply — no
//! letterbox offset to get wrong.

use crate::develop::ai::{self, Activation, SegConfig};

/// SAM2's input contract. ImageNet normalisation and a 1024² input, like
/// BiRefNet; the decoder emits **logits**, so coverage needs a sigmoid.
pub const SAM2: SegConfig = SegConfig {
    size: 1024,
    mean: [0.485, 0.456, 0.406],
    std: [0.229, 0.224, 0.225],
    activation: Activation::Sigmoid,
};

/// A clicked prompt, in framed-output UV.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Point {
    pub x: f32,
    pub y: f32,
    /// True adds this region to the selection, false carves it out — SAM2's
    /// foreground/background labels (1 and 0).
    pub include: bool,
}

/// Builds the decoder's `point_coords` (`[N, 2]`, model pixel space) and
/// `point_labels` (`[N]`, 1 = include, 0 = exclude) from UV clicks.
///
/// The image is squished to `size`×`size`, so both axes scale by `size` —
/// getting this wrong is the classic SAM failure where the mask lands offset
/// from the click, which is why it is a plain, tested function.
pub fn point_tensors(points: &[Point], size: u32) -> (Vec<f32>, Vec<f32>) {
    let s = size as f32;
    let mut coords = Vec::with_capacity(points.len() * 2);
    let mut labels = Vec::with_capacity(points.len());
    for p in points {
        coords.push(p.x.clamp(0.0, 1.0) * s);
        coords.push(p.y.clamp(0.0, 1.0) * s);
        labels.push(if p.include { 1.0 } else { 0.0 });
    }
    (coords, labels)
}

/// Index of the highest-IoU mask. SAM2 proposes several candidates per click
/// (typically whole-object vs part-of-object) plus a predicted quality for each;
/// taking the best-scoring one is what makes a single click usually "just work".
pub fn best_mask_index(ious: &[f32]) -> usize {
    ious.iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// Extracts mask `idx` from the decoder's `[count, h, w]` logits and turns it
/// into an R8 coverage at `out_w`×`out_h`.
///
/// The logits are a soft signed field (positive inside the object); a sigmoid
/// gives a nicely feathered edge rather than a hard binary cut, which is what we
/// want for a mask that will be blended.
pub fn mask_to_coverage(
    logits: &[f32],
    count: usize,
    h: usize,
    w: usize,
    idx: usize,
    out_w: u32,
    out_h: u32,
) -> Vec<u8> {
    let plane = h * w;
    if plane == 0 || count == 0 {
        return vec![0u8; (out_w.max(1) * out_h.max(1)) as usize];
    }
    let idx = idx.min(count - 1);
    let start = idx * plane;
    let slice = logits.get(start..start + plane).unwrap_or(&[]);
    // Reuse the shared matte -> coverage path (sigmoid + resize) so SAM2 and
    // BiRefNet cannot drift in how a mask is turned into coverage.
    let cfg = SegConfig { size: 0, ..SAM2 };
    ai::matte_to_coverage(slice, w as u32, h as u32, &cfg, out_w, out_h)
}

// ------------------------------------------------------------------------
// ONNX sessions — only with `--features ai`.
// ------------------------------------------------------------------------
#[cfg(feature = "ai")]
pub use imp::{Embedding, Sam2};

#[cfg(feature = "ai")]
mod imp {
    use super::*;
    use anyhow::{anyhow, Result};
    use std::path::Path;

    /// The encoder's output: one image embedding plus the two high-resolution
    /// feature maps the decoder needs to produce a sharp mask.
    pub struct Embedding {
        embed: (Vec<i64>, Vec<f32>),
        feat0: (Vec<i64>, Vec<f32>),
        feat1: (Vec<i64>, Vec<f32>),
    }

    /// A loaded SAM2 pair.
    pub struct Sam2 {
        encoder: ort::session::Session,
        decoder: ort::session::Session,
    }

    impl Sam2 {
        /// Loads `encoder.onnx` + `decoder.onnx` from `dir`. Common alternative
        /// filenames from the popular exports are accepted too.
        pub fn load(dir: &Path) -> Result<Self> {
            let encoder = build(&pick(
                dir,
                &["encoder.onnx", "image_encoder.onnx", "sam2_encoder.onnx"],
            )?)?;
            let decoder = build(&pick(
                dir,
                &[
                    "decoder.onnx",
                    "sam2_decoder.onnx",
                    "prompt_encoder_mask_decoder.onnx",
                ],
            )?)?;
            Ok(Self { encoder, decoder })
        }

        /// Runs the expensive half. Cache the result for as long as the photo and
        /// its edit are unchanged.
        pub fn encode(&mut self, rgba: &[u8], w: u32, h: u32) -> Result<Embedding> {
            let input = ai::preprocess(rgba, w, h, &SAM2);
            let size = SAM2.size as i64;
            let tensor = ort::value::Value::from_array(([1_i64, 3, size, size], input))?;
            let name = input_name(&self.encoder, &["image", "input_image", "pixel_values"])?;

            // Output order varies by export, so resolve by name where possible
            // and fall back to positional — a swapped embedding and feature map
            // would produce a plausible but wrong mask. Resolved *before* `run`,
            // which borrows the session mutably for as long as its outputs live.
            let i_embed = output_index(&self.encoder, &["image_embed", "image_embeddings"], 0);
            let i_feat0 = output_index(&self.encoder, &["high_res_feats_0"], 1);
            let i_feat1 = output_index(&self.encoder, &["high_res_feats_1"], 2);

            let outputs = self.encoder.run(ort::inputs![name.as_str() => tensor])?;
            Ok(Embedding {
                embed: take_shaped(&outputs, i_embed)?,
                feat0: take_shaped(&outputs, i_feat0)?,
                feat1: take_shaped(&outputs, i_feat1)?,
            })
        }

        /// Runs the cheap half: embedding + clicks -> coverage at `out_w`×`out_h`.
        pub fn decode(
            &mut self,
            emb: &Embedding,
            points: &[Point],
            out_w: u32,
            out_h: u32,
        ) -> Result<Vec<u8>> {
            if points.is_empty() {
                return Err(anyhow!("no points to select from"));
            }
            let (coords, labels) = point_tensors(points, SAM2.size);
            let n = points.len() as i64;

            let embed_t =
                ort::value::Value::from_array((emb.embed.0.clone(), emb.embed.1.clone()))?;
            let feat0_t =
                ort::value::Value::from_array((emb.feat0.0.clone(), emb.feat0.1.clone()))?;
            let feat1_t =
                ort::value::Value::from_array((emb.feat1.0.clone(), emb.feat1.1.clone()))?;
            let coords_t = ort::value::Value::from_array(([1_i64, n, 2], coords))?;
            let labels_t = ort::value::Value::from_array(([1_i64, n], labels))?;
            // No previous-mask refinement: zeros with the flag off.
            let mask_t =
                ort::value::Value::from_array(([1_i64, 1, 256, 256], vec![0f32; 256 * 256]))?;
            let has_mask_t = ort::value::Value::from_array(([1_i64], vec![0f32]))?;

            let d = &self.decoder;
            let n_embed = input_name(d, &["image_embed", "image_embeddings"])?;
            let n_feat0 = input_name(d, &["high_res_feats_0"])?;
            let n_feat1 = input_name(d, &["high_res_feats_1"])?;
            let n_coords = input_name(d, &["point_coords"])?;
            let n_labels = input_name(d, &["point_labels"])?;
            let n_mask = input_name(d, &["mask_input"])?;
            let n_has = input_name(d, &["has_mask_input"])?;

            // As in `encode`: resolve output positions before the mutable borrow.
            let i_masks = output_index(&self.decoder, &["masks", "low_res_masks"], 0);
            let i_ious = output_index(&self.decoder, &["iou_predictions", "iou_scores"], 1);
            let n_outputs = self.decoder.outputs.len();

            let outputs = self.decoder.run(ort::inputs![
                n_embed.as_str() => embed_t,
                n_feat0.as_str() => feat0_t,
                n_feat1.as_str() => feat1_t,
                n_coords.as_str() => coords_t,
                n_labels.as_str() => labels_t,
                n_mask.as_str() => mask_t,
                n_has.as_str() => has_mask_t,
            ])?;

            let (mshape, masks) = take_shaped(&outputs, i_masks)?;
            // Missing IoU scores are survivable — `best_mask_index` then takes the
            // first candidate, which is SAM2's own default ordering.
            let ious = if i_ious < n_outputs {
                take_shaped(&outputs, i_ious)
                    .map(|(_, d)| d)
                    .unwrap_or_default()
            } else {
                Vec::new()
            };

            // masks is [1, count, h, w]; drop the batch dimension.
            let dims: Vec<usize> = mshape.iter().map(|&d| d.max(0) as usize).collect();
            let (count, mh, mw) = match dims.len() {
                4 => (dims[1], dims[2], dims[3]),
                3 => (dims[0], dims[1], dims[2]),
                _ => return Err(anyhow!("unexpected mask output shape {dims:?}")),
            };
            let idx = best_mask_index(&ious);
            Ok(mask_to_coverage(&masks, count, mh, mw, idx, out_w, out_h))
        }
    }

    /// On a Mac on the CPU: CoreML's compile made the first click take up to
    /// three minutes, for an encode 0.4 s quicker ([`ai::commit_without_compile`]).
    fn build(path: &Path) -> Result<ort::session::Session> {
        ai::commit_without_compile(path, "sam2")
    }

    /// First of `names` that exists in `dir`, with an error naming them all.
    fn pick(dir: &Path, names: &[&str]) -> Result<std::path::PathBuf> {
        for n in names {
            let p = dir.join(n);
            if p.exists() {
                return Ok(p);
            }
        }
        Err(anyhow!(
            "none of {names:?} found in {} — put a SAM2 ONNX export there",
            dir.display()
        ))
    }

    /// Resolves an input name, erroring with what the model actually wants —
    /// the difference between a five-second fix and an afternoon.
    fn input_name(session: &ort::session::Session, candidates: &[&str]) -> Result<String> {
        session
            .inputs
            .iter()
            .find(|i| candidates.contains(&i.name.as_str()))
            .map(|i| i.name.clone())
            .ok_or_else(|| {
                let have: Vec<&str> = session.inputs.iter().map(|i| i.name.as_str()).collect();
                anyhow!("model has no input named any of {candidates:?}; it has {have:?}")
            })
    }

    /// Position of the first output named in `names`, else `fallback`. Must be
    /// called before `run`, whose outputs borrow the session.
    fn output_index(session: &ort::session::Session, names: &[&str], fallback: usize) -> usize {
        names
            .iter()
            .find_map(|n| session.outputs.iter().position(|o| &o.name == n))
            .unwrap_or(fallback)
    }

    /// Copies output `idx` out as (shape, data).
    fn take_shaped(
        outputs: &ort::session::SessionOutputs,
        idx: usize,
    ) -> Result<(Vec<i64>, Vec<f32>)> {
        let (shape, data) = outputs[idx].try_extract_tensor::<f32>()?;
        Ok((shape.to_vec(), data.to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_map_from_uv_to_model_pixels_with_labels() {
        let pts = [
            Point {
                x: 0.5,
                y: 0.25,
                include: true,
            },
            Point {
                x: 0.0,
                y: 1.0,
                include: false,
            },
        ];
        let (coords, labels) = point_tensors(&pts, 1024);
        assert_eq!(
            coords,
            vec![512.0, 256.0, 0.0, 1024.0],
            "uv scales by the input edge"
        );
        assert_eq!(labels, vec![1.0, 0.0], "include/exclude become 1/0");
    }

    #[test]
    fn out_of_range_clicks_are_clamped_into_the_frame() {
        let pts = [Point {
            x: 1.5,
            y: -0.2,
            include: true,
        }];
        let (coords, _) = point_tensors(&pts, 100);
        assert_eq!(coords, vec![100.0, 0.0]);
    }

    #[test]
    fn the_highest_scoring_candidate_wins() {
        assert_eq!(best_mask_index(&[0.1, 0.9, 0.4]), 1);
        // No scores at all still yields a usable index rather than panicking.
        assert_eq!(best_mask_index(&[]), 0);
    }

    #[test]
    fn mask_logits_become_coverage_for_the_chosen_candidate() {
        // Two 2x2 candidates: the first empty, the second full.
        let logits = vec![-10.0, -10.0, -10.0, -10.0, 10.0, 10.0, 10.0, 10.0];
        let empty = mask_to_coverage(&logits, 2, 2, 2, 0, 2, 2);
        let full = mask_to_coverage(&logits, 2, 2, 2, 1, 2, 2);
        assert!(
            empty.iter().all(|&c| c < 5),
            "negative logits -> no coverage"
        );
        assert!(
            full.iter().all(|&c| c > 250),
            "positive logits -> full coverage"
        );
        // An out-of-range index clamps rather than reading past the buffer.
        assert_eq!(mask_to_coverage(&logits, 2, 2, 2, 99, 2, 2), full);
    }

    #[test]
    fn a_degenerate_mask_yields_empty_coverage_of_the_right_size() {
        assert_eq!(mask_to_coverage(&[], 0, 0, 0, 0, 3, 2), vec![0u8; 6]);
    }
}
