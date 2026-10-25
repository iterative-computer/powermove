//! The built-in effects (`builtins.json`, the kernel's GLSL) evaluated in
//! f64 from their formulas: each program's arithmetic on premultiplied
//! values, GL's `texture()` (bilinear, clamp to edge, texel centres) and
//! the compositor's ping-pong passes (`u_tex` the previous pass, `u_orig`
//! the effect's input). Written from the GLSL text, never from the
//! engine's translation of it.
//!
//! Where the formula leaves the value open, the reference says so instead
//! of picking one: a Posterize level at a boundary of its input's storage
//! precision is either neighbouring level (E-R3, "nearest level of the
//! reference"), and Gradient's dither adds `hash(v_px)` jitter whose value
//! GLSL ES does not define (`sin` of a large argument), so its output is
//! the range the jitter spans. Grain and Displace are made of that hash
//! noise: Grain's output is the band its noise amplitude allows around the
//! input, Displace's the values its displacement field can reach.

/// A premultiplied RGBA image, rows top-down (`texture()` addresses it
/// in GL row order through [`Image::sample`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<f64>,
}

pub type Rgba = [f64; 4];

impl Image {
    pub fn new(width: usize, height: usize) -> Image {
        Image {
            width,
            height,
            rgba: vec![0.0; width * height * 4],
        }
    }

    pub fn at(&self, x: usize, y: usize) -> Rgba {
        let i = (y * self.width + x) * 4;
        [
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ]
    }

    fn put(&mut self, x: usize, y: usize, c: Rgba) {
        let i = (y * self.width + x) * 4;
        self.rgba[i..i + 4].copy_from_slice(&c);
    }

    /// A texel, clamped to the edge.
    fn texel(&self, x: i64, y: i64) -> Rgba {
        let x = x.clamp(0, self.width as i64 - 1) as usize;
        let y = y.clamp(0, self.height as i64 - 1) as usize;
        self.at(x, y)
    }

    /// GLSL `texture(sampler, st)`: `st` in GL's texture space (y up from
    /// the bottom row), bilinear between texel centres, clamp to edge.
    pub fn sample(&self, st: [f64; 2]) -> Rgba {
        let u = st[0] * self.width as f64 - 0.5;
        let v = (1.0 - st[1]) * self.height as f64 - 0.5;
        let (x0, y0) = (u.floor(), v.floor());
        let (fx, fy) = (u - x0, v - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let a = self.texel(x0, y0);
        let b = self.texel(x0 + 1, y0);
        let c = self.texel(x0, y0 + 1);
        let d = self.texel(x0 + 1, y0 + 1);
        std::array::from_fn(|k| {
            let top = a[k] + (b[k] - a[k]) * fx;
            let bottom = c[k] + (d[k] - c[k]) * fx;
            top + (bottom - top) * fy
        })
    }
}

/// The compositor's coordinates of a pass (`effectViewportCoordinates`):
/// `u_uv` (content-space offset and scale), `u_pxOrigin`, `u_coordRes`,
/// and `u_time` (composition seconds).
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub uv: [f64; 4],
    pub origin: [f64; 2],
    pub resolution: [f64; 2],
    pub time: f64,
}

impl Frame {
    /// A pass over the whole frame of `image`.
    pub fn full(image: &Image, time: f64) -> Frame {
        Frame {
            uv: [0.0, 0.0, 1.0, 1.0],
            origin: [0.0, 0.0],
            resolution: [image.width as f64, image.height as f64],
            time,
        }
    }
}

/// A parameter as the compositor binds it: numbers and toggles as floats,
/// colors as RGB in 0…1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Param {
    Float(f64),
    Color([f64; 3]),
}

impl Param {
    fn float(self) -> f64 {
        match self {
            Param::Float(v) => v,
            Param::Color(_) => 0.0,
        }
    }
    fn color(self) -> [f64; 3] {
        match self {
            Param::Color(c) => c,
            Param::Float(_) => [0.0; 3],
        }
    }
}

/// What the formula leaves open at a pixel: the output is any of these
/// points, or any value in this per-channel range.
#[derive(Clone, Debug, PartialEq)]
pub enum Open {
    Points(Vec<Rgba>),
    Range(Rgba, Rgba),
}

