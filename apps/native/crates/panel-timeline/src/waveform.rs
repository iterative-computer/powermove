//! A clip's waveform as the timeline draws it: the loudness of what the clip
//! plays, read at any zoom from a pyramid of levels, on a perceptual scale,
//! one value per drawn column (two columns per point: the device pixels of a
//! Retina display), carried to the painter as a compact string.
//!
//! The painter fills one smooth shape from these columns in the strip's own
//! hue, lighter at the peaks and fading into the slab ([`Style`]).
use std::sync::Arc;

/// Drawn columns per point.
pub const COLUMNS_PER_POINT: f64 = 2.;
/// The quietest level drawn, in dB below full scale: anything quieter is
/// silence, and the scale between it and full scale is even in dB, so quiet
/// speech still has a shape instead of a flat floor with a few spikes.
pub const FLOOR_DB: f32 = 54.;

/// How the waveform is drawn: the owner picks one (`PM_WAVEFORM` names it
/// meanwhile: `mirror`, `floor` or `body`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Style {
    /// Symmetric about the strip's middle, as Logic and Final Cut draw it.
    Mirror,
    /// Standing on the strip's floor, a soft filled shape (the glass
    /// timeline concept).
    #[default]
    Floor,
    /// Standing on the floor: the RMS as the filled body, the peaks as a
    /// fainter line over it.
    Body,
}
impl Style {
    pub fn name(self) -> &'static str {
        match self {
            Style::Mirror => "mirror",
            Style::Floor => "floor",
            Style::Body => "body",
        }
    }
    pub fn parse(name: &str) -> Option<Style> {
        match name {
            "mirror" => Some(Style::Mirror),
            "floor" => Some(Style::Floor),
            "body" => Some(Style::Body),
            _ => None,
        }
    }
    /// The style chosen for this process (`PM_WAVEFORM`), else the default.
    pub fn chosen() -> Style {
        static CHOSEN: std::sync::OnceLock<Style> = std::sync::OnceLock::new();
        *CHOSEN.get_or_init(|| {
            std::env::var("PM_WAVEFORM")
                .ok()
                .and_then(|v| Style::parse(&v))
                .unwrap_or_default()
        })
    }
}

/// One channel slot's levels at every resolution: level 0 is the finest
/// (`rate` buckets per second), each next level halves it, the larger peak
/// and the root of the mean square of each pair.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pyramid {
    pub peak: Vec<Vec<f32>>,
    pub rms: Vec<Vec<f32>>,
}
impl Pyramid {
    pub fn new(peak: Vec<f32>, rms: Vec<f32>) -> Self {
        let mut p = Pyramid {
            peak: vec![peak],
            rms: vec![rms],
        };
        while p.peak.last().is_some_and(|l| l.len() > 1) {
            let (peak, rms) = (p.peak.last().unwrap(), p.rms.last().unwrap());
            let next_peak = peak.chunks(2).map(|c| c.iter().copied().fold(0., f32::max)).collect();
            let next_rms = rms
                .chunks(2)
                .map(|c| (c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32).sqrt())
                .collect();
            p.peak.push(next_peak);
            p.rms.push(next_rms);
        }
        p
    }
}

/// What a clip's media sounds like, for drawing: the combined channels and,
/// for a stereo clip, each side ([`Levels::channels`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Levels {
    /// Buckets per second at the finest level.
    pub rate: f64,
    /// Every channel together.
    pub combined: Pyramid,
    /// Left and right, when the clip is stereo.
    pub sides: Option<[Pyramid; 2]>,
}
impl Levels {
    /// From per-bucket (peak, RMS) of two channel slots (pm-audio's
    /// `ClipLevels`).
    pub fn new(rate: f64, channels: usize, peak: &[[f32; 2]], rms: &[[f32; 2]]) -> Self {
        let pick = |v: &[[f32; 2]], k: usize| v.iter().map(|b| b[k]).collect::<Vec<_>>();
        let combined = Pyramid::new(
            peak.iter().map(|[a, b]| a.max(*b)).collect(),
            rms.iter()
                .map(|[a, b]| ((a * a + b * b) / 2.).sqrt())
                .collect(),
        );
        let sides = (channels >= 2).then(|| {
            [
                Pyramid::new(pick(peak, 0), pick(rms, 0)),
                Pyramid::new(pick(peak, 1), pick(rms, 1)),
            ]
        });
        Levels {
            rate,
            combined,
            sides,
        }
    }
    /// The media's length in seconds.
    pub fn duration(&self) -> f64 {
        self.combined.peak.first().map_or(0., |l| l.len() as f64 / self.rate)
    }
}

