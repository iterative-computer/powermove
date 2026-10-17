//! three.js r180's `AnimationMixer` as today's runtime drives it: every
//! clip of the file `clipAction(clip).play()`ed at weight 1, looping
//! (`LoopRepeat`), and sampled with `mixer.setTime(t)`.
//!
//! * Clip time: `t − duration · ⌊t / duration⌋` once `t ≥ duration` (a clip
//!   of zero duration gives NaN, as in JavaScript, and its interpolants
//!   return their last key).
//! * Interpolants: `LinearInterpolant` (quaternions through
//!   `Quaternion.slerpFlat`), `DiscreteInterpolant`, and glTF's
//!   `GLTFCubicSplineInterpolant` (quaternions normalized), with
//!   `Interpolant.evaluate`'s interval search and end-value copies.
//! * Blending: `PropertyMixer.accumulate` per bound property in clip
//!   order, lerp (slerp for quaternions) with weight 1 / k for the k-th
//!   clip; with a cumulative weight of at least 1 nothing of the original
//!   value remains.
//! * Times and values are f32 (`Float32Array`), results f64 (the mixer's
//!   `Float64Array`).

use super::{Clip, Interpolation, Model, Property, Track};

/// `Quaternion.slerpFlat` on `[x, y, z, w]`.
pub fn slerp_flat(a: [f64; 4], b: [f64; 4], t: f64) -> [f64; 4] {
    let [mut x0, mut y0, mut z0, mut w0] = a;
    let [x1, y1, z1, w1] = b;
    if t == 0.0 {
        return a;
    }
    if t == 1.0 {
        return b;
    }
    if w0 != w1 || x0 != x1 || y0 != y1 || z0 != z1 {
        let mut s = 1.0 - t;
        let mut t = t;
        let cos = x0 * x1 + y0 * y1 + z0 * z1 + w0 * w1;
        let dir = if cos >= 0.0 { 1.0 } else { -1.0 };
        let sqr_sin = 1.0 - cos * cos;
        if sqr_sin > f64::EPSILON {
            let sin = sqr_sin.sqrt();
            let len = libm::atan2(sin, cos * dir);
            s = libm::sin(s * len) / sin;
            t = libm::sin(t * len) / sin;
        }
        let t_dir = t * dir;
        x0 = x0 * s + x1 * t_dir;
        y0 = y0 * s + y1 * t_dir;
        z0 = z0 * s + z1 * t_dir;
        w0 = w0 * s + w1 * t_dir;
        if s == 1.0 - t {
            let f = 1.0 / (x0 * x0 + y0 * y0 + z0 * z0 + w0 * w0).sqrt();
            x0 *= f;
            y0 *= f;
            z0 *= f;
            w0 *= f;
        }
    }
    [x0, y0, z0, w0]
}

/// The value of key `k` (for cubic splines the value between the
/// tangents).
fn key(track: &Track, size: usize, k: usize, out: &mut [f64]) {
    let base = if track.interpolation == Interpolation::CubicSpline {
        k * size * 3 + size
    } else {
        k * size
    };
    for (i, o) in out.iter_mut().enumerate().take(size) {
        *o = f64::from(track.values.get(base + i).copied().unwrap_or(0.0));
    }
}

/// `Interpolant.evaluate(t)` from a fresh interpolant (`_cachedIndex` 0).
pub fn evaluate(track: &Track, t: f64, out: &mut [f64]) {
    let size = if track.interpolation == Interpolation::CubicSpline {
        track.value_size / 3
    } else {
        track.value_size
    };
    let pp = &track.times;
    let n = pp.len();
    if n == 0 || size == 0 {
        return;
    }
    let at = |i: usize| -> Option<f64> { pp.get(i).map(|v| f64::from(*v)) };
    // Forward scan from index 0 (`t1 = pp[0]`, `t0 = pp[-1]` undefined).
    let mut i1 = 0usize;
    let mut t1 = at(0);
    let mut t0: Option<f64> = None;
    let found;
    'seek: {
        let right;
        'linear: {
            if !t1.is_some_and(|t1| t < t1) {
                let give_up = i1 + 2;
                loop {
                    let Some(t1v) = t1 else {
                        if t0.is_some_and(|t0| t < t0) {
                            break;
                        }
                        key(track, size, n - 1, out);
                        return;
                    };
                    if i1 == give_up {
                        break;
                    }
                    t0 = Some(t1v);
                    i1 += 1;
                    t1 = at(i1);
                    if t1.is_some_and(|t1| t < t1) {
                        found = i1;
                        break 'seek;
                    }
                }
                right = n;
                break 'linear;
            }
            // `!(t >= t0)` with t0 undefined: before the first key.
            key(track, size, 0, out);
            return;
        }
        // Binary search in [i1, right).
        let mut lo = i1;
        let mut hi = right;
        while lo < hi {
            let mid = (lo + hi) >> 1;
            if t < f64::from(pp[mid]) {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        if lo == 0 {
            key(track, size, 0, out);
            return;
        }
        if lo >= n {
            key(track, size, n - 1, out);
            return;
        }
        found = lo;
    }
    interpolate(track, size, found, t, out);
}