impl Open {
    /// The value the formula allows that is nearest to `engine` (largest
    /// channel difference).
    pub fn nearest(&self, engine: Rgba) -> Rgba {
        match self {
            Open::Range(lo, hi) => std::array::from_fn(|k| engine[k].clamp(lo[k], hi[k])),
            Open::Points(points) => *points
                .iter()
                .min_by(|a, b| distance(a, &engine).total_cmp(&distance(b, &engine)))
                .expect("at least one point"),
        }
    }
}

fn distance(a: &Rgba, b: &Rgba) -> f64 {
    (0..4).map(|k| (a[k] - b[k]).abs()).fold(0.0, f64::max)
}

/// An effect's output: the image, where the formula leaves a pixel open,
/// and the pixels it took from fractional texture positions (resampled).
#[derive(Clone, Debug)]
pub struct Output {
    pub image: Image,
    pub open: Vec<Option<Open>>,
    pub resampled: bool,
}

/// The built-in effects the reference models, with how many passes each
/// runs: all sixteen.
pub fn model(id: &str) -> Option<u8> {
    Some(match id {
        "blur" => 2,
        "glow" | "shadow" => 3,
        "motionblurDir" | "sharpen" | "color" | "levels" | "duotone" | "vignette" | "chroma"
        | "pixelate" | "posterize" | "invert" | "gradient" | "grain" | "displace" => 1,
        _ => return None,
    })
}

