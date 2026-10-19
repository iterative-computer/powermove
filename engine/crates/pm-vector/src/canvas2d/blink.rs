//! Blink's Canvas2D path construction (Chromium 152:
//! `modules/canvas/canvas2d/canvas_path.cc`, `platform/geometry/
//! path_builder.cc`, `core/svg/svg_path_string_source.cc`,
//! `svg_path_builder.cc`, `svg_parser_utilities.cc`): what `moveTo`,
//! `lineTo`, `arcTo`, `ellipse`, `bezierCurveTo`, `closePath`, and
//! `new Path2D(svg)` leave in the `SkPath` that is drawn.
//!
//! Portions are derived from Chromium, Copyright The Chromium Authors, used
//! under the BSD 3-Clause license.

use super::skpath::{P, SkPath};
use crate::canvas::NegativeRadius;

/// `kPiFloat`, `kTwoPiFloat`.
const PI_F: f32 = core::f32::consts::PI;
const TWO_PI_F: f32 = PI_F * 2.0;

/// `base::saturated_cast<float>(double)`.
#[inline]
pub fn sat_f32(v: f64) -> f32 {
    if v.is_nan() {
        0.0
    } else if v >= f64::from(f32::MAX) {
        f32::MAX
    } else if v <= f64::from(f32::MIN) {
        f32::MIN
    } else {
        v as f32
    }
}

/// A canvas path: Blink's line fast path (a lone `moveTo` plus `lineTo` is
/// drawn as a line, never filled), else an `SkPath`.
#[derive(Debug, Clone, Default)]
pub struct CanvasPath {
    /// A lone move and optional line not yet moved into `path`.
    line: Option<(P, Option<P>)>,
    pub path: SkPath,
}

/// What a draw of the path sees.
#[derive(Debug, Clone)]
pub enum Drawn<'a> {
    Empty,
    Line(P, P),
    Path(&'a SkPath),
}

