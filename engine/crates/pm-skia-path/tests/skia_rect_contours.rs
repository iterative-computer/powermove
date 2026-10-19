//! `SkPath::isRect` from Skia chrome/m152 at
//! 747cf654ed6f864f4402e7d6eb54a6ede60141e5 (ffp-contract=off) for the
//! paths in skia-rect-contour-cases.inc. skia-rect-contour-expected.txt is
//! the output of tests/skia-rect-contour-oracle.cpp: "0", or
//! "1 left top right bottom closed clockwise".
use pm_skia_path::PathBuilder;

#[test]
fn rect_contours_match_skia_152() {
    let cases: Vec<&str> = include_str!("skia-rect-contour-cases.inc")
        .lines()
        .map(|l| l.trim().trim_end_matches(',').trim_matches('"'))
        .filter(|l| !l.is_empty())
        .collect();
    let expected: Vec<&str> = include_str!("skia-rect-contour-expected.txt")
        .lines()
        .collect();
    assert_eq!(cases.len(), expected.len());
    for (case, skia) in cases.iter().zip(&expected) {
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
                "Z" => b.close(),
                _ => panic!("unknown verb {v}"),
            }
        }
        let got = match b.finish().unwrap().is_rect() {
            None => "0".to_string(),
            Some(rc) => format!(
                "1 {} {} {} {} {} {}",
                rc.rect.left(),
                rc.rect.top(),
                rc.rect.right(),
                rc.rect.bottom(),
                u8::from(rc.closed),
                u8::from(rc.clockwise)
            ),
        };
        assert_eq!(&got, skia, "{case}");
    }
}