/// `precision`: the largest storage error of the input's values (half an
/// 8-bit code at depth 8), which decides where Posterize's level is open.
/// `None` for an effect the reference does not model.
pub fn run(
    id: &str,
    params: &[Param],
    input: &Image,
    frame: &Frame,
    precision: f64,
) -> Option<Output> {
    let p = |i: usize| params.get(i).copied().unwrap_or(Param::Float(0.0));
    let (w, h) = (input.width, input.height);
    let texel = [1.0 / w as f64, 1.0 / h as f64];
    let mut open = vec![None; w * h];
    let mut resampled = false;
    let image = match id {
        "blur" => {
            let r = p(0).float();
            resampled = r >= 0.25;
            let pass = |src: &Image, dir: [f64; 2]| {
                per_pixel(src, |x, y, st| {
                    if r < 0.25 {
                        return src.at(x, y);
                    }
                    let sigma = r * 0.5;
                    let n = (r * 0.6).clamp(3.0, 28.0).trunc();
                    let mut sum = [0.0; 4];
                    let mut wsum = 0.0;
                    for i in -(n as i64)..=(n as i64) {
                        let off = i as f64 / n * r;
                        let wt = (-(off * off) / (2.0 * sigma * sigma + 1e-4)).exp();
                        let c = src.sample([
                            st[0] + dir[0] * texel[0] * off,
                            st[1] + dir[1] * texel[1] * off,
                        ]);
                        for k in 0..4 {
                            sum[k] += c[k] * wt;
                        }
                        wsum += wt;
                    }
                    sum.map(|v| v / wsum.max(1e-4))
                })
            };
            let horizontal = pass(input, [1.0, 0.0]);
            pass(&horizontal, [0.0, 1.0])
        }
        "motionblurDir" => {
            resampled = true;
            let a = p(1).float().to_radians();
            let dir = [
                a.cos() * texel[0] * p(0).float(),
                a.sin() * texel[1] * p(0).float(),
            ];
            per_pixel(input, |_, _, st| {
                let mut s = [0.0; 4];
                for i in 0..17 {
                    let t = i as f64 / 16.0 - 0.5;
                    let c = input.sample([st[0] + dir[0] * t, st[1] + dir[1] * t]);
                    for k in 0..4 {
                        s[k] += c[k];
                    }
                }
                s.map(|v| v / 17.0)
            })
        }
        "sharpen" => per_pixel(input, |_, _, st| {
            let c = input.sample(st);
            let n = [
                input.sample([st[0] + texel[0], st[1]]),
                input.sample([st[0] - texel[0], st[1]]),
                input.sample([st[0], st[1] + texel[1]]),
                input.sample([st[0], st[1] - texel[1]]),
            ];
            let k = p(0).float() / 100.0;
            let mut o = c;
            for ch in 0..3 {
                let b = n.iter().map(|q| q[ch]).sum::<f64>();
                o[ch] = (c[ch] + (c[ch] - b * 0.25) * k).clamp(0.0, 4.0);
            }
            o
        }),
        "glow" => {
            resampled = true;
            let threshold = p(0).float() / 100.0;
            let bright = per_pixel(input, |x, y, _| {
                let c = input.at(x, y);
                let l = luma(&[c[0], c[1], c[2]].map(|v| v / c[3].max(0.001)));
                let k = smoothstep(threshold - 0.08, threshold + 0.08, l);
                c.map(|v| v * k)
            });
            let r = p(1).float().max(0.5);
            let blur = |src: &Image, dir: [f64; 2]| gaussian_pass(src, dir, texel, r, 20);
            let horizontal = blur(&bright, [1.0, 0.0]);
            let light = blur(&horizontal, [0.0, 1.0]);
            let gain = p(2).float() / 100.0;
            per_pixel(input, |x, y, _| {
                let (o, l) = (input.at(x, y), light.at(x, y));
                std::array::from_fn(|k| o[k] + l[k] * gain)
            })
        }
        "color" => unpremultiplied(input, |x, _, _| {
            let mut x = x.map(|v| v * 2f64.powf(p(0).float()));
            x = x.map(|v| (v - 0.5) * (1.0 + p(1).float() / 100.0) + 0.5);
            let l = luma(&x);
            x = x.map(|v| l + (v - l) * (1.0 + p(2).float() / 100.0));
            if p(3).float().abs() > 0.01 {
                let mut hsv = rgb2hsv(x.map(|v| v.max(0.0)));
                hsv[0] = fract(hsv[0] + p(3).float() / 360.0);
                x = hsv2rgb(hsv);
            }
            let t = p(4).float();
            x = [
                x[0] + t * 0.0035,
                x[1] + t * 0.06 * 0.0035,
                x[2] - t * 0.0035,
            ];
            x.map(|v| v.clamp(0.0, 8.0))
        }),
        "levels" => unpremultiplied(input, |x, _, _| {
            let (black, white) = (p(0).float() / 100.0, p(1).float() / 100.0);
            let g = 1.0 / p(2).float().max(0.01);
            x.map(|v| {
                ((v - black) / (white - black).max(1e-3))
                    .clamp(0.0, 1.0)
                    .powf(g)
            })
        }),
        "duotone" => unpremultiplied(input, |x, _, _| {
            let t = smoothstep(0.0, 1.0, luma(&x));
            let (s, hi, amount) = (p(0).color(), p(1).color(), p(2).float() / 100.0);
            std::array::from_fn(|k| {
                let tone = s[k] + (hi[k] - s[k]) * t;
                x[k] + (tone - x[k]) * amount
            })
        }),
        "vignette" => per_pixel(input, |x, y, _| {
            let c = input.at(x, y);
            let uv = v_uv(frame, w, h, x, y);
            let q = [(uv[0] - 0.5) * 2.0, (uv[1] - 0.5) * 2.0];
            let d = q[0].hypot(q[1]) * 0.72;
            let v = 1.0 - smoothstep(1.0 - p(1).float() / 100.0, 1.0, d) * (p(0).float() / 100.0);
            [c[0] * v, c[1] * v, c[2] * v, c[3]]
        }),
        "chroma" => {
            resampled = true;
            per_pixel(input, |x, y, st| {
                let uv = v_uv(frame, w, h, x, y);
                let dir = [uv[0] - 0.5, 1.0 - uv[1] - 0.5];
                let a = p(0).float() * 2.0;
                let d = [dir[0] * texel[0] * a, dir[1] * texel[1] * a];
                let r = input.sample([st[0] + d[0], st[1] + d[1]])[0];
                let g = input.sample(st);
                let b = input.sample([st[0] - d[0], st[1] - d[1]])[2];
                [r, g[1], b, g[3]]
            })
        }
        "pixelate" => {
            resampled = true;
            let block = p(0).float().max(1.0);
            per_pixel(input, |x, y, _| {
                let px = v_px(frame, x, y);
                let local = [px[0] - frame.origin[0], px[1] - frame.origin[1]];
                let center = local.map(|v| ((v / block).floor() + 0.5) * block);
                input.sample([center[0] / w as f64, 1.0 - center[1] / h as f64])
            })
        }
        "posterize" => {
            let n = p(0).float().max(2.0);
            let level = |v: f64| (v * n + 0.5).floor() / n;
            let mut out = Image::new(w, h);
            for y in 0..h {
                for x in 0..w {
                    let c = input.at(x, y);
                    if c[3] < 0.0005 {
                        out.put(x, y, c);
                        continue;
                    }
                    let u = [c[0] / c[3], c[1] / c[3], c[2] / c[3]];
                    out.put(
                        x,
                        y,
                        [
                            level(u[0]) * c[3],
                            level(u[1]) * c[3],
                            level(u[2]) * c[3],
                            c[3],
                        ],
                    );
                    // A level at the boundary of the input's storage
                    // precision may be either neighbour.
                    let spread = precision / c[3];
                    let choices: Vec<[f64; 2]> = u
                        .iter()
                        .map(|v| [level(v - spread), level(v + spread)])
                        .collect();
                    if choices.iter().any(|c| c[0] != c[1]) {
                        let mut points = Vec::new();
                        for r in [choices[0][0], choices[0][1]] {
                            for g in [choices[1][0], choices[1][1]] {
                                for b in [choices[2][0], choices[2][1]] {
                                    points.push([r * c[3], g * c[3], b * c[3], c[3]]);
                                }
                            }
                        }
                        open[y * w + x] = Some(Open::Points(points));
                    }
                }
            }
            out
        }
        "invert" => unpremultiplied(input, |x, _, _| {
            let k = p(0).float() / 100.0;
            x.map(|v| v + (1.0 - v - v) * k)
        }),
        "gradient" => {
            let mut out = Image::new(w, h);
            for y in 0..h {
                for x in 0..w {
                    let c = input.at(x, y);
                    if c[3] < 0.0005 {
                        out.put(x, y, c);
                        continue;
                    }
                    let u = [c[0] / c[3], c[1] / c[3], c[2] / c[3]];
                    let uv = v_uv(frame, w, h, x, y);
                    let at = |t: f64| gradient(params, frame, uv, u, t).map(|v| v * c[3]);
                    let t = gradient_t(params, frame, uv);
                    // `(hash(v_px) - .5) * dither * .004`: hash ∈ [0, 1).
                    let jitter = p(9).float() * 0.002;
                    let samples = [at(t - jitter), at(t), at(t + jitter)];
                    let mid = samples[1];
                    out.put(x, y, [mid[0], mid[1], mid[2], c[3]]);
                    if jitter > 0.0 {
                        let bound = |k: usize, f: fn(f64, f64) -> f64, from: f64| {
                            if k == 3 {
                                c[3]
                            } else {
                                samples.iter().map(|s| s[k]).fold(from, f)
                            }
                        };
                        let lo: Rgba = std::array::from_fn(|k| bound(k, f64::min, f64::INFINITY));
                        let hi: Rgba =
                            std::array::from_fn(|k| bound(k, f64::max, f64::NEG_INFINITY));
                        open[y * w + x] = Some(Open::Range(lo, hi));
                    }
                }
            }
            out
        }
        "grain" => {
            // `n = hash(…) - .5` ∈ [-½, ½); `o = clamp(c.rgb + n·k·c.a, 0, 8)`.
            let k = p(0).float() / 100.0;
            let mut out = Image::new(w, h);
            for y in 0..h {
                for x in 0..w {
                    let c = input.at(x, y);
                    let at = |n: f64| -> Rgba {
                        [
                            (c[0] + n * k * c[3]).clamp(0.0, 8.0),
                            (c[1] + n * k * c[3]).clamp(0.0, 8.0),
                            (c[2] + n * k * c[3]).clamp(0.0, 8.0),
                            c[3],
                        ]
                    };
                    out.put(x, y, at(0.0));
                    if k * c[3] != 0.0 {
                        open[y * w + x] = Some(Open::Range(at(-0.5), at(0.5)));
                    }
                }
            }
            out
        }
        "displace" => {
            // `d = (fbm(p) - .5, fbm(p + 31.7) - .5) · amount · texel · 2`
            // with `fbm` ∈ [0, 31/32): each axis moves by -amount to
            // 15/16 · amount pixels (y up in texture space), and the bilinear
            // sample there lies within the texels one pixel around it.
            resampled = true;
            let a = p(0).float().abs();
            let (back, ahead) = (
                (a + 1.0).ceil() as i64,
                (a * 15.0 / 16.0 + 1.0).ceil() as i64,
            );
            // Texture x moves right by d.x; texture y up by d.y, which is
            // image rows above: rows y - ahead … y + back.
            let lo = window(input, back, ahead, ahead, back, f64::min);
            let hi = window(input, back, ahead, ahead, back, f64::max);
            for (i, o) in open.iter_mut().enumerate() {
                let (l, u) = (lo.at(i % w, i / w), hi.at(i % w, i / w));
                if l != u {
                    *o = Some(Open::Range(l, u));
                }
            }
            input.clone()
        }
        "shadow" => {
            resampled = true;
            let a = p(1).float().to_radians();
            let off = [
                a.cos() * p(0).float() * texel[0],
                -a.sin() * p(0).float() * texel[1],
            ];
            let cast = per_pixel(input, |_, _, st| {
                [
                    0.0,
                    0.0,
                    0.0,
                    input.sample([st[0] - off[0], st[1] - off[1]])[3],
                ]
            });
            let r = p(2).float().max(0.5);
            let horizontal = gaussian_pass(&cast, [1.0, 0.0], texel, r, 18);
            let shade = gaussian_pass(&horizontal, [0.0, 1.0], texel, r, 18);
            let (opacity, color) = (p(3).float() / 100.0, p(4).color());
            per_pixel(input, |x, y, _| {
                let o = input.at(x, y);
                let sa = shade.at(x, y)[3] * opacity;
                [
                    o[0] + color[0] * sa * (1.0 - o[3]),
                    o[1] + color[1] * sa * (1.0 - o[3]),
                    o[2] + color[2] * sa * (1.0 - o[3]),
                    o[3] + sa * (1.0 - o[3]),
                ]
            })
        }
        _ => return None,
    };
    Some(Output {
        image,
        open,
        resampled,
    })
}

