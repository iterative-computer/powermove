//! Apply Color LUT against an independent reference: a plain `.cube` reader
//! and tetrahedral lookup written here, checked against FFmpeg's `lut3d` and
//! `lut1d` filters on the same pixels, then against the rendered result at
//! every project depth.
//!
//! Stated rules. Interpolation is tetrahedral (as DaVinci Resolve's default
//! and OpenColorIO's). An input outside the table's domain is clamped into it
//! for the lookup and the table's own values are kept as written, above 1 or
//! below 0 included (Resolve and FFmpeg do the same; neither extrapolates).
//! A table whose rows are the grid itself returns the pixel's own bits.
mod support;

use pm_protocol::{
    color::Precision,
    common::{Size, Ticks},
};
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::{Value, json};
use support::doc::*;

/// The reference: nothing here comes from `pm_color`.
mod reference {
    #[derive(Default)]
    pub struct Lut {
        pub min: [f64; 3],
        pub max: [f64; 3],
        pub shaper: Vec<[f64; 3]>,
        pub size: usize,
        pub cube: Vec<[f64; 3]>,
        pub cube_range: [f64; 2],
    }

    pub fn read(text: &str) -> Lut {
        let mut lut = Lut {
            max: [1.; 3],
            cube_range: [0., 1.],
            ..Lut::default()
        };
        let (mut one, mut three) = (0, 0);
        let mut one_range = None;
        let mut three_range = None;
        let mut rows = vec![];
        for line in text.lines() {
            let fields: Vec<&str> = line.split('#').next().unwrap().split_whitespace().collect();
            let number = |i: usize| fields[i].parse::<f64>().unwrap();
            match fields.first().copied() {
                None | Some("TITLE") => {}
                Some("LUT_1D_SIZE") => one = fields[1].parse().unwrap(),
                Some("LUT_3D_SIZE") => three = fields[1].parse().unwrap(),
                Some("DOMAIN_MIN") => lut.min = [number(1), number(2), number(3)],
                Some("DOMAIN_MAX") => lut.max = [number(1), number(2), number(3)],
                Some("LUT_1D_INPUT_RANGE") => one_range = Some([number(1), number(2)]),
                Some("LUT_3D_INPUT_RANGE") => three_range = Some([number(1), number(2)]),
                Some(_) => rows.push([number(0), number(1), number(2)]),
            }
        }
        assert_eq!(rows.len(), one + three * three * three);
        lut.cube = rows.split_off(one);
        lut.shaper = rows;
        lut.size = three;
        // The first table's input range is the file's domain.
        if let Some([lo, hi]) = one_range.or(if one == 0 { three_range } else { None }) {
            lut.min = [lo; 3];
            lut.max = [hi; 3];
        }
        if one > 0
            && let Some(range) = three_range
        {
            lut.cube_range = range;
        }
        lut
    }

    fn unit(v: f64, lo: f64, hi: f64) -> f64 {
        ((v - lo) / (hi - lo)).clamp(0., 1.)
    }

    /// One channel of a 1D table, linear between rows.
    fn one(table: &[[f64; 3]], channel: usize, v: f64) -> f64 {
        let x = v * (table.len() - 1) as f64;
        let lo = (x.floor() as usize).min(table.len() - 2);
        let t = x - lo as f64;
        table[lo][channel] + (table[lo + 1][channel] - table[lo][channel]) * t
    }

    /// FFmpeg's `interp_tetrahedral`, in f64: the six tetrahedra that share
    /// the cell's black-to-white diagonal.
    fn tetrahedral(lut: &Lut, p: [f64; 3]) -> [f64; 3] {
        let n = lut.size;
        let scaled = p.map(|v| v * (n - 1) as f64);
        let prev = scaled.map(|v| (v.floor() as usize).min(n - 2));
        let d: [f64; 3] = std::array::from_fn(|c| scaled[c] - prev[c] as f64);
        let at = |r: usize, g: usize, b: usize| {
            lut.cube[(prev[0] + r) + n * ((prev[1] + g) + n * (prev[2] + b))]
        };
        let (c000, c111) = (at(0, 0, 0), at(1, 1, 1));
        let mix = |w: [f64; 4], a: [f64; 3], b: [f64; 3]| -> [f64; 3] {
            std::array::from_fn(|c| w[0] * c000[c] + w[1] * a[c] + w[2] * b[c] + w[3] * c111[c])
        };
        let (r, g, b) = (d[0], d[1], d[2]);
        if r > g {
            if g > b {
                mix([1. - r, r - g, g - b, b], at(1, 0, 0), at(1, 1, 0))
            } else if r > b {
                mix([1. - r, r - b, b - g, g], at(1, 0, 0), at(1, 0, 1))
            } else {
                mix([1. - b, b - r, r - g, g], at(0, 0, 1), at(1, 0, 1))
            }
        } else if b > g {
            mix([1. - b, b - g, g - r, r], at(0, 0, 1), at(0, 1, 1))
        } else if b > r {
            mix([1. - g, g - b, b - r, r], at(0, 1, 0), at(0, 1, 1))
        } else {
            mix([1. - g, g - r, r - b, b], at(0, 1, 0), at(1, 1, 0))
        }
    }

