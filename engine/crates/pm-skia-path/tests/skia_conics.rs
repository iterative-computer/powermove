//! Bitwise path records captured from Skia chrome/m152 at
//! 747cf654ed6f864f4402e7d6eb54a6ede60141e5 (ffp-contract=off).
//! These exercise rational strokes before device-space scan conversion.
use pm_skia_path::{LineCap, LineJoin, Path, PathBuilder, PathSegment, Point, Stroke};

fn record_hash(path: &Path) -> (u32, u64) {
    let mut count = 0;
    let mut hash = 14_695_981_039_346_656_037u64;
    let mut add = |v: u32| {
        count += 1;
        hash = (hash ^ u64::from(v)).wrapping_mul(1_099_511_628_211);
    };
    let mut last = Point::zero();
    let mut first = Point::zero();

    for segment in path.segments() {
        let (verb, points, weight): (_, Vec<_>, _) = match segment {
            PathSegment::MoveTo(p) => {
                first = p;
                (0, vec![p], None)
            }
            PathSegment::LineTo(p) => (1, vec![last, p], None),
            PathSegment::QuadTo(c, p) => (2, vec![last, c, p], None),
            PathSegment::ConicTo(c, p, w) => (3, vec![last, c, p], Some(w)),
            PathSegment::CubicTo(a, b, p) => (4, vec![last, a, b, p], None),
            PathSegment::Close => {
                // SkPath::Iter exposes the implicit closing line as well.
                if last != first {
                    add(1);
                    for p in [last, first] {
                        add(p.x.to_bits());
                        add(p.y.to_bits());
                    }
                }
                (5, vec![], None)
            }
        };
        add(verb);
        for p in &points {
            add(p.x.to_bits());
            add(p.y.to_bits());
        }
        if let Some(w) = weight {
            add(w.to_bits());
        }
        if let Some(&p) = points.last() {
            last = p;
        }
    }
    (count, hash)
}

#[test]
fn rational_strokes_match_skia_152_path_records() {
    let expected = [
        (64, 16_475_933_818_476_730_904),
        (85, 3_754_275_025_917_462_003),
        (82, 15_440_996_654_433_247_036),
        (46, 16_434_631_017_148_447_705),
        (138, 4_215_324_800_312_635_908),
    ];
    for (k, expected) in expected.into_iter().enumerate() {
        let mut b = PathBuilder::new();
        match k {
            0 => {
                b.move_to(0., 20.);
                b.conic_to(0., 0., 20., 0., core::f32::consts::FRAC_1_SQRT_2);
            }
            1 => {
                b.move_to(3.25, 19.5);
                b.conic_to(27.125, -8.75, 41.75, 31.25, 2.5);
            }
            2 => {
                b.move_to(1., 1.);
                b.conic_to(9., 1., 4., 1., 0.5);
            }
            3 => {
                b.move_to(1., 1.);
                b.conic_to(1., 1., 10., 8., 0.5);
            }
            4 => {
                b.move_to(-2.75, 9.125);
                b.conic_to(11.5, -3.25, 26.625, 17.875, 0.125);
                b.line_to(31., 20.);
                b.close();
            }
            _ => unreachable!(),
        }
        let stroke = Stroke {
            width: 3.75,
            miter_limit: 4.,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
        };
        let p = b.finish().unwrap().stroke(&stroke, 1.).unwrap();
        assert_eq!(record_hash(&p), expected, "case {k}");
    }
}