/// The per-channel minimum or maximum (`f`) of the texels from `left`
/// columns left to `right` right and `up` rows above to `down` below each
/// pixel, clamped to the image (clamp-to-edge sampling).
fn window(src: &Image, left: i64, right: i64, up: i64, down: i64, f: fn(f64, f64) -> f64) -> Image {
    let (w, h) = (src.width as i64, src.height as i64);
    let mut rows = Image::new(src.width, src.height);
    for y in 0..h {
        for x in 0..w {
            let mut acc = src.texel(x, y);
            for dx in (x - left).max(0)..=(x + right).min(w - 1) {
                let t = src.texel(dx, y);
                acc = std::array::from_fn(|k| f(acc[k], t[k]));
            }
            rows.put(x as usize, y as usize, acc);
        }
    }
    let mut out = Image::new(src.width, src.height);
    for y in 0..h {
        for x in 0..w {
            let mut acc = rows.texel(x, y);
            for dy in (y - up).max(0)..=(y + down).min(h - 1) {
                let t = rows.texel(x, dy);
                acc = std::array::from_fn(|k| f(acc[k], t[k]));
            }
            out.put(x as usize, y as usize, acc);
        }
    }
    out
}

/// One pass of Glow's or Shadow's blur: `2 taps + 1` samples at
/// `i / taps · r` texels along `dir`, Gaussian weights with σ = r / 2.
fn gaussian_pass(src: &Image, dir: [f64; 2], texel: [f64; 2], r: f64, taps: i64) -> Image {
    let sigma = r * 0.5;
    per_pixel(src, |_, _, st| {
        let mut sum = [0.0; 4];
        let mut ws = 0.0;
        for i in -taps..=taps {
            let x = i as f64 / taps as f64 * r;
            let wt = (-(x * x) / (2.0 * sigma * sigma + 1e-4)).exp();
            let c = src.sample([st[0] + dir[0] * texel[0] * x, st[1] + dir[1] * texel[1] * x]);
            for k in 0..4 {
                sum[k] += c[k] * wt;
            }
            ws += wt;
        }
        sum.map(|v| v / ws.max(1e-4))
    })
}