/// The (peak, RMS) of `p` between media times `t0` and `t1` (seconds): the
/// level whose buckets are no wider than half the span, over every bucket the
/// span touches; a span narrower than the finest bucket reads the finest
/// level between bucket centres, so a zoomed-in waveform stays smooth.
pub fn column(p: &Pyramid, rate: f64, t0: f64, t1: f64) -> (f32, f32) {
    let Some(finest) = p.peak.first().filter(|l| !l.is_empty()) else {
        return (0., 0.);
    };
    let (t0, t1) = (t0.min(t1), t0.max(t1));
    let span = (t1 - t0) * rate;
    if span < 1. {
        let at = (t0 + t1) / 2. * rate - 0.5;
        let lerp = |level: &[f32]| {
            let i = at.floor();
            let f = (at - i) as f32;
            let get = |k: f64| {
                if k < 0. || k >= level.len() as f64 {
                    0.
                } else {
                    level[k as usize]
                }
            };
            get(i) * (1. - f) + get(i + 1.) * f
        };
        return (lerp(finest), lerp(&p.rms[0]));
    }
    // Exactly the buckets the span touches, each level taking its odd ends:
    // a loud moment never bleeds into the next column.
    let mut a = (t0 * rate).floor().max(0.) as usize;
    let mut b = ((t1 * rate).ceil().max(0.) as usize).min(finest.len());
    if a >= b {
        return (0., 0.);
    }
    let (mut top, mut squares, mut count) = (0f32, 0f64, 0f64);
    let mut take = |level: usize, i: usize| {
        let width = (1usize << level) as f64;
        top = top.max(p.peak[level][i]);
        squares += f64::from(p.rms[level][i]).powi(2) * width;
        count += width;
    };
    let mut level = 0;
    while a < b && level < p.peak.len() {
        if level + 1 == p.peak.len() {
            for i in a..b {
                take(level, i);
            }
            break;
        }
        if a % 2 == 1 {
            take(level, a);
            a += 1;
        }
        if b % 2 == 1 && b > a {
            b -= 1;
            take(level, b);
        }
        a /= 2;
        b /= 2;
        level += 1;
    }
    (top, (squares / count.max(1.)).sqrt() as f32)
}

/// A linear level as drawn: even in dB from [`FLOOR_DB`] below full scale
/// (0) to full scale (1).
pub fn perceptual(level: f32) -> f32 {
    if level <= 0. {
        return 0.;
    }
    (1. + 20. * level.log10() / FLOOR_DB).clamp(0., 1.)
}

/// A clip's columns: `count` columns from the strip's left at `left_time`
/// (seconds on the clip's own timeline), `step` seconds apart, each read at
/// the media time `source` gives for it. Returns perceptual (peak, RMS) per
/// column, 0–255.
pub fn columns(
    p: &Pyramid,
    rate: f64,
    source: &dyn Fn(f64) -> f64,
    left_time: f64,
    step: f64,
    count: usize,
) -> (Vec<u8>, Vec<u8>) {
    let q = |v: f32| (perceptual(v) * 255.).round() as u8;
    let mut peaks = Vec::with_capacity(count);
    let mut rms = Vec::with_capacity(count);
    for i in 0..count {
        let t = left_time + i as f64 * step;
        let (a, b) = (source(t), source(t + step));
        let (peak, mean) = column(p, rate, a, b);
        peaks.push(q(peak));
        rms.push(q(mean));
    }
    (peaks, rms)
}

/// Bytes as the op carries them (two hex digits each).
pub fn encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 15) as usize] as char);
    }
    s
}
/// [`encode`]'s bytes back, as 0–1.
pub fn decode(text: &str) -> Vec<f32> {
    let digit = |c: u8| match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        _ => 0,
    };
    text.as_bytes()
        .chunks(2)
        .map(|p| {
            let hi = digit(p[0]);
            let lo = p.get(1).map_or(0, |c| digit(*c));
            f32::from(hi * 16 + lo) / 255.
        })
        .collect()
}

/// Columns softened for drawing: a [1, 2, 1] filter twice, so the outline
/// joins smoothly, never below two thirds of a column's own height, so a
/// transient keeps most of its height while its neighbours rise toward it.
pub fn smooth(values: &[f32]) -> Vec<f32> {
    let pass = |v: &[f32]| -> Vec<f32> {
        (0..v.len())
            .map(|i| {
                let a = v[i.saturating_sub(1)];
                let c = v[(i + 1).min(v.len() - 1)];
                (a + 2. * v[i] + c) / 4.
            })
            .collect()
    };
    if values.is_empty() {
        return vec![];
    }
    pass(&pass(values))
        .into_iter()
        .zip(values)
        .map(|(soft, v)| soft.max(v * 0.66))
        .collect()
}

