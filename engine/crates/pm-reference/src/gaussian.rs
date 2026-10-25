//! The exact separable Gaussian blur.
//!
//! The image is taken as constant over each pixel square (the box
//! reconstruction of its samples). Convolving that function with the 2D
//! Gaussian of standard deviations `σx`, `σy` and reading it at pixel
//! centers gives, per axis, the weights
//! `w(k) = Φ((k + ½)/σ) − Φ((k − ½)/σ)` (Φ the standard normal CDF), which
//! is what [`kernel`] returns: no normalization, no truncation beyond 8σ
//! (tails below 1e-15). σ comes from the authored formula of the effect,
//! mask feather or filter being gated; σ = 0 is the identity.

/// Pixels outside the image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// Transparent black (CSS `filter: blur()`, mask feathers).
    Transparent,
    /// The nearest edge pixel.
    Clamp,
}

/// Standard normal CDF.
fn phi(x: f64) -> f64 {
    0.5 * libm::erfc(-x / std::f64::consts::SQRT_2)
}

/// Weights `w(−r) … w(r)` of a Gaussian with standard deviation `sigma`
/// pixels.
pub fn kernel(sigma: f64) -> Vec<f64> {
    assert!(sigma >= 0.0 && sigma.is_finite(), "sigma {sigma}");
    if sigma == 0.0 {
        return vec![1.0];
    }
    let r = (8.0 * sigma).ceil() as i64;
    (-r..=r)
        .map(|k| {
            let k = k as f64;
            // The smaller tail of each difference keeps precision far out.
            if k >= 0.0 {
                phi(-(k - 0.5) / sigma) - phi(-(k + 0.5) / sigma)
            } else {
                phi((k + 0.5) / sigma) - phi((k - 0.5) / sigma)
            }
        })
        .collect()
}

/// Blurs `image` (`width × height` pixels of `channels` interleaved
/// values, premultiplied when it has alpha) by σ = (`sigma_x`, `sigma_y`).
pub fn blur(
    image: &[f64],
    width: usize,
    height: usize,
    channels: usize,
    sigma: (f64, f64),
    edge: Edge,
) -> Vec<f64> {
    assert_eq!(image.len(), width * height * channels);
    let pass = |src: &[f64], kernel: &[f64], horizontal: bool| -> Vec<f64> {
        let r = (kernel.len() / 2) as i64;
        let (len, lines) = if horizontal {
            (width, height)
        } else {
            (height, width)
        };
        let at = |line: usize, i: usize| {
            if horizontal {
                line * width + i
            } else {
                i * width + line
            }
        };
        let mut out = vec![0.0; src.len()];
        for line in 0..lines {
            for i in 0..len {
                for (j, w) in kernel.iter().enumerate() {
                    let mut k = i as i64 + j as i64 - r;
                    if k < 0 || k >= len as i64 {
                        match edge {
                            Edge::Transparent => continue,
                            Edge::Clamp => k = k.clamp(0, len as i64 - 1),
                        }
                    }
                    let (o, s) = (at(line, i) * channels, at(line, k as usize) * channels);
                    for c in 0..channels {
                        out[o + c] += w * src[s + c];
                    }
                }
            }
        }
        out
    };
    let x = pass(image, &kernel(sigma.0), true);
    pass(&x, &kernel(sigma.1), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_moments() {
        for sigma in [0.3, 1.0, 2.5, 17.0] {
            let k = kernel(sigma);
            let r = (k.len() / 2) as f64;
            let sum: f64 = k.iter().sum();
            let var: f64 = k
                .iter()
                .enumerate()
                .map(|(i, w)| w * (i as f64 - r).powi(2))
                .sum();
            assert!((sum - 1.0).abs() < 1e-14, "{sigma}: {sum}");
            // Box reconstruction adds the box's variance, 1/12 (Sheppard's
            // correction, exact up to terms in exp(−2π²σ²)).
            if sigma >= 1.0 {
                assert!((var - (sigma * sigma + 1.0 / 12.0)).abs() < 1e-6, "{sigma}: {var}");
            }
        }
        assert_eq!(kernel(0.0), vec![1.0]);
    }

    #[test]
    fn blur_preserves_mass_and_constants() {
        let (w, h) = (80, 80);
        let mut img = vec![0.0; w * h];
        img[40 * w + 40] = 1.0;
        let out = blur(&img, w, h, 1, (2.0, 3.0), Edge::Transparent);
        let mass: f64 = out.iter().sum();
        assert!((mass - 1.0).abs() < 1e-9);
        let flat = blur(&vec![0.5; w * h], w, h, 1, (4.0, 4.0), Edge::Clamp);
        assert!(flat.iter().all(|v| (v - 0.5).abs() < 1e-14));
    }
}
