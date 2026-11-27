//! pm-vector against today's TypeScript geometry, through
//! tests/vectors/geometry-vectors.json written by
//! tests/vectors/generate-geometry-vectors.ts (bun, recording Canvas2D).
//!
//! Comparisons are bit for bit, with two measured exceptions:
//!
//! - Values computed through `cos`, `sin`, or `tan` (polygon and star
//!   vertices, path rotations and repeater rotations, and everything composed
//!   from them) match within a relative error of [`TRIG_TOLERANCE`]. pm-vector
//!   uses the `libm` crate on every target (section 7, determinism); bun's
//!   JavaScriptCore uses the platform libm and Electron's V8 its own
//!   implementation, and the three differ in the last bit for some inputs
//!   (`trig_sources_agree_within_ulps` measures how often).
//! - Mask coverage, whose reference is a double-precision transliteration of
//!   the FRAG_MASK shader while pm-vector evaluates it in single precision as
//!   the GPU does.

use kurbo::{BezPath, PathEl, Point};
use pm_vector::CanvasPath;
use pm_vector::canvas::CanvasTransform;
use pm_vector::corner::{self, CornerContent};
use pm_vector::mask::{LayerMasks, MaskMode, MaskShape, MaskValues, layer_inverse};
use pm_vector::path::{
    PathLayer, PathNode, PathValues, Vertex, group_matrix, path_points, trace_path,
};
use pm_vector::shape::{ShapeContent, ShapeKind, ShapeRaster, StrokePaint, shape_geometry};
use serde_json::Value;

/// Largest relative error accepted for trigonometry-dependent values,
/// relative to the largest magnitude among the compared numbers.
const TRIG_TOLERANCE: f64 = 1e-14;

/// Counts exact and tolerance matches for trigonometry-dependent values.
#[derive(Default, Debug)]
struct Near {
    exact: u64,
    near: u64,
    worst: f64,
}

impl Near {
    fn check(&mut self, a: &[f64], b: &[f64]) -> bool {
        if a.len() != b.len() {
            return false;
        }
        if same_all(a, b) {
            self.exact += 1;
            return true;
        }
        let scale = a
            .iter()
            .chain(b)
            .filter(|v| v.is_finite())
            .fold(1f64, |m, v| m.max(v.abs()));
        for (&x, &y) in a.iter().zip(b) {
            if same(x, y) || x == y {
                continue;
            }
            if !(x.is_finite() && y.is_finite()) {
                return false;
            }
            let error = (x - y).abs() / scale;
            self.worst = self.worst.max(error);
            if error > TRIG_TOLERANCE {
                return false;
            }
        }
        self.near += 1;
        true
    }

    fn check_path(&mut self, a: &BezPath, b: &BezPath) -> bool {
        let (ta, ca) = flatten(a);
        let (tb, cb) = flatten(b);
        ta == tb && self.check(&ca, &cb)
    }
}

/// Element tags and coordinates of a path.
fn flatten(p: &BezPath) -> (Vec<u8>, Vec<f64>) {
    let (mut tags, mut coords) = (Vec::new(), Vec::new());
    for el in p.elements() {
        let (tag, points): (u8, &[Point]) = match el {
            PathEl::MoveTo(p) => (0, core::slice::from_ref(p)),
            PathEl::LineTo(p) => (1, core::slice::from_ref(p)),
            PathEl::QuadTo(..) => panic!("no quadratic segments are produced"),
            PathEl::CurveTo(a, b, c) => {
                tags.push(3);
                coords.extend([a.x, a.y, b.x, b.y, c.x, c.y]);
                continue;
            }
            PathEl::ClosePath => (4, &[]),
        };
        tags.push(tag);
        points.iter().for_each(|p| coords.extend([p.x, p.y]));
    }
    (tags, coords)
}

fn vectors() -> Value {
    serde_json::from_str(include_str!("vectors/geometry-vectors.json")).unwrap()
}

/// A number as the generator encodes it; missing or null is NaN
/// (`Number(undefined)`).
fn num(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap(),
        Value::String(s) => match s.as_str() {
            "NaN" => f64::NAN,
            "Infinity" => f64::INFINITY,
            "-Infinity" => f64::NEG_INFINITY,
            "-0" => -0.0,
            other => panic!("not a number: {other}"),
        },
        Value::Null => f64::NAN,
        other => panic!("not a number: {other}"),
    }
}

fn field(v: &Value, key: &str) -> f64 {
    v.get(key).map_or(f64::NAN, num)
}

