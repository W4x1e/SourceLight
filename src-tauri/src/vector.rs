//! The dot product every embedding comparison is built on: search ranking,
//! near-duplicate grouping, tag scoring, and grouping faces into people.

/// How many partial sums [`dot`] keeps.
const LANES: usize = 16;

/// The dot product of two vectors, over the length of the shorter.
///
/// Summed in sixteen lanes, not left to right. A single running total of
/// floats has to be added in order — reassociating it would change the result
/// — so the compiler cannot vectorise it; sixteen independent partial sums it
/// can, four SIMD registers' worth, so no add waits on the one before. On an M4
/// that made 50,000 products of 768 floats 6× faster (14.9 ms to 2.5), and it
/// is the inner loop of grouping faces, which compares every face with every
/// other. The result differs from the left-to-right sum only in the last bits.
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let (a, b) = (&a[..n], &b[..n]);
    let (whole_a, whole_b) = (a.chunks_exact(LANES), b.chunks_exact(LANES));
    let tail: f32 = whole_a
        .remainder()
        .iter()
        .zip(whole_b.remainder())
        .map(|(x, y)| x * y)
        .sum();
    let mut lanes = [0.0f32; LANES];
    for (x, y) in whole_a.zip(whole_b) {
        // Indexing all three is what vectorises.
        for k in 0..LANES {
            lanes[k] += x[k] * y[k];
        }
    }
    lanes.iter().sum::<f32>() + tail
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_order(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b).map(|(x, y)| x * y).sum()
    }

    /// Every length around the lane width, so the tail is exercised with and
    /// without whole lanes before it.
    #[test]
    fn matches_the_plain_sum_at_every_length() {
        for n in 0..=40 {
            let a: Vec<f32> = (0..n).map(|i| (i as f32 * 0.37).sin()).collect();
            let b: Vec<f32> = (0..n).map(|i| (i as f32 * 0.91).cos()).collect();
            let (got, want) = (dot(&a, &b), in_order(&a, &b));
            assert!((got - want).abs() < 1e-5, "n = {n}: {got} vs {want}");
        }
    }

    #[test]
    fn a_length_mismatch_uses_the_shorter() {
        assert_eq!(dot(&[1.0, 2.0, 3.0], &[4.0, 5.0]), 14.0);
        assert_eq!(dot(&[], &[1.0]), 0.0);
    }
}
