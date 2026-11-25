//! Rendered-frame graders. `frames` compares a frame with a reference within a
//! perceptual (CIEDE2000) tolerance; `frame-stats` asserts statistics of a
//! frame, optionally relative to the fixture's own frame at the same time.
use super::{Context, Expect};
use crate::artifacts::Frame;
use crate::image::{self, Rect, Rgba};
use crate::verdict::Verdict;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FramesSpec {
    pub times: Vec<f64>,
    pub reference: Reference,
    #[serde(default = "full")]
    pub region: Rect,
    #[serde(default)]
    pub tolerance: Tolerance,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Reference {
    /// PNG under `data/`; `{t}` is replaced by the time (e.g. `1.5`).
    File(String),
    /// Every pixel of the region is this colour.
    Solid([u8; 3]),
    /// The fixture's own frame at the same time: the run must leave it alone.
    Baseline,
}

/// Mean and 95th-percentile CIEDE2000 limits. ΔE 1 is about one just-noticeable
/// difference; the defaults allow codec and colour-pipeline round trips.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Tolerance {
    pub mean: f64,
    pub p95: f64,
}

impl Default for Tolerance {
    fn default() -> Self {
        Self {
            mean: 2.0,
            p95: 5.0,
        }
    }
}

fn full() -> Rect {
    Rect::FULL
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatsSpec {
    pub times: Vec<f64>,
    /// `all`: every time must satisfy every check; `any`: one time must.
    #[serde(default = "all")]
    pub quantifier: String,
    pub checks: Vec<StatCheck>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatCheck {
    pub stat: Stat,
    #[serde(default = "full")]
    pub region: Rect,
    /// Subtract the baseline frame's value of the same statistic.
    #[serde(default)]
    pub relative: bool,
    /// Divide by the baseline frame's value of the same statistic.
    #[serde(default)]
    pub ratio: bool,
    pub expect: Expect,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stat {
    /// Mean CIELAB L* (0..100).
    MeanL,
    /// 10th / 90th percentile of L*.
    P10L,
    P90L,
    /// Mean sRGB channel, 0..1.
    MeanR,
    MeanG,
    MeanB,
    /// Mean red minus mean blue, 0..1 scale: positive is warm.
    Warmth,
    /// Mean CIELAB chroma.
    Chroma,
    /// Share of pixels within `tol` ΔE of a colour.
    Near {
        rgb: [u8; 3],
        tol: f64,
    },
    /// Share of pixels within `tol` ΔE of any of the colours.
    NearAny {
        colors: Vec<[u8; 3]>,
        tol: f64,
    },
    /// Mean ΔE between this frame and the baseline frame.
    DeltaEBaseline,
    /// Pearson correlation of L* with the baseline frame (1: same tones,
    /// -1: a negative).
    CorrelationBaseline,
    /// Fine detail: mean Sobel gradient magnitude of L* (a blur lowers it, a
    /// dot pattern raises it).
    Detail,
    /// Sideways displacement between the red and blue channels, in pixels:
    /// the shift (up to `max`) that best aligns their horizontal edges. 0 for
    /// an ordinary picture, whatever its colours.
    ChannelShift {
        #[serde(default = "shift_max")]
        max: u32,
    },
}

fn shift_max() -> u32 {
    24
}

fn all() -> String {
    "all".into()
}

fn load(cx: &Context, frames: &[Frame], t: f64) -> Result<Rgba, String> {
    let f = frames
        .iter()
        .find(|f| (f.time - t).abs() < 1e-6)
        .ok_or_else(|| format!("no frame at {t}s"))?;
    Rgba::decode_png(&cx.read(&f.png)?)
}

pub fn grade_reference(spec: &FramesSpec, cx: &Context) -> Verdict {
    let mut v = Verdict::new("frames");
    for &t in &spec.times {
        let frame = match load(cx, &cx.artifacts.frames, t) {
            Ok(f) => f,
            Err(e) => {
                v.missing(format!("rendered frame: {e}"));
                continue;
            }
        };
        let reference = match &spec.reference {
            Reference::Solid(rgb) => Ok(Rgba::solid(frame.width, frame.height, *rgb)),
            Reference::Baseline => load(cx, &cx.artifacts.baseline_frames, t),
            Reference::File(p) => {
                let path = cx.data_root.join(p.replace("{t}", &format!("{t}")));
                std::fs::read(&path)
                    .map_err(|e| format!("{}: {e} (render references first)", path.display()))
                    .and_then(|b| Rgba::decode_png(&b))
            }
        };
        let reference = match reference {
            Ok(r) => r,
            Err(e) => {
                v.missing(format!("reference at {t}s: {e}"));
                continue;
            }
        };
        match image::delta_e_map(&frame, &reference, &spec.region) {
            Err(e) => v.check(false, format!("at {t}s: {e}")),
            Ok(d) => {
                let (m, p) = (image::mean(&d), image::percentile(&d, 95.0));
                v.check(
                    m <= spec.tolerance.mean && p <= spec.tolerance.p95,
                    format!(
                        "at {t}s ΔE2000 mean {m:.2} (≤ {}), p95 {p:.2} (≤ {})",
                        spec.tolerance.mean, spec.tolerance.p95
                    ),
                );
                v.score = Some(v.score.map_or(m, |s: f64| s.max(m)));
            }
        }
    }
    v
}

/// Compute one statistic of a frame region (with the baseline for the
/// statistics that compare against it).
pub fn stat(s: &Stat, frame: &Rgba, base: Option<&Rgba>, r: &Rect) -> Result<f64, String> {
    let px = frame.region(r);
    let labs = || px.iter().map(|&p| image::lab(p)).collect::<Vec<_>>();
    let ch = |i: usize| image::mean(&px.iter().map(|p| p[i] as f64 / 255.0).collect::<Vec<_>>());
    let need = || base.ok_or_else(|| "this statistic needs the baseline frame".to_string());
    Ok(match s {
        Stat::MeanL => image::mean(&labs().iter().map(|l| l[0]).collect::<Vec<_>>()),
        Stat::P10L => image::percentile(&labs().iter().map(|l| l[0]).collect::<Vec<_>>(), 10.0),
        Stat::P90L => image::percentile(&labs().iter().map(|l| l[0]).collect::<Vec<_>>(), 90.0),
        Stat::MeanR => ch(0),
        Stat::MeanG => ch(1),
        Stat::MeanB => ch(2),
        Stat::Warmth => ch(0) - ch(2),
        Stat::Chroma => image::mean(
            &labs()
                .iter()
                .map(|l| (l[1] * l[1] + l[2] * l[2]).sqrt())
                .collect::<Vec<_>>(),
        ),
        Stat::Near { rgb, tol } => near(&px, std::slice::from_ref(rgb), *tol),
        Stat::NearAny { colors, tol } => near(&px, colors, *tol),
        Stat::DeltaEBaseline => image::mean(&image::delta_e_map(frame, need()?, r)?),
        Stat::Detail => detail(frame, r),
        Stat::ChannelShift { max } => channel_shift(frame, r, *max),
        Stat::CorrelationBaseline => {
            let b = need()?;
            if (b.width, b.height) != (frame.width, frame.height) {
                return Err("frame and baseline sizes differ".into());
            }
            let x: Vec<f64> = labs().iter().map(|l| l[0]).collect();
            let y: Vec<f64> = b.region(r).iter().map(|&p| image::lab(p)[0]).collect();
            pearson(&x, &y)
        }
    })
}

/// A region as a grid of one channel: (width, height, values row by row).
fn grid(frame: &Rgba, r: &Rect, f: impl Fn([u8; 3]) -> f64) -> (usize, usize, Vec<f64>) {
    let (x0, y0, x1, y1) = r.pixels(frame.width, frame.height);
    let px = frame.region(r);
    (
        (x1 - x0) as usize,
        (y1 - y0) as usize,
        px.into_iter().map(f).collect(),
    )
}

fn detail(frame: &Rgba, r: &Rect) -> f64 {
    let (w, h, l) = grid(frame, r, |p| image::lab(p)[0]);
    if w < 3 || h < 3 {
        return 0.0;
    }
    let at = |x: usize, y: usize| l[y * w + x];
    let mut sum = 0.0;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let gx = at(x + 1, y - 1) + 2.0 * at(x + 1, y) + at(x + 1, y + 1)
                - at(x - 1, y - 1)
                - 2.0 * at(x - 1, y)
                - at(x - 1, y + 1);
            let gy = at(x - 1, y + 1) + 2.0 * at(x, y + 1) + at(x + 1, y + 1)
                - at(x - 1, y - 1)
                - 2.0 * at(x, y - 1)
                - at(x + 1, y - 1);
            sum += (gx * gx + gy * gy).sqrt() / 8.0;
        }
    }
    sum / ((w - 2) * (h - 2)) as f64
}

fn channel_shift(frame: &Rgba, r: &Rect, max: u32) -> f64 {
    let (w, h, red) = grid(frame, r, |p| p[0] as f64);
    let (_, _, blue) = grid(frame, r, |p| p[2] as f64);
    let max = max as usize;
    if w <= 2 * max + 2 {
        return 0.0;
    }
    // Horizontal edges of each channel.
    let edges = |c: &[f64]| -> Vec<f64> {
        let mut g = vec![0.0; w * h];
        for y in 0..h {
            for x in 0..w - 1 {
                g[y * w + x] = c[y * w + x + 1] - c[y * w + x];
            }
        }
        g
    };
    let (gr, gb) = (edges(&red), edges(&blue));
    let score = |dx: isize| {
        let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
        for y in 0..h {
            for x in max..w - 1 - max {
                let a = gr[y * w + x];
                let b = gb[y * w + (x as isize + dx) as usize];
                sxy += a * b;
                sxx += a * a;
                syy += b * b;
            }
        }
        if sxx == 0.0 || syy == 0.0 {
            0.0
        } else {
            sxy / (sxx * syy).sqrt()
        }
    };
    // Smallest shift wins ties, so an aligned picture reads 0.
    let mut best = (0isize, score(0));
    for d in 1..=max as isize {
        for dx in [d, -d] {
            let s = score(dx);
            if s > best.1 + 1e-9 {
                best = (dx, s);
            }
        }
    }
    best.0.unsigned_abs() as f64
}

fn near(px: &[[u8; 3]], colors: &[[u8; 3]], tol: f64) -> f64 {
    if px.is_empty() {
        return 0.0;
    }
    let targets: Vec<[f64; 3]> = colors.iter().map(|&c| image::lab(c)).collect();
    px.iter()
        .filter(|&&p| {
            let l = image::lab(p);
            targets.iter().any(|t| image::delta_e2000(l, *t) <= tol)
        })
        .count() as f64
        / px.len() as f64
}

fn pearson(x: &[f64], y: &[f64]) -> f64 {
    let (mx, my) = (image::mean(x), image::mean(y));
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for (a, b) in x.iter().zip(y) {
        sxy += (a - mx) * (b - my);
        sxx += (a - mx) * (a - mx);
        syy += (b - my) * (b - my);
    }
    if sxx == 0.0 || syy == 0.0 {
        // A flat frame: identical flat frames correlate fully.
        return if sxx == syy { 1.0 } else { 0.0 };
    }
    sxy / (sxx * syy).sqrt()
}

pub fn grade_stats(spec: &StatsSpec, cx: &Context) -> Verdict {
    grade_stats_on(
        spec,
        cx,
        &cx.artifacts.frames,
        &cx.artifacts.baseline_frames,
        Verdict::new("frame-stats"),
    )
}

/// Frame statistics over a given pair of frame sets (the run's, or those a
/// panel probe rendered before and after its steps).
pub fn grade_stats_on(
    spec: &StatsSpec,
    cx: &Context,
    frames: &[Frame],
    baseline: &[Frame],
    mut v: Verdict,
) -> Verdict {
    let any = spec.quantifier == "any";
    let mut per_time: Vec<(f64, bool, Vec<String>)> = Vec::new();
    for &t in &spec.times {
        let frame = match load(cx, frames, t) {
            Ok(f) => f,
            Err(e) => {
                v.missing(format!("rendered frame: {e}"));
                continue;
            }
        };
        let base = load(cx, baseline, t).ok();
        let mut ok_all = true;
        let mut lines = Vec::new();
        for c in &spec.checks {
            let value = stat(&c.stat, &frame, base.as_ref(), &c.region).and_then(|x| {
                if !(c.relative || c.ratio) {
                    return Ok(x);
                }
                let b = base
                    .as_ref()
                    .ok_or("a relative check needs the baseline frame")?;
                let y = stat(&c.stat, b, Some(b), &c.region)?;
                if c.relative {
                    Ok(x - y)
                } else if y.abs() < 1e-9 {
                    Err("the baseline value is zero, so no ratio".into())
                } else {
                    Ok(x / y)
                }
            });
            match value {
                Err(e) => {
                    v.missing(format!("at {t}s {:?}: {e}", c.stat));
                    ok_all = false;
                }
                Ok(x) => {
                    let good = c.expect.holds(Some(&Expect::num(x)));
                    ok_all &= good;
                    lines.push(format!(
                        "{}{} {}{x:.3} expected {}",
                        if good { "" } else { "✗ " },
                        stat_name(&c.stat),
                        if c.relative {
                            "Δ "
                        } else if c.ratio {
                            "× "
                        } else {
                            ""
                        },
                        c.expect.describe()
                    ));
                }
            }
        }
        per_time.push((t, ok_all, lines));
    }
    if per_time.is_empty() {
        return v;
    }
    if any {
        let hit = per_time.iter().find(|p| p.1);
        v.check(
            hit.is_some(),
            match hit {
                Some((t, _, l)) => format!("at {t}s: {}", l.join(", ")),
                None => format!(
                    "no time satisfies every check: {}",
                    per_time
                        .iter()
                        .map(|(t, _, l)| format!("{t}s [{}]", l.join(", ")))
                        .collect::<Vec<_>>()
                        .join("; ")
                ),
            },
        );
    } else {
        for (t, ok, l) in per_time {
            v.check(ok, format!("at {t}s: {}", l.join(", ")));
        }
    }
    v
}

fn stat_name(s: &Stat) -> String {
    match s {
        Stat::Near { rgb, tol } => format!("near({rgb:?}, ΔE {tol})"),
        Stat::NearAny { colors, tol } => format!("near-any({colors:?}, ΔE {tol})"),
        other => serde_json::to_string(other)
            .unwrap_or_default()
            .trim_matches('"')
            .into(),
    }
}