impl CanvasPath {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.line.is_none() && self.path.is_empty()
    }

    fn is_line(&self) -> bool {
        matches!(self.line, Some((_, Some(_))))
    }

    /// `UpdatePathFromLineOrArcIfNecessaryForMutation`.
    fn flush_line(&mut self) {
        if let Some((m, l)) = self.line.take() {
            self.path = SkPath::new();
            self.path.move_to(m);
            if let Some(l) = l {
                self.path.line_to(l);
            }
        }
    }

    /// The geometry a `fill()` or `stroke()` draws.
    pub fn drawn(&self) -> Drawn<'_> {
        match self.line {
            Some((m, Some(l))) => Drawn::Line(m, l),
            Some((_, None)) => Drawn::Empty,
            None if self.path.is_empty() => Drawn::Empty,
            None => Drawn::Path(&self.path),
        }
    }

    pub fn move_to(&mut self, x: f64, y: f64) {
        let p = [sat_f32(x), sat_f32(y)];
        if !(p[0].is_finite() && p[1].is_finite()) {
            return;
        }
        if self.is_empty() {
            self.line = Some((p, None));
        } else {
            self.flush_line();
            self.path.move_to(p);
        }
    }

    pub fn line_to(&mut self, x: f64, y: f64) {
        let p = [sat_f32(x), sat_f32(y)];
        if !(p[0].is_finite() && p[1].is_finite()) {
            return;
        }
        if self.is_empty() {
            self.line = Some((p, None));
        }
        if let Some((m, None)) = self.line {
            // `CanCreateLineTo`: a lone move becomes a line.
            self.path = SkPath::new();
            self.line = Some((m, Some(p)));
            return;
        }
        self.flush_line();
        self.path.line_to(p);
    }

    pub fn bezier_curve_to(&mut self, c1x: f64, c1y: f64, c2x: f64, c2y: f64, x: f64, y: f64) {
        let v = [c1x, c1y, c2x, c2y, x, y].map(sat_f32);
        if !v.iter().all(|f| f.is_finite()) {
            return;
        }
        self.flush_line();
        if self.path.last_pt().is_none() {
            self.path.move_to([v[0], v[1]]);
        }
        self.path.cubic_to([v[0], v[1]], [v[2], v[3]], [v[4], v[5]]);
    }

    /// `rect(x, y, w, h)`: `Path::AddRect` of the float corners
    /// `(x, y)` and `(x + w, y + h)` (`SkPath::addRect`: a move, three lines,
    /// and a close, clockwise from the top left as given, so a negative size
    /// keeps its sign).
    pub fn rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        self.flush_line();
        if ![x, y, w, h].iter().all(|v| v.is_finite()) {
            return;
        }
        let (x, y, w, h) = (sat_f32(x), sat_f32(y), sat_f32(w), sat_f32(h));
        let (r, b) = (x + w, y + h);
        self.path.move_to([x, y]);
        self.path.line_to([r, y]);
        self.path.line_to([r, b]);
        self.path.line_to([x, b]);
        self.path.close();
    }

    /// `arcTo`; `Err` for a negative radius (an `IndexSizeError`).
    pub fn arc_to(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, r: f64) -> Result<(), NegativeRadius> {
        let (x1, y1, x2, y2, r) = (sat_f32(x1), sat_f32(y1), sat_f32(x2), sat_f32(y2), sat_f32(r));
        if ![x1, y1, x2, y2, r].iter().all(|f| f.is_finite()) {
            return Ok(());
        }
        if r < 0.0 {
            return Err(NegativeRadius);
        }
        self.flush_line();
        let p1 = [x1, y1];
        let p2 = [x2, y2];
        match self.path.last_pt() {
            None => self.path.move_to(p1),
            Some(cur) if p1 == cur || p1 == p2 || r == 0.0 => {
                self.line_to(f64::from(x1), f64::from(y1));
            }
            Some(_) => self.path.arc_to_tangent(p1, p2, r),
        }
        Ok(())
    }

    /// `ellipse(x, y, rx, ry, rotation, start, end, anticlockwise)` with
    /// rotation 0 (the only form the legacy rasterizer calls); `Err` for a
    /// negative radius.
    #[allow(clippy::too_many_arguments)]
    pub fn ellipse(&mut self, x: f64, y: f64, rx: f64, ry: f64, start: f64, end: f64, anticlockwise: bool) -> Result<(), NegativeRadius> {
        let (x, y, rx, ry) = (sat_f32(x), sat_f32(y), sat_f32(rx), sat_f32(ry));
        let (mut start, mut end) = (sat_f32(start), sat_f32(end));
        if ![x, y, rx, ry, start, end].iter().all(|f| f.is_finite()) {
            return Ok(());
        }
        if rx < 0.0 || ry < 0.0 {
            return Err(NegativeRadius);
        }
        self.flush_line();
        canonicalize_angle(&mut start, &mut end);
        let adjusted = adjust_end_angle(start, end, anticlockwise);
        if rx == 0.0 || ry == 0.0 || start == adjusted {
            self.degenerate_ellipse(x, y, rx, ry, start, adjusted, anticlockwise);
            return Ok(());
        }
        // PathBuilder::AddEllipse without rotation.
        let oval = [x - rx, y - ry, x + rx, y + ry];
        let start_deg = start * (180.0 / PI_F);
        let sweep_deg = (adjusted - start) * (180.0 / PI_F);
        if web_core_float_nearly_equal(sweep_deg.abs(), 360.0) {
            let s180 = 180.0f32.copysign(sweep_deg);
            self.path.arc_to_oval(oval, start_deg, s180, false);
            self.path.arc_to_oval(oval, start_deg + s180, s180, false);
        } else {
            self.path.arc_to_oval(oval, start_deg, sweep_deg, false);
        }
        Ok(())
    }

    /// `DegenerateEllipse` (rotation 0): lines to the start point, every
    /// quarter-turn extreme in between, and the end point.
    #[allow(clippy::too_many_arguments)]
    fn degenerate_ellipse(&mut self, x: f32, y: f32, rx: f32, ry: f32, start: f32, end: f32, anticlockwise: bool) {
        let half_pi = PI_F / 2.0;
        let line_at = |this: &mut Self, a: f32| {
            let p = [rx * libm::cosf(a), ry * libm::sinf(a)];
            this.line_to(f64::from(x + p[0]), f64::from(y + p[1]));
        };
        line_at(self, start);
        if (rx == 0.0 && ry == 0.0) || start == end {
            return;
        }
        if !anticlockwise {
            let mut a = start - libm::fmodf(start, half_pi) + half_pi;
            while a < end {
                line_at(self, a);
                a += half_pi;
            }
        } else {
            let mut a = start - libm::fmodf(start, half_pi);
            while a > end {
                line_at(self, a);
                a -= half_pi;
            }
        }
        line_at(self, end);
    }

    pub fn close_path(&mut self) {
        if self.is_empty() {
            return;
        }
        if self.is_line() {
            let (m, l) = self.line.unwrap();
            if m == l.unwrap() {
                // A zero-length line: closePath leaves a lone move.
                self.line = Some((m, None));
                self.path = SkPath::new();
                return;
            }
        }
        self.flush_line();
        self.path.close();
    }
}

