//! The GPU test on this computer, for numbers to compare a phone's with:
//!
//!   cd mobile/gpu-test
//!   CARGO_TARGET_DIR=../src-tauri/target cargo run --release --example bench -- <a JPEG>
//!
//! Frames are fitted to a 1080 × 2400 screen, an upright phone's.
//!
//! Tests the JPEG, then the same photo enlarged to 12 MP.

fn main() {
    let mut args = std::env::args().skip(1);
    let source = std::path::PathBuf::from(args.next().expect("usage: bench <jpeg>"));
    let large = std::env::temp_dir().join("sl-gpu-test-12mp.jpg");
    sl_gpu_test::bench::enlarge(&source, &large, 4000).expect("enlarging");
    let (report, live) =
        sl_gpu_test::bench::run(&[("photo", &source), ("12 MP", &large)], (1080, 2400));
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    if let Some(live) = live {
        for as_jpeg in [false, true] {
            let f = live.frame(0.5, 0.2, as_jpeg).expect("live frame");
            println!(
                "live frame ({}): {} x {}, render {:.1} ms, encode {:.1} ms, {} bytes",
                if as_jpeg { "jpeg" } else { "rgba" },
                f.width,
                f.height,
                f.render_ms,
                f.encode_ms,
                f.bytes.len()
            );
        }
    }
    let _ = std::fs::remove_file(large);
}