fn interpolate(track: &Track, size: usize, i1: usize, t: f64, out: &mut [f64]) {
    let t0 = f64::from(track.times[i1 - 1]);
    let t1 = f64::from(track.times[i1]);
    let v = |i: usize| f64::from(track.values.get(i).copied().unwrap_or(0.0));
    let quaternion = track.property == Property::Quaternion;
    match track.interpolation {
        Interpolation::Step => key(track, size, i1 - 1, out),
        Interpolation::Linear => {
            let alpha = (t - t0) / (t1 - t0);
            if quaternion && size == 4 {
                let a = [
                    v((i1 - 1) * 4),
                    v((i1 - 1) * 4 + 1),
                    v((i1 - 1) * 4 + 2),
                    v((i1 - 1) * 4 + 3),
                ];
                let b = [v(i1 * 4), v(i1 * 4 + 1), v(i1 * 4 + 2), v(i1 * 4 + 3)];
                out[..4].copy_from_slice(&slerp_flat_interpolant(a, b, alpha));
            } else {
                let w1 = alpha;
                let w0 = 1.0 - w1;
                for (i, o) in out.iter_mut().enumerate().take(size) {
                    *o = v((i1 - 1) * size + i) * w0 + v(i1 * size + i) * w1;
                }
            }
        }
        Interpolation::CubicSpline => {
            let stride2 = size * 2;
            let stride3 = size * 3;
            let td = t1 - t0;
            let p = (t - t0) / td;
            let pp = p * p;
            let ppp = pp * p;
            let offset1 = i1 * stride3;
            let offset0 = offset1 - stride3;
            let s2 = -2.0 * ppp + 3.0 * pp;
            let s3 = ppp - pp;
            let s0 = 1.0 - s2;
            let s1 = s3 - pp + p;
            for (i, o) in out.iter_mut().enumerate().take(size) {
                let p0 = v(offset0 + i + size);
                let m0 = v(offset0 + i + stride2) * td;
                let p1 = v(offset1 + i + size);
                let m1 = v(offset1 + i) * td;
                *o = s0 * p0 + s1 * m0 + s2 * p1 + s3 * m1;
            }
            if quaternion && size == 4 {
                let q = super::math::normalize_quat([out[0], out[1], out[2], out[3]]);
                out[..4].copy_from_slice(&q);
            }
        }
    }
}

/// `QuaternionLinearInterpolant`: `Quaternion.slerpFlat(result, 0, values,
/// offset0, values, offset1, alpha)`, r180's (with the short-path flip and
/// the near-parallel normalized lerp).
fn slerp_flat_interpolant(a: [f64; 4], b: [f64; 4], t: f64) -> [f64; 4] {
    slerp_flat(a, b, t)
}

/// Mixer output for one bound property.
#[derive(Debug, Clone, PartialEq)]
pub struct Animated {
    pub object: usize,
    pub property: Property,
    pub value: Vec<f64>,
}

/// The clip-local time of `time` for a looping clip.
pub fn clip_time(clip: &Clip, time: f64) -> f64 {
    let d = clip.duration;
    if time == 0.0 {
        return 0.0;
    }
    if time >= d || time < 0.0 {
        time - d * (time / d).floor()
    } else {
        time
    }
}

