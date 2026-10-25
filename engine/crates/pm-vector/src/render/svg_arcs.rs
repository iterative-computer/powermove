//! Fine cubic approximations before usvg's fixed 0.1 user-unit arc conversion.
//! The source rewrite is confined to geometry that has the same SVG meaning
//! after conversion. In particular, CSS type selectors and relative shape
//! lengths must be left to usvg.

use core::ops::Range;

use kurbo::{Arc, BezPath, Ellipse, PathEl, Point, Rect, Shape, SvgArc, Vec2};

use super::super::svg_depth::{Tag, for_each_tag};
use super::attributes;

// At the gate's 800% scale (9.6 device px per SVG user unit), this is below
// 0.00001 device px. Even a 1,000x zoom keeps approximation error below
// 0.001 device px; usvg's f32 path storage dominates at ordinary coordinates.
const ARC_TOLERANCE: f64 = 0.000001;
const MAX_COORDINATE: f64 = 1_000_000.0;

pub(super) fn normalize(text: &str) -> Option<String> {
    let mut css_shapes = false;
    for_each_tag(text.as_bytes(), |tag| {
        if let Tag::Open { start, end, .. } = tag {
            let name = text[start + 1..end]
                .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
                .next()
                .unwrap_or("");
            css_shapes |= name.rsplit(':').next() == Some("style");
        }
    });
    let markers = text.contains("marker-") || text.contains("marker:") || text.contains("marker=");
    let mut tags = Vec::new();
    for_each_tag(text.as_bytes(), |tag| tags.push(tag));
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    for (i, tag) in tags.iter().enumerate() {
        let Tag::Open { start, end, empty } = *tag else {
            continue;
        };
        let source = &text[start..end];
        let name_end = source
            .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
            .unwrap_or(source.len());
        let name = &source[1..name_end];
        let attrs = attributes(source);
        // A shape written `<rect …></rect>` (Sketch's spelling) is a shape
        // without content, as `<rect …/>` is.
        let close = match tags.get(i + 1) {
            Some(Tag::Close { start: c, end: e })
                if !empty
                    && text[end..*c].trim().is_empty()
                    && text[*c + 2..*e - 1].trim() == name =>
            {
                Some(*c)
            }
            _ => None,
        };
        if name == "path" && !markers {
            if let Some((_, d, range)) = attrs.iter().find(|(n, _, _)| *n == "d")
                && let Some(new_d) = precise_path(d)
            {
                // Attribute ranges include the name and quotes; replace
                // only the value so other attributes keep their spelling.
                let quote = source[range.clone()].find(['"', '\'']).unwrap();
                let value_start = range.start + quote + 1;
                edits.push((start + value_start..start + value_start + d.len(), new_d));
            }
        } else if !css_shapes
            && !markers
            && (empty || close.is_some())
            && matches!(name, "circle" | "ellipse" | "rect")
            && let Some(d) = shape_path(name, &attrs)
        {
            // Keep every authored attribute, including class, id, style,
            // transform, and geometry attributes used by [attr] selectors
            // or references. usvg ignores the extra geometry on <path>.
            edits.push((start + 1..start + name_end, "path".into()));
            let tail = if empty { 2 } else { 1 };
            edits.push((end - tail..end - tail, format!(" d=\"{d}\"")));
            if let Some(c) = close {
                edits.push((c + 2..c + 2 + name.len(), "path".into()));
            }
        }
    }
    if edits.is_empty() {
        return None;
    }
    edits.sort_by_key(|(range, _)| core::cmp::Reverse(range.start));
    let mut out = text.to_owned();
    for (range, replacement) in edits {
        out.replace_range(range, &replacement);
    }
    Some(out)
}

fn length(attrs: &[(&str, &str, Range<usize>)], name: &str, default: f64) -> Option<f64> {
    let Some((_, value, _)) = attrs.iter().find(|(n, _, _)| *n == name) else {
        return Some(default);
    };
    let s = value.trim();
    let (s, factor) = if let Some(s) = s.strip_suffix("px") {
        (s, 1.0)
    } else if let Some(s) = s.strip_suffix("in") {
        (s, 96.0)
    } else if let Some(s) = s.strip_suffix("cm") {
        (s, 96.0 / 2.54)
    } else if let Some(s) = s.strip_suffix("mm") {
        (s, 96.0 / 25.4)
    } else if let Some(s) = s.strip_suffix("pt") {
        (s, 96.0 / 72.0)
    } else if let Some(s) = s.strip_suffix("pc") {
        (s, 16.0)
    } else {
        // Percentages and font-relative lengths require context from
        // usvg's unit resolver. Keep those shapes in their original form.
        (s, 1.0)
    };
    let n = s.parse::<f64>().ok()? * factor;
    (n.is_finite() && n.abs() <= MAX_COORDINATE).then_some(n)
}