fn nums(v: &Value) -> Vec<f64> {
    v.as_array().unwrap().iter().map(num).collect()
}

fn matrix(v: &Value) -> [f64; 6] {
    nums(v).try_into().unwrap()
}

/// Bitwise equality, so NaN matches NaN and the sign of zero counts.
fn same(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}

fn same_all(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| same(x, y))
}

/// Element-wise exact path equality (sign of zero ignored: Canvas2D cannot
/// observe it in coordinates).
fn same_path(a: &BezPath, b: &BezPath) -> bool {
    a.elements() == b.elements()
}

/// Replays recorded Canvas2D path calls through pm-vector's CanvasPath.
fn replay(ops: &Value) -> BezPath {
    let mut p = CanvasPath::new();
    for op in ops.as_array().unwrap() {
        let op = op.as_array().unwrap();
        let a: Vec<f64> = op[1..].iter().map(num).collect();
        match op[0].as_str().unwrap() {
            "M" => p.move_to(a[0], a[1]),
            "L" => p.line_to(a[0], a[1]),
            "C" => p.bezier_curve_to(a[0], a[1], a[2], a[3], a[4], a[5]),
            "Z" => p.close_path(),
            "R" => p.rect(a[0], a[1], a[2], a[3]),
            "A" => p.arc_to(a[0], a[1], a[2], a[3], a[4]).unwrap(),
            "E" => p.ellipse(a[0], a[1], a[2], a[3], a[4], a[5], a[6]).unwrap(),
            other => panic!("unknown op {other}"),
        }
    }
    p.finish()
}

/// An independent parser for the path data `roundedRectPath` emits
/// (`M`, `L`, `l`, `c`, `a r r 0 0 1 dx dy`, `Z`), as `Path2D` reads it.
fn parse_svg(d: &str) -> BezPath {
    let tokens: Vec<&str> = d.split_whitespace().collect();
    let mut p = CanvasPath::new();
    let mut at = Point::ZERO;
    let mut i = 0;
    let n = |i: usize| -> f64 { tokens[i].parse().unwrap() };
    while i < tokens.len() {
        match tokens[i] {
            "M" => {
                at = Point::new(n(i + 1), n(i + 2));
                p.move_to(at.x, at.y);
                i += 3;
            }
            "L" => {
                at = Point::new(n(i + 1), n(i + 2));
                p.line_to(at.x, at.y);
                i += 3;
            }
            "l" => {
                at = Point::new(at.x + n(i + 1), at.y + n(i + 2));
                p.line_to(at.x, at.y);
                i += 3;
            }
            "c" => {
                let o = at;
                at = Point::new(o.x + n(i + 5), o.y + n(i + 6));
                p.bezier_curve_to(
                    o.x + n(i + 1),
                    o.y + n(i + 2),
                    o.x + n(i + 3),
                    o.y + n(i + 4),
                    at.x,
                    at.y,
                );
                i += 7;
            }
            "a" => {
                assert_eq!(&tokens[i + 3..i + 6], ["0", "0", "1"], "{d}");
                let r = n(i + 1);
                assert_eq!(r, n(i + 2));
                at = Point::new(at.x + n(i + 6), at.y + n(i + 7));
                p.svg_arc(r, r, false, true, at);
                i += 8;
            }
            "Z" => {
                p.close_path();
                i += 1;
            }
            other => panic!("unexpected token {other} in {d}"),
        }
    }
    p.finish()
}

fn shape_content(d: &Value) -> ShapeContent {
    let radius = |key: &str| match d.get(key) {
        None | Some(Value::Null) => None,
        Some(v) => Some(num(v)),
    };
    ShapeContent {
        shape: ShapeKind::from_name(d["shape"].as_str().unwrap()),
        w: field(d, "w"),
        h: field(d, "h"),
        radius: field(d, "radius"),
        independent_corners: d
            .get("independentCorners")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        corner_radii: [
            radius("radiusTL"),
            radius("radiusTR"),
            radius("radiusBR"),
            radius("radiusBL"),
        ],
        smoothing: field(d, "smoothing"),
        stroke: field(d, "stroke"),
        points: field(d, "points"),
    }
}

