//! The GPU test (docs/MOBILE.md section 4, step 1): the desktop's Develop
//! pipeline, from the shared `sl-develop` crate, on the phone's GPU, with
//! timings. Also where the phone makes its GPU device (`bench::phone_gpu`),
//! which editing uses too.

pub mod bench;
