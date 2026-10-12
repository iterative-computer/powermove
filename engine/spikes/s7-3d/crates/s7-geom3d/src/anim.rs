//! Animation sampling with three.js r180's semantics, so poses equal today's:
//!
//! * every clip plays at weight 1, looping (`LoopRepeat`), started at time 0 and sampled
//!   with `mixer.setTime(t)`: clip time = t − duration · ⌊t / duration⌋;
//! * interpolants: linear (lerp; quaternions through `Quaternion.slerpFlat`), step
//!   (`DiscreteInterpolant`), and glTF cubic splines (`GLTFCubicSplineInterpolant`, with
//!   quaternions normalized afterwards); keys outside the range clamp to the end values;
//! * clips touching the same property are blended equally in clip order by
//!   `PropertyMixer.accumulate` (lerp or slerp with weight 1/k for the k-th clip);
//! * results are f64, keyframe times and values are f32 (three.js keeps them in
//!   `Float32Array`s and interpolates into the mixer's `Float64Array`).

use crate::model::{Channel, Interpolation, Model, Path};
use glam::{DQuat, DVec3};

/// A node's animated local transform and morph weights at one time.
#[derive(Clone, Debug)]
pub struct Pose {
    pub translation: Vec<DVec3>,
    pub rotation: Vec<DQuat>,
    pub scale: Vec<DVec3>,
    /// Morph weights per node (`None`: the mesh's or node's static weights).
    pub weights: Vec<Option<Vec<f64>>>,
}

/// three.js `Quaternion.slerpFlat` on xyzw arrays.
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

/// Samples one channel at clip time `t` into `out` (`size` values).
fn sample(c: &Channel, t: f64, out: &mut [f64]) {
    let n = c.times.len();
    let size = c.size;
    let quat = c.path == Path::Rotation;
    let key_value = |k: usize, out: &mut [f64]| {
        let base = if c.interpolation == Interpolation::CubicSpline { k * size * 3 + size } else { k * size };
        for i in 0..size {
            out[i] = c.values[base + i] as f64;
        }
    };
    if n == 0 {
        return;
    }
    // Interpolant.evaluate: before the first key and at or after the last key, copy.
    if t < c.times[0] as f64 {
        key_value(0, out);
        return;
    }
    if t >= c.times[n - 1] as f64 {
        // copySampleValue_ copies the key as stored (no normalization, even for splines).
        key_value(n - 1, out);
        return;
    }
    // i1: the first key with time > t.
    let i1 = c.times.partition_point(|&k| (k as f64) <= t);
    let (t0, t1) = (c.times[i1 - 1] as f64, c.times[i1] as f64);
    match c.interpolation {
        Interpolation::Step => key_value(i1 - 1, out),
        Interpolation::Linear => {
            let w1 = (t - t0) / (t1 - t0);
            if quat {
                let v = |k: usize| -> [f64; 4] { std::array::from_fn(|i| c.values[k * 4 + i] as f64) };
                out[..4].copy_from_slice(&slerp_flat(v(i1 - 1), v(i1), w1));
            } else {
                let w0 = 1.0 - w1;
                for i in 0..size {
                    out[i] = c.values[(i1 - 1) * size + i] as f64 * w0 + c.values[i1 * size + i] as f64 * w1;
                }
            }
        }
        Interpolation::CubicSpline => {
            let td = t1 - t0;
            let p = (t - t0) / td;
            let pp = p * p;
            let ppp = pp * p;
            let s2 = -2.0 * ppp + 3.0 * pp;
            let s3 = ppp - pp;
            let s0 = 1.0 - s2;
            let s1 = s3 - pp + p;
            let (o0, o1) = ((i1 - 1) * size * 3, i1 * size * 3);
            for i in 0..size {
                let p0 = c.values[o0 + i + size] as f64;
                let m0 = c.values[o0 + i + 2 * size] as f64 * td;
                let p1 = c.values[o1 + i + size] as f64;
                let m1 = c.values[o1 + i] as f64 * td;
                out[i] = s0 * p0 + s1 * m0 + s2 * p1 + s3 * m1;
            }
            if quat {
                normalize4(out);
            }
        }
    }
}

/// three.js `Quaternion.normalize`.
fn normalize4(v: &mut [f64]) {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2] + v[3] * v[3]).sqrt();
    if l == 0.0 {
        v[..4].copy_from_slice(&[0.0, 0.0, 0.0, 1.0]);
    } else {
        let inv = 1.0 / l;
        for x in v.iter_mut().take(4) {
            *x *= inv;
        }
    }
}

/// The pose of every node at mixer time `time`.
pub fn pose(model: &Model, time: f64) -> Pose {
    let n = model.nodes.len();
    let mut pose = Pose {
        translation: model.nodes.iter().map(|n| n.translation).collect(),
        rotation: model.nodes.iter().map(|n| n.rotation).collect(),
        scale: model.nodes.iter().map(|n| n.scale).collect(),
        weights: vec![None; n],
    };
    // (node, path) → (accumulated value, cumulative weight)
    let mut acc: std::collections::HashMap<(usize, u8), (Vec<f64>, f64)> = std::collections::HashMap::new();
    let mut order: Vec<(usize, u8)> = Vec::new();
    let mut tmp = Vec::new();
    for clip in &model.clips {
        let duration = clip.duration as f64;
        let t = if duration > 0.0 && (time >= duration || time < 0.0) { time - duration * (time / duration).floor() } else { time };
        for c in &clip.channels {
            if c.node >= n {
                continue;
            }
            tmp.clear();
            tmp.resize(c.size, 0.0);
            sample(c, t, &mut tmp);
            let key = (c.node, c.path as u8);
            match acc.get_mut(&key) {
                None => {
                    acc.insert(key, (tmp.clone(), 1.0));
                    order.push(key);
                }
                Some((value, weight)) => {
                    *weight += 1.0;
                    let mix = 1.0 / *weight;
                    if c.path == Path::Rotation {
                        let a: [f64; 4] = std::array::from_fn(|i| value[i]);
                        let b: [f64; 4] = std::array::from_fn(|i| tmp[i]);
                        value[..4].copy_from_slice(&slerp_flat(a, b, mix));
                    } else {
                        let s = 1.0 - mix;
                        for (v, x) in value.iter_mut().zip(&tmp) {
                            *v = *v * s + *x * mix;
                        }
                    }
                }
            }
        }
    }
    for key in order {
        let (value, _) = &acc[&key];
        let node = key.0;
        match key.1 {
            k if k == Path::Translation as u8 => pose.translation[node] = DVec3::new(value[0], value[1], value[2]),
            k if k == Path::Rotation as u8 => pose.rotation[node] = DQuat::from_xyzw(value[0], value[1], value[2], value[3]),
            k if k == Path::Scale as u8 => pose.scale[node] = DVec3::new(value[0], value[1], value[2]),
            _ => pose.weights[node] = Some(value.clone()),
        }
    }
    pose
}