/// `WebCoreFloatNearlyEqual`.
fn web_core_float_nearly_equal(a: f32, b: f32) -> bool {
    if a == b {
        return true;
    }
    (a - b).abs() <= 4.0 * f32::EPSILON * a.abs().max(b.abs())
}

/// `CanonicalizeAngle`.
fn canonicalize_angle(start: &mut f32, end: &mut f32) {
    let mut s = libm::fmodf(*start, TWO_PI_F);
    if s < 0.0 {
        s += TWO_PI_F;
        if s >= TWO_PI_F {
            s -= TWO_PI_F;
        }
    }
    let delta = s - *start;
    *start = s;
    *end += delta;
}

/// `AdjustEndAngle`.
fn adjust_end_angle(start: f32, end: f32, anticlockwise: bool) -> f32 {
    let mut new_end = end;
    if !anticlockwise && end - start >= TWO_PI_F {
        new_end = start + TWO_PI_F;
    } else if anticlockwise && start - end >= TWO_PI_F {
        new_end = start - TWO_PI_F;
    } else if !anticlockwise && start > end {
        new_end = start + (TWO_PI_F - libm::fmodf(start - end, TWO_PI_F));
    } else if anticlockwise && start < end {
        new_end = start - (TWO_PI_F - libm::fmodf(end - start, TWO_PI_F));
    }
    new_end
}

