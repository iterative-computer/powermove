//! Arc-length timing for a layer's original numeric position channels
//! (`core/anim/motion-path.ts` `createPathTiming`), behind the expression
//! helper `motionPath(layerId, channel, progress, start, end)` (`anim.ts`
//! `motionPathValue`).
//!
//! The table maps distance to source time only; positions are always
//! evaluated from the original curves. Every operation keeps the TypeScript
//! order so results are the legacy floats bit for bit (`Math.hypot` is V8's
//! scaled Kahan sum, [`hypot`]).

/// V8's `Math.hypot` over finite or non-finite numbers: `Infinity` wins over
/// NaN, all zeros give 0, otherwise the Kahan-compensated sum of the squares
/// scaled by the largest magnitude.
pub fn hypot(values: &[f64]) -> f64 {
    let mut max = 0.0_f64;
    let mut nan = false;
    for &v in values {
        if v.is_nan() {
            nan = true;
        } else if v.abs() > max {
            max = v.abs();
        }
    }
    if max == f64::INFINITY {
        return f64::INFINITY;
    }
    if nan {
        return f64::NAN;
    }
    if max == 0.0 {
        return 0.0;
    }
    let mut sum = 0.0_f64;
    let mut compensation = 0.0_f64;
    for &v in values {
        let n = v.abs() / max;
        let summand = n * n - compensation;
        let preliminary = sum + summand;
        compensation = (preliminary - sum) - summand;
        sum = preliminary;
    }
    sum.sqrt() * max
}

/// `Math.max(...values)`: NaN when any is NaN, `-Infinity` for none.
fn js_max(values: impl IntoIterator<Item = f64>) -> f64 {
    let mut out = f64::NEG_INFINITY;
    for v in values {
        if v.is_nan() {
            return f64::NAN;
        }
        if v > out || (v == 0.0 && out == 0.0 && out.is_sign_negative() && !v.is_sign_negative()) {
            out = v;
        }
    }
    out
}

fn distance(a: &[f64], b: &[f64]) -> f64 {
    let diffs: Vec<f64> = a.iter().zip(b).map(|(x, y)| x - y).collect();
    hypot(&diffs)
}

/// A distance-to-time table over `[start, end]` (`PathTiming`).
#[derive(Clone, Debug)]
pub struct PathTiming {
    /// `(time, distance)` rows, distance increasing.
    table: Vec<(f64, f64)>,
    total: f64,
    start: f64,
    end: f64,
}

/// Reads one position sample; errors are thrown `Error` messages.
pub type Read<'r> = dyn FnMut(f64) -> Result<Vec<f64>, String> + 'r;

impl PathTiming {
    /// `createPathTiming(sample, knots, start, end)`.
    pub fn new(
        sample: &mut Read<'_>,
        knots: &[f64],
        start: f64,
        end: f64,
    ) -> Result<PathTiming, String> {
        if !start.is_finite() || !end.is_finite() || end <= start {
            return Err("Motion path needs a finite time range".into());
        }
        let mut timing = PathTiming {
            table: vec![(start, 0.0)],
            total: 0.0,
            start,
            end,
        };
        let mut times: Vec<f64> = Vec::with_capacity(knots.len() + 2);
        times.push(start);
        times.extend(
            knots
                .iter()
                .copied()
                .filter(|&t| t.is_finite() && t > start && t < end),
        );
        times.push(end);
        times.sort_by(|a, b| a.partial_cmp(b).expect("finite times"));
        times.dedup();
        for i in 1..times.len() {
            let av = read(sample, times[i - 1])?;
            let bv = read(sample, times[i])?;
            timing.subdivide(sample, times[i - 1], av, times[i], bv, 0)?;
        }
        // `!(total > 1e-9)`: NaN has no movement either.
        if timing.total.partial_cmp(&1e-9) != Some(std::cmp::Ordering::Greater) {
            return Err("The selected position path has no movement".into());
        }
        Ok(timing)
    }

