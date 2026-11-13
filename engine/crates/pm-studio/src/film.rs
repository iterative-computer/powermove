//! Linear radiance reconstruction by importance-sampling the pixel footprint.
//! Width follows Cycles: Blackman-Harris 1.5 has support [-1.5, 1.5].
//! https://github.com/blender/blender/blob/main/intern/cycles/scene/film.cpp
pub const FILTER_TABLE_SIZE: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PixelFilter {
    Box,
    BlackmanHarris { width: f32 },
}
impl Default for PixelFilter {
    fn default() -> Self {
        Self::BlackmanHarris { width: 1.5 }
    }
}
impl PixelFilter {
    /// Inverse CDF in pixel offsets; uniform samples have equal radiance weight.
    /// Accumulation, emission, coverage and guides share this footprint before
    /// denoising or tone mapping. Live retains its temporal frame jitter.
    pub fn quantiles(self) -> Result<[f32; FILTER_TABLE_SIZE], String> {
        let width = match self {
            Self::Box => {
                return Ok(std::array::from_fn(|i| {
                    i as f32 / (FILTER_TABLE_SIZE - 1) as f32 - 0.5
                }));
            }
            Self::BlackmanHarris { width } => width,
        };
        if !width.is_finite() || width <= 0. || width > 4. {
            return Err("Pixel filter width must be finite and in 0..=4 pixels".into());
        }
        const STEPS: usize = 16384;
        let mut cdf = vec![0.; STEPS + 1];
        let density = |x: f64| {
            let angle = std::f64::consts::TAU * (x + 0.5);
            (0.35875 - 0.48829 * angle.cos() + 0.14128 * (2. * angle).cos()
                - 0.01168 * (3. * angle).cos())
            .max(0.)
        };
        for i in 1..=STEPS {
            let a = (i - 1) as f64 / STEPS as f64 - 0.5;
            let b = i as f64 / STEPS as f64 - 0.5;
            cdf[i] = cdf[i - 1] + (density(a) + density(b)) * 0.5;
        }
        let total = cdf[STEPS];
        let mut cursor = 1;
        let mut table = [0.; FILTER_TABLE_SIZE];
        for (i, result) in table.iter_mut().enumerate().take(FILTER_TABLE_SIZE / 2) {
            let target = i as f64 / (FILTER_TABLE_SIZE - 1) as f64 * total;
            while cursor < STEPS && cdf[cursor] < target {
                cursor += 1;
            }
            let t = (target - cdf[cursor - 1]) / (cdf[cursor] - cdf[cursor - 1]);
            *result = (((cursor - 1) as f64 + t) / STEPS as f64 - 0.5) as f32 * (2. * width);
        }
        for i in 0..FILTER_TABLE_SIZE / 2 {
            table[FILTER_TABLE_SIZE - 1 - i] = -table[i];
        }
        Ok(table)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reconstruction_is_symmetric_monotonic_and_has_cycles_support() {
        let table = PixelFilter::default().quantiles().unwrap();
        assert_eq!(table[0], -1.5);
        assert_eq!(table[FILTER_TABLE_SIZE - 1], 1.5);
        assert!(table.windows(2).all(|p| p[0] < p[1]));
        assert!(table.iter().sum::<f32>().abs() < 0.0001);
        for i in 0..FILTER_TABLE_SIZE {
            assert_eq!(table[i], -table[FILTER_TABLE_SIZE - 1 - i]);
        }
        let center_fraction =
            table.iter().filter(|x| x.abs() < 0.5).count() as f32 / table.len() as f32;
        assert!(
            (0.75..0.78).contains(&center_fraction),
            "central footprint {center_fraction}"
        );
    }
    #[test]
    fn box_reference_and_invalid_widths_are_explicit() {
        let table = PixelFilter::Box.quantiles().unwrap();
        assert_eq!(table[0], -0.5);
        assert_eq!(table[FILTER_TABLE_SIZE - 1], 0.5);
        for width in [0., -1., 5., f32::NAN, f32::INFINITY] {
            assert!(PixelFilter::BlackmanHarris { width }.quantiles().is_err());
        }
    }
}
