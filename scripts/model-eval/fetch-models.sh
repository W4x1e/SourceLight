#!/usr/bin/env bash
# Fetches the exact model files the Florence-2 and SigLIP 2 spikes measured, each pinned to a
# commit, into ./models (git-ignored). About 5.7 GB in all; pass "min" for just
# the recommended SigLIP 2 fp16 pair and Florence-2-base (~1.2 GB).
set -euo pipefail
cd "$(dirname "$0")"
get() { # repo commit file dest
  mkdir -p "$(dirname "$4")"
  [ -s "$4" ] || curl -sfL -o "$4" "https://huggingface.co/$1/resolve/$2/$3"
  echo "$(du -h "$4" | cut -f1)  $4"
}
min=${1:-}

# SigLIP 2 base-patch16-224 — Apache-2.0 (google/siglip2-base-patch16-224).
R=onnx-community/siglip2-base-patch16-224-ONNX; H=ba1f3b0843f24bc5417d38e19c37b287d719b2f4
for f in config.json preprocessor_config.json tokenizer.json tokenizer_config.json special_tokens_map.json \
         onnx/vision_model_fp16.onnx onnx/text_model_fp16.onnx; do get $R $H $f models/siglip2/$(basename $f); done
[ "$min" = min ] || for f in onnx/vision_model.onnx onnx/text_model.onnx onnx/vision_model_quantized.onnx \
         onnx/text_model_quantized.onnx; do get $R $H $f models/siglip2/$(basename $f); done

# Florence-2 — MIT. The fp16 merged decoder is an invalid graph (ORT refuses
# it), so the fp32 decoder is used with the fp16 encoders.
for size in base large; do
  [ "$min" = min ] && [ $size = large ] && continue
  if [ $size = base ]; then R=onnx-community/Florence-2-base-ft; H=e88a44eaf3791a35eae0c5a47b3dbcd36e67eb6f
  else R=onnx-community/Florence-2-large-ft; H=04ace74913c28d7ec94af32cee5c111a10126b6f; fi
  for f in config.json generation_config.json preprocessor_config.json tokenizer.json \
           onnx/vision_encoder_fp16.onnx onnx/embed_tokens_fp16.onnx onnx/encoder_model_fp16.onnx \
           onnx/decoder_model_merged.onnx; do get $R $H $f models/florence-$size/$(basename $f); done
done

# PP-OCR — Apache-2.0 (PaddleOCR), ONNX conversions from RapidOCR.
# PP-OCRv4 mobile text detection and PP-OCRv3 English recognition, each
# checked against its sha256.
R=SWHL/RapidOCR; H=1cfba2e90fc938db55889873735088de210cc173
get $R $H PP-OCRv4/ch_PP-OCRv4_det_infer.onnx models/ocr/det.onnx
get $R $H PP-OCRv3/en_PP-OCRv3_rec_infer.onnx models/ocr/rec_en.onnx
sha256sum -c <<SUMS
d2a7720d45a54257208b1e13e36a8479894cb74155a5efe29462512d42f49da9  models/ocr/det.onnx
ef7abd8bd3629ae57ea2c28b425c1bd258a871b93fd2fe7c433946ade9b5d9ea  models/ocr/rec_en.onnx
SUMS