/// `mixer.setTime(time)` with every clip playing: the blended value of
/// every bound property, in binding order.
pub fn sample(model: &Model, time: f64) -> Vec<Animated> {
    // Bindings by track name (`PropertyMixer`s shared by name).
    let mut order: Vec<String> = Vec::new();
    let mut acc: std::collections::HashMap<String, (Animated, f64)> =
        std::collections::HashMap::new();
    let mut tmp = Vec::new();
    for clip in &model.clips {
        let t = clip_time(clip, time);
        for track in &clip.tracks {
            let Some(object) = track.target else {
                continue;
            };
            let size = if track.interpolation == Interpolation::CubicSpline {
                track.value_size / 3
            } else {
                track.value_size
            };
            tmp.clear();
            tmp.resize(size, 0.0);
            evaluate(track, t, &mut tmp);
            match acc.get_mut(&track.name) {
                None => {
                    order.push(track.name.clone());
                    acc.insert(
                        track.name.clone(),
                        (
                            Animated {
                                object,
                                property: track.property,
                                value: tmp.clone(),
                            },
                            1.0,
                        ),
                    );
                }
                Some((a, weight)) => {
                    *weight += 1.0;
                    let mix = 1.0 / *weight;
                    if track.property == Property::Quaternion
                        && a.value.len() >= 4
                        && tmp.len() >= 4
                    {
                        let x = [a.value[0], a.value[1], a.value[2], a.value[3]];
                        let y = [tmp[0], tmp[1], tmp[2], tmp[3]];
                        a.value[..4].copy_from_slice(&slerp_flat(x, y, mix));
                    } else {
                        let s = 1.0 - mix;
                        for (v, x) in a.value.iter_mut().zip(&tmp) {
                            *v = *v * s + *x * mix;
                        }
                    }
                }
            }
        }
    }
    order
        .into_iter()
        .filter_map(|n| acc.remove(&n).map(|(a, _)| a))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(times: &[f32], values: &[f32], size: usize, i: Interpolation) -> Track {
        Track {
            name: "n.position".into(),
            target: Some(0),
            property: Property::Position,
            interpolation: i,
            times: times.to_vec(),
            values: values.to_vec(),
            value_size: if i == Interpolation::CubicSpline {
                size * 3
            } else {
                size
            },
        }
    }

    #[test]
    fn evaluate_clamps_and_interpolates() {
        let t = track(
            &[0.0, 1.0, 2.0],
            &[0.0, 10.0, 30.0],
            1,
            Interpolation::Linear,
        );
        let mut o = [0.0];
        evaluate(&t, -1.0, &mut o);
        assert_eq!(o[0], 0.0);
        evaluate(&t, 0.5, &mut o);
        assert_eq!(o[0], 5.0);
        evaluate(&t, 1.5, &mut o);
        assert_eq!(o[0], 20.0);
        evaluate(&t, 2.0, &mut o);
        assert_eq!(o[0], 30.0);
        evaluate(&t, 9.0, &mut o);
        assert_eq!(o[0], 30.0);
        let s = track(&[0.0, 1.0, 2.0], &[0.0, 10.0, 30.0], 1, Interpolation::Step);
        evaluate(&s, 1.99, &mut o);
        assert_eq!(o[0], 10.0);
        // Many keys: the binary search path.
        let times: Vec<f32> = (0..50).map(|k| k as f32).collect();
        let values: Vec<f32> = (0..50).map(|k| (k * 2) as f32).collect();
        let m = track(&times, &values, 1, Interpolation::Linear);
        evaluate(&m, 37.25, &mut o);
        assert_eq!(o[0], 74.5);
    }

    #[test]
    fn single_key_and_nan_times_copy_the_last_key() {
        let t = track(&[0.0], &[4.0], 1, Interpolation::Linear);
        let mut o = [0.0];
        evaluate(&t, f64::NAN, &mut o);
        assert_eq!(o[0], 4.0);
        let clip = Clip {
            name: String::new(),
            tracks: vec![],
            duration: 0.0,
        };
        assert!(clip_time(&clip, 1.0).is_nan());
        assert_eq!(clip_time(&clip, 0.0), 0.0);
    }

    #[test]
    fn slerp_endpoints() {
        let a = [0.0, 0.0, 0.0, 1.0];
        let b = [
            0.0,
            0.0,
            std::f64::consts::FRAC_1_SQRT_2,
            std::f64::consts::FRAC_1_SQRT_2,
        ];
        assert_eq!(slerp_flat(a, b, 0.0), a);
        assert_eq!(slerp_flat(a, b, 1.0), b);
        let m = slerp_flat(a, b, 0.5);
        let l = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2] + m[3] * m[3]).sqrt();
        assert!((l - 1.0).abs() < 1e-12);
    }
}