    pub fn apply(lut: &Lut, rgb: [f64; 3]) -> [f64; 3] {
        let mut p: [f64; 3] = std::array::from_fn(|c| unit(rgb[c], lut.min[c], lut.max[c]));
        if !lut.shaper.is_empty() {
            p = std::array::from_fn(|c| one(&lut.shaper, c, p[c]));
            if lut.size == 0 {
                return p;
            }
            p = p.map(|v| unit(v, lut.cube_range[0], lut.cube_range[1]));
        }
        tetrahedral(lut, p)
    }
}

const W: u32 = 64;
const H: u32 = 64;

/// Rows of a table of `count` grid points per axis: a grade that mixes
/// channels (so trilinear and tetrahedral disagree) and leaves 0..1.
fn graded(r: f64, g: f64, b: f64) -> [f64; 3] {
    [
        1.15 * (r * (0.35 + 0.65 * g)).sqrt() - 0.05,
        0.5 * g + 0.6 * b * b - 0.1 * r * b,
        (b + 0.3 * r * (1.0 - g)).powf(0.8) * 1.1,
    ]
}

fn cube_rows(size: usize, grade: impl Fn(f64, f64, f64) -> [f64; 3]) -> String {
    let at = |i: usize| i as f64 / (size - 1) as f64;
    let mut text = String::new();
    for b in 0..size {
        for g in 0..size {
            for r in 0..size {
                let v = grade(at(r), at(g), at(b));
                text += &format!("{:.9} {:.9} {:.9}\n", v[0], v[1], v[2]);
            }
        }
    }
    text
}

fn curve_rows(size: usize, curve: impl Fn(f64) -> [f64; 3]) -> String {
    (0..size)
        .map(|i| {
            let v = curve(i as f64 / (size - 1) as f64);
            format!("{:.9} {:.9} {:.9}\n", v[0], v[1], v[2])
        })
        .collect()
}

/// A camera-like shaper: more rows for the shadows.
fn shaper(v: f64) -> [f64; 3] {
    [v.powf(0.45), v.powf(0.5), v.sqrt() * 0.9 + 0.1 * v]
}

/// The tables a colourist brings: cubes of 17, 33 and 65, each domain
/// convention, a 1D table alone, and a 1D shaper ahead of a cube.
fn tables() -> Vec<(&'static str, String)> {
    let cube = |size| format!("LUT_3D_SIZE {size}\n{}", cube_rows(size, graded));
    vec![
        ("cube 17", cube(17)),
        ("cube 33", cube(33)),
        ("cube 65", cube(65)),
        (
            "cube 33, DOMAIN_MIN/MAX per channel",
            format!(
                "TITLE \"wide\"\nDOMAIN_MIN -0.25 -0.125 0\nDOMAIN_MAX 1.5 1.25 2\nLUT_3D_SIZE 33\n{}",
                cube_rows(33, graded)
            ),
        ),
        (
            "cube 17, LUT_3D_INPUT_RANGE",
            format!(
                "LUT_3D_SIZE 17\nLUT_3D_INPUT_RANGE -0.5 1.5\n{}",
                cube_rows(17, graded)
            ),
        ),
        (
            "1D 1024",
            format!("LUT_1D_SIZE 1024\n{}", curve_rows(1024, shaper)),
        ),
        (
            "1D 4096, LUT_1D_INPUT_RANGE",
            format!(
                "LUT_1D_SIZE 4096\nLUT_1D_INPUT_RANGE -0.25 1.5\n{}",
                curve_rows(4096, shaper)
            ),
        ),
        (
            "1D shaper then cube 33",
            format!(
                "LUT_1D_SIZE 512\nLUT_1D_INPUT_RANGE 0 1.5\nLUT_3D_SIZE 33\nLUT_3D_INPUT_RANGE 0 1\n{}{}",
                curve_rows(512, shaper),
                cube_rows(33, graded)
            ),
        ),
    ]
}

