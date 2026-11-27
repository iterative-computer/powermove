//! Subsampled chroma reconstructed at its sample positions with a
//! high-quality filter independent of any engine choice: separable
//! Lanczos-3 (a windowed sinc), weights normalized, edge samples repeated.
//! Chroma sample positions follow H.273 chroma location (for example
//! type 0, "left": co-sited across, between rows for 4:2:0).

fn lanczos3(x: f64) -> f64 {
    if x == 0.0 {
        1.0
    } else if x.abs() >= 3.0 {
        0.0
    } else {
        let px = std::f64::consts::PI * x;
        3.0 * px.sin() * (px / 3.0).sin() / (px * px)
    }
}

/// One chroma plane (`chroma` = its width and height) at luma resolution
/// (`luma`), for subsampling `factors` (luma pixels per chroma sample) with
/// chroma sample 0 at `origin` (luma pixels).
pub fn reconstruct_lanczos3(
    plane: &[f64],
    chroma: (usize, usize),
    luma: (usize, usize),
    factors: (usize, usize),
    origin: (f64, f64),
) -> Vec<f64> {
    let taps = |p: usize, origin: f64, factor: usize, size: usize| -> Vec<(usize, f64)> {
        if factor == 1 {
            return vec![(p.min(size - 1), 1.0)];
        }
        let u = (p as f64 - origin) / factor as f64;
        let base = u.floor() as i64;
        let raw: Vec<(usize, f64)> = (base - 2..=base + 3)
            .map(|i| (i.clamp(0, size as i64 - 1) as usize, lanczos3(u - i as f64)))
            .collect();
        let sum: f64 = raw.iter().map(|t| t.1).sum();
        raw.into_iter().map(|(i, w)| (i, w / sum)).collect()
    };
    let (w, h) = luma;
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        let ty = taps(y, origin.1, factors.1, chroma.1);
        for x in 0..w {
            let tx = taps(x, origin.0, factors.0, chroma.0);
            out.push(
                ty.iter()
                    .flat_map(|&(j, wy)| tx.iter().map(move |&(i, wx)| (j, i, wy * wx)))
                    .map(|(j, i, wt)| wt * plane[j * chroma.0 + i])
                    .sum(),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_on_their_positions_are_exact_and_flat_stays_flat() {
        let plane: Vec<f64> = (0..8 * 4).map(|i| f64::from(i % 7) / 7.0).collect();
        let up = reconstruct_lanczos3(&plane, (8, 4), (16, 8), (2, 2), (0.0, 0.5));
        // Co-sited columns, rows midway: a column on a sample still mixes
        // rows, so check across only on a 4:2:2-shaped call.
        let across = reconstruct_lanczos3(&plane, (8, 4), (16, 4), (2, 1), (0.0, 0.0));
        for y in 0..4 {
            for i in 0..8 {
                assert!((across[y * 16 + 2 * i] - plane[y * 8 + i]).abs() < 1e-12);
            }
        }
        assert_eq!(up.len(), 128);
        let flat = reconstruct_lanczos3(&[0.25; 32], (8, 4), (16, 8), (2, 2), (0.5, 0.5));
        assert!(flat.iter().all(|v| (v - 0.25).abs() < 1e-12));
    }
}