fn shape_path(name: &str, attrs: &[(&str, &str, Range<usize>)]) -> Option<String> {
    if attrs.iter().any(|(n, _, _)| *n == "d")
        || attrs.iter().any(|(n, v, _)| {
            *n == "style"
                && v.split(';').any(|declaration| {
                    declaration.split_once(':').is_some_and(|(property, _)| {
                        matches!(
                            property.trim(),
                            "cx" | "cy" | "r" | "rx" | "ry" | "width" | "height" | "x" | "y" | "d"
                        )
                    })
                })
        })
    {
        return None;
    }
    let path = match name {
        "circle" => {
            let cx = length(attrs, "cx", 0.0)?;
            let cy = length(attrs, "cy", 0.0)?;
            let r = length(attrs, "r", 0.0)?;
            if r <= 0.0 {
                return None;
            }
            Ellipse::new((cx, cy), (r, r), 0.0).to_path(ARC_TOLERANCE)
        }
        "ellipse" => {
            let cx = length(attrs, "cx", 0.0)?;
            let cy = length(attrs, "cy", 0.0)?;
            let rx = length(attrs, "rx", f64::NAN)?;
            let ry = length(attrs, "ry", f64::NAN)?;
            // SVG 2 copies the specified radius to the missing one.
            let rx = if rx.is_nan() || rx < 0.0 { ry } else { rx };
            let ry = if ry.is_nan() || ry < 0.0 { rx } else { ry };
            if !rx.is_finite() || !ry.is_finite() || rx <= 0.0 || ry <= 0.0 {
                return None;
            }
            Ellipse::new((cx, cy), (rx, ry), 0.0).to_path(ARC_TOLERANCE)
        }
        "rect" => {
            let x = length(attrs, "x", 0.0)?;
            let y = length(attrs, "y", 0.0)?;
            let width = length(attrs, "width", 0.0)?;
            let height = length(attrs, "height", 0.0)?;
            if width <= 0.0 || height <= 0.0 {
                return None;
            }
            let mut rx = length(attrs, "rx", f64::NAN)?;
            let mut ry = length(attrs, "ry", f64::NAN)?;
            if rx < 0.0 {
                rx = f64::NAN;
            }
            if ry < 0.0 {
                ry = f64::NAN;
            }
            if rx.is_nan() && ry.is_nan() {
                return None;
            } else {
                let resolved_rx = if rx.is_nan() { ry } else { rx };
                let resolved_ry = if ry.is_nan() { rx } else { ry };
                let rx = resolved_rx.clamp(0.0, width / 2.0);
                let ry = resolved_ry.clamp(0.0, height / 2.0);
                if rx == 0.0 || ry == 0.0 {
                    Rect::new(x, y, x + width, y + height).to_path(ARC_TOLERANCE)
                } else {
                    rounded_rect(x, y, width, height, rx, ry)
                }
            }
        }
        _ => return None,
    };
    // to_svg uses absolute commands and enough decimal precision to retain
    // the authored f64 coordinates until usvg stores its f32 path.
    Some(path.to_svg())
}

fn rounded_rect(x: f64, y: f64, w: f64, h: f64, rx: f64, ry: f64) -> BezPath {
    let mut p = BezPath::new();
    p.move_to((x + rx, y));
    p.line_to((x + w - rx, y));
    for (center, start, next) in [
        (
            (x + w - rx, y + ry),
            -core::f64::consts::FRAC_PI_2,
            (x + w, y + h - ry),
        ),
        ((x + w - rx, y + h - ry), 0.0, (x + rx, y + h)),
        (
            (x + rx, y + h - ry),
            core::f64::consts::FRAC_PI_2,
            (x, y + ry),
        ),
        ((x + rx, y + ry), core::f64::consts::PI, (x + rx, y)),
    ] {
        Arc::new(center, (rx, ry), start, core::f64::consts::FRAC_PI_2, 0.0)
            .to_cubic_beziers(ARC_TOLERANCE, |a, b, c| p.curve_to(a, b, c));
        if next != (x + rx, y) {
            p.line_to(next);
        }
    }
    p.close_path();
    p
}