/// Gradient's `t` before the dither (the formula's lines up to the jitter).
fn gradient_t(params: &[Param], frame: &Frame, uv: [f64; 2]) -> f64 {
    let p = |i: usize| params.get(i).copied().unwrap_or(Param::Float(0.0)).float();
    let (angle, cx, cy, spread, radial) = (p(2), p(3), p(4), p(5), p(7));
    let q = [uv[0] - 0.5 - cx / 200.0, 1.0 - uv[1] - 0.5 + cy / 200.0];
    let span = (spread / 100.0).max(0.001);
    if radial > 0.5 {
        let aspect = frame.resolution[0] / frame.resolution[1].max(1.0);
        (q[0] * aspect).hypot(q[1]) * 2.0 / span
    } else {
        let a = angle.to_radians();
        (q[0] * a.cos() - q[1] * a.sin()) / span + 0.5
    }
}

/// Gradient's straight output color at `t` (before the dither's clamp).
fn gradient(params: &[Param], _frame: &Frame, _uv: [f64; 2], x: [f64; 3], t: f64) -> [f64; 3] {
    let p = |i: usize| params.get(i).copied().unwrap_or(Param::Float(0.0));
    let t = t.clamp(0.0, 1.0);
    let t = t.powf(0.5f64.ln() / (p(6).float() / 100.0).clamp(0.01, 0.99).ln());
    let (start, end) = (p(0).color(), p(1).color());
    let mut g: [f64; 3] = std::array::from_fn(|k| start[k] + (end[k] - start[k]) * t);
    if p(8).float() > 0.5 {
        let hsv = rgb2hsv(g);
        g = hsv2rgb([hsv[0], hsv[1], rgb2hsv(x.map(|v| v.max(0.0)))[2]]);
    }
    let amount = p(10).float() / 100.0;
    std::array::from_fn(|k| x[k] + (g[k] - x[k]) * amount)
}