#[test]
fn rectangle_frames_match_skia_152_path_records() {
    let expected = [
        (56, 18262213083876283474u64),
        (80, 739897739276918341u64),
        (68, 479402714892600777u64),
        (44, 11904902325730817774u64),
        (80, 4452492057560981061u64),
    ];
    for (k, expected) in expected.into_iter().enumerate() {
        let mut b = PathBuilder::new();
        b.move_to(2.125, 3.75);
        if k == 4 {
            b.line_to(2.125, 45.125);
            b.line_to(14.375, 45.125);
            b.line_to(14.375, 3.75);
        } else {
            b.line_to(14.375, 3.75);
            b.line_to(14.375, 45.125);
            b.line_to(2.125, 45.125);
        }
        b.close();
        let p = b.finish().unwrap();
        let stroke = Stroke {
            width: if k == 0 {
                14.
            } else if k == 1 {
                2.
            } else {
                3.75
            },
            miter_limit: if k == 2 { 0.5 } else { 4. },
            line_cap: LineCap::Butt,
            line_join: if k == 2 {
                LineJoin::Miter
            } else if k == 3 {
                LineJoin::Bevel
            } else {
                LineJoin::Round
            },
        };
        let out = if k == 3 {
            p.stroke_rect(&stroke, true).unwrap()
        } else {
            p.stroke(&stroke, 1.).unwrap()
        };
        assert_eq!(record_hash(&out), expected, "rectangle {k}");
    }
}

#[test]
fn convex_stroke_and_fill_matches_skia_152_path_records() {
    // tests/skia-center-oracle.cpp: SkStroke ignores the center of a
    // convex, line-only, closed contour and keeps only its larger offset.
    let expected = [
        (29, 9_894_305_336_873_942_857u64),
        (23, 3_058_211_389_671_761_532),
        (34, 18_178_562_680_614_805_173),
        (66, 1_170_642_342_792_557_455),
        (28, 16_412_707_246_739_295_283),
    ];
    let polys: [&[(f32, f32)]; 5] = [
        &[(2.5, 3.25), (20.75, 3.25), (14.125, 30.5), (-4.125, 30.5)],
        &[(2.5, 3.25), (-4.125, 30.5), (14.125, 30.5), (20.75, 3.25)],
        &[(1., 1.), (17.5, 4.25), (6.375, 19.)],
        &[(1., 1.), (6.375, 19.), (17.5, 4.25)],
        &[
            (10., 0.5),
            (0.75, 7.25),
            (4.25, 18.5),
            (15.75, 18.5),
            (19.25, 7.25),
        ],
    ];
    let width = [3.75, 3.75, 2., 5.5, 3.];
    let miter = [4., 4., 4., 4., 1.5];
    let join = [
        LineJoin::Miter,
        LineJoin::Miter,
        LineJoin::Bevel,
        LineJoin::Round,
        LineJoin::Miter,
    ];
    for (k, expected) in expected.into_iter().enumerate() {
        let mut b = PathBuilder::new();
        b.move_to(polys[k][0].0, polys[k][0].1);
        for &(x, y) in &polys[k][1..] {
            b.line_to(x, y);
        }
        b.close();
        let p = b.finish().unwrap();
        assert!(p.can_ignore_center(), "polygon {k}");
        let stroke = Stroke {
            width: width[k],
            miter_limit: miter[k],
            line_cap: LineCap::Butt,
            line_join: join[k],
        };
        let out = p.stroke_ignoring_center(&stroke, 1.).unwrap();
        assert_eq!(record_hash(&out), expected, "polygon {k}");
    }
    // Concave, open or curved paths keep the center.
    let mut b = PathBuilder::new();
    b.move_to(0., 0.);
    b.line_to(10., 0.);
    b.line_to(5., 2.);
    b.line_to(10., 10.);
    b.line_to(0., 10.);
    b.close();
    assert!(!b.finish().unwrap().can_ignore_center());
    let mut b = PathBuilder::new();
    b.move_to(0., 0.);
    b.line_to(10., 0.);
    b.line_to(10., 10.);
    assert!(!b.finish().unwrap().can_ignore_center());
    let mut b = PathBuilder::new();
    b.move_to(0., 0.);
    b.quad_to(10., 0., 10., 10.);
    b.close();
    assert!(!b.finish().unwrap().can_ignore_center());
}
