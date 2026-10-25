//! CSS Color 4 binary-search gamut mapping with local MINDE for sRGB output.
//!
//! The caller chooses the SDR boundary where mapping applies. Extended color
//! values remain available elsewhere in the pipeline until that boundary.

use crate::transfer::{display_decode, display_encode};

const JND: f64 = 0.02;
const EPSILON: f64 = 0.0001;

fn multiply(matrix: [[f64; 3]; 3], value: [f64; 3]) -> [f64; 3] {
    matrix.map(|row| row[0] * value[0] + row[1] * value[1] + row[2] * value[2])
}

fn srgb_to_oklab(rgb: [f64; 3]) -> [f64; 3] {
    // CSS Color 4 sample matrices: sRGB -> XYZ D65 -> LMS -> Oklab.
    let linear = rgb.map(display_decode);
    let xyz = multiply(
        [
            [506752.0 / 1228815.0, 87881.0 / 245763.0, 12673.0 / 70218.0],
            [87098.0 / 409605.0, 175762.0 / 245763.0, 12673.0 / 175545.0],
            [7918.0 / 409605.0, 87881.0 / 737289.0, 1001167.0 / 1053270.0],
        ],
        linear,
    );
    let lms = multiply(
        [
            [0.8190224379967030, 0.3619062600528904, -0.1288737815209879],
            [0.0329836539323885, 0.9292868615863434, 0.0361446663506424],
            [0.0481771893596242, 0.2642395317527308, 0.6335478284694309],
        ],
        xyz,
    )
    .map(libm::cbrt);
    multiply(
        [
            [0.2104542683093140, 0.7936177747023054, -0.0040720430116193],
            [1.9779985324311684, -2.4285922420485799, 0.4505937096174110],
            [0.0259040424655478, 0.7827717124575296, -0.8086757549230774],
        ],
        lms,
    )
}

fn oklab_to_srgb(lab: [f64; 3]) -> [f64; 3] {
    // CSS Color 4 sample inverse matrices, preserving out-of-range values.
    let lms = multiply(
        [
            [1.0, 0.3963377773761749, 0.2158037573099136],
            [1.0, -0.1055613458156586, -0.0638541728258133],
            [1.0, -0.0894841775298119, -1.2914855480194092],
        ],
        lab,
    )
    .map(|channel| channel * channel * channel);
    let xyz = multiply(
        [
            [1.2268798758459243, -0.5578149944602171, 0.2813910456659647],
            [-0.0405757452148008, 1.1122868032803170, -0.0717110580655164],
            [-0.0763729366746601, -0.4214933324022432, 1.5869240198367816],
        ],
        lms,
    );
    multiply(
        [
            [12831.0 / 3959.0, -329.0 / 214.0, -1974.0 / 3959.0],
            [
                -851781.0 / 878810.0,
                1648619.0 / 878810.0,
                36519.0 / 878810.0,
            ],
            [705.0 / 12673.0, -2585.0 / 12673.0, 705.0 / 667.0],
        ],
        xyz,
    )
    .map(display_encode)
}

fn in_gamut(rgb: [f64; 3]) -> bool {
    rgb.iter().all(|&channel| (0.0..=1.0).contains(&channel))
}

fn clip(rgb: [f64; 3]) -> [f64; 3] {
    rgb.map(|channel| channel.clamp(0.0, 1.0))
}

fn delta_ok(clipped_rgb: [f64; 3], lab: [f64; 3]) -> f64 {
    let clipped = srgb_to_oklab(clipped_rgb);
    libm::sqrt(
        (clipped[0] - lab[0]).powi(2)
            + (clipped[1] - lab[1]).powi(2)
            + (clipped[2] - lab[2]).powi(2),
    )
}