/// `v_uv` at pixel (x, y): content space, top-left origin.
fn v_uv(frame: &Frame, w: usize, h: usize, x: usize, y: usize) -> [f64; 2] {
    [
        frame.uv[0] + (x as f64 + 0.5) / w as f64 * frame.uv[2],
        frame.uv[1] + (y as f64 + 0.5) / h as f64 * frame.uv[3],
    ]
}

/// `v_px` at pixel (x, y): composition pixels.
fn v_px(frame: &Frame, x: usize, y: usize) -> [f64; 2] {
    [
        x as f64 + 0.5 + frame.origin[0],
        y as f64 + 0.5 + frame.origin[1],
    ]
}

/// A pass computing every pixel from its position and `v_st`.
fn per_pixel(src: &Image, f: impl Fn(usize, usize, [f64; 2]) -> Rgba) -> Image {
    let (w, h) = (src.width, src.height);
    let mut out = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let st = [
                (x as f64 + 0.5) / w as f64,
                1.0 - (y as f64 + 0.5) / h as f64,
            ];
            out.put(x, y, f(x, y, st));
        }
    }
    out
}

/// The color effects' shape: below alpha 0.0005 the texel is kept; else
/// the straight color is transformed and premultiplied again.
fn unpremultiplied(src: &Image, f: impl Fn([f64; 3], usize, usize) -> [f64; 3]) -> Image {
    per_pixel(src, |x, y, _| {
        let c = src.at(x, y);
        if c[3] < 0.0005 {
            return c;
        }
        let o = f([c[0] / c[3], c[1] / c[3], c[2] / c[3]], x, y);
        [o[0] * c[3], o[1] * c[3], o[2] * c[3], c[3]]
    })
}

