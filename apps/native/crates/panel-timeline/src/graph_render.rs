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
    /// The channel the curve plots (`position.x`), its legend name and its
    /// place among the curves shown: they choose its ink (pm-design
    /// `graph_curve`).
    pub path: String,
    pub name: String,
    pub index: usize,
    /// How many layers' curves of this channel are shown as this one (the
    /// same property on several layers draws once, v1).
    pub layers: usize,
    pub segments: Vec<Segment>,
    pub points: Vec<GraphPoint>,
}
/// A key's mark in the graph, as v1 draws it: a square for a hold, an
/// hourglass for an eased key, a diamond for a linear one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyShape {
    Linear,
    Bezier,
    Hold,
}
impl KeyShape {
    fn name(self) -> &'static str {
        match self {
            KeyShape::Linear => "linear",
            KeyShape::Bezier => "bezier",
            KeyShape::Hold => "hold",
        }
    }
}
#[derive(Clone, Debug)]
pub struct GraphPoint {
    pub id: String,
    pub point: Point,
    pub selected: bool,
    pub shape: KeyShape,
    /// Direction handles shown in the Value Graph: a selected key's, and an
    /// eased side of any key (v1).
    pub handles: Vec<(EaseSide, Point)>,
    pub active_handle: Option<EaseSide>,
}
#[derive(Clone, Debug)]
pub struct Graph {
    curves: Vec<Curve>,
    pub min: f64,
    pub max: f64,
    pub selection: Option<Rect>,
    /// The channels plotted, as `(layer, channel)`, and whether the key
    /// selection chose them (`TimelineModel::graph_series`).
    pub series: Vec<(String, String)>,
    pub picked: bool,
    /// The values the graph plots in Speed mode are rates.
    pub speed: bool,
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
/// A value as the graph prints it: at most two decimals, none it does not
/// need (v1's `graphValueText`).
pub fn value_text(value: f64) -> String {
    let rounded = (value * 100.).round() / 100.;
    let text = format!("{rounded:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" { "0".into() } else { text.into() }
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
    let speed = model.graph_mode == crate::GraphMode::Speed;
    let (series, picked) = model.graph_series();
    if series.is_empty() {
        return None;
    }
    // The same channel on several layers draws once, as the target layer's
    // (else the first), and the legend counts the layers (v1).
    let target = model.graph_property().map(|(layer, _)| layer);
    let mut chosen: Vec<(String, String, usize)> = vec![];
    for (layer, path) in &series {
        match chosen.iter_mut().find(|c| c.1 == *path) {
            Some(c) => {
                c.2 += 1;
                if target.as_deref() == Some(layer.as_str()) {
                    c.0 = layer.clone();
                }
            }
            None => chosen.push((layer.clone(), path.clone(), 1)),
        }
    }
    // `side` picks the difference: -1 before `seconds`, 1 after, 0 across.
    let mut plot = |layer: &str, path: &str, seconds: f64, side: i8, reach: f64| -> Option<f64> {
        if !speed {
            return value(layer, path, seconds);
        }
        let h = reach.clamp(1e-6, 1e-3);
        let (a, b) = match side {
            -1 => (seconds - h, seconds),
            1 => (seconds, seconds + h),
            _ => (seconds - h / 2., seconds + h / 2.),
        };
        let rate = (value(layer, path, b)? - value(layer, path, a)?) / (b - a);
        rate.is_finite().then_some(rate)
    };
    let mut curves = vec![];
    let mut range = (f64::INFINITY, f64::NEG_INFINITY);
    for (number, (layer_id, path, layers)) in chosen.iter().enumerate() {
        let (Some(layer), Some(channel)) = (index.layer(layer_id), index.channel(layer_id, path))
        else {
            continue;
        };
        if channel.keys.is_empty() {
            continue;
        }
        let at = |key: &crate::Key| layer.from + key.time;
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
                let Some(value) = plot(layer_id, path, seconds, side, reach) else {
                    previous = None;
                    continue;
                };
                let point = Point { seconds, value };
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
        let interp = |key: &pm_model::Value, field: &str| {
            key.get(field).and_then(pm_model::Value::as_str).map(str::to_owned)
        };
        let mut points = vec![];
        for (i, key) in channel.keys.iter().enumerate() {
            let seconds = at(key);
            let last = i + 1 == channel.keys.len();
            let Some(value) = plot(layer_id, path, seconds, if last { -1 } else { 1 }, 1e-3) else {
                continue;
            };
            let point = Point { seconds, value };
            range = (range.0.min(point.value), range.1.max(point.value));
            let selected = model.selection.key_selected(layer_id, &key.id);
            let (inp, out) = (interp(&stored[i], "inInterp"), interp(&stored[i], "outInterp"));
            let hold = stored[i].get("hold").and_then(pm_model::Value::as_bool) == Some(true)
                || out.as_deref() == Some("hold");
            let shape = if hold {
                KeyShape::Hold
            } else if inp.as_deref() == Some("bezier") || out.as_deref() == Some("bezier") {
                KeyShape::Bezier
            } else {
                KeyShape::Linear
            };
            let eased = |side: EaseSide| match side {
                EaseSide::In => inp.as_deref() == Some("bezier"),
                _ => out.as_deref() == Some("bezier"),
            };
            let handles: Vec<_> = handles
                .get(i)
                .into_iter()
                .flat_map(|[before, after]| [(EaseSide::In, *before), (EaseSide::Out, *after)])
                .filter(|(side, _)| selected || eased(*side))
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
            points.push(GraphPoint {
                id: key.id.clone(),
                point,
                selected,
                shape,
                handles,
                active_handle: None,
            });
        }
        curves.push(Curve {
            id: format!("{layer_id}:{path}"),
            path: path.clone(),
            name: crate::property_label(path),
            index: number,
            layers: *layers,
            segments,
            points,
        });
    }
    if curves.is_empty() {
        return None;
    }
    // Merge Selected Curves: the curves shown fold into the one motion they
    // describe together, the length of the vector they make (their combined
    // speed in the Speed Graph). It is a view of the same keys, so it shows
    // no key points or handles of its own.
    if model.graph_merge && curves.len() > 1 {
        let mut times: Vec<f64> = curves
            .iter()
            .flat_map(|c| c.points.iter().map(|p| p.point.seconds))
            .collect();
        times.sort_by(f64::total_cmp);
        times.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        let mut at = |seconds: f64, side: i8, reach: f64| -> Option<Point> {
            let mut total = 0.;
            for (layer, path, _) in &chosen {
                total += plot(layer, path, seconds, side, reach)?.powi(2);
            }
            Some(Point {
                seconds,
                value: total.sqrt(),
            })
            .filter(|p| p.value.is_finite())
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
            let count = curves.len();
            curves = vec![Curve {
                id: "merged".into(),
                path: String::new(),
                name: format!("Merged · {count} curves"),
                index: 0,
                layers: 1,
                segments,
                points: vec![],
            }];
        }
    }
    // v1's value range: the curves' extent, a span of at least one, and
    // 22% of it either side.
    let (mut min, mut max) = range;
    if !min.is_finite() {
        (min, max) = (0., 1.);
    }
    if max - min < 1e-6 {
        max = min + 1.;
    }
    let pad = (max - min) * 0.22;
    let mut graph = Graph::new(curves, min - pad, max + pad).ok()?;
    graph.series = series;
    graph.picked = picked;
    graph.speed = speed;
    Some(graph)
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
            selection: None,
            series: vec![],
            picked: false,
            speed: false,
        })
    }
    /// The graph's paint plan: value lines, curves, their handles and keys
    /// in `bounds` (the plot), and the legend in `legend` (v1's strip above
    /// the plot: a swatch and the channel's name per curve).
    pub fn prepare(
        &self,
        v: Viewport,
        bounds: Rect,
        legend: Rect,
        t: &Tokens,
    ) -> Result<Vec<Op>, Error> {
        if !v.valid()
            || ![bounds.x, bounds.y, bounds.width, bounds.height]
                .iter()
                .all(|x| x.is_finite())
            || bounds.width <= 0.
            || bounds.height <= 0.
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
        // Everything a curve draws stays inside the plot and the strip above
        // it, never over the legend.
        let text = (t.typography.minimum * t.typography.dense_leading) as f64;
        let below = legend.y + legend.height;
        let plot = Rect::new(
            bounds.x,
            below,
            bounds.width,
            (bounds.y + bounds.height + t.metrics.inset as f64 - below).max(0.),
        );
        for value in ticks(self.min, self.max, bounds.height) {
            let y = point(Point { seconds: 0., value })[1];
            // Hairlines in the panel's line ink, the zero line a step
            // stronger; the value just above each, on the inset.
            let (ink, alpha) = t.line();
            let mut line = Op::new(
                format!("graph-grid:{value}"),
                "rect",
                Rect::new(bounds.x, y, bounds.width, t.metrics.border as f64),
                if value == 0. { c.muted } else { ink },
            );
            line.opacity = if value == 0. { 0.5 } else { alpha as f64 };
            line.clip = Some(plot);
            ops.push(line);
            let mut label = Op::new(
                format!("graph-value:{value}"),
                "text",
                Rect::new(bounds.x + t.metrics.gap as f64, y - text, bounds.width, text),
                c.muted,
            );
            label.detail = json!({"tabular":true});
            label.text = Some(tick_text(value));
            label.font_size = t.typography.minimum as f64;
            label.clip = Some(plot);
            ops.push(label);
        }
        let from = v.scroll_time;
        let to = from + (v.width - v.layer_column) / v.pps;
        let right = v.width;
        for curve in &self.curves {
            let ink = if curve.path.is_empty() {
                c.accent
            } else {
                t.graph_curve(&curve.path, curve.index)
            };
            let first = curve.segments.partition_point(|s| s.to.seconds < from);
            let end = curve.segments.partition_point(|s| s.from.seconds <= to);
            let mut path = Op::new(format!("curve:{}", curve.id), "curve", bounds, ink);
            path.stroke = 1.5;
            path.clip = Some(plot);
            path.detail = json!({"segments":curve.segments[first..end].iter().map(|s|json!({"from":point(s.from),"to":point(s.to),"controls":s.controls.map(|points|points.map(point))})).collect::<Vec<_>>()});
            ops.push(path);
            // Past its last key a curve holds that value: it is printed at
            // the right edge, over the curve (v1).
            if let Some(last) = curve.points.last().map(|k| k.point).or(curve.segments.last().map(|s| s.to))
                && point(last)[0] < right - 55.
            {
                let y = point(last)[1];
                if y >= bounds.y && y <= bounds.y + bounds.height {
                    let mut o = Op::new(
                        format!("curve-end:{}", curve.id),
                        "text",
                        Rect::new(bounds.x, y - text - t.metrics.border as f64, right - 16. - bounds.x, text),
                        ink,
                    );
                    o.text = Some(value_text(last.value));
                    o.font_size = t.typography.minimum as f64;
                    o.detail = json!({"align":"right","tabular":true});
                    o.clip = Some(plot);
                    ops.push(o);
                }
            }
            let start = curve.points.partition_point(|key| key.point.seconds < from);
            let end = curve.points.partition_point(|key| key.point.seconds <= to);
            for key in &curve.points[start..end] {
                let p = point(key.point);
                for (side, handle) in &key.handles {
                    let h = point(*handle);
                    let side_name = side_name(*side);
                    let handle_ink = if key.selected {
                        t.graph_handle_selected()
                    } else {
                        ink
                    };
                    let mut line = Op::new(
                        format!("graph-tangent:{}:{side_name}", key.id),
                        "line",
                        bounds,
                        handle_ink,
                    );
                    line.stroke = 1.;
                    line.detail = json!({"from":p,"to":h,"dash":2.});
                    line.clip = Some(plot);
                    ops.push(line);
                    // A handle dot: filled when its key is selected, hollow
                    // (the canvas inside, the curve's ink round it) else.
                    let mut dot = Op::new(
                        format!("graph-handle:{}:{side_name}", key.id),
                        "circle",
                        Rect::new(h[0] - 3., h[1] - 3., 6., 6.),
                        if key.active_handle == Some(*side) {
                            c.accent
                        } else if key.selected {
                            handle_ink
                        } else {
                            c.panel
                        },
                    );
                    dot.detail = json!({"keyline":handle_ink,"keylineWidth":1.});
                    dot.clip = Some(plot);
                    ops.push(dot);
                }
                let size = GRAPH_VERTEX;
                let mut op = Op::new(
                    format!("graph-key:{}", key.id),
                    "graph-key",
                    Rect::new(p[0] - size / 2., p[1] - size / 2., size, size),
                    ink,
                );
                op.detail = json!({"shape":key.shape.name(),"selected":key.selected,"ring":c.accent});
                op.clip = Some(plot);
                ops.push(op);
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
            op.clip = Some(plot);
            ops.push(op);
        }
        // The legend: the canvas under it, a swatch and the channel's name
        // per curve, in the curves' order; the surface lays the names out.
        let mut ground = Op::new("graph-legend-ground", "rect", legend, c.panel);
        ground.opacity = 1.;
        ops.push(ground);
        let rate = if self.speed { "/s" } else { "" };
        let items: Vec<_> = self
            .curves
            .iter()
            .map(|curve| {
                let ink = if curve.path.is_empty() {
                    c.accent
                } else {
                    t.graph_curve(&curve.path, curve.index)
                };
                let mut name = curve.name.clone();
                if !rate.is_empty() {
                    name.push_str(" /s");
                }
                if curve.layers > 1 {
                    name.push_str(&format!(" · {} layers", curve.layers));
                }
                json!({"color":ink,"text":name})
            })
            .collect();
        let mut o = Op::new("graph-legend", "legend", legend, c.secondary);
        o.font_size = t.typography.minimum as f64;
        o.detail = json!({"items":items,"inset":t.metrics.inset});
        ops.push(o);
        Ok(ops)
    }
}
/// A key mark's size in the graph (v1's `GRAPH_VERTEX`).
pub const GRAPH_VERTEX: f64 = 8.;
/// v1's value ticks: about one every 44 points of plot, on a 1, 2 or 5 step.
fn ticks(min: f64, max: f64, height: f64) -> Vec<f64> {
    if !(min.is_finite() && max.is_finite() && max > min) {
        return vec![];
    }
    let rough = (max - min) / (height / 44.).floor().max(2.);
    let magnitude = 10f64.powf(rough.log10().floor());
    let step = [1., 2., 5., 10.]
        .into_iter()
        .find(|n| n * magnitude >= rough)
        .unwrap_or(10.)
        * magnitude;
    let mut ticks = vec![];
    let mut i = (min / step).ceil();
    while i <= (max / step).floor() && ticks.len() < 100 {
        let value = format!("{:.12}", i * step).parse::<f64>().unwrap_or(i * step);
        ticks.push(if value == 0. { 0. } else { value });
        i += 1.;
    }
    ticks
}
/// A tick's label: one decimal at most (v1).
fn tick_text(value: f64) -> String {
    let rounded = (value * 10.).round() / 10.;
    let text = format!("{rounded:.1}");
    let text = text.strip_suffix(".0").unwrap_or(&text);
    if text == "-0" { "0".into() } else { text.into() }
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
    fn value_ticks_follow_v1_one_per_44_points_on_round_steps() {
        assert_eq!(super::ticks(0., 100., 220.), [0., 20., 40., 60., 80., 100.]);
        assert_eq!(super::ticks(-0.3, 0.9, 88.), [0.]);
        assert!(super::ticks(1., 1., 100.).is_empty());
        assert_eq!(super::tick_text(0.25), "0.3");
        assert_eq!(super::tick_text(50.), "50");
        assert_eq!(super::value_text(79.6149), "79.61");
        assert_eq!(super::value_text(-0.001), "0");
    }
}