/// A field of `W` × `H` colours: red across, green down, blue in bands.
/// `wide` reaches from -0.25 to 1.5, outside every default domain.
fn field(wide: bool) -> String {
    let stretch = if wide { "c=c*1.75-0.25;" } else { "" };
    format!(
        "void main(){{vec2 p=gl_FragCoord.xy/vec2({W}.0,{H}.0);\
         vec3 c=vec3(p.x,p.y,fract(p.x*5.0+p.y*3.0));{stretch}fragColor=vec4(c,1.0);}}"
    )
}

fn lut_effect(amount: f64, input: f64, output: f64) -> Value {
    json!([{
        "id":"effect","type":"applyColorLut","version":2,"on":true,
        "p":{"amount":ch(json!(amount)),"inputSpace":ch(json!(input)),"outputSpace":ch(json!(output))},
        "inputs":{"lut":{"kind":"asset","assetId":"grade"}}
    }])
}

fn project_with(code: &str, fx: Option<Value>, depth: u32) -> Value {
    let mut source = layer(
        "owner",
        "shader",
        json!({"w":W,"h":H,"code":code,"uniforms":{}}),
        0.,
        0.,
        1.,
    );
    if let Some(fx) = fx {
        source["fx"] = fx;
    }
    let mut value = project(W, H, 1., no_fill(), vec![source]);
    asset(&mut value, "grade", "lut", "cube", 0, 0, 0.);
    value["colorDepth"] = json!(depth);
    value
}

fn resolver(dir: &tempfile::TempDir, text: &str) -> MapResolver {
    let path = dir.path().join("grade.cube");
    std::fs::write(&path, text).unwrap();
    let mut resolver = MapResolver::default();
    resolver.paths.insert("grade".into(), path);
    resolver
}

fn draw(renderer: &mut Renderer, value: &Value, resolver: &MapResolver) -> Vec<f32> {
    let mut doc = load(value);
    doc.set_saving_app_version(Some("1.3.0".into()));
    renderer.set_main_effects_for_document(&doc).unwrap();
    let frame = renderer
        .render(
            &doc,
            "Ptest-root",
            Ticks::ZERO,
            &RenderRequest::sized(Size::new(W, H), Precision::Float32),
            resolver,
        )
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    renderer.read_rgba(&frame).unwrap()
}

/// One step of half-float storage at `v`.
fn half_step(v: f64) -> f64 {
    let exponent = v.abs().max(2f64.powi(-14)).log2().floor() as i32;
    2f64.powi(exponent - 10)
}

/// The largest difference between the rendered table and the reference over
/// the field, in units of `tolerance(reference value)`.
fn worst(
    pixels: &[f32],
    inputs: &[f32],
    lut: &reference::Lut,
    tolerance: impl Fn(f64) -> f64,
) -> (f64, String) {
    let mut worst = (0f64, String::new());
    for (i, (out, input)) in pixels
        .chunks_exact(4)
        .zip(inputs.chunks_exact(4))
        .enumerate()
    {
        let rgb = [input[0], input[1], input[2]].map(f64::from);
        let expected = reference::apply(lut, rgb);
        for c in 0..3 {
            let error = (f64::from(out[c]) - expected[c]).abs() / tolerance(expected[c]);
            if error > worst.0 {
                worst = (
                    error,
                    format!(
                        "pixel {i} channel {c}: in {rgb:?}, rendered {}, reference {}",
                        out[c], expected[c]
                    ),
                );
            }
        }
        assert_eq!(out[3], 1.0, "alpha at pixel {i}");
    }
    worst
}