/// The prelude's `luma`: Rec. 709 weights.
pub fn luma(c: &[f64; 3]) -> f64 {
    c[0] * 0.2126 + c[1] * 0.7152 + c[2] * 0.0722
}

/// GLSL `smoothstep`.
pub fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn fract(x: f64) -> f64 {
    x - x.floor()
}

/// GLSL `mod`.
fn modulo(x: f64, y: f64) -> f64 {
    x - y * (x / y).floor()
}

/// The prelude's `rgb2hsv` (the branchless form with `e = 1e-10`).
pub fn rgb2hsv(c: [f64; 3]) -> [f64; 3] {
    let k = [0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0];
    let p = if c[1] >= c[2] {
        [c[1], c[2], k[0], k[1]]
    } else {
        [c[2], c[1], k[3], k[2]]
    };
    let q = if c[0] >= p[0] {
        [c[0], p[1], p[2], p[0]]
    } else {
        [p[0], p[1], p[3], c[0]]
    };
    let d = q[0] - q[3].min(q[1]);
    let e = 1e-10;
    [
        (q[2] + (q[3] - q[1]) / (6.0 * d + e)).abs(),
        d / (q[0] + e),
        q[0],
    ]
}

/// The prelude's `hsv2rgb`.
pub fn hsv2rgb(c: [f64; 3]) -> [f64; 3] {
    let offsets = [0.0, 4.0, 2.0];
    std::array::from_fn(|k| {
        let rgb = ((modulo(c[0] * 6.0 + offsets[k], 6.0) - 3.0).abs() - 1.0).clamp(0.0, 1.0);
        c[2] * (1.0 + (rgb - 1.0) * c[1])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: usize, h: usize, c: Rgba) -> Image {
        let mut i = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                i.put(x, y, c);
            }
        }
        i
    }

    #[test]
    fn sampling_is_bilinear_between_texel_centres_in_gl_rows() {
        let mut i = Image::new(2, 2);
        i.put(0, 0, [1.0, 0.0, 0.0, 1.0]); // top-left
        i.put(0, 1, [0.0, 0.0, 1.0, 1.0]); // bottom-left
        // GL st (0.25, 0.75) is the top-left texel's centre.
        assert_eq!(i.sample([0.25, 0.75]), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(i.sample([0.25, 0.25]), [0.0, 0.0, 1.0, 1.0]);
        let mid = i.sample([0.25, 0.5]);
        assert_eq!(mid, [0.5, 0.0, 0.5, 1.0]);
        // Clamped beyond the edge.
        assert_eq!(i.sample([-3.0, 0.75]), [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn a_flat_image_is_a_fixed_point_of_every_blur() {
        let c = [0.2, 0.3, 0.4, 0.5];
        let img = flat(9, 7, c);
        let frame = Frame::full(&img, 0.0);
        for (id, params) in [
            ("blur", vec![Param::Float(6.0)]),
            (
                "motionblurDir",
                vec![Param::Float(12.0), Param::Float(30.0)],
            ),
            ("sharpen", vec![Param::Float(80.0)]),
            ("chroma", vec![Param::Float(4.0)]),
            ("pixelate", vec![Param::Float(3.0)]),
        ] {
            let out = run(id, &params, &img, &frame, 0.0).unwrap();
            for v in out.image.rgba.chunks_exact(4) {
                for k in 0..4 {
                    assert!((v[k] - c[k]).abs() < 1e-12, "{id}: {v:?}");
                }
            }
        }
    }

    #[test]
    fn hsv_round_trips_and_matches_the_definition() {
        for c in [
            [0.9, 0.2, 0.1],
            [0.1, 0.8, 0.3],
            [0.2, 0.3, 0.7],
            [0.5, 0.5, 0.5],
        ] {
            let hsv = rgb2hsv(c);
            let back = hsv2rgb(hsv);
            for k in 0..3 {
                assert!((back[k] - c[k]).abs() < 1e-8, "{c:?} {back:?}");
            }
        }
        // Pure red: hue 0, full saturation, value 1.
        let red = rgb2hsv([1.0, 0.0, 0.0]);
        assert!(red[0].abs() < 1e-9 && (red[1] - 1.0).abs() < 1e-9 && red[2] == 1.0);
    }

    #[test]
    fn posterize_levels_at_a_boundary_are_open() {
        // Six levels: 0.25 is exactly between levels 1/6 and 2/6 (x·n + ½ = 2).
        let img = flat(1, 1, [0.25, 0.1, 0.9, 1.0]);
        let frame = Frame::full(&img, 0.0);
        let out = run("posterize", &[Param::Float(6.0)], &img, &frame, 0.5 / 255.0).unwrap();
        let Some(Open::Points(points)) = &out.open[0] else {
            panic!("a boundary value is open: {:?}", out.open[0]);
        };
        assert!(points.iter().any(|p| (p[0] - 1.0 / 6.0).abs() < 1e-12));
        assert!(points.iter().any(|p| (p[0] - 2.0 / 6.0).abs() < 1e-12));
        assert_eq!(
            Open::Points(points.clone()).nearest([0.17, 0.0, 1.0, 1.0])[0],
            1.0 / 6.0
        );
        // Away from a boundary the level is exact.
        let img = flat(1, 1, [0.4, 0.1, 0.9, 1.0]);
        let out = run("posterize", &[Param::Float(6.0)], &img, &frame, 0.5 / 255.0).unwrap();
        assert!(out.open[0].is_none());
    }

    #[test]
    fn grain_and_displace_are_the_ranges_their_noise_allows() {
        let mut img = flat(9, 9, [0.5, 0.4, 0.3, 1.0]);
        img.put(4, 4, [1.0, 1.0, 1.0, 1.0]);
        let frame = Frame::full(&img, 0.0);
        let grain = run(
            "grain",
            &[Param::Float(20.0), Param::Float(1.0)],
            &img,
            &frame,
            0.0,
        )
        .unwrap();
        let Some(Open::Range(lo, hi)) = &grain.open[0] else {
            panic!()
        };
        assert!(
            (lo[0] - 0.4).abs() < 1e-12 && (hi[0] - 0.6).abs() < 1e-12,
            "{lo:?} {hi:?}"
        );
        let displace = run(
            "displace",
            &[Param::Float(1.0), Param::Float(1.0), Param::Float(0.0)],
            &img,
            &frame,
            0.0,
        )
        .unwrap();
        // The bright texel is reachable from its neighbours, not from afar.
        assert!(matches!(&displace.open[3 * 9 + 3], Some(Open::Range(_, hi)) if hi[0] == 1.0));
        assert!(
            displace.open[0].is_none()
                || matches!(&displace.open[0], Some(Open::Range(_, hi)) if hi[0] < 1.0)
        );
    }

    #[test]
    fn glow_adds_its_light_over_the_input() {
        // A uniform bright image: the light equals the input (k = 1), so
        // the output is the input times 1 + intensity, alpha included.
        let img = flat(8, 8, [0.9, 0.9, 0.9, 1.0]);
        let frame = Frame::full(&img, 0.0);
        let out = run(
            "glow",
            &[Param::Float(10.0), Param::Float(4.0), Param::Float(150.0)],
            &img,
            &frame,
            0.0,
        )
        .unwrap();
        let px = out.image.at(4, 4);
        assert!(
            (px[0] - 0.9 * 2.5).abs() < 1e-9 && (px[3] - 2.5).abs() < 1e-9,
            "{px:?}"
        );
    }

    #[test]
    fn shadow_casts_below_and_right_at_its_angle() {
        // An opaque dot at the centre; at 0° the shadow falls to the right.
        let mut img = Image::new(21, 21);
        img.put(10, 10, [1.0, 1.0, 1.0, 1.0]);
        let frame = Frame::full(&img, 0.0);
        let params = [
            Param::Float(4.0),
            Param::Float(0.0),
            Param::Float(0.5),
            Param::Float(100.0),
            Param::Color([0.0; 3]),
        ];
        let out = run("shadow", &params, &img, &frame, 0.0).unwrap();
        assert!(out.image.at(14, 10)[3] > 0.5, "{:?}", out.image.at(14, 10));
        assert!(out.image.at(6, 10)[3] < 1e-6);
    }
}
