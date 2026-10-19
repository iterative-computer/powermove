//! `SkPath::isConvex()` from Skia chrome/m152 at
//! 747cf654ed6f864f4402e7d6eb54a6ede60141e5 (ffp-contract=off) for the
//! paths in skia-convex-cases.inc; tests/skia-convex-oracle.cpp prints them.
//! The analytic scan converter walks convex paths differently, so the
//! classification decides coverage.
use pm_skia_path::PathBuilder;

/// One line per case of tests/skia-convex-oracle.cpp's output. The last
/// three contours double back to their start before turning, where m152's
/// `Convexicator::addPt` keeps the reversed vector (`fLastVec` non-zero).
#[rustfmt::skip]
const SKIA: [bool; 21] = [
    true, false, false, false, false, true, true, false, false, true, false,
    true, false, true, false, true, false, false,
    false, false, false,
];

#[test]
fn convexity_matches_skia_152() {
    let cases: Vec<&str> = include_str!("skia-convex-cases.inc")
        .lines()
        .map(|l| l.trim().trim_end_matches(',').trim_matches('"'))
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(cases.len(), SKIA.len());
    for (case, &skia) in cases.iter().zip(&SKIA) {
        let mut b = PathBuilder::new();
        let mut toks = case.split(' ');
        while let Some(v) = toks.next() {
            let mut n = || toks.next().unwrap().parse::<f32>().unwrap();
            match v {
                "M" => {
                    let (x, y) = (n(), n());
                    b.move_to(x, y)
                }
                "L" => {
                    let (x, y) = (n(), n());
                    b.line_to(x, y)
                }
                "Q" => {
                    let (a, c, x, y) = (n(), n(), n(), n());
                    b.quad_to(a, c, x, y)
                }
                "C" => {
                    let (a, c, d, e, x, y) = (n(), n(), n(), n(), n(), n());
                    b.cubic_to(a, c, d, e, x, y)
                }
                "Z" => b.close(),
                _ => panic!("unknown verb {v}"),
            }
        }
        let path = b.finish().unwrap();
        assert_eq!(path.is_convex(), skia, "{case}");
    }
}