/// `GenericParseNumber<float>` with leading and trailing whitespace and an
/// optional comma after.
fn parse_number(s: &[u8], i: &mut usize) -> Option<f32> {
    skip_spaces(s, i);
    let mut j = *i;
    let mut sign = 1.0f32;
    if j < s.len() && s[j] == b'+' {
        j += 1;
    } else if j < s.len() && s[j] == b'-' {
        j += 1;
        sign = -1.0;
    }
    if j >= s.len() || !(s[j].is_ascii_digit() || s[j] == b'.') {
        return None;
    }
    let start = j;
    while j < s.len() && s[j].is_ascii_digit() {
        j += 1;
    }
    let mut integer = 0.0f32;
    if j > start {
        let mut multiplier = 1.0f32;
        let mut k = j;
        while k > start {
            integer += multiplier * f32::from(s[k - 1] - b'0');
            multiplier *= 10.0;
            k -= 1;
        }
        if !integer.is_finite() {
            return None;
        }
    }
    let mut decimal = 0.0f32;
    let mut digits = j - start;
    if j < s.len() && s[j] == b'.' {
        j += 1;
        if j >= s.len() || !s[j].is_ascii_digit() {
            return None;
        }
        let mut frac = 1.0f32;
        while j < s.len() && s[j].is_ascii_digit() {
            frac *= 0.1;
            decimal += f32::from(s[j] - b'0') * frac;
            j += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    let mut number = integer + decimal;
    number *= sign;
    if j + 1 < s.len() && (s[j] == b'e' || s[j] == b'E') && s[j + 1] != b'x' && s[j + 1] != b'm' {
        j += 1;
        let mut neg = false;
        if s[j] == b'+' {
            j += 1;
        } else if s[j] == b'-' {
            j += 1;
            neg = true;
        }
        if j >= s.len() || !s[j].is_ascii_digit() {
            return None;
        }
        let mut exponent = 0.0f32;
        while j < s.len() && s[j].is_ascii_digit() {
            exponent *= 10.0;
            exponent += f32::from(s[j] - b'0');
            j += 1;
        }
        if neg {
            exponent = -exponent;
        }
        if exponent > 38.0 {
            return None;
        }
        if exponent != 0.0 {
            number *= 10f64.powf(f64::from(exponent)) as f32;
        }
    }
    if !number.is_finite() {
        return None;
    }
    *i = j;
    skip_spaces_or_delimiter(s, i);
    Some(number)
}

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c')
}

fn skip_spaces(s: &[u8], i: &mut usize) {
    while *i < s.len() && is_space(s[*i]) {
        *i += 1;
    }
}

fn skip_spaces_or_delimiter(s: &[u8], i: &mut usize) {
    skip_spaces(s, i);
    if *i < s.len() && s[*i] == b',' {
        *i += 1;
        skip_spaces(s, i);
    }
}

fn parse_flag(s: &[u8], i: &mut usize) -> Option<bool> {
    let f = match s.get(*i)? {
        b'0' => false,
        b'1' => true,
        _ => return None,
    };
    *i += 1;
    skip_spaces_or_delimiter(s, i);
    Some(f)
}

/// `BuildPathFromString`: SVG path data as `new Path2D(d)` builds it. Parsing
/// stops at the first error, keeping the segments before it.
pub fn path_from_svg(d: &str) -> SkPath {
    let s = d.as_bytes();
    let mut path = SkPath::new();
    let mut i = 0usize;
    skip_spaces(s, &mut i);
    let mut prev: Option<u8> = None;
    let mut current: P = [0.0, 0.0];
    let mut subpath: P = [0.0, 0.0];
    let mut last_control: P = [0.0, 0.0];
    let add = |a: P, b: P| [a[0] + b[0], a[1] + b[1]];
    while i < s.len() {
        let look = s[i];
        let cmd = if b"MmLlHhVvCcSsQqTtAaZz".contains(&look) {
            if prev.is_none() && look != b'M' && look != b'm' {
                break;
            }
            i += 1;
            look
        } else {
            let Some(p) = prev else { break };
            let number_start = look.is_ascii_digit() || matches!(look, b'+' | b'-' | b'.');
            if !number_start || p == b'Z' || p == b'z' {
                break;
            }
            match p {
                b'M' => b'L',
                b'm' => b'l',
                other => other,
            }
        };
        macro_rules! num {
            () => {
                match parse_number(s, &mut i) {
                    Some(v) => v,
                    None => break,
                }
            };
        }
        let rel = cmd.is_ascii_lowercase();
        let base = if rel { current } else { [0.0, 0.0] };
        let last_cmd = prev.map(|c| c.to_ascii_uppercase());
        match cmd.to_ascii_uppercase() {
            b'Z' => {
                skip_spaces(s, &mut i);
                path.close();
                current = subpath;
            }
            b'M' => {
                let p = [num!(), num!()];
                let p = if rel { add(current, p) } else { p };
                path.move_to(p);
                subpath = p;
                current = p;
            }
            b'L' => {
                let p = [num!(), num!()];
                let p = if rel { add(current, p) } else { p };
                path.line_to(p);
                current = p;
            }
            b'H' => {
                let x = num!();
                let p = if rel { add(current, [x, 0.0]) } else { [x, current[1]] };
                path.line_to(p);
                current = p;
            }
            b'V' => {
                let y = num!();
                let p = if rel { add(current, [0.0, y]) } else { [current[0], y] };
                path.line_to(p);
                current = p;
            }
            b'C' => {
                let (c0, c1, p) = ([num!(), num!()], [num!(), num!()], [num!(), num!()]);
                let (c0, c1, p) = if rel { (add(base, c0), add(base, c1), add(base, p)) } else { (c0, c1, p) };
                path.cubic_to(c0, c1, p);
                last_control = c1;
                current = p;
            }
            b'S' => {
                let (c1, p) = ([num!(), num!()], [num!(), num!()]);
                let (c1, p) = if rel { (add(base, c1), add(base, p)) } else { (c1, p) };
                let mut c0 = current;
                if matches!(last_cmd, Some(b'C') | Some(b'S')) {
                    c0 = add(c0, [current[0] - last_control[0], current[1] - last_control[1]]);
                }
                path.cubic_to(c0, c1, p);
                last_control = c1;
                current = p;
            }
            b'Q' => {
                let (c0, p) = ([num!(), num!()], [num!(), num!()]);
                let (c0, p) = if rel { (add(base, c0), add(base, p)) } else { (c0, p) };
                path.quad_to(c0, p);
                last_control = c0;
                current = p;
            }
            b'T' => {
                let p = [num!(), num!()];
                let p = if rel { add(base, p) } else { p };
                let mut c0 = current;
                if matches!(last_cmd, Some(b'Q') | Some(b'T')) {
                    c0 = add(c0, [current[0] - last_control[0], current[1] - last_control[1]]);
                }
                path.quad_to(c0, p);
                last_control = c0;
                current = p;
            }
            b'A' => {
                let rx = num!();
                let ry = num!();
                let angle = num!();
                let Some(large) = parse_flag(s, &mut i) else { break };
                let Some(sweep) = parse_flag(s, &mut i) else { break };
                let p = [num!(), num!()];
                let p = if rel { add(base, p) } else { p };
                path.arc_to_svg([rx, ry], angle, large, sweep, p);
                current = p;
            }
            _ => break,
        }
        prev = Some(cmd);
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas2d::skpath::Verb;

    #[test]
    fn lone_line_stays_a_line() {
        let mut p = CanvasPath::new();
        p.move_to(0.0, 5.0);
        p.line_to(10.0, 5.0);
        assert!(matches!(p.drawn(), Drawn::Line([0.0, 5.0], [10.0, 5.0])));
        p.line_to(10.0, 9.0);
        assert!(matches!(p.drawn(), Drawn::Path(_)));
    }

    #[test]
    fn numbers_parse_like_blink() {
        let mut i = 0;
        assert_eq!(parse_number(b"12.5,", &mut i), Some(12.5));
        assert_eq!(i, 5);
        let mut i = 0;
        // Blink accumulates decimals with frac *= 0.1 in float.
        let v = parse_number(b"0.3", &mut i).unwrap();
        assert_eq!(v, 0.1f32 * 3.0);
        let mut i = 0;
        assert_eq!(parse_number(b"-1e2", &mut i), Some(-100.0));
    }

    #[test]
    fn svg_relative_commands_and_arcs() {
        let p = path_from_svg("M 10 0 l 5 0 a 5 5 0 0 1 5 5 L 20 20 Z");
        assert_eq!(p.verbs[0], Verb::Move);
        assert_eq!(p.pts[1], [15.0, 0.0]);
        assert!(p.verbs.iter().any(|v| matches!(v, Verb::Conic(_))));
        assert_eq!(*p.verbs.last().unwrap(), Verb::Close);
        // Errors keep the segments before them.
        let e = path_from_svg("M 0 0 L 5 5 L x");
        assert_eq!(e.verbs.len(), 2);
    }
}