/// A clip's media read through `map`, a time map sampled evenly over the
/// clip (retimed or reversed clips): the media time at clip time `t`.
pub fn mapped(map: &Arc<[f64]>, duration: f64) -> impl Fn(f64) -> f64 + '_ {
    move |t: f64| {
        let n = map.len();
        if n == 0 {
            return t;
        }
        if n == 1 || duration <= 0. {
            return map[0];
        }
        let at = (t / duration * (n - 1) as f64).clamp(0., (n - 1) as f64);
        let i = at.floor() as usize;
        let f = at - i as f64;
        let j = (i + 1).min(n - 1);
        map[i] * (1. - f) + map[j] * f
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pyramid(peaks: Vec<f32>) -> Pyramid {
        let rms = peaks.iter().map(|p| p * 0.5).collect();
        Pyramid::new(peaks, rms)
    }

    /// Quiet sound has a shape: a level 40 dB down still draws a quarter of
    /// the height; silence draws nothing; full scale draws the whole.
    #[test]
    fn the_scale_is_even_in_decibels() {
        assert_eq!(perceptual(0.), 0.);
        assert_eq!(perceptual(1.), 1.);
        assert!((perceptual(0.01) - (1. - 40. / FLOOR_DB)).abs() < 1e-5);
        assert!(perceptual(0.01) > 0.25, "linear would draw 1 %");
        assert_eq!(perceptual(1e-4), 0., "below the floor is silence");
    }

    /// Every zoom reads real data: a span many buckets wide reads a coarser
    /// level and still finds the one loud bucket; a span narrower than a
    /// bucket reads between bucket centres, so it changes smoothly.
    #[test]
    fn every_zoom_reads_the_loudest_moment_under_it() {
        let mut peaks = vec![0.1; 4000];
        peaks[2345] = 0.9;
        let p = pyramid(peaks);
        assert_eq!(column(&p, 400., 0., 10.).0, 0.9, "the whole 10 s");
        assert_eq!(column(&p, 400., 5.8, 5.9).0, 0.9, "a tenth of a second");
        assert_eq!(column(&p, 400., 0., 5.8).0, 0.1, "before it");
        // Narrower than a bucket: between the loud bucket and its neighbour.
        let centre = 2345.5 / 400.;
        let half = column(&p, 400., centre + 0.5 / 400. - 1e-4, centre + 0.5 / 400. + 1e-4).0;
        assert!((half - 0.5).abs() < 0.02, "{half}");
    }

    /// A trimmed clip reads its media from the trim: trimming the In edge
    /// moves the strip's start, not the waveform under a point of the media.
    #[test]
    fn a_trimmed_clip_reads_the_same_media_where_it_was() {
        let mut peaks = vec![0.; 400];
        peaks[200] = 1.; // 0.5 s into the media
        let p = pyramid(peaks);
        let read = |trim: f64| {
            let source = move |t: f64| t + trim;
            // Columns 1/400 s apart from the clip's start.
            let (cols, _) = columns(&p, 400., &source, 0., 1. / 400., 400 - (trim * 400.) as usize);
            cols.iter().position(|v| *v == 255).map(|i| i as f64 / 400. + trim)
        };
        assert_eq!(read(0.), Some(0.5));
        assert_eq!(read(0.25), Some(0.5), "same media time after a 0.25 s trim");
    }

    /// A reversed clip reads its media backwards through its time map.
    #[test]
    fn a_reversed_clip_draws_its_media_backwards() {
        let mut peaks = vec![0.; 400];
        peaks[40] = 1.; // 0.1 s into 1 s of media
        let p = pyramid(peaks);
        let map: Arc<[f64]> = vec![1., 0.].into();
        let source = mapped(&map, 1.);
        let (cols, _) = columns(&p, 400., &source, 0., 1. / 400., 400);
        let at = cols.iter().position(|v| *v == 255).unwrap() as f64 / 400.;
        assert!((at - 0.9).abs() < 0.01, "{at}");
    }

    /// The op's string carries the columns exactly.
    #[test]
    fn columns_survive_the_op() {
        let bytes = [0u8, 1, 127, 128, 254, 255];
        let back = decode(&encode(&bytes));
        assert_eq!(back.iter().map(|v| (v * 255.).round() as u8).collect::<Vec<_>>(), bytes);
    }

    /// Smoothing joins neighbours without flattening a transient.
    #[test]
    fn smoothing_keeps_a_transient() {
        let mut v = vec![0.; 9];
        v[4] = 1.;
        let s = smooth(&v);
        assert!(s[4] >= 0.66 && s[3] > 0. && s[5] > 0. && s[0] == 0.);
    }

    /// A stereo clip keeps each side; the combined shape is the louder peak.
    #[test]
    fn a_stereo_clip_keeps_both_sides() {
        let levels = Levels::new(400., 2, &[[0.2, 0.8]; 8], &[[0.1, 0.4]; 8]);
        assert_eq!(levels.combined.peak[0][0], 0.8);
        let [l, r] = levels.sides.as_ref().unwrap();
        assert_eq!((l.peak[0][0], r.peak[0][0]), (0.2, 0.8));
        assert!(Levels::new(400., 1, &[[0.5, 0.5]; 8], &[[0.2, 0.2]; 8]).sides.is_none());
    }
}
