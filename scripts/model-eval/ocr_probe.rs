//! OCR spike, step 4: times the PP-OCR models through
//! the app's own session builder, on DirectML and on the CPU, with the exact
//! input shapes a Python run of the pipeline recorded.
//!
//! Not built with the app: copy it to src-tauri/tests/ to run, then delete it.
//!
//! OCR_DIR=<this folder> cargo test --features ai --test ocr_probe -- --ignored --nocapture
#![cfg(feature = "ai")]

use std::time::Instant;

fn run(sess: &mut ort::session::Session, shape: &[i64]) -> f64 {
    let n: i64 = shape.iter().product();
    let data = vec![0.1f32; n as usize];
    let t = ort::value::Value::from_array((shape.to_vec(), data)).unwrap();
    let start = Instant::now();
    let out = sess.run(ort::inputs!["x" => t]).unwrap();
    let (_s, v) = out[0].try_extract_tensor::<f32>().unwrap();
    assert!(!v.is_empty());
    start.elapsed().as_secs_f64() * 1000.0
}

fn stats(label: &str, mut ms: Vec<f64>) {
    let total: f64 = ms.iter().sum();
    ms.sort_by(f64::total_cmp);
    println!(
        "  {label:<22} mean {:>7.1} ms  median {:>7.1}  p90 {:>7.1}  max {:>7.1}",
        total / ms.len() as f64,
        ms[ms.len() / 2],
        ms[ms.len() * 9 / 10],
        ms[ms.len() - 1]
    );
}

#[test]
#[ignore]
fn ocr_timing() {
    let dir = std::path::PathBuf::from(std::env::var("OCR_DIR").expect("OCR_DIR"));
    for side in ["1600", "960"] {
        let run_json: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("ocr-data").join("out").join(format!("ocr_{side}.json"))).unwrap(),
        )
        .unwrap();
        let photos = run_json["photos"].as_object().unwrap();
        for backend in ["directml", "cpu x4"] {
            let load = |name: &str| {
                let path = dir.join("models").join("ocr").join(name);
                if backend == "directml" {
                    sourcelight_lib::develop::ai::commit_on_gpu_or_cpu(
                        &path,
                        "ocr",
                        sourcelight_lib::develop::ai::Workload::Background,
                    )
                    .unwrap()
                } else {
                    sourcelight_lib::develop::ai::ensure_ort_dylib();
                    ort::session::Session::builder()
                        .unwrap()
                        .with_intra_threads(4)
                        .unwrap()
                        .commit_from_file(&path)
                        .unwrap()
                }
            };
            let t = Instant::now();
            let mut det = load("det.onnx");
            let mut rec = load("rec_en.onnx");
            let load_ms = t.elapsed().as_secs_f64() * 1000.0;
            // Warm up on both orientations and a typical line batch.
            run(&mut det, &[1, 3, 1600, 1056]);
            run(&mut det, &[1, 3, 1056, 1600]);
            run(&mut rec, &[6, 3, 48, 320]);
            let (mut det_ms, mut rec_ms, mut all_ms) = (vec![], vec![], vec![]);
            for (_id, p) in photos {
                let d = p["det_input"].as_array().unwrap();
                let (h, w) = (d[0].as_i64().unwrap(), d[1].as_i64().unwrap());
                let a = run(&mut det, &[1, 3, h, w]);
                let mut b = 0.0;
                for s in p["rec_batches"].as_array().unwrap() {
                    let s: Vec<i64> = s.as_array().unwrap().iter().map(|x| x.as_i64().unwrap()).collect();
                    b += run(&mut rec, &s);
                }
                det_ms.push(a);
                rec_ms.push(b);
                all_ms.push(a + b);
            }
            println!("side {side}, {backend} (sessions loaded in {load_ms:.0} ms), {} photos:", photos.len());
            stats("detect", det_ms);
            stats("recognise (all lines)", rec_ms);
            stats("both", all_ms);
        }
    }
}
