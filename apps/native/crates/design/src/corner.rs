//! The continuous ("squircle") corner AppKit draws for `kCACornerCurveContinuous`, as cubic Béziers:
//! for shapes drawn outside a layer's own corner (a path clipping a web page, the ground filling the
//! corner a native layer rounds off), so they meet that layer's corner exactly.

/// How far a continuous corner of radius 1 reaches along each side.
pub const CONTINUOUS_REACH: f64 = 1.528_665;

/// A radius that fits a `width` × `height` rectangle: its reach is at most half the shorter side.
pub fn fit(radius: f64, width: f64, height: f64) -> f64 {
    radius.min(width.min(height) / 2. / CONTINUOUS_REACH).max(0.)
}

/// The corner at `corner` of radius `radius`, from `corner + a·reach` to `corner + b·reach` (`a`, `b`
/// unit vectors along its two sides, pointing away from it): its start point and three cubic
/// segments, each `[control 1, control 2, end]`.
pub fn continuous_corner(
    corner: (f64, f64),
    a: (f64, f64),
    b: (f64, f64),
    radius: f64,
) -> ((f64, f64), [[(f64, f64); 3]; 3]) {
    let at = |u: f64, v: f64| {
        (
            corner.0 + (a.0 * u + b.0 * v) * radius,
            corner.1 + (a.1 * u + b.1 * v) * radius,
        )
    };
    (
        at(CONTINUOUS_REACH, 0.),
        [
            [at(1.088_493, 0.), at(0.868_407, 0.), at(0.669_934, 0.064_413)],
            [at(0.372_824, 0.188_345), at(0.188_345, 0.372_824), at(0.064_413, 0.669_934)],
            [at(0., 0.868_407), at(0., 1.088_493), at(0., CONTINUOUS_REACH)],
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_corner_runs_from_one_side_to_the_other_symmetrically() {
        let (start, curves) = continuous_corner((10., 20.), (1., 0.), (0., 1.), 4.);
        let k = CONTINUOUS_REACH * 4.;
        assert_eq!(start, (10. + k, 20.));
        assert_eq!(curves[2][2], (10., 20. + k));
        // The middle segment is mirrored about the diagonal.
        let [p, q, _] = curves[1];
        assert!((p.0 - q.1).abs() < 1e-9 && (p.1 - q.0).abs() < 1e-9);
    }

    #[test]
    fn a_radius_fits_its_rectangle() {
        assert_eq!(fit(12., 100., 100.), 12.);
        assert!((fit(12., 20., 100.) * CONTINUOUS_REACH - 10.).abs() < 1e-9);
    }
}
