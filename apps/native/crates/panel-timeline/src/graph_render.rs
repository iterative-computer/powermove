//! Graph paint plan. Evaluated curve segments and the value domain come from
//! the native evaluator; the renderer never fits, samples or changes a curve.
use crate::render::{Error, Op, Rect, Viewport};
use pm_design::Tokens;
use pm_kernel::commands::keyframes::EaseSide;
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
    /// Direction handles of a selected key in the Value Graph.
    pub handles: Vec<(EaseSide, Point)>,
    pub active_handle: Option<EaseSide>,
}
#[derive(Clone, Debug)]
pub struct Graph {
    curves: Vec<Curve>,
    pub min: f64,
    pub max: f64,
    pub grid: Vec<(f64, String)>,
    pub selection: Option<Rect>,
}
/// What a graph with no curve in it says.
pub const EMPTY: &str = "Select an animated property to see its curve.";
/// The note in the middle of a graph `region` that has no curve.
pub fn empty_note(region: Rect, t: &Tokens) -> Op {
    let mut note = Op::new("graph-empty", "text", region, t.colors.muted);
    note.text = Some(EMPTY.into());
    note.font_size = t.typography.body as f64;
    note.detail = json!({"align":"center"});
    note
}
fn valid(p: Point) -> bool {
    p.seconds.is_finite() && p.value.is_finite()
}
/// Build evaluated curves for the channels in view. Sampling is the evaluator's:
/// `value` answers one composition time, so holds, eases and expressions plot exactly.
/// The Speed Graph plots the evaluated rate of change, one-sided at keys so a
/// speed jump at a key shows as one.
pub fn sample(
    model: &crate::TimelineModel,
    mut value: impl FnMut(&str, &str, f64) -> Option<f64>,
) -> Option<Graph> {
    const SAMPLES: usize = 24;
    let index = &model.index;
    let (layer_id, path) = model.graph_property()?;
    let layer = index.layer(&layer_id)?;
    let speed = model.graph_mode == crate::GraphMode::Speed;
    let mut curves = vec![];
    let mut range = (f64::INFINITY, f64::NEG_INFINITY);
    for channel in model.track_channels(&layer_id, &path) {
        if channel.keys.is_empty() {
            continue;
        }
        let at = |key: &crate::Key| layer.from + key.time;
        // `side` picks the difference: -1 before `seconds`, 1 after, 0 across.
        let mut plot = |seconds: f64, side: i8, reach: f64| -> Option<f64> {
            if !speed {
                return value(&layer_id, &channel.path, seconds);
            }
            let h = reach.clamp(1e-6, 1e-3);
            let (a, b) = match side {
                -1 => (seconds - h, seconds),
                1 => (seconds, seconds + h),
                _ => (seconds - h / 2., seconds + h / 2.),
            };
            let rate = (value(&layer_id, &channel.path, b)? - value(&layer_id, &channel.path, a)?)
                / (b - a);
            rate.is_finite().then_some(rate)
        };
        let mut segments = vec![];
        for pair in channel.keys.windows(2) {
            let (from, to) = (at(&pair[0]), at(&pair[1]));
            let reach = (to - from) / SAMPLES as f64 / 4.;
            let mut previous = None;
            for step in 0..=SAMPLES {
                let seconds = from + (to - from) * step as f64 / SAMPLES as f64;
                let side = match step {
                    0 => 1,
                    SAMPLES => -1,
                    _ => 0,
                };
                let point = Point {
                    seconds,
                    value: plot(seconds, side, reach)?,
                };
                range = (range.0.min(point.value), range.1.max(point.value));
                if let Some(previous) = previous {
                    segments.push(Segment {
                        from: previous,
                        to: point,
                        controls: None,
                    });
                }
                previous = Some(point);
            }
        }
        let stored: Vec<_> = channel.keys.iter().map(|k| k.stored.to_value()).collect();
        let handles = if speed {
            vec![]
        } else {
            pm_kernel::commands::keyframes::handle_points(&stored)
        };
        let mut points = vec![];
        for (i, key) in channel.keys.iter().enumerate() {
            let seconds = at(key);
            let last = i + 1 == channel.keys.len();
            let point = Point {
                seconds,
                value: plot(seconds, if last { -1 } else { 1 }, 1e-3)?,
            };
            range = (range.0.min(point.value), range.1.max(point.value));
            let selected = model.selection.key_selected(&layer_id, &key.id);
            let handles: Vec<_> = handles
                .get(i)
                .filter(|_| selected)
                .into_iter()
                .flat_map(|[before, after]| [(EaseSide::In, *before), (EaseSide::Out, *after)])
                .filter_map(|(side, p)| {
                    let (t, v) = p?;
                    Some((
                        side,
                        Point {
                            seconds: layer.from + t,
                            value: v,
                        },
                    ))
                })
                .collect();
            for (_, p) in &handles {
                range = (range.0.min(p.value), range.1.max(p.value));
            }
            points.push(GraphPoint {
                id: key.id.clone(),
                point,
                selected,
                handles,
                active_handle: None,
            });
        }
        curves.push(Curve {
            id: format!("{layer_id}:{}", channel.path),
            name: crate::property_label(&channel.path),
            focused: true,
            segments,
            points,
        });
    }
    if curves.is_empty() {
        return None;
    }
    // Merge Selected Curves: two axes of one motion read as one curve, the
    // length of the vector they make together (their combined speed in the
    // Speed Graph). It is a view of the same keys, so it shows no key
    // points or handles of its own.
    if model.graph_merge && curves.len() > 1 {
        let length = |points: &[Point]| Point {
            seconds: points[0].seconds,
            value: points.iter().map(|p| p.value * p.value).sum::<f64>().sqrt(),
        };
        // Every curve was sampled between the same key times only when its
        // keys agree; sample each span of the union of key times instead.
        let mut times: Vec<f64> = curves
            .iter()
            .flat_map(|c| c.points.iter().map(|p| p.point.seconds))
            .collect();
        times.sort_by(f64::total_cmp);
        times.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        let channels: Vec<String> = model
            .track_channels(&layer_id, &path)
            .into_iter()
            .filter(|c| !c.keys.is_empty())
            .map(|c| c.path.clone())
            .collect();
        let mut at = |seconds: f64, side: i8, reach: f64| -> Option<Point> {
            let mut points = vec![];
            for channel in &channels {
                let value = if speed {
                    let h = reach.clamp(1e-6, 1e-3);
                    let (a, b) = match side {
                        -1 => (seconds - h, seconds),
                        1 => (seconds, seconds + h),
                        _ => (seconds - h / 2., seconds + h / 2.),
                    };
                    (value(&layer_id, channel, b)? - value(&layer_id, channel, a)?) / (b - a)
                } else {
                    value(&layer_id, channel, seconds)?
                };
                points.push(Point { seconds, value });
            }
            Some(length(&points)).filter(|p| p.value.is_finite())
        };
        let mut segments = vec![];
        range = (f64::INFINITY, f64::NEG_INFINITY);
        for pair in times.windows(2) {
            let (from, to) = (pair[0], pair[1]);
            let reach = (to - from) / SAMPLES as f64 / 4.;
            let mut previous = None;
            for step in 0..=SAMPLES {
                let seconds = from + (to - from) * step as f64 / SAMPLES as f64;
                let side = match step {
                    0 => 1,
                    SAMPLES => -1,
                    _ => 0,
                };
                let point = at(seconds, side, reach)?;
                range = (range.0.min(point.value), range.1.max(point.value));
                if let Some(previous) = previous {
                    segments.push(Segment {
                        from: previous,
                        to: point,
                        controls: None,
                    });
                }
                previous = Some(point);
            }
        }
        if !segments.is_empty() {
            curves = vec![Curve {
                id: format!("{layer_id}:{path}"),
                name: crate::property_label(&path),
                focused: true,
                segments,
                points: vec![],
            }];
        }
    }
    let margin = ((range.1 - range.0) * 0.1).max(1.);
    Graph::new(curves, range.0 - margin, range.1 + margin).ok()
}
impl Graph {
    pub fn curves(&self) -> &[Curve] {
        &self.curves
    }
    /// The layer of the curve showing key `key` (key ids may repeat across
    /// layers).
    pub fn key_layer(&self, key: &str) -> Option<&str> {
        self.curves
            .iter()
            .find(|c| c.points.iter().any(|p| p.id == key))
            .and_then(|c| c.id.rsplit_once(':'))
            .map(|(layer, _)| layer)
    }
    /// The value at `y` in a graph drawn into `area`.
    pub fn value_at(&self, area: Rect, y: f64) -> f64 {
        self.max - (y - area.y) / area.height.max(1.) * (self.max - self.min)
    }
    /// Value change for a vertical pointer travel across `height` points.
    pub fn value_per_point(&self, height: f64) -> f64 {
        (self.max - self.min) / height.max(1.)
    }
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
                    .any(|p| !valid(p.point) || p.handles.iter().any(|(_, p)| !valid(*p)))
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
            grid: grid(min, max),
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
            // Hairlines in the panel's line ink; the value just above, on the inset.
            let (ink, alpha) = t.line();
            let mut line = Op::new(
                format!("graph-grid:{label}"),
                "rect",
                Rect::new(bounds.x, y, bounds.width, t.metrics.border as f64),
                ink,
            );
            line.opacity = alpha as f64;
            let text = (t.typography.minimum * t.typography.dense_leading) as f64;
            // The plot plus room above it for the top line's label.
            let labelled = Rect::new(
                bounds.x,
                bounds.y - text,
                bounds.width,
                bounds.height + text,
            );
            line.clip = Some(labelled);
            ops.push(line);
            let mut label_op = Op::new(
                format!("graph-value:{label}"),
                "text",
                Rect::new(
                    bounds.x + t.metrics.gap as f64,
                    y - text,
                    bounds.width,
                    text,
                ),
                c.muted,
            );
            label_op.detail = json!({"tabular":true});
            label_op.text = Some(label.clone());
            label_op.font_size = t.typography.minimum as f64;
            label_op.clip = Some(labelled);
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
                // The name sits just above its curve's end, inside the plot.
                let line = (t.typography.minimum * t.typography.dense_leading) as f64;
                let y = (p[1] - line - t.metrics.border as f64)
                    .clamp(bounds.y, (bounds.y + bounds.height - line).max(bounds.y));
                let mut name = Op::new(
                    format!("curve-name:{}", curve.id),
                    "text",
                    Rect::new(p[0] + t.metrics.gap as f64, y, bounds.width, line),
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
                for (side, handle) in &key.handles {
                    let h = point(*handle);
                    let side_name = side_name(*side);
                    let mut line = Op::new(
                        format!("graph-tangent:{}:{side_name}", key.id),
                        "line",
                        bounds,
                        c.secondary,
                    );
                    line.stroke = 1.;
                    line.detail = json!({"from":p,"to":h});
                    line.clip = Some(bounds);
                    ops.push(line);
                    let mut dot = Op::new(
                        format!("graph-handle:{}:{side_name}", key.id),
                        "circle",
                        Rect::new(h[0] - 3., h[1] - 3., 6., 6.),
                        if key.active_handle == Some(*side) {
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
/// Value lines at a round step giving four or five lines across the range,
/// labelled with only the decimals the step needs.
fn grid(min: f64, max: f64) -> Vec<(f64, String)> {
    let raw = (max - min) / 4.;
    if !(raw.is_finite() && raw > 0.) {
        return vec![];
    }
    let magnitude = 10f64.powf(raw.log10().floor());
    let step = [1., 2., 2.5, 5., 10.]
        .into_iter()
        .map(|f| f * magnitude)
        .find(|s| *s >= raw)
        .unwrap_or(10. * magnitude);
    let decimals = (-(step.log10().floor())
        + if (step / magnitude).fract() != 0. {
            1.
        } else {
            0.
        })
    .max(0.) as usize;
    let mut value = (min / step).ceil() * step;
    let mut lines = vec![];
    while value <= max + step * 1e-9 && lines.len() < 12 {
        let shown = if value.abs() < step * 1e-9 { 0. } else { value };
        lines.push((value, format!("{shown:.decimals$}")));
        value += step;
    }
    lines
}
/// The handle side as it appears in op ids: `graph-handle:<key>:<in|out>`.
pub fn side_name(side: EaseSide) -> &'static str {
    match side {
        EaseSide::In => "in",
        _ => "out",
    }
}
#[cfg(test)]
mod grid_tests {
    #[test]
    fn value_lines_land_on_round_steps_with_only_the_decimals_they_need() {
        let lines = super::grid(0., 100.);
        assert_eq!(
            lines.iter().map(|(_, l)| l.as_str()).collect::<Vec<_>>(),
            ["0", "25", "50", "75", "100"]
        );
        let small = super::grid(-0.3, 0.9);
        assert_eq!(
            small.iter().map(|(_, l)| l.as_str()).collect::<Vec<_>>(),
            ["0.0", "0.5"]
        );
        assert!(
            small
                .iter()
                .all(|(v, _)| (-0.3 - 1e-9..=0.9 + 1e-9).contains(v))
        );
        assert!(super::grid(1., 1.).is_empty());
    }
}