#[test]
fn corners_match_corner_geometry_ts() {
    let v = vectors();
    let cases = v["shapes"].as_array().unwrap();
    for case in cases {
        let d: CornerContent = shape_content(&case["d"]).corners();
        let expected = &case["corners"];
        assert!(
            same_all(&corner::corner_radii(&d), &nums(&expected["radii"])),
            "radii {case}"
        );
        assert!(
            same(corner::corner_smoothing(&d), num(&expected["smoothing"])),
            "smoothing {case}"
        );
        assert!(
            same(corner::corner_inset(&d), num(&expected["inset"])),
            "inset {case}"
        );
        assert_eq!(
            corner::is_uniform_circular(&d),
            expected["uniform"].as_bool().unwrap(),
            "uniform {case}"
        );
        let svg = expected["svg"].as_str().unwrap();
        assert_eq!(corner::rounded_rect_svg(&d), svg, "svg {case}");
        assert!(
            same_path(&corner::rounded_rect_path(&d), &parse_svg(svg)),
            "outline {case}"
        );
    }
    assert!(cases.len() > 400);
}

#[test]
fn shapes_match_raster_shape() {
    let v = vectors();
    let (mut drawn, mut svg_outlines, mut thrown) = (0, 0, 0);
    let mut trig = Near::default();
    for case in v["shapes"].as_array().unwrap() {
        let d = shape_content(&case["d"]);
        let scale = num(&case["scale"]);
        let raster = ShapeRaster::new(&d, scale);
        let g = &case["geometry"];
        for (got, key) in [
            (raster.pad, "pad"),
            (raster.w, "w"),
            (raster.h, "h"),
            (raster.density, "density"),
            (raster.width, "width"),
            (raster.height, "height"),
        ] {
            assert!(
                same(got, num(&g[key])),
                "{key}: {got} vs {} in {case}",
                g[key]
            );
        }
        let s = raster.selection;
        let sel = &g["selection"];
        assert!(
            same_all(
                &[s.x0, s.y0, s.x1, s.y1, s.w, s.h],
                &["x0", "y0", "x1", "y1", "w", "h"].map(|k| num(&sel[k]))
            ),
            "selection {case}"
        );

        let r = &case["raster"];
        let geometry = shape_geometry(&d);
        if r.get("error").is_some() {
            assert!(geometry.is_err(), "expected IndexSizeError {case}");
            thrown += 1;
            continue;
        }
        let geometry = geometry.unwrap_or_else(|e| panic!("{e} {case}"));
        assert!(
            same(raster.width, num(&r["width"])) && same(raster.height, num(&r["height"])),
            "{case}"
        );
        assert!(
            same(raster.pad, num(&r["anchorX"])) && same(raster.w, num(&r["w"])),
            "{case}"
        );
        let events = r["events"].as_array().unwrap();
        let fill = events.iter().find(|e| e["op"] == "fill");
        let stroke = events.iter().find(|e| e["op"] == "stroke");
        assert_eq!(fill.is_some(), geometry.fill, "fill {case}");
        assert_eq!(stroke.is_some(), geometry.stroke.is_some(), "stroke {case}");
        let to_pixels = raster.shape_to_pixels().as_coeffs();
        for e in events {
            assert!(
                same_all(&to_pixels, &nums(&e["ctm"])),
                "ctm {to_pixels:?} vs {} in {case}",
                e["ctm"]
            );
            let path = &e["path"];
            let expected = if let Some(svg) = path.get("svg") {
                let svg = svg.as_str().unwrap();
                assert_eq!(corner::rounded_rect_svg(&d.corners()), svg);
                svg_outlines += 1;
                parse_svg(svg)
            } else {
                replay(&path["ops"])
            };
            if matches!(d.shape, ShapeKind::Polygon | ShapeKind::Star) {
                assert!(
                    trig.check_path(&geometry.path, &expected),
                    "path {:?} vs {:?} in {case}",
                    geometry.path,
                    expected
                );
            } else {
                assert!(
                    same_path(&geometry.path, &expected),
                    "path {:?} vs {:?} in {case}",
                    geometry.path,
                    expected
                );
            }
        }
        if let (Some(e), Some(s)) = (stroke, &geometry.stroke) {
            assert!(
                same(s.style.width, num(&e["lineWidth"])),
                "lineWidth {case}"
            );
            let cap = if s.style.start_cap == kurbo::Cap::Round {
                "round"
            } else {
                "butt"
            };
            assert_eq!(e["lineCap"], cap, "{case}");
            assert_eq!(e["lineJoin"], "round", "{case}");
            assert!(s.style.end_cap == s.style.start_cap && s.style.join == kurbo::Join::Round);
            let color = e["style"] == case["d"]["color"];
            assert_eq!(color, s.paint == StrokePaint::Color, "stroke paint {case}");
        }
        drawn += 1;
    }
    eprintln!(
        "shapes: {drawn} drawn, {svg_outlines} smoothed outlines, {thrown} throw; polygon and star paths {trig:?}"
    );
    assert!(
        drawn > 400 && svg_outlines > 50 && thrown >= 2,
        "{drawn} {svg_outlines} {thrown}"
    );
}

