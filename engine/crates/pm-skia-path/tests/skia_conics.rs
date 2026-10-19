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

    for segment in path.segments() {
        let (verb, points, weight): (_, Vec<_>, _) = match segment {
            PathSegment::MoveTo(p) => (0, vec![p], None),
            PathSegment::LineTo(p) => (1, vec![last, p], None),
            PathSegment::QuadTo(c, p) => (2, vec![last, c, p], None),
            PathSegment::ConicTo(c, p, w) => (3, vec![last, c, p], Some(w)),
            PathSegment::CubicTo(a, b, p) => (4, vec![last, a, b, p], None),
            PathSegment::Close => (5, vec![], None),
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