#[test]
fn the_rendered_lut_matches_the_reference_at_8_16_and_32_bit() {
    let dir = tempfile::tempdir().unwrap();
    let mut renderer = Renderer::headless().unwrap();
    for (name, text) in tables() {
        let files = resolver(&dir, &text);
        let lut = reference::read(&text);
        for depth in [8, 16, 32] {
            for wide in [false, true] {
                let code = field(wide);
                let inputs = draw(&mut renderer, &project_with(&code, None, depth), &files);
                let graded = project_with(&code, Some(lut_effect(100., 0., 0.)), depth);
                let pixels = draw(&mut renderer, &graded, &files);
                // Float storage: 1e-4. Half-float storage (8 and 16-bit
                // projects): one stored code of the reference value.
                let (error, at) = if depth == 32 {
                    worst(&pixels, &inputs, &lut, |_| 1e-4)
                } else {
                    worst(&pixels, &inputs, &lut, half_step)
                };
                assert!(
                    error <= 1.0,
                    "{name}, {depth}-bit, wide {wide}: {error:.3} of the tolerance at {at}"
                );
            }
        }
    }
}

#[test]
fn an_identity_lut_returns_the_same_bits() {
    let dir = tempfile::tempdir().unwrap();
    let mut renderer = Renderer::headless().unwrap();
    let identity = |r, g, b| [r, g, b];
    // A partly transparent field too: the colour is divided by alpha for the
    // table and multiplied back.
    let translucent = format!(
        "void main(){{vec2 p=gl_FragCoord.xy/vec2({W}.0,{H}.0);float a=0.2+0.6*p.y;\
         fragColor=vec4(vec3(p.x,p.y,fract(p.x*5.0+p.y*3.0))*a,a);}}"
    );
    for size in [17, 33, 65] {
        let text = format!("LUT_3D_SIZE {size}\n{}", cube_rows(size, identity));
        let files = resolver(&dir, &text);
        for depth in [8, 16, 32] {
            for code in [field(false), translucent.clone()] {
                let inputs = draw(&mut renderer, &project_with(&code, None, depth), &files);
                let graded = project_with(&code, Some(lut_effect(100., 0., 0.)), depth);
                let pixels = draw(&mut renderer, &graded, &files);
                let changed = pixels
                    .iter()
                    .zip(&inputs)
                    .filter(|(a, b)| a.to_bits() != b.to_bits())
                    .count();
                assert_eq!(
                    changed, 0,
                    "identity cube {size} at {depth}-bit changed {changed} values"
                );
            }
        }
    }
}

#[test]
fn values_outside_the_domain_clamp_for_the_lookup_and_keep_the_tables_output() {
    let dir = tempfile::tempdir().unwrap();
    let mut renderer = Renderer::headless().unwrap();
    // The table doubles its input: 1.0 in gives 2.0 out, and so must 3.0 in.
    let text = format!(
        "LUT_3D_SIZE 17\n{}",
        cube_rows(17, |r, g, b| [2. * r, 2. * g - 0.5, 2. * b])
    );
    let files = resolver(&dir, &text);
    let code = "void main(){fragColor=vec4(3.0,-1.0,0.5,1.0);}";
    let graded = project_with(code, Some(lut_effect(100., 0., 0.)), 32);
    let pixels = draw(&mut renderer, &graded, &files);
    assert_eq!(&pixels[..4], &[2.0, -0.5, 1.0, 1.0]);
}

fn ffmpeg() -> std::path::PathBuf {
    std::path::PathBuf::from(
        std::env::var_os("PM_FFMPEG_DIR").expect("source engine/env.sh before this test"),
    )
    .join("bin/ffmpeg")
}