/// Map an encoded, straight sRGB triplet to the sRGB gamut using CSS Color 4
/// §14.2.2 binary search with local MINDE. In-gamut inputs are returned
/// exactly. Alpha is handled by the caller.
///
/// This operation belongs at an SDR output boundary; callers keep extended
/// RGB values in working storage until they choose that boundary. Non-finite
/// inputs are passed through for upstream validation.
pub fn css_srgb(rgb: [f64; 3]) -> [f64; 3] {
    if !rgb.iter().all(|channel| channel.is_finite()) || in_gamut(rgb) {
        return rgb;
    }

    let origin = srgb_to_oklab(rgb);
    if !origin.iter().all(|channel| channel.is_finite()) {
        return clip(rgb);
    }
    if origin[0] >= 1.0 {
        return [1.0; 3];
    }
    if origin[0] <= 0.0 {
        return [0.0; 3];
    }

    let chroma = libm::hypot(origin[1], origin[2]);
    let current = oklab_to_srgb(origin);
    if !current.iter().all(|channel| channel.is_finite()) {
        return clip(rgb);
    }
    let mut clipped = clip(current);
    if delta_ok(clipped, origin) < JND || chroma == 0.0 || !chroma.is_finite() {
        return clipped;
    }

    // The unit a,b direction is the constant OkLCh hue. Retaining it avoids
    // unnecessary atan2/cos/sin round trips during the chroma search.
    let direction = [origin[1] / chroma, origin[2] / chroma];
    let (mut min, mut max) = (0.0, chroma);
    let mut min_in_gamut = true;
    while max - min > EPSILON {
        let current_chroma = (min + max) * 0.5;
        let current_lab = [
            origin[0],
            current_chroma * direction[0],
            current_chroma * direction[1],
        ];
        let current = oklab_to_srgb(current_lab);
        if !current.iter().all(|channel| channel.is_finite()) {
            max = current_chroma;
            continue;
        }
        if min_in_gamut && in_gamut(current) {
            min = current_chroma;
            continue;
        }
        clipped = clip(current);
        let difference = delta_ok(clipped, current_lab);
        if difference < JND {
            if JND - difference < EPSILON {
                return clipped;
            }
            min_in_gamut = false;
            min = current_chroma;
        } else {
            max = current_chroma;
        }
    }
    clipped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_gamut_is_bit_exact_and_neutral_extremes_map_to_endpoints() {
        let ordinary = [0.123_456_789, 0.543_210_987, 0.9];
        assert_eq!(css_srgb(ordinary), ordinary);
        let nearly_in_gamut = css_srgb([1.001, 0.2, 0.3]);
        assert!((nearly_in_gamut[0] - 1.0).abs() < 1e-12);
        assert!((nearly_in_gamut[1] - 0.2).abs() < 1e-12);
        assert!((nearly_in_gamut[2] - 0.3).abs() < 1e-12);
        assert_eq!(css_srgb([2.0; 3]), [1.0; 3]);
        assert_eq!(css_srgb([-0.1; 3]), [0.0; 3]);
    }

    #[test]
    fn local_minde_matches_wpt_oklch_samples() {
        // Inputs are encoded sRGB conversions of oklch(80% 0.35 30) and
        // oklch(60% 0.35 60) using the CSS Color 4 sample matrices. WPT's
        // canvas-srgb-gamut-mapping.html gives their MINDE output in 8-bit
        // sRGB coordinates, independently of these conversion functions.
        for (rgb, expected) in [
            (
                [1.40671530241805, -0.21907629207175694, -0.18751316144355487],
                [255.0, 153.55, 135.55],
            ),
            (
                [
                    1.0135949638751496,
                    -0.22647951735446825,
                    -0.3724720233721723,
                ],
                [195.29, 98.41, 0.0],
            ),
        ] {
            let mapped = css_srgb(rgb);
            assert!(in_gamut(mapped));
            for channel in 0..3 {
                assert!(
                    (mapped[channel] * 255.0 - expected[channel]).abs() < 2.0,
                    "channel {channel}: {:?} vs {:?}",
                    mapped,
                    expected
                );
            }
        }
    }
}
