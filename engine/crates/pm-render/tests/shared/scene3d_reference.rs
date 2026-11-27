//! Independent f64 display transform and pixel classes for the 3D gates.
//! The input is Three.js RGBA32F captured at 8x and averaged in linear
//! premultiplied light, or the native renderer's resolved linear target.
use pm_reference::compare::{self, Class};

/// Scene3d targets retain GL's bottom row first; independent captures and PNG
/// files have their top row first. Convert before geometry or pixel comparison.
pub fn top_down_linear(mut pixels: Vec<f32>, width: usize, height: usize) -> Vec<f32> {
    assert_eq!(pixels.len(), width * height * 4);
    let stride = width * 4;
    for y in 0..height / 2 {
        let lo = y * stride;
        let hi = (height - 1 - y) * stride;
        for x in 0..stride {
            pixels.swap(lo + x, hi + x);
        }
    }
    pixels
}

/// Captured r180 linear targets emitted through a linear output function.
/// The live renderer omits that no-op for shadow depth and linear scene
/// light. Restore only this source line for exact program comparison.
pub fn restore_linear_output(mut fragment: String) -> String {
    let live = "vec4 linearToOutputTexel( vec4 value ) { return value; }";
    let old = "vec4 linearToOutputTexel( vec4 value ) {\n\treturn LinearTransferOETF( vec4( value.rgb * mat3( 1.0000,-0.0000,-0.0000,-0.0000,1.0000,0.0000,0.0000,0.0000,1.0000 ), value.a ) );\n}";
    assert!(fragment.contains(live), "shadow output transfer");
    fragment = fragment.replacen(live, old, 1);
    fragment
}

fn aces(rgb: [f64; 3], exposure: f64) -> [f64; 3] {
    let [r, g, b] = rgb.map(|v| v * (exposure / 0.6));
    let input = [
        0.59719 * r + 0.35458 * g + 0.04823 * b,
        0.07600 * r + 0.90834 * g + 0.01566 * b,
        0.02840 * r + 0.13383 * g + 0.83777 * b,
    ];
    let fit =
        |v: f64| (v * (v + 0.0245786) - 0.000090537) / (v * (0.983729 * v + 0.4329510) + 0.238081);
    let [x, y, z] = input.map(fit);
    [
        1.60475 * x - 0.53108 * y - 0.07367 * z,
        -0.10208 * x + 1.10813 * y - 0.00605 * z,
        -0.00327 * x - 0.07276 * y + 1.07602 * z,
    ]
    .map(|v| v.clamp(0.0, 1.0))
}

fn encode(v: f64) -> f64 {
    if v <= 0.0031308 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// r180 ACES once after blending and resolving, then one Standard encode.
/// A scene background is already an authored linear display color and joins
/// after ACES. The result is premultiplied display RGBA, normalized to 0..1.
pub fn display(linear: &[f32], exposure: f64, background: Option<[f64; 3]>) -> Vec<f64> {
    let mut out = Vec::with_capacity(linear.len());
    for src in linear.chunks_exact(4) {
        let a = f64::from(src[3]).clamp(0.0, 1.0);
        let rgb = if a > 0.0 {
            aces([0, 1, 2].map(|c| f64::from(src[c]) / a), exposure).map(|v| v * a)
        } else {
            [0.0; 3]
        };
        let (rgb, alpha) = if let Some(bg) = background {
            ([0, 1, 2].map(|c| rgb[c] + (1.0 - a) * bg[c]), 1.0)
        } else {
            (rgb, a)
        };
        if alpha > 0.0 {
            out.extend(rgb.map(|v| encode((v / alpha).clamp(0.0, 1.0)) * alpha));
        } else {
            out.extend([0.0; 3]);
        }
        out.push(alpha);
    }
    out
}

/// The scene is rasterized; partially covered pixels and strong 3x3 color
/// changes are edges. Coverage is the reference's own alpha before the
/// display background (`coverage`, premultiplied linear RGBA): a composited
/// background makes every displayed pixel opaque and would hide a sliver
/// the reference only partly covers. The 8 CV span is the existing 3D card
/// criterion. Two-pixel dilation is the shared gate-policy edge radius.
pub fn classes(reference: &[f64], coverage: &[f32], width: usize, height: usize) -> Vec<Class> {
    assert_eq!(reference.len(), width * height * 4);
    assert_eq!(coverage.len(), width * height * 4);
    let mut edge = vec![false; width * height];
    for y in 0..height {
        for x in 0..width {
            let i = y * width + x;
            let alpha = f64::from(coverage[4 * i + 3]);
            if alpha > 0.0 && alpha < 1.0 {
                edge[i] = true;
                continue;
            }
            for channel in 0..4 {
                let center = reference[4 * i + channel];
                let mut lo = center;
                let mut hi = center;
                for yy in y.saturating_sub(1)..=(y + 1).min(height - 1) {
                    for xx in x.saturating_sub(1)..=(x + 1).min(width - 1) {
                        let value = reference[4 * (yy * width + xx) + channel];
                        lo = lo.min(value);
                        hi = hi.max(value);
                    }
                }
                if (hi - lo) * 255.0 > 8.0 {
                    edge[i] = true;
                    break;
                }
            }
        }
    }
    compare::classify(&edge, &vec![false; width * height], width, height, 2)
}