// Rewrite only A/a, leaving every other command's source spelling intact.
// Kurbo parses the prefix to obtain the exact current point and subpath
// position; it is never used for the arc's 0.1-tolerance cubic geometry.
fn precise_path(d: &str) -> Option<String> {
    if !d.as_bytes().iter().any(|b| matches!(b, b'A' | b'a')) || d.contains('&') {
        return None;
    }
    let bytes = d.as_bytes();
    let (mut i, mut emitted) = (0, 0);
    let mut out = String::with_capacity(d.len() + 128);
    let mut changed = false;
    let mut last_command = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if !c.is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        if matches!(c, b'S' | b's') && matches!(last_command, b'A' | b'a') {
            // SVG S after A has its first control point at the current
            // point. After A->C, usvg would instead reflect the last arc
            // cubic control point. Expand this S command explicitly.
            out.push_str(&d[emitted..i]);
            let prefix = BezPath::from_svg(&out).ok()?;
            let mut from = current_point(&prefix)?;
            let relative = c == b's';
            let mut previous_control: Option<Point> = None;
            i += 1;
            let mut count = 0;
            loop {
                let mut at = i;
                separators(bytes, &mut at);
                if at == bytes.len() || bytes[at].is_ascii_alphabetic() {
                    i = at;
                    break;
                }
                let x2 = number(d, &mut at)?;
                let y2 = number(d, &mut at)?;
                let x = number(d, &mut at)?;
                let y = number(d, &mut at)?;
                let control2 = if relative {
                    from + Vec2::new(x2, y2)
                } else {
                    Point::new(x2, y2)
                };
                let to = if relative {
                    from + Vec2::new(x, y)
                } else {
                    Point::new(x, y)
                };
                let control1 = previous_control.map_or(from, |p| from + (from - p));
                if [control1.x, control1.y, control2.x, control2.y, to.x, to.y]
                    .iter()
                    .any(|n| !n.is_finite() || n.abs() > MAX_COORDINATE)
                {
                    return None;
                }
                out.push_str(&format!(
                    " C {} {} {} {} {} {}",
                    control1.x, control1.y, control2.x, control2.y, to.x, to.y
                ));
                from = to;
                previous_control = Some(control2);
                count += 1;
                i = at;
            }
            if count == 0 {
                return None;
            }
            emitted = i;
            last_command = c;
            continue;
        }
        if c != b'A' && c != b'a' {
            if matches!(
                c,
                b'M' | b'm'
                    | b'L'
                    | b'l'
                    | b'H'
                    | b'h'
                    | b'V'
                    | b'v'
                    | b'C'
                    | b'c'
                    | b'S'
                    | b's'
                    | b'Q'
                    | b'q'
                    | b'T'
                    | b't'
                    | b'Z'
                    | b'z'
            ) {
                last_command = c;
            }
            i += 1;
            continue;
        }
        out.push_str(&d[emitted..i]);
        let prefix = BezPath::from_svg(&out).ok()?;
        let mut from = current_point(&prefix)?;
        let relative = c == b'a';
        i += 1;
        let mut count = 0;
        loop {
            let mut at = i;
            separators(bytes, &mut at);
            if at == bytes.len() || bytes[at].is_ascii_alphabetic() {
                i = at;
                break;
            }
            let rx = number(d, &mut at)?;
            let ry = number(d, &mut at)?;
            let rotation = number(d, &mut at)?;
            let large_arc = flag(bytes, &mut at)?;
            let sweep = flag(bytes, &mut at)?;
            let x = number(d, &mut at)?;
            let y = number(d, &mut at)?;
            let to = if relative {
                from + Vec2::new(x, y)
            } else {
                Point::new(x, y)
            };
            if [from.x, from.y, to.x, to.y, rx, ry]
                .iter()
                .any(|n| !n.is_finite() || n.abs() > MAX_COORDINATE)
            {
                return None;
            }
            let arc = SvgArc {
                from,
                to,
                radii: Vec2::new(rx, ry),
                x_rotation: rotation.to_radians(),
                large_arc,
                sweep,
            };
            if let Some(arc) = Arc::from_svg_arc(&arc) {
                let mut curves = Vec::new();
                arc.to_cubic_beziers(ARC_TOLERANCE, |a, b, c| curves.push((a, b, c)));
                let last = curves.len().checked_sub(1)?;
                for (n, (a, b, c)) in curves.into_iter().enumerate() {
                    let c = if n == last { to } else { c };
                    out.push_str(&format!(
                        " C {} {} {} {} {} {}",
                        a.x, a.y, b.x, b.y, c.x, c.y
                    ));
                }
            } else if from != to {
                out.push_str(&format!(" L {} {}", to.x, to.y));
            }
            from = to;
            count += 1;
            i = at;
        }
        if count == 0 {
            return None;
        }
        emitted = i;
        changed = true;
        last_command = c;
    }
    if !changed {
        return None;
    }
    out.push_str(&d[emitted..]);
    BezPath::from_svg(&out).ok()?;
    Some(out)
}

