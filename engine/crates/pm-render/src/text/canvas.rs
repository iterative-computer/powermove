//! Canvas2DRecorderContext narrows every transform argument to float before
//! recording. SkCanvas then concatenates in SkM44's right-associated order.
use kurbo::Affine;

pub(super) fn narrow(m: Affine) -> Affine {
    Affine::new(m.as_coeffs().map(|v| f64::from(v as f32)))
}

pub(super) fn translate(m: Affine, x: f64, y: f64) -> Affine {
    let [a, b, c, d, e, f] = m.as_coeffs().map(|v| v as f32);
    let (x, y) = (x as f32, y as f32);
    Affine::new([a, b, c, d, a * x + (c * y + e), b * x + (d * y + f)].map(f64::from))
}

pub(super) fn scale(m: Affine, x: f64, y: f64) -> Affine {
    let [a, b, c, d, e, f] = m.as_coeffs().map(|v| v as f32);
    let (x, y) = (x as f32, y as f32);
    Affine::new([a * x, b * x, c * y, d * y, e, f].map(f64::from))
}

pub(super) fn transform(m: Affine, rhs: [f64; 6]) -> Affine {
    let [a, b, c, d, e, f] = m.as_coeffs().map(|v| v as f32);
    let [g, h, i, j, k, l] = rhs.map(|v| v as f32);
    Affine::new(
        [
            a * g + (c * h + e * 0.),
            b * g + (d * h + f * 0.),
            a * i + (c * j + e * 0.),
            b * i + (d * j + f * 0.),
            a * k + (c * l + e),
            b * k + (d * l + f),
        ]
        .map(f64::from),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_operations_match_pinned_skm44_bits() {
        // pm-vector/tools/graphite-oracle/canvas.cpp, -ffp-contract=off.
        let mut caption = scale(Affine::IDENTITY, 1.25, 1.25);
        caption = translate(caption, 27.3, 71.9);
        caption = translate(caption, 460.13, 52.08);
        caption = scale(caption, 1.16, 1.16);
        caption = translate(caption, -460.13, -52.08);
        assert_eq!(
            caption.as_coeffs().map(|v| (v as f32).to_bits()),
            [0x3fb99999, 0, 0, 0x3fb99999, 0xc2679aa0, 0x429eeb03,]
        );
        let mut animated = narrow(Affine::new([1.75, 0., 0., 1.75, -345.123, -87.314]));
        animated = translate(animated, 732.123, 422.91);
        animated = transform(animated, [0.75, 0.125, -0.18, 0.91, 3.37, 8.81]);
        assert_eq!(
            animated.as_coeffs().map(|v| (v as f32).to_bits()),
            [
                0x3fa80000, 0x3e600000, 0xbea147ae, 0x3fcbd70b, 0x446b7f58, 0x44270c8b,
            ]
        );
    }
}
