//! SHA-256 helpers for verifying downloaded artifacts before they are installed
//! and — for the models and the vision model's server — executed.
//!
//! Every download is checked against a digest pinned in the source — the
//! models in `ai_models.rs` and `vlm.rs`, the llama.cpp runtime in
//! `llama_pack.rs` — so a file is always verified, and an upstream
//! re-upload cannot slip past.

use sha2::{Digest, Sha256};

/// Lower-case hex of a byte slice.
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Lower-case hex SHA-256 of `data`, one-shot.
pub fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

/// Whether `s` is 64 hex digits — the shape of a SHA-256 digest.
pub fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_the_known_abc_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn a_single_byte_flip_changes_the_digest() {
        assert_ne!(sha256_hex(b"abc"), sha256_hex(b"abd"));
    }
}