fn path_values(v: &Value) -> PathValues {
    let vertices = v["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| Vertex {
            x: num(&p["x"]),
            y: num(&p["y"]),
            in_x: num(&p["inX"]),
            in_y: num(&p["inY"]),
            out_x: num(&p["outX"]),
            out_y: num(&p["outY"]),
        })
        .collect();
    PathValues {
        x: field(v, "x"),
        y: field(v, "y"),
        rotation: field(v, "rotation"),
        scale_x: field(v, "scaleX"),
        scale_y: field(v, "scaleY"),
        closed: v["closed"].as_bool().unwrap(),
        fill_enabled: v["fillEnabled"].as_bool().unwrap(),
        fill_opacity: field(v, "fillOpacity"),
        stroke_width: field(v, "strokeWidth"),
        stroke_opacity: field(v, "strokeOpacity"),
        trim_start: field(v, "trimStart"),
        trim_end: field(v, "trimEnd"),
        trim_offset: field(v, "trimOffset"),
        copies: field(v, "copies"),
        repeat_x: field(v, "repeatX"),
        repeat_y: field(v, "repeatY"),
        repeat_rotation: field(v, "repeatRotation"),
        vertices,
    }
}

/// FNV-1a 64 over the little-endian bits of every coordinate, as the generator computes it.
fn points_digest(points: &[Point]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for p in points {
        for c in [p.x, p.y] {
            for byte in c.to_bits().to_le_bytes() {
                hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    format!("{hash:016x}")
}

#[test]
fn paths_match_vector_paths_ts() {
    let v = vectors();
    let (mut trimmed, mut events_checked, mut copies_max) = (0, 0, 0);
    let (mut matrices, mut ctms, mut bounds) = (Near::default(), Near::default(), Near::default());
    for case in v["paths"].as_array().unwrap() {
        let nodes: Vec<PathNode> = case["paths"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| PathNode {
                id: p["id"].as_str().unwrap().to_owned(),
                parent: p["parent"].as_str().map(str::to_owned),
                values: path_values(&p["values"]),
            })
            .collect();
        for (i, (node, expected)) in nodes
            .iter()
            .zip(case["paths"].as_array().unwrap())
            .enumerate()
        {
            let points = path_points(&node.values);
            assert_eq!(
                points.len() as u64,
                expected["pointCount"].as_u64().unwrap()
            );
            if let Some(full) = expected.get("points") {
                let full: Vec<f64> = full.as_array().unwrap().iter().flat_map(nums).collect();
                let got: Vec<f64> = points.iter().flat_map(|p| [p.x, p.y]).collect();
                assert!(same_all(&got, &full), "points of {} in {case}", node.id);
            }
            assert_eq!(
                points_digest(&points),
                expected["pointsDigest"].as_str().unwrap(),
                "points of {}",
                node.id
            );
            let traced = trace_path(&node.values);
            assert!(
                same_path(&traced, &replay(&expected["trace"])),
                "trace of {}: {traced:?} in {case}",
                node.id
            );
            if traced
                .elements()
                .iter()
                .any(|e| matches!(e, PathEl::LineTo(_)))
                && node.values.trim_end - node.values.trim_start < 100.0
            {
                trimmed += 1;
            }
            let m = group_matrix(&nodes, i).as_coeffs();
            assert!(
                matrices.check(&m, &nums(&expected["matrix"])),
                "matrix of {}: {m:?} vs {}",
                node.id,
                expected["matrix"]
            );
        }

        let layer = PathLayer::new(&nodes);
        let r = &case["raster"];
        let b = layer.bounds;
        let sel = &r["selection"];
        assert!(
            bounds.check(
                &[b.x0, b.y0, b.x1, b.y1, b.w, b.h],
                &["x0", "y0", "x1", "y1", "w", "h"].map(|k| num(&sel[k]))
            ),
            "bounds {b:?} vs {sel}"
        );
        assert!(bounds.check(&[b.w, b.h], &[num(&r["w"]), num(&r["h"])]));
        let raster = layer.raster(num(&case["scale"]));
        assert!(
            same(raster.width, num(&r["width"])) && same(raster.height, num(&r["height"])),
            "{raster:?} vs {r}"
        );
        assert!(bounds.check(
            &[raster.anchor_x, raster.anchor_y],
            &[num(&r["anchorX"]), num(&r["anchorY"])]
        ));

        let base = layer.bitmap_transform(&raster);
        let mut expected = r["events"].as_array().unwrap().iter();
        for draw in &layer.draws {
            copies_max = copies_max.max(draw.copy + 1);
            let ctm = draw.transform_from(base);
            let mut check = |op: &str, alpha: f64| {
                let e = expected
                    .next()
                    .unwrap_or_else(|| panic!("missing {op} event"));
                assert_eq!(e["op"], op);
                assert!(
                    ctms.check(&ctm.0, &nums(&e["ctm"])),
                    "ctm {:?} vs {}",
                    ctm.0,
                    e["ctm"]
                );
                assert!(
                    same(alpha, num(&e["alpha"])),
                    "alpha {alpha} vs {}",
                    e["alpha"]
                );
                let path = &e["path"];
                let reference = match path.get("index") {
                    Some(i) => trace_path(&nodes[i.as_u64().unwrap() as usize].values),
                    None => replay(&path["ops"]),
                };
                assert!(same_path(&layer.traced[draw.path], &reference), "draw path");
                events_checked += 1;
                e
            };
            if let Some(alpha) = draw.fill {
                check("fill", alpha);
            }
            if let Some(stroke) = &draw.stroke {
                let e = check("stroke", stroke.alpha);
                assert!(
                    same(stroke.style.width, num(&e["lineWidth"])),
                    "lineWidth {} vs {}",
                    stroke.style.width,
                    e["lineWidth"]
                );
                assert_eq!(e["lineCap"], "butt");
                assert_eq!(e["lineJoin"], "miter");
            }
        }
        assert!(expected.next().is_none(), "extra events in {case}");
    }
    eprintln!(
        "paths: {trimmed} trimmed traces, {events_checked} draw events; group matrices {matrices:?}; draw transforms {ctms:?}; bounds {bounds:?}"
    );
    assert!(
        trimmed > 20 && events_checked > 1000 && copies_max == 256,
        "{trimmed} {events_checked} {copies_max}"
    );
}

fn mask_values(m: &Value) -> MaskValues {
    MaskValues {
        x: field(m, "x"),
        y: field(m, "y"),
        w: field(m, "w"),
        h: field(m, "h"),
        rotation: field(m, "rotation"),
        feather: field(m, "feather"),
        shape: MaskShape::from_name(m["shape"].as_str().unwrap()),
        mode: MaskMode::from_name(m["mode"].as_str().unwrap()),
        path: m.get("path").map(path_values),
    }
}

#[test]
fn masks_match_compositor_ts() {
    let v = vectors();
    let (mut parametric, mut path_route, mut samples, mut worst) = (0, 0, 0, 0f64);
    let mut ctms = Near::default();
    for case in v["masks"].as_array().unwrap() {
        let masks: Vec<MaskValues> = case["masks"]
            .as_array()
            .unwrap()
            .iter()
            .map(mask_values)
            .collect();
        let world = matrix(&case["world"]);
        match LayerMasks::new(&masks) {
            LayerMasks::None => {
                assert!(case.get("parametric").is_none() && case.get("path").is_none())
            }
            LayerMasks::Parametric(p) => {
                let expected = &case["parametric"];
                let inverse = layer_inverse(world);
                if expected.get("skipped").is_some() {
                    assert!(inverse.is_none(), "{case}");
                    continue;
                }
                let inverse = inverse.unwrap_or_else(|| panic!("inverse {case}"));
                assert!(
                    same_all(&inverse.as_coeffs(), &nums(&expected["inv"])),
                    "inverse {case}"
                );
                assert_eq!(p.masks.len() as u64, expected["cnt"].as_u64().unwrap());
                assert_eq!(u64::from(p.has_add), expected["hasAdd"].as_u64().unwrap());
                let (g, q) = (nums(&expected["g"]), nums(&expected["q"]));
                for (i, m) in p.masks.iter().enumerate() {
                    let k = i * 4;
                    let got_g = [m.center.x, m.center.y, m.size.0, m.size.1];
                    let got_q = [
                        m.rotation,
                        m.feather,
                        f64::from(u8::from(m.shape == MaskShape::Ellipse)),
                        f64::from(u8::from(m.mode == MaskMode::Subtract)),
                    ];
                    assert!(
                        same_all(&got_g, &g[k..k + 4]),
                        "u_g[{i}] {got_g:?} vs {:?}",
                        &g[k..k + 4]
                    );
                    assert!(
                        same_all(&got_q, &q[k..k + 4]),
                        "u_q[{i}] {got_q:?} vs {:?}",
                        &q[k..k + 4]
                    );
                }
                for s in expected["samples"].as_array().unwrap() {
                    // Skip points within 0.01 px of a hard (unfeathered) edge, where
                    // single and double precision may disagree on the step.
                    if num(&s["edge"]) < 0.01 {
                        continue;
                    }
                    let at = nums(&s["at"]);
                    let got = f64::from(p.coverage(Point::new(at[0], at[1])));
                    let diff = (got - num(&s["cov"])).abs();
                    worst = worst.max(diff);
                    assert!(
                        diff < 1e-4,
                        "coverage {got} vs {} at {at:?} in {case}",
                        s["cov"]
                    );
                    samples += 1;
                }
                parametric += 1;
            }
            LayerMasks::Path(p) => {
                let expected = &case["path"];
                assert_eq!(p.start_covered, expected["startCovered"].as_bool().unwrap());
                let items = expected["masks"].as_array().unwrap();
                assert_eq!(p.masks.len(), items.len());
                for (m, e) in p.masks.iter().zip(items) {
                    let ctm: CanvasTransform = m.transform_from(world);
                    assert!(
                        ctms.check(&ctm.0, &nums(&e["ctm"])),
                        "mask ctm {:?} vs {}",
                        ctm.0,
                        e["ctm"]
                    );
                    assert!(
                        same_path(&m.path, &replay(&e["ops"])),
                        "mask path {:?} in {case}",
                        m.path
                    );
                    let composite = if m.mode == MaskMode::Subtract {
                        "destination-out"
                    } else {
                        "source-over"
                    };
                    assert_eq!(e["composite"], composite);
                    let filter = e["filter"].as_str().unwrap();
                    let sigma: f64 = filter
                        .strip_prefix("blur(")
                        .and_then(|f| f.strip_suffix("px)"))
                        .and_then(|f| f.parse().ok())
                        .unwrap_or_else(|| panic!("filter {filter}"));
                    let got = m.blur_sigma(kurbo::Affine::new(world));
                    assert!(
                        same(got, sigma) || (got == 0.0 && sigma == 0.0),
                        "blur {got} vs {filter}"
                    );
                }
                path_route += 1;
            }
        }
    }
    eprintln!(
        "masks: {parametric} parametric, {path_route} path route, {samples} coverage samples, worst coverage error {worst:e}; path-route transforms {ctms:?}"
    );
    assert!(
        parametric > 100 && path_route > 30 && samples > 1000,
        "{parametric} {path_route} {samples}"
    );
}

/// Distance in units in the last place between two finite doubles.
fn ulps(a: f64, b: f64) -> u64 {
    let key = |x: f64| {
        let bits = x.to_bits() as i64;
        if bits < 0 { i64::MIN - bits } else { bits }
    };
    key(a).abs_diff(key(b))
}

#[test]
fn trig_sources_agree_within_ulps() {
    let v = vectors();
    let trig = &v["trig"];
    let angles = nums(&trig["angles"]);
    type Function = (&'static str, fn(f64) -> f64);
    let functions: [Function; 3] = [("cos", libm::cos), ("sin", libm::sin), ("tan", libm::tan)];
    for source in ["jsc", "v8"] {
        let Some(values) = trig.get(source) else {
            eprintln!("trig: no {source} values recorded");
            continue;
        };
        for (name, f) in functions {
            let expected = nums(&values[name]);
            let (mut exact, mut worst) = (0, 0);
            for (&x, &e) in angles.iter().zip(&expected) {
                let d = ulps(f(x), e);
                exact += u64::from(d == 0);
                worst = worst.max(d);
            }
            eprintln!(
                "trig: libm {name} vs {source} ({}): {exact} of {} identical, worst {worst} ulp",
                values["version"].as_str().unwrap_or("?"),
                angles.len()
            );
            // cos and sin are faithfully rounded everywhere; tan (used only by
            // corner smoothing, whose output is rounded to 4 decimals) may differ
            // by 2 ulp from JavaScriptCore's.
            let limit = if name == "tan" { 2 } else { 1 };
            assert!(worst <= limit, "{name} vs {source}: {worst} ulp");
        }
    }
}
