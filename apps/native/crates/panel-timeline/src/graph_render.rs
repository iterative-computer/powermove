//! Graph paint plan. Evaluated curve segments and the value domain come from
//! the native evaluator; the renderer never fits, samples or changes a curve.
use crate::render::{Error, Op, Rect, Viewport};
use pm_design::Tokens;
use serde_json::json;
#[derive(Clone, Copy, Debug)]
pub struct Point {
    pub seconds: f64,
    pub value: f64,
}
#[derive(Clone, Debug)]
pub struct Segment {
    pub from: Point,
    pub to: Point,
    pub controls: Option<[Point; 2]>,
}
#[derive(Clone, Debug)]
pub struct Curve {
    pub id: String,
    pub name: String,
    pub focused: bool,
    pub segments: Vec<Segment>,
    pub points: Vec<GraphPoint>,
}
#[derive(Clone, Debug)]
pub struct GraphPoint {
    pub id: String,
    pub point: Point,
    pub selected: bool,
    pub handles: Vec<Point>,
    pub active_handle: Option<usize>,
}
#[derive(Clone, Debug)]
pub struct Graph {
    curves: Vec<Curve>,
    pub min: f64,
    pub max: f64,
    pub grid: Vec<(f64, String)>,
    pub selection: Option<Rect>,
}
fn valid(p: Point) -> bool {
    p.seconds.is_finite() && p.value.is_finite()
}
impl Graph {
    /// Segment endpoints are monotone in time; control points may overshoot.
    pub fn new(mut curves: Vec<Curve>, min: f64, max: f64) -> Result<Self, Error> {
        if !min.is_finite() || !max.is_finite() || max <= min {
            return Err(Error::InvalidGeometry);
        }
        for c in &mut curves {
            if c.segments.iter().any(|s| {
                !valid(s.from)
                    || !valid(s.to)
                    || s.to.seconds < s.from.seconds
                    || s.controls.is_some_and(|v| v.iter().any(|p| !valid(*p)))
            }) || c
                .segments
                .windows(2)
                .any(|w| w[0].to.seconds > w[1].from.seconds)
                || c.points
                    .iter()
                    .any(|p| !valid(p.point) || p.handles.iter().any(|p| !valid(*p)))
            {
                return Err(Error::InvalidTime);
            }
        }
        for curve in &mut curves {
            curve
                .points
                .sort_by(|a, b| a.point.seconds.total_cmp(&b.point.seconds));
        }
        Ok(Self {
            curves,
            min,
            max,
            grid: vec![],
            selection: None,
        })
    }
    pub fn prepare(&self, v: Viewport, bounds: Rect, t: &Tokens) -> Result<Vec<Op>, Error> {
        if !v.valid()
            || ![bounds.x, bounds.y, bounds.width, bounds.height]
                .iter()
                .all(|x| x.is_finite())
            || bounds.width <= 0.
            || bounds.height <= 0.
            || self.grid.iter().any(|(value, _)| !value.is_finite())
        {
            return Err(Error::InvalidGeometry);
        }
        let point = |p: Point| {
            [
                v.x(p.seconds),
                bounds.y + bounds.height * (1. - (p.value - self.min) / (self.max - self.min)),
            ]
        };
        let mut ops = vec![];
        let c = &t.colors;
        for (value, label) in &self.grid {
            let y = point(Point {
                seconds: 0.,
                value: *value,
            })[1];
            let mut line = Op::new(
                format!("graph-grid:{label}"),
                "rect",
                Rect::new(bounds.x, y, bounds.width, t.metrics.border as f64),
                c.separator,
            );
            line.clip = Some(bounds);
            ops.push(line);
            let mut label_op = Op::new(
                format!("graph-value:{label}"),
                "text",
                Rect::new(bounds.x, y, bounds.width, t.typography.minimum as f64),
                c.muted,
            );
            label_op.text = Some(label.clone());
            label_op.font_size = t.typography.minimum as f64;
            label_op.clip = Some(bounds);
            ops.push(label_op);
        }
        let from = v.scroll_time;
        let to = from + (v.width - v.layer_column) / v.pps;
        for curve in &self.curves {
            let first = curve.segments.partition_point(|s| s.to.seconds < from);
            let end = curve.segments.partition_point(|s| s.from.seconds <= to);
            let mut path = Op::new(
                format!("curve:{}", curve.id),
                "curve",
                bounds,
                if curve.focused { c.text } else { c.secondary },
            );
            path.stroke = 1.5;
            path.clip = Some(bounds);
            path.detail = json!({"segments":curve.segments[first..end].iter().map(|s|json!({"from":point(s.from),"to":point(s.to),"controls":s.controls.map(|points|points.map(point))})).collect::<Vec<_>>()});
            ops.push(path);
            if let Some(segment) = curve.segments.last() {
                let p = point(segment.to);
                let mut name = Op::new(
                    format!("curve-name:{}", curve.id),
                    "text",
                    Rect::new(p[0], p[1], bounds.width, t.typography.minimum as f64),
                    if curve.focused { c.text } else { c.secondary },
                );
                name.text = Some(curve.name.clone());
                name.font_size = t.typography.minimum as f64;
                name.clip = Some(bounds);
                ops.push(name);
            }
            let start = curve.points.partition_point(|key| key.point.seconds < from);
            let end = curve.points.partition_point(|key| key.point.seconds <= to);
            for key in &curve.points[start..end] {
                if key.point.seconds < from || key.point.seconds > to {
                    continue;
                }
                let p = point(key.point);
                let size = t.metrics.keyframe as f64 - 2.;
                let mut op = Op::new(
                    format!("graph-key:{}", key.id),
                    "square",
                    Rect::new(p[0] - size / 2., p[1] - size / 2., size, size),
                    if key.selected { c.accent } else { c.text },
                );
                op.detail = json!({"keyline":key.selected.then_some(c.panel),"keylineWidth":1.});
                op.clip = Some(bounds);
                ops.push(op);
                for (i, handle) in key.handles.iter().enumerate() {
                    let h = point(*handle);
                    let mut line = Op::new(
                        format!("graph-tangent:{}:{i}", key.id),
                        "line",
                        bounds,
                        c.secondary,
                    );
                    line.stroke = 1.;
                    line.detail = json!({"from":p,"to":h});
                    line.clip = Some(bounds);
                    ops.push(line);
                    let mut dot = Op::new(
                        format!("graph-handle:{}:{i}", key.id),
                        "circle",
                        Rect::new(h[0] - 3., h[1] - 3., 6., 6.),
                        if key.active_handle == Some(i) {
                            c.accent
                        } else {
                            c.text
                        },
                    );
                    dot.clip = Some(bounds);
                    ops.push(dot);
                }
            }
        }
        if let Some(rect) = self.selection {
            if ![rect.x, rect.y, rect.width, rect.height]
                .iter()
                .all(|v| v.is_finite())
                || rect.width < 0.
                || rect.height < 0.
            {
                return Err(Error::InvalidGeometry);
            }
            let mut op = Op::new("graph-selection", "ring", rect, c.secondary);
            op.stroke = 1.;
            op.clip = Some(bounds);
            ops.push(op);
            for (i, (x, y)) in [
                (rect.x, rect.y),
                (rect.x + rect.width, rect.y),
                (rect.x, rect.y + rect.height),
                (rect.x + rect.width, rect.y + rect.height),
            ]
            .into_iter()
            .enumerate()
            {
                let mut op = Op::new(
                    format!("graph-transform-handle:{i}"),
                    "square",
                    Rect::new(x - 3., y - 3., 6., 6.),
                    c.text,
                );
                op.clip = Some(bounds);
                ops.push(op);
            }
        }
        Ok(ops)
    }
}
