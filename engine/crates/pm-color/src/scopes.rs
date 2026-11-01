//! CPU scopes over straight RGB code values in an explicitly selected matrix.
//! No implicit EOTF/tone map: the caller selects the analysis stage. Density is
//! integer sample counts, row-major (vertical level, horizontal source column).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Matrix {
    Bt709,
    Bt2020,
}
impl Matrix {
    fn weights(self) -> [f64; 3] {
        match self {
            Self::Bt709 => [0.2126, 0.7152, 0.0722],
            Self::Bt2020 => [0.2627, 0.6780, 0.0593],
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct ScopeConfig {
    pub columns: usize,
    pub levels: usize,
    pub vector_bins: usize,
    pub min: f64,
    pub max: f64,
    pub matrix: Matrix,
}
impl Default for ScopeConfig {
    fn default() -> Self {
        Self {
            columns: 256,
            levels: 256,
            vector_bins: 256,
            min: 0.,
            max: 1.,
            matrix: Matrix::Bt709,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Distribution {
    pub bins: Vec<u64>,
    pub below: u64,
    pub above: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scopes {
    pub pixels: u64,
    pub histogram: [Distribution; 3],
    pub luma_histogram: Distribution,
    pub waveform: Vec<u64>,
    /// Independent R, G, B planes; concatenate horizontally for a parade.
    pub parade: [Vec<u64>; 3],
    /// Cb horizontally, Cr vertically, low to high on each axis.
    pub vectorscope: Vec<u64>,
    pub vector_outside: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScopeError {
    Dimensions,
    Configuration,
    NonfinitePixel(usize),
}

fn bin(v: f64, min: f64, max: f64, n: usize) -> usize {
    (((v - min) / (max - min)).clamp(0., 1.) * (n - 1) as f64).round() as usize
}
fn distribution(n: usize) -> Distribution {
    Distribution {
        bins: vec![0; n],
        below: 0,
        above: 0,
    }
}
fn add(d: &mut Distribution, v: f64, cfg: ScopeConfig) {
    d.below += u64::from(v < cfg.min);
    d.above += u64::from(v > cfg.max);
    d.bins[bin(v, cfg.min, cfg.max, cfg.levels)] += 1;
}
/// Dimensions and all finite samples are validated before producing a result.
/// Bin centres span inclusive min..max; under/over samples count at edges too.
/// Allocations are capped at 16 million counters (~128 MiB) across all planes.
pub fn compute(
    rgb: &[[f64; 3]],
    width: usize,
    height: usize,
    cfg: ScopeConfig,
) -> Result<Scopes, ScopeError> {
    if width == 0 || height == 0 || width.checked_mul(height) != Some(rgb.len()) {
        return Err(ScopeError::Dimensions);
    }
    let cells = cfg
        .columns
        .checked_mul(cfg.levels)
        .and_then(|n| n.checked_mul(4))
        .and_then(|n| {
            cfg.vector_bins
                .checked_mul(cfg.vector_bins)
                .and_then(|v| n.checked_add(v))
        })
        .and_then(|n| cfg.levels.checked_mul(4).and_then(|v| n.checked_add(v)));
    if cfg.columns == 0
        || cfg.levels < 2
        || cfg.vector_bins < 2
        || !cfg.min.is_finite()
        || !cfg.max.is_finite()
        || cfg.min >= cfg.max
        || !(cfg.max - cfg.min).is_finite()
        || cells.is_none_or(|n| n > 16_000_000)
    {
        return Err(ScopeError::Configuration);
    }
    let weights = cfg.matrix.weights();
    // Bounded code values prevent finite inputs overflowing the luma/chroma math.
    if let Some((i, _)) = rgb
        .iter()
        .enumerate()
        .find(|(_, p)| p.iter().any(|v| !v.is_finite() || v.abs() > 1e100))
    {
        return Err(ScopeError::NonfinitePixel(i));
    }
    let mut out = Scopes {
        pixels: rgb.len() as u64,
        histogram: std::array::from_fn(|_| distribution(cfg.levels)),
        luma_histogram: distribution(cfg.levels),
        waveform: vec![0; cfg.columns * cfg.levels],
        parade: std::array::from_fn(|_| vec![0; cfg.columns * cfg.levels]),
        vectorscope: vec![0; cfg.vector_bins * cfg.vector_bins],
        vector_outside: 0,
    };
    for (i, p) in rgb.iter().enumerate() {
        let x = ((i % width) as u128 * cfg.columns as u128 / width as u128) as usize;
        let y = p.iter().zip(weights).map(|(v, w)| v * w).sum::<f64>();
        add(&mut out.luma_histogram, y, cfg);
        out.waveform[bin(y, cfg.min, cfg.max, cfg.levels) * cfg.columns + x] += 1;
        for (c, value) in p.iter().enumerate() {
            add(&mut out.histogram[c], *value, cfg);
            out.parade[c][bin(*value, cfg.min, cfg.max, cfg.levels) * cfg.columns + x] += 1;
        }
        let cb = (p[2] - y) / (2. * (1. - weights[2]));
        let cr = (p[0] - y) / (2. * (1. - weights[0]));
        out.vector_outside += u64::from(!(-0.5..=0.5).contains(&cb) || !(-0.5..=0.5).contains(&cr));
        out.vectorscope[bin(cr, -0.5, 0.5, cfg.vector_bins) * cfg.vector_bins
            + bin(cb, -0.5, 0.5, cfg.vector_bins)] += 1;
    }
    Ok(out)
}