    fn subdivide(
        &mut self,
        sample: &mut Read<'_>,
        a: f64,
        av: Vec<f64>,
        b: f64,
        bv: Vec<f64>,
        depth: u32,
    ) -> Result<(), String> {
        if self.table.len() > 65536 {
            return Err("Motion path timing is too complex".into());
        }
        let times = [0.25, 0.5, 0.75].map(|q| a + (b - a) * q);
        let values = [
            read(sample, times[0])?,
            read(sample, times[1])?,
            read(sample, times[2])?,
        ];
        let chain: [&[f64]; 5] = [&av, &values[0], &values[1], &values[2], &bv];
        let mut length = 0.0;
        for i in 1..chain.len() {
            length += distance(chain[i - 1], chain[i]);
        }
        let chord = distance(&av, &bv);
        if !length.is_finite() || !chord.is_finite() {
            return Err("Motion path distance must be finite".into());
        }
        let timing_error = js_max(values.iter().enumerate().map(|(i, v)| {
            let linear: Vec<f64> = av
                .iter()
                .enumerate()
                .map(|(axis, &n)| n + (bv[axis] - n) * (i as f64 + 1.0) / 4.0)
                .collect();
            distance(v, &linear)
        }));
        let largest = js_max(chain.iter().flat_map(|p| p.iter().map(|v| v.abs())));
        let tolerance = 1e-5 + largest * 1e-8 + length * 1e-7;
        if depth < 18 && (depth < 2 || length - chord > tolerance || timing_error > tolerance) {
            let [_, middle, _] = values;
            self.subdivide(sample, a, av, times[1], middle.clone(), depth + 1)?;
            self.subdivide(sample, times[1], middle, b, bv, depth + 1)?;
        } else {
            for i in 1..chain.len() {
                self.total += distance(chain[i - 1], chain[i]);
                self.table
                    .push((if i == 4 { b } else { times[i - 1] }, self.total));
            }
        }
        Ok(())
    }

    /// The path's length.
    pub fn length(&self) -> f64 {
        self.total
    }

    /// `sourceTime(progress)`.
    pub fn source_time(&self, progress: f64) -> Result<f64, String> {
        if !progress.is_finite() {
            return Err("Motion path progress must be finite".into());
        }
        if progress <= 0.0 {
            return Ok(self.start);
        }
        if progress >= 1.0 {
            return Ok(self.end);
        }
        let target = progress * self.total;
        let (mut lo, mut hi) = (0usize, self.table.len() - 1);
        while hi - lo > 1 {
            let mid = (lo + hi) / 2;
            if self.table[mid].1 < target {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let (a, b) = (self.table[lo], self.table[hi]);
        let length = b.1 - a.1;
        Ok(if length > 0.0 {
            a.0 + (b.0 - a.0) * (target - a.1) / length
        } else {
            b.0
        })
    }

    /// `position(progress)`: the original curves at the source time.
    pub fn position(&self, sample: &mut Read<'_>, progress: f64) -> Result<Vec<f64>, String> {
        let time = self.source_time(progress)?;
        read(sample, time)
    }
}

/// `read(time)`: one to three finite numbers.
fn read(sample: &mut Read<'_>, time: f64) -> Result<Vec<f64>, String> {
    let value = sample(time)?;
    if value.is_empty() || value.len() > 3 || !value.iter().all(|v| v.is_finite()) {
        return Err("Motion path requires finite numeric position channels".into());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hypot_matches_v8_edge_cases() {
        assert_eq!(hypot(&[]), 0.0);
        assert_eq!(hypot(&[3.0, 4.0]), 5.0);
        assert_eq!(hypot(&[-2.0]), 2.0);
        assert!(hypot(&[f64::NAN, 1.0]).is_nan());
        assert_eq!(hypot(&[f64::NAN, f64::NEG_INFINITY]), f64::INFINITY);
        assert_eq!(hypot(&[0.0, -0.0]), 0.0);
    }

    #[test]
    fn a_straight_evenly_timed_path_is_linear_in_distance() {
        let mut sample = |t: f64| Ok(vec![t * 100.0, 0.0]);
        let timing = PathTiming::new(&mut sample, &[], 0.0, 2.0).unwrap();
        assert!((timing.length() - 200.0).abs() < 1e-9);
        assert_eq!(timing.source_time(0.5).unwrap(), 1.0);
        assert_eq!(timing.position(&mut sample, 1.0).unwrap(), vec![200.0, 0.0]);
    }

    #[test]
    fn refuses_what_typescript_refuses() {
        let mut still = |_: f64| Ok(vec![5.0]);
        assert_eq!(
            PathTiming::new(&mut still, &[], 0.0, 1.0).unwrap_err(),
            "The selected position path has no movement"
        );
        let mut bad = |_: f64| Ok(vec![f64::NAN]);
        assert_eq!(
            PathTiming::new(&mut bad, &[], 0.0, 1.0).unwrap_err(),
            "Motion path requires finite numeric position channels"
        );
        let mut line = |t: f64| Ok(vec![t]);
        assert_eq!(
            PathTiming::new(&mut line, &[], 1.0, 1.0).unwrap_err(),
            "Motion path needs a finite time range"
        );
    }
}
