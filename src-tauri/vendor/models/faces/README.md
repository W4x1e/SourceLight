# Bundled face models

These three models ship inside the app, unmodified. Every other model is
downloaded when first needed (`src-tauri/src/ai_models.rs`).

| File | Model | Licence |
|---|---|---|
| `yunet.onnx` | YuNet face detection (2023mar), from huggingface.co/opencv/face_detection_yunet | MIT, (c) 2020 Shiqi Yu |
| `mesh.onnx` | MediaPipe Face Landmarker face mesh (FaceMesh V2), as an ONNX conversion from github.com/MediaMolder/MediaMolder | Apache-2.0, (c) Google |
| `blendshapes.onnx` | MediaPipe Face Landmarker blendshapes (Blendshape V2), from the same conversion | Apache-2.0, (c) Google |

[NOTICE](../../../../NOTICE) has the full attributions.