/// The reference against FFmpeg's own lookup on the same float pixels.
/// FFmpeg reads a `.cube` domain as a scale only, so tables here start at 0;
/// a 1D shaper ahead of a cube is `lut1d` then `lut3d`.
#[test]
fn the_reference_agrees_with_ffmpeg() {
    let dir = tempfile::tempdir().unwrap();
    let (w, h) = (97usize, 61usize);
    // Planar float, FFmpeg's plane order: green, blue, red.
    let pixel = |i: usize| -> [f32; 3] {
        let (x, y) = (
            (i % w) as f32 / (w - 1) as f32,
            (i / w) as f32 / (h - 1) as f32,
        );
        [x, y, (x * 5.3 + y * 3.1).fract()].map(|v| v * 1.75 - 0.25)
    };
    let mut raw = vec![];
    for plane in [1, 2, 0] {
        for i in 0..w * h {
            raw.extend(pixel(i)[plane].to_le_bytes());
        }
    }
    let input = dir.path().join("in.raw");
    std::fs::write(&input, &raw).unwrap();
    let run = |filters: &str| -> Vec<[f32; 3]> {
        let output = dir.path().join("out.raw");
        let status = std::process::Command::new(ffmpeg())
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "gbrpf32le",
            ])
            .args(["-s", &format!("{w}x{h}"), "-i"])
            .arg(&input)
            .args(["-vf", filters, "-f", "rawvideo", "-pix_fmt", "gbrpf32le"])
            .arg(&output)
            .status()
            .unwrap();
        assert!(status.success(), "ffmpeg {filters}");
        let bytes = std::fs::read(output).unwrap();
        assert_eq!(bytes.len(), w * h * 12);
        let at = |plane: usize, i: usize| {
            let o = (plane * w * h + i) * 4;
            f32::from_le_bytes(bytes[o..o + 4].try_into().unwrap())
        };
        (0..w * h).map(|i| [at(2, i), at(0, i), at(1, i)]).collect()
    };
    let write = |name: &str, text: &str| -> String {
        let path = dir.path().join(name);
        std::fs::write(&path, text).unwrap();
        path.to_str().unwrap().to_owned()
    };
    let check = |name: &str, lut: &reference::Lut, filtered: Vec<[f32; 3]>| {
        let mut worst = 0f64;
        for (i, out) in filtered.iter().enumerate() {
            let expected = reference::apply(lut, pixel(i).map(f64::from));
            for c in 0..3 {
                worst = worst.max((f64::from(out[c]) - expected[c]).abs());
            }
        }
        assert!(worst < 2e-6, "{name}: {worst:e} from FFmpeg");
    };
    for size in [17, 33, 65] {
        let text = format!("LUT_3D_SIZE {size}\n{}", cube_rows(size, graded));
        let file = write("cube.cube", &text);
        check(
            &format!("cube {size}"),
            &reference::read(&text),
            run(&format!("lut3d=file={file}:interp=tetrahedral")),
        );
    }
    // FFmpeg reads DOMAIN lines only after the size line.
    let wide = format!(
        "LUT_3D_SIZE 33\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 2 2 2\n{}",
        cube_rows(33, graded)
    );
    let file = write("wide.cube", &wide);
    check(
        "cube 33 over 0..2",
        &reference::read(&wide),
        run(&format!("lut3d=file={file}:interp=tetrahedral")),
    );
    let curve = format!("LUT_1D_SIZE 1024\n{}", curve_rows(1024, shaper));
    let curve_file = write("curve.cube", &curve);
    check(
        "1D 1024",
        &reference::read(&curve),
        run(&format!("lut1d=file={curve_file}:interp=linear")),
    );
    let cube = format!("LUT_3D_SIZE 33\n{}", cube_rows(33, graded));
    let cube_file = write("after.cube", &cube);
    let both = format!(
        "LUT_1D_SIZE 1024\nLUT_3D_SIZE 33\n{}{}",
        curve_rows(1024, shaper),
        cube_rows(33, graded)
    );
    check(
        "1D shaper then cube 33",
        &reference::read(&both),
        run(&format!(
            "lut1d=file={curve_file}:interp=linear,lut3d=file={cube_file}:interp=tetrahedral"
        )),
    );
}

/// Working (E-encoded Rec.709) to a LUT space and back, in f64 from the
/// vendors' published curves (`pm_reference`, not the shader's source).
type Space = (
    &'static str,
    f64,
    Box<dyn Fn([f64; 3]) -> [f64; 3]>,
    Box<dyn Fn([f64; 3]) -> [f64; 3]>,
);