fn current_point(path: &BezPath) -> Option<Point> {
    match path.elements().last()? {
        PathEl::MoveTo(p) | PathEl::LineTo(p) => Some(*p),
        PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => Some(*p),
        PathEl::ClosePath => path.elements().iter().rev().find_map(|e| {
            if let PathEl::MoveTo(p) = e {
                Some(*p)
            } else {
                None
            }
        }),
    }
}

fn separators(bytes: &[u8], at: &mut usize) {
    while *at < bytes.len() && (bytes[*at].is_ascii_whitespace() || bytes[*at] == b',') {
        *at += 1;
    }
}

fn flag(bytes: &[u8], at: &mut usize) -> Option<bool> {
    separators(bytes, at);
    let result = match bytes.get(*at)? {
        b'0' => false,
        b'1' => true,
        _ => return None,
    };
    *at += 1;
    Some(result)
}

fn number(s: &str, at: &mut usize) -> Option<f64> {
    let b = s.as_bytes();
    separators(b, at);
    let start = *at;
    if b.get(*at).is_some_and(|c| matches!(c, b'+' | b'-')) {
        *at += 1;
    }
    let mut digits = 0;
    while b.get(*at).is_some_and(u8::is_ascii_digit) {
        *at += 1;
        digits += 1;
    }
    if b.get(*at) == Some(&b'.') {
        *at += 1;
        while b.get(*at).is_some_and(u8::is_ascii_digit) {
            *at += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    if b.get(*at).is_some_and(|c| matches!(c, b'e' | b'E')) {
        *at += 1;
        if b.get(*at).is_some_and(|c| matches!(c, b'+' | b'-')) {
            *at += 1;
        }
        let exp = *at;
        while b.get(*at).is_some_and(u8::is_ascii_digit) {
            *at += 1;
        }
        if *at == exp {
            return None;
        }
    }
    let value = s[start..*at].parse::<f64>().ok()?;
    value.is_finite().then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_arc_keeps_endpoints_and_relative_commands() {
        let d = "M 10 20 a 17.5 9 30 0 1 35 5 L 50 50 A 8 13 0 01 70 60 z";
        let fine = precise_path(d).unwrap();
        assert!(!fine.contains('A') && !fine.contains('a'));
        let original = BezPath::from_svg(d).unwrap();
        let actual = BezPath::from_svg(&fine).unwrap();
        assert_eq!(current_point(&original), current_point(&actual));
        assert!(actual.elements().len() > original.elements().len());
        let xml = format!(r#"<svg><path id="arc" stroke="red" d="{d}"/></svg>"#);
        let normalized = normalize(&xml).unwrap();
        let doc = usvg::roxmltree::Document::parse(&normalized).unwrap();
        let node = doc
            .descendants()
            .find(|n| n.attribute("id") == Some("arc"))
            .unwrap();
        assert_eq!(node.attribute("stroke"), Some("red"));
        assert_eq!(node.attribute("d"), Some(fine.as_str()));
    }

    #[test]
    fn arc_normalization_preserves_subpaths_and_sensitive_commands() {
        let d = "M10 10 A6 4 0 0 1 20 20 z m4 0 a0 2 0 0 1 2 3";
        let fine = precise_path(d).unwrap();
        let original = BezPath::from_svg(d).unwrap();
        let actual = BezPath::from_svg(&fine).unwrap();
        assert_eq!(current_point(&original), current_point(&actual));
        assert_eq!(
            original
                .elements()
                .iter()
                .filter(|e| matches!(e, PathEl::ClosePath))
                .count(),
            actual
                .elements()
                .iter()
                .filter(|e| matches!(e, PathEl::ClosePath))
                .count()
        );
        let smooth = precise_path("M0 0 A10 10 0 0 1 10 10 S20 20 30 30").unwrap();
        assert!(smooth.ends_with("C 10 10 20 20 30 30"), "{smooth}");
        assert_eq!(
            normalize(r#"<svg><path marker-mid="url(#m)" d="M0 0 A10 10 0 0 1 10 10"/></svg>"#),
            None
        );
    }

    #[test]
    fn source_keeps_style_and_unresolved_units() {
        let styled =
            r#"<svg><style>circle { fill: red }</style><circle cx="1" cy="2" r="3"/></svg>"#;
        assert_eq!(normalize(styled), None);
        let percent = r#"<svg><circle cx="50%" r="10"/></svg>"#;
        assert_eq!(normalize(percent), None);
        let geometry_css = r#"<svg><circle r="10" style="r: 20px; fill: blue"/></svg>"#;
        assert_eq!(normalize(geometry_css), None);
    }

    #[test]
    fn basic_shapes_get_fine_curves_without_changing_paint_or_transform() {
        let source = r#"<svg viewBox="0 0 200 100"><circle id="a" cx="56" cy="24" r="17.5" fill="red" transform="rotate(7)"/><ellipse id="b" cx="90" cy="20" rx="12" ry="8"/><rect id="c" x="10" y="30" width="60" height="14" rx="6" ry="3" class="round"/></svg>"#;
        let normalized = normalize(source).unwrap();
        assert_eq!(normalized.matches("<path").count(), 3);
        assert!(normalized.contains(r#"fill="red" transform="rotate(7)""#));
        assert!(normalized.contains(r#"class="round""#));
        let doc = usvg::roxmltree::Document::parse(&normalized).unwrap();
        for id in ["a", "b", "c"] {
            let d = doc
                .descendants()
                .find(|n| n.attribute("id") == Some(id))
                .unwrap()
                .attribute("d")
                .unwrap();
            let curves = BezPath::from_svg(d)
                .unwrap()
                .elements()
                .iter()
                .filter(|e| matches!(e, PathEl::CurveTo(..)))
                .count();
            assert!(curves >= 4, "{id}: {curves}");
        }
    }

    #[test]
    fn shapes_written_with_a_close_tag_get_fine_curves_too() {
        let source = r#"<svg viewBox="0 0 120 100"><g><rect id="c" x="20" y="16" width="80" height="64" rx="8"></rect> <circle id="d" cx="60" cy="50" r="9">
</circle><rect id="e" width="5" height="5" rx="1"><title>kept</title></rect></g></svg>"#;
        let normalized = normalize(source).unwrap();
        assert_eq!(normalized.matches("<path").count(), 2, "{normalized}");
        assert_eq!(normalized.matches("</path>").count(), 2, "{normalized}");
        assert!(normalized.contains("<title>kept</title></rect>"));
        let doc = usvg::roxmltree::Document::parse(&normalized).unwrap();
        for id in ["c", "d"] {
            let node = doc
                .descendants()
                .find(|n| n.attribute("id") == Some(id))
                .unwrap();
            assert_eq!(node.tag_name().name(), "path");
            assert!(node.attribute("d").is_some(), "{id}");
        }
    }

    #[test]
    fn shape_units_and_auto_radii_follow_svg_geometry() {
        let source = r#"<svg><circle cx="1in" cy="0" r="12pt"/><rect width="4" height="14" rx="6" ry="auto"/></svg>"#;
        let normalized = normalize(source).unwrap();
        let doc = usvg::roxmltree::Document::parse(&normalized).unwrap();
        let circle = doc.descendants().find(|n| n.has_tag_name("path")).unwrap();
        let d = circle.attribute("d").unwrap();
        let path = BezPath::from_svg(d).unwrap();
        let bounds = path.bounding_box();
        assert!((bounds.x0 - 80.0).abs() < 0.001, "{bounds:?}");
        assert!((bounds.x1 - 112.0).abs() < 0.001, "{bounds:?}");
        // auto cannot be resolved by this source rewrite; usvg retains it.
        assert!(normalized.contains(r#"<rect width="4" height="14" rx="6" ry="auto"/>"#));
        let rect = r#"<svg><rect width="4" height="14" rx="6"/></svg>"#;
        let normalized = normalize(rect).unwrap();
        let doc = usvg::roxmltree::Document::parse(&normalized).unwrap();
        let d = doc
            .descendants()
            .find(|n| n.has_tag_name("path"))
            .unwrap()
            .attribute("d")
            .unwrap();
        let path = BezPath::from_svg(d).unwrap();
        assert_eq!(path.bounding_box().width(), 4.0);
        assert_eq!(path.bounding_box().height(), 14.0);
    }
}