fn spaces() -> Vec<Space> {
    use pm_reference::camera_log::Curve;
    use pm_reference::color::{Primaries as P, Transfer as T, apply, invert, mul};
    let e_decode = |v: f64| T::Srgb.to_linear(v);
    let e_encode = |v: f64| T::Srgb.from_linear(v);
    let through = |name: &'static str,
                   index: f64,
                   gamut: Option<P>,
                   encode: Box<dyn Fn(f64) -> f64>,
                   decode: Box<dyn Fn(f64) -> f64>|
     -> Space {
        let to = gamut.map(|g| mul(&invert(&g.to_xyz()), &P::BT709.to_xyz()));
        let from = to.as_ref().map(invert);
        (
            name,
            index,
            Box::new(move |working| {
                let linear = working.map(e_decode);
                to.as_ref()
                    .map_or(linear, |m| apply(m, linear))
                    .map(&encode)
            }),
            Box::new(move |code| {
                let linear = code.map(&decode);
                from.as_ref()
                    .map_or(linear, |m| apply(m, linear))
                    .map(e_encode)
            }),
        )
    };
    let log = |name, index, curve: Curve, gamut| {
        through(
            name,
            index,
            Some(gamut),
            Box::new(move |v| curve.from_linear(v)),
            Box::new(move |v| curve.to_linear(v)),
        )
    };
    vec![
        through("Rec.709 Linear", 1., None, Box::new(|v| v), Box::new(|v| v)),
        log("Apple Log", 2., Curve::AppleLog, P::BT2020),
        log("S-Log3 (S-Gamut3.Cine)", 3., Curve::SLog3, P::S_GAMUT3_CINE),
        log("S-Log3 (S-Gamut3)", 4., Curve::SLog3, P::S_GAMUT3),
        log("V-Log", 5., Curve::VLog, P::V_GAMUT),
        log("LogC3", 6., Curve::LogC3, P::AWG3),
        log("LogC4", 7., Curve::LogC4, P::AWG4),
        through(
            "PQ",
            8.,
            Some(P::BT2020),
            Box::new(|v| T::Pq.from_linear((v * 203.0 / 10000.0).max(0.0))),
            Box::new(|v| T::Pq.to_linear(v) * 10000.0 / 203.0),
        ),
    ]
}

/// A technical LUT is applied on the encoding it was made for: the pixel is
/// taken to the stated camera encoding, looked up, and brought back.
#[test]
fn a_lut_is_applied_on_its_stated_input_encoding() {
    let dir = tempfile::tempdir().unwrap();
    let mut renderer = Renderer::headless().unwrap();
    // A grade that mixes channels, on a domain wide enough for every space.
    let text = format!(
        "LUT_3D_SIZE 33\nLUT_3D_INPUT_RANGE -0.25 1.25\n{}",
        cube_rows(33, |r, g, b| {
            let (r, g, b) = (1.5 * r - 0.25, 1.5 * g - 0.25, 1.5 * b - 0.25);
            [
                0.9 * r + 0.1 * g,
                0.85 * g + 0.1 * b + 0.02,
                0.8 * b + 0.15 * r,
            ]
        })
    );
    let files = resolver(&dir, &text);
    let lut = reference::read(&text);
    let code = field(false);
    let inputs = draw(&mut renderer, &project_with(&code, None, 32), &files);
    for (name, index, encode, decode) in spaces() {
        let graded = project_with(&code, Some(lut_effect(100., index, index)), 32);
        let pixels = draw(&mut renderer, &graded, &files);
        let mut worst = 0f64;
        for (out, input) in pixels.chunks_exact(4).zip(inputs.chunks_exact(4)) {
            let working = [input[0], input[1], input[2]].map(f64::from);
            let expected = decode(reference::apply(&lut, encode(working)));
            for c in 0..3 {
                worst = worst.max((f64::from(out[c]) - expected[c]).abs());
            }
        }
        assert!(worst < 1e-4, "{name}: {worst:e} from the reference");
    }
}

#[test]
fn an_identity_lut_returns_the_same_bits_through_every_input_encoding() {
    let dir = tempfile::tempdir().unwrap();
    let mut renderer = Renderer::headless().unwrap();
    let text = format!("LUT_3D_SIZE 33\n{}", cube_rows(33, |r, g, b| [r, g, b]));
    let files = resolver(&dir, &text);
    // In gamut for every camera space, so each code stays inside the table.
    let code = format!(
        "void main(){{vec2 p=gl_FragCoord.xy/vec2({W}.0,{H}.0);\
         fragColor=vec4(0.25+0.5*vec3(p.x,p.y,fract(p.x*5.0+p.y*3.0)),1.0);}}"
    );
    let inputs = draw(&mut renderer, &project_with(&code, None, 32), &files);
    for (name, index, _, _) in spaces() {
        let graded = project_with(&code, Some(lut_effect(100., index, index)), 32);
        let pixels = draw(&mut renderer, &graded, &files);
        let changed = pixels
            .iter()
            .zip(&inputs)
            .filter(|(a, b)| a.to_bits() != b.to_bits())
            .count();
        assert_eq!(changed, 0, "{name} changed {changed} values");
    }
}

/// A LUT whose file is gone or broken stops the frame with a sentence a
/// person can act on; it is never drawn as if the effect were off.
#[test]
fn a_missing_or_broken_lut_is_named_in_a_plain_sentence() {
    let dir = tempfile::tempdir().unwrap();
    let mut renderer = Renderer::headless().unwrap();
    let mut value = project_with(&field(false), Some(lut_effect(100., 0., 0.)), 16);
    value["assets"]["grade"]["name"] = json!("Film Look.cube");
    let mut refusal = |resolver: &MapResolver| -> String {
        let mut doc = load(&value);
        doc.set_saving_app_version(Some("1.3.0".into()));
        renderer.set_main_effects_for_document(&doc).unwrap();
        renderer
            .render(
                &doc,
                "Ptest-root",
                Ticks::ZERO,
                &RenderRequest::sized(Size::new(W, H), Precision::Float32),
                resolver,
            )
            .err()
            .expect("the frame is refused")
            .to_string()
    };
    let missing = refusal(&MapResolver::default());
    assert!(
        missing
            .contains("The LUT “Film Look.cube” can’t be found. Relink it or remove the effect."),
        "{missing}"
    );
    assert!(!missing.contains("no definition"), "{missing}");
    let broken = refusal(&resolver(&dir, "LUT_3D_SIZE 2\n0 0 0\n1 1 1\n"));
    assert!(
        broken.contains(
            "The LUT “Film Look.cube” couldn’t be read. The table has 2 rows; its size needs 8."
        ),
        "{broken}"
    );
}

/// A 17-point, 12-bit `.3dl`: the renderer against FFmpeg's reader and
/// lookup. FFmpeg divides the table by 4096; white is 4095 here (as in
/// OpenColorIO), so its result is scaled by 4096/4095 before comparing.
#[test]
fn a_3dl_table_matches_ffmpeg() {
    let dir = tempfile::tempdir().unwrap();
    let size = 17;
    let mut text =
        String::from("0 64 128 192 256 320 384 448 512 576 640 704 768 832 896 960 1023\n");
    let at = |i: usize| i as f64 / (size - 1) as f64;
    for r in 0..size {
        for g in 0..size {
            for b in 0..size {
                let v = graded(at(r), at(g), at(b)).map(|v| (v.clamp(0., 1.) * 4095.).round());
                text += &format!("{} {} {}\n", v[0], v[1], v[2]);
            }
        }
    }
    let file = dir.path().join("grade.3dl");
    std::fs::write(&file, &text).unwrap();
    let mut files = MapResolver::default();
    files.paths.insert("grade".into(), file.clone());
    let mut renderer = Renderer::headless().unwrap();
    let code = field(true);
    let inputs = draw(&mut renderer, &project_with(&code, None, 32), &files);
    let mut graded_project = project_with(&code, Some(lut_effect(100., 0., 0.)), 32);
    graded_project["assets"]["grade"]["format"] = json!("3dl");
    let pixels = draw(&mut renderer, &graded_project, &files);

    let count = (W * H) as usize;
    let mut raw = vec![];
    for plane in [1, 2, 0] {
        for pixel in inputs.chunks_exact(4) {
            raw.extend(pixel[plane].to_le_bytes());
        }
    }
    let input = dir.path().join("in.raw");
    let output = dir.path().join("out.raw");
    std::fs::write(&input, &raw).unwrap();
    let status = std::process::Command::new(ffmpeg())
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "gbrpf32le",
        ])
        .args(["-s", &format!("{W}x{H}"), "-i"])
        .arg(&input)
        .args([
            "-vf",
            &format!("lut3d=file={}:interp=tetrahedral", file.display()),
        ])
        .args(["-f", "rawvideo", "-pix_fmt", "gbrpf32le"])
        .arg(&output)
        .status()
        .unwrap();
    assert!(status.success());
    let bytes = std::fs::read(output).unwrap();
    let at = |plane: usize, i: usize| {
        let o = (plane * count + i) * 4;
        f64::from(f32::from_le_bytes(bytes[o..o + 4].try_into().unwrap())) * 4096. / 4095.
    };
    let mut worst = 0f64;
    for (i, out) in pixels.chunks_exact(4).enumerate() {
        for (c, plane) in [2, 0, 1].into_iter().enumerate() {
            worst = worst.max((f64::from(out[c]) - at(plane, i)).abs());
        }
    }
    assert!(worst < 2e-6, "{worst:e} from FFmpeg");
}
