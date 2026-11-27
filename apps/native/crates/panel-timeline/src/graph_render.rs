//! Graph paint plan, as v1's Graph Editor draws it (main's
//! `extensions/timeline/timeline.ts` `drawGraph`, `drawGraphTime` and
//! `graph-presentation.ts`). Evaluated curve segments and the value domain
//! come from the native evaluator; the renderer never fits, samples or
//! changes a curve.
use crate::render::{Error, Op, Rect, SceneSelection, Viewport};
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
    /// The axis colour (`graphAxisColor`): X, Y and Z keep theirs whatever
    /// is selected.
    pub color: u32,
    pub focused: bool,
    pub segments: Vec<Segment>,
    pub points: Vec<GraphPoint>,
}
#[derive(Clone, Debug)]
pub struct GraphPoint {
    pub id: String,
    pub point: Point,
    pub selected: bool,
    /// Direction handles shown in the Value Graph: a selected key's, and
    /// every Bézier side's (v1).
    pub handles: Vec<(EaseSide, Point)>,
    pub active_handle: Option<EaseSide>,
    pub shape: KeyShape,
}
/// A key's mark in the graph (`graphKeyShape`): a square holds, an
/// hourglass eases (Bézier), a diamond is linear.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyShape {
    #[default]
    Linear,
    Bezier,
    Hold,
}
impl KeyShape {
    /// The shape of stored key `key`.
    pub fn of(key: &pm_model::Value) -> Self {
        let text = |k: &str| key.get(k).and_then(pm_model::Value::as_str);
        if key.get("hold").and_then(pm_model::Value::as_bool) == Some(true)
            || text("outInterp") == Some("hold")
        {
            Self::Hold
        } else if text("inInterp") == Some("bezier") || text("outInterp") == Some("bezier") {
            Self::Bezier
        } else {
            Self::Linear
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Bezier => "bezier",
            Self::Hold => "hold",
        }
    }
}
/// The strip at the graph's top naming its curves and quick graph controls.
pub const LEGEND: f64 = 28.;
/// The plot sits this far under the graph's top, clear of the legend, and
/// `PLOT_BOTTOM` over its foot: keys at the value range's ends never clip.
pub const PLOT_TOP: f64 = 42.;
pub const PLOT_BOTTOM: f64 = 16.;
/// A key mark's side, and how near a press must be to take a key, handle or grip.
pub const VERTEX: f64 = 8.;
pub const POINT_HIT: f64 = 10.;
pub const HANDLE_HIT: f64 = 12.;
pub const GRIP_HIT: f64 = 6.;
/// What a pointer hits inside the Graph Editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphHit {
    Grip(usize),
    Handle {
        layer: String,
        key: String,
        side: EaseSide,
    },
    Key {
        layer: String,
        key: String,
    },
    SelectionBox,
    LegendMode,
    LegendFit,
}
/// Live hover & active drag decoration drawn on top of the graph.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GraphDecor {
    pub hover: Option<GraphHit>,
    pub active_handle: Option<(String, EaseSide)>,
}
/// The plot inside a graph `region`: values map into it.
pub fn plot(region: Rect) -> Rect {
    Rect::new(
        region.x,
        region.y + PLOT_TOP,
        region.width,
        (region.height - PLOT_TOP - PLOT_BOTTOM).max(1.),
    )
}
/// Value lines about every 44 points (`graphValueTicks`).
pub fn value_ticks(min: f64, max: f64, height: f64) -> Vec<f64> {
    if !min.is_finite() || !max.is_finite() || max <= min {
        return vec![];
    }
    let rough = (max - min) / ((height / 44.).floor().max(2.));
    let magnitude = 10f64.powf(rough.log10().floor());
    let step = [1., 2., 5., 10.]
        .into_iter()
        .find(|n| n * magnitude >= rough)
        .unwrap_or(10.)
        * magnitude;
    let mut ticks = vec![];
    let mut i = (min / step).ceil();
    while i <= (max / step).floor() && ticks.len() < 100 {
        let v = i * step;
        ticks.push(format!("{v:.12e}").parse::<f64>().unwrap_or(v));
        i += 1.;
    }
    ticks
}
/// A value as v1 prints it (`api.util.round`): at most `places` decimals,
/// no trailing zeros.
pub fn value_text(value: f64, places: i32) -> String {
    let f = 10f64.powi(places);
    let v = (value * f).round() / f;
    let v = if v == 0. { 0. } else { v };
    let text = format!("{v:.*}", places.max(0) as usize);
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}
/// The ruler's time step (`niceStep`): the first that spaces lines more
/// than 62 points apart.
pub fn time_step(pps: f64) -> f64 {
    [
        1. / 30.,
        0.1,
        0.2,
        0.5,
        1.,
        2.,
        5.,
        10.,
        15.,
        30.,
        60.,
        120.,
        300.,
    ]
    .into_iter()
    .find(|s| s * pps > 62.)
    .unwrap_or(600.)
}
#[derive(Clone, Debug)]
pub struct Graph {
    curves: Vec<Curve>,
    pub min: f64,
    pub max: f64,
    pub selection: Option<Rect>,
    /// Merge Selected Curves: one curve, in the accent.
    pub merged: bool,
    /// The Speed Graph: values are rates, so the box only marks the group.
    pub speed: bool,
    /// What this graph shows (mode, merge, layer and property): a range kept
    /// steady across rebuilds is kept only for the same view.
    pub view: String,
    pub series: Vec<(String, String)>,
    pub picked: bool,
}
/// A curve's value where the playhead stands (v1 `drawGraphTime`): a dot on
/// the curve in its ink at `(x, y)` and the value written right of the head,
/// its baseline at `label` (`None`: no room for it).
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub x: f64,
    pub y: f64,
    pub color: u32,
    pub text: String,
    pub label: Option<f64>,
}
impl Curve {
    /// The plotted value at composition `seconds`: held before the first
    /// segment and after the last (and across a gap), on the segment's line
    /// or Bézier between.
    pub fn value_at(&self, seconds: f64) -> Option<f64> {
        let Some(first) = self.segments.first() else {
            return self.points.first().map(|p| p.point.value);
        };
        if seconds <= first.from.seconds {
            return Some(first.from.value);
        }
        let mut held = first.from.value;
        for s in &self.segments {
            if seconds < s.from.seconds {
                return Some(held);
            }
            if seconds <= s.to.seconds {
                let span = s.to.seconds - s.from.seconds;
                if span <= 1e-12 {
                    return Some(s.to.value);
                }
                let Some([a, b]) = s.controls else {
                    let u = (seconds - s.from.seconds) / span;
                    return Some(s.from.value + (s.to.value - s.from.value) * u);
                };
                let cubic = |p0: f64, p1: f64, p2: f64, p3: f64, u: f64| {
                    let v = 1. - u;
                    v * v * v * p0 + 3. * v * v * u * p1 + 3. * v * u * u * p2 + u * u * u * p3
                };
                // Time runs forward along a segment: bisect for its parameter.
                let (mut lo, mut hi) = (0., 1.);
                for _ in 0..48 {
                    let mid = (lo + hi) / 2.;
                    if cubic(s.from.seconds, a.seconds, b.seconds, s.to.seconds, mid) < seconds {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                let u = (lo + hi) / 2.;
                return Some(cubic(s.from.value, a.value, b.value, s.to.value, u));
            }
            held = s.to.value;
        }
        Some(held)
    }
}
/// What a graph with no curve in it says.
pub const EMPTY: &str = "Select an animated property to edit its curve";
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
    t: &Tokens,
    mut value: impl FnMut(&str, &str, f64) -> Option<f64>,
) -> Option<Graph> {
    const SAMPLES: usize = 64;
    let clean_zero = |v: f64| if v.abs() < 1e-6 { 0. } else { v };
    let index = &model.index;
    let (series, picked) = model.graph_series();
    let (layer_id, path) = model.graph_property().or_else(|| series.first().cloned())?;
    index.layer(&layer_id)?;
    let speed = model.graph_mode == crate::GraphMode::Speed;
    let mut curves = vec![];
    let mut range = (f64::INFINITY, f64::NEG_INFINITY);
    let mut chosen: Vec<(String, String)> = Vec::new();
    for pair in &series {
        if !chosen.iter().any(|(_, path)| path == &pair.1) {
            chosen.push(pair.clone());
        }
    }
    // AE-style X/Y unification:
    // - Linked Scale (`!layer.scale_unlinked`) collapses `scale.x`/`y`/`z` into
    //   one unified Value curve named "Scale".
    // - Combined Position (`!layer.position_separated`) collapses
    //   `position.x`/`y`/`z` into one unified Speed curve with draggable Speed
    //   Bezier handles until Separate Dimensions is turned on.
    enum PlotItem {
        Single {
            layer_id: String,
            channel_path: String,
            unified_scale: bool,
        },
        CombinedPositionSpeed {
            layer_id: String,
            axes: Vec<String>,
        },
    }
    let mut items: Vec<PlotItem> = Vec::new();
    let mut handled_scale: std::collections::HashSet<String> = Default::default();
    let mut handled_position: std::collections::HashSet<String> = Default::default();
    for (lid, ch) in &chosen {
        let Some(layer) = index.layer(lid) else { continue };
        if !layer.scale_unlinked && matches!(ch.as_str(), "scale.x" | "scale.y" | "scale.z") {
            let scale_axes: Vec<_> = chosen
                .iter()
                .filter(|(l, p)| {
                    l == lid && matches!(p.as_str(), "scale.x" | "scale.y" | "scale.z")
                })
                .collect();
            if scale_axes.len() >= 2 {
                if handled_scale.insert(lid.clone()) {
                    items.push(PlotItem::Single {
                        layer_id: lid.clone(),
                        channel_path: ch.clone(),
                        unified_scale: true,
                    });
                }
                continue;
            }
        }
        if !layer.position_separated
            && matches!(ch.as_str(), "position.x" | "position.y" | "position.z")
        {
            let pos_axes: Vec<String> = chosen
                .iter()
                .filter(|(l, p)| {
                    l == lid && matches!(p.as_str(), "position.x" | "position.y" | "position.z")
                })
                .map(|(_, p)| p.clone())
                .collect();
            if pos_axes.len() >= 2 {
                if handled_position.insert(lid.clone()) {
                    items.push(PlotItem::CombinedPositionSpeed {
                        layer_id: lid.clone(),
                        axes: pos_axes,
                    });
                }
                continue;
            }
        }
        items.push(PlotItem::Single {
            layer_id: lid.clone(),
            channel_path: ch.clone(),
            unified_scale: false,
        });
    }
    let mut all_speed = !items.is_empty();
    let ease_influence = |stored_key: &pm_model::Value, field: &str| -> f64 {
        stored_key
            .get(field)
            .and_then(|e| e.get("influence"))
            .and_then(pm_model::Value::as_f64)
            .filter(|v| v.is_finite())
            .unwrap_or(33.33)
            .clamp(0.1, 100.)
    };
    for item in items {
        match item {
            PlotItem::CombinedPositionSpeed { layer_id, axes } => {
                let Some(layer) = index.layer(&layer_id) else { continue };
                let Some(primary) = index.channel(&layer_id, &axes[0]) else { continue };
                if primary.keys.is_empty() {
                    continue;
                }
                let mut times: Vec<f64> = axes
                    .iter()
                    .filter_map(|ax| index.channel(&layer_id, ax))
                    .flat_map(|ch| ch.keys.iter().map(|k| layer.from + k.time))
                    .collect();
                times.sort_by(f64::total_cmp);
                times.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
                if times.is_empty() {
                    continue;
                }
                let mut combined_speed = |seconds: f64, side: i8, reach: f64| -> Option<f64> {
                    let h = reach.clamp(1e-6, 1e-3);
                    let (a, b) = match side {
                        -1 => (seconds - h, seconds),
                        1 => (seconds, seconds + h),
                        _ => (seconds - h / 2., seconds + h / 2.),
                    };
                    let mut sum_sq = 0.;
                    for ax in &axes {
                        let rate = (value(&layer_id, ax, b)? - value(&layer_id, ax, a)?) / (b - a);
                        if !rate.is_finite() {
                            return None;
                        }
                        sum_sq += rate * rate;
                    }
                    let mag = clean_zero(sum_sq.sqrt());
                    mag.is_finite().then_some(mag)
                };
                let first = times[0];
                let last = *times.last().unwrap();
                let start = layer.from.min(first);
                let end = (layer.from + layer.duration).max(last);
                let mut segments = vec![];
                if start < first - 1e-9 {
                    segments.push(Segment {
                        from: Point { seconds: start, value: 0. },
                        to: Point { seconds: first, value: 0. },
                        controls: None,
                    });
                    range = (range.0.min(0.), range.1.max(0.));
                }
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
                        let point = Point {
                            seconds,
                            value: combined_speed(seconds, side, reach)?,
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
                if end > last + 1e-9 {
                    segments.push(Segment {
                        from: Point { seconds: last, value: 0. },
                        to: Point { seconds: end, value: 0. },
                        controls: None,
                    });
                    range = (range.0.min(0.), range.1.max(0.));
                }
                let stored: Vec<_> = primary.keys.iter().map(|k| k.stored.to_value()).collect();
                let mut points = vec![];
                for (i, key) in primary.keys.iter().enumerate() {
                    let seconds = layer.from + key.time;
                    let is_last = i + 1 == primary.keys.len();
                    let in_spd = if i > 0 {
                        combined_speed(seconds, -1, 1e-3).unwrap_or(0.)
                    } else {
                        0.
                    };
                    let out_spd = if !is_last {
                        combined_speed(seconds, 1, 1e-3).unwrap_or(0.)
                    } else {
                        0.
                    };
                    let point_val = if is_last { in_spd } else { out_spd };
                    range = (range.0.min(point_val), range.1.max(point_val));
                    let selected = axes.iter().any(|ax| {
                        index.channel(&layer_id, ax).is_some_and(|ch| {
                            ch.keys.iter().any(|k| {
                                (k.time - key.time).abs() < 1e-9
                                    && model.selection.key_selected(&layer_id, &k.id)
                            })
                        })
                    });
                    let shape = KeyShape::of(&stored[i]);
                    let interp = |idx: usize, field: &str| {
                        stored[idx].get(field).and_then(pm_model::Value::as_str) == Some("bezier")
                    };
                    let holds = |idx: usize| KeyShape::of(&stored[idx]) == KeyShape::Hold;
                    let out_ok =
                        i + 1 < primary.keys.len() && !holds(i) && (selected || interp(i, "outInterp"));
                    let in_ok = i > 0 && !holds(i - 1) && (selected || interp(i, "inInterp"));
                    let mut handles = vec![];
                    if in_ok {
                        let dt = (key.time - primary.keys[i - 1].time).abs();
                        if dt > 1e-6 {
                            let inf = ease_influence(&stored[i], "inEase");
                            handles.push((
                                EaseSide::In,
                                Point {
                                    seconds: seconds - (inf / 100.) * dt,
                                    value: in_spd,
                                },
                            ));
                        }
                    }
                    if out_ok {
                        let dt = (primary.keys[i + 1].time - key.time).abs();
                        if dt > 1e-6 {
                            let inf = ease_influence(&stored[i], "outEase");
                            handles.push((
                                EaseSide::Out,
                                Point {
                                    seconds: seconds + (inf / 100.) * dt,
                                    value: out_spd,
                                },
                            ));
                        }
                    }
                    points.push(GraphPoint {
                        id: key.id.clone(),
                        point: Point {
                            seconds,
                            value: point_val,
                        },
                        selected,
                        handles,
                        active_handle: None,
                        shape,
                    });
                }
                curves.push(Curve {
                    id: format!("{layer_id}:position"),
                    name: if speed {
                        "Position".into()
                    } else {
                        "Position (Speed)".into()
                    },
                    color: t.colors.accent,
                    focused: true,
                    segments,
                    points,
                });
            }
            PlotItem::Single {
                layer_id,
                channel_path,
                unified_scale,
            } => {
                if !speed {
                    all_speed = false;
                }
                let Some(layer) = index.layer(&layer_id) else { continue; };
                let Some(channel) = index.channel(&layer_id, &channel_path) else { continue; };
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
                    let rate = clean_zero(
                        (value(&layer_id, &channel.path, b)? - value(&layer_id, &channel.path, a)?)
                            / (b - a),
                    );
                    rate.is_finite().then_some(rate)
                };
                let mut segments = vec![];
                // Before its first key and after its last the channel holds still:
                // the curve runs flat across the layer, as v1 draws it.
                let first = at(&channel.keys[0]);
                let last = at(channel.keys.last().unwrap());
                let start = layer.from.min(first);
                let end = (layer.from + layer.duration).max(last);
                let held =
                    |seconds: f64, side: i8, plot: &mut dyn FnMut(f64, i8, f64) -> Option<f64>| {
                        if speed {
                            Some(0.)
                        } else {
                            plot(seconds, side, 1e-3)
                        }
                    };
                let lead = if start < first - 1e-9 {
                    held(first, 1, &mut plot).map(|v| Segment {
                        from: Point {
                            seconds: start,
                            value: v,
                        },
                        to: Point {
                            seconds: first,
                            value: v,
                        },
                        controls: None,
                    })
                } else {
                    None
                };
                let tail = if end > last + 1e-9 {
                    held(last, -1, &mut plot).map(|v| Segment {
                        from: Point {
                            seconds: last,
                            value: v,
                        },
                        to: Point {
                            seconds: end,
                            value: v,
                        },
                        controls: None,
                    })
                } else {
                    None
                };
                let stored: Vec<_> = channel.keys.iter().map(|k| k.stored.to_value()).collect();
                let value_handles = if speed {
                    vec![]
                } else {
                    pm_kernel::commands::keyframes::handle_points(&stored)
                };
                // For non-expression, non-spring Value Graph segments, use exact
                // cubic Béziers (razor-sharp at any zoom); fall back to dense
                // sampling for expressions, springs, and Speed Graph.
                let has_expr = channel
                    .expression
                    .as_ref()
                    .is_some_and(|e| !e.to_string_lossy().trim().is_empty());
                let exact_bezier = !speed
                    && !has_expr
                    && stored.iter().all(|k| k.get("spring").is_none());
                segments.extend(lead);
                for (seg_idx, pair) in channel.keys.windows(2).enumerate() {
                    let (from, to) = (at(&pair[0]), at(&pair[1]));
                    if exact_bezier {
                        let v0 = pair[0].value.as_f64().or_else(|| plot(from, 1, 1e-3))?;
                        let v1 = pair[1].value.as_f64().or_else(|| plot(to, -1, 1e-3))?;
                        let p0 = Point { seconds: from, value: v0 };
                        let p3 = Point { seconds: to, value: v1 };
                        range = (range.0.min(v0).min(v1), range.1.max(v0).max(v1));
                        if KeyShape::of(&stored[seg_idx]) == KeyShape::Hold {
                            let corner = Point { seconds: to, value: v0 };
                            segments.push(Segment { from: p0, to: corner, controls: None });
                            segments.push(Segment { from: corner, to: p3, controls: None });
                        } else {
                            let out_is_bezier = stored[seg_idx]
                                .get("outInterp")
                                .and_then(pm_model::Value::as_str)
                                == Some("bezier")
                                || stored[seg_idx].get("eo").is_some();
                            let in_is_bezier = stored[seg_idx + 1]
                                .get("inInterp")
                                .and_then(pm_model::Value::as_str)
                                == Some("bezier")
                                || stored[seg_idx + 1].get("ei").is_some();
                            let controls = if out_is_bezier || in_is_bezier {
                                let c1 = value_handles
                                    .get(seg_idx)
                                    .and_then(|h| h[1])
                                    .map(|(t, v)| Point { seconds: layer.from + t, value: v })
                                    .unwrap_or_else(|| Point {
                                        seconds: from + (to - from) / 3.,
                                        value: v0 + (v1 - v0) / 3.,
                                    });
                                let c2 = value_handles
                                    .get(seg_idx + 1)
                                    .and_then(|h| h[0])
                                    .map(|(t, v)| Point { seconds: layer.from + t, value: v })
                                    .unwrap_or_else(|| Point {
                                        seconds: from + (to - from) * (2. / 3.),
                                        value: v0 + (v1 - v0) * (2. / 3.),
                                    });
                                // Sample curve interior for exact Y range (handles overshoot).
                                for step in 1..16 {
                                    let u = step as f64 / 16.;
                                    let r = 1. - u;
                                    let y = r * r * r * v0
                                        + 3. * r * r * u * c1.value
                                        + 3. * r * u * u * c2.value
                                        + u * u * u * v1;
                                    if y.is_finite() {
                                        range = (range.0.min(y), range.1.max(y));
                                    }
                                }
                                Some([c1, c2])
                            } else {
                                None
                            };
                            segments.push(Segment { from: p0, to: p3, controls });
                        }
                        continue;
                    }
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
                segments.extend(tail);
                let mut points = vec![];
                for (i, key) in channel.keys.iter().enumerate() {
                    let seconds = at(key);
                    let last = i + 1 == channel.keys.len();
                    let point = Point {
                        seconds,
                        value: plot(seconds, if last { -1 } else { 1 }, 1e-3)?,
                    };
                    range = (range.0.min(point.value), range.1.max(point.value));
                    let selected = if unified_scale {
                        model.track_channels(&layer_id, "scale").iter().any(|ch| {
                            ch.keys.iter().any(|k| {
                                (k.time - key.time).abs() < 1e-9
                                    && model.selection.key_selected(&layer_id, &k.id)
                            })
                        })
                    } else {
                        model.selection.key_selected(&layer_id, &key.id)
                    };
                    let shape = KeyShape::of(&stored[i]);
                    let interp = |key: usize, field: &str| {
                        stored[key].get(field).and_then(pm_model::Value::as_str) == Some("bezier")
                    };
                    let holds = |key: usize| KeyShape::of(&stored[key]) == KeyShape::Hold;
                    // v1: a side shows its handle when it has a neighbour that way,
                    // doesn't hold, and the key is selected or that side is Bézier.
                    let out = i + 1 < channel.keys.len()
                        && !holds(i)
                        && (selected || interp(i, "outInterp"));
                    let inn = i > 0 && !holds(i - 1) && (selected || interp(i, "inInterp"));
                    let handles: Vec<_> = if speed {
                        let mut h = vec![];
                        if inn {
                            let dt = (key.time - channel.keys[i - 1].time).abs();
                            if dt > 1e-6 {
                                let inf = ease_influence(&stored[i], "inEase");
                                let v = plot(seconds, -1, 1e-3).unwrap_or(point.value);
                                h.push((
                                    EaseSide::In,
                                    Point {
                                        seconds: seconds - (inf / 100.) * dt,
                                        value: v,
                                    },
                                ));
                            }
                        }
                        if out {
                            let dt = (channel.keys[i + 1].time - key.time).abs();
                            if dt > 1e-6 {
                                let inf = ease_influence(&stored[i], "outEase");
                                let v = plot(seconds, 1, 1e-3).unwrap_or(point.value);
                                h.push((
                                    EaseSide::Out,
                                    Point {
                                        seconds: seconds + (inf / 100.) * dt,
                                        value: v,
                                    },
                                ));
                            }
                        }
                        h
                    } else {
                        value_handles
                            .get(i)
                            .into_iter()
                            .flat_map(|[before, after]| {
                                [(EaseSide::In, *before), (EaseSide::Out, *after)]
                            })
                            .filter(|(side, _)| if *side == EaseSide::In { inn } else { out })
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
                            .collect()
                    };
                    // The range is the curve's (v1): a handle may reach past it, and
                    // selecting a key never rescales the graph.
                    points.push(GraphPoint {
                        id: key.id.clone(),
                        point,
                        selected,
                        handles,
                        active_handle: None,
                        shape,
                    });
                }
                curves.push(Curve {
                    id: format!("{layer_id}:{}", channel.path),
                    name: if unified_scale {
                        "Scale".into()
                    } else {
                        model.channel_label(&layer_id, &channel.path)
                    },
                    // v1's axis colours (`graphAxisColor`), in this appearance.
                    color: if unified_scale {
                        t.colors.accent
                    } else {
                        t.graph_curve(&channel.path, curves.len())
                    },
                    focused: true,
                    segments,
                    points,
                });
            }
        }
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
        let channels: Vec<(String, String)> = curves
            .iter()
            .filter_map(|c| {
                let (l, ch) = c.id.rsplit_once(':')?;
                Some((l.to_owned(), ch.to_owned()))
            })
            .collect();
        let mut at = |seconds: f64, side: i8, reach: f64| -> Option<Point> {
            let mut points = vec![];
            for (lid, channel) in &channels {
                let value = if speed {
                    let h = reach.clamp(1e-6, 1e-3);
                    let (a, b) = match side {
                        -1 => (seconds - h, seconds),
                        1 => (seconds, seconds + h),
                        _ => (seconds - h / 2., seconds + h / 2.),
                    };
                    (value(lid, channel, b)? - value(lid, channel, a)?) / (b - a)
                } else {
                    value(lid, channel, seconds)?
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
                name: model.channel_label(&layer_id, &path),
                color: 0,
                focused: true,
                segments,
                points: vec![],
            }];
        }
    }
    // v1: a flat curve spans one unit; then 22% of the span above and below.
    if range.1 - range.0 < 1e-6 {
        range.1 = range.0 + 1.;
    }
    let margin = (range.1 - range.0) * 0.22;
    let merged = model.graph_merge && curves.len() == 1 && curves[0].points.is_empty();
    let mut graph = Graph::new(curves, range.0 - margin, range.1 + margin).ok()?;
    graph.merged = merged;
    graph.speed = speed || all_speed;
    graph.series = series;
    graph.picked = picked;
    graph.view = format!(
        "{}:{}:{layer_id}:{path}",
        if speed || all_speed { "speed" } else { "value" },
        model.graph_merge
    );
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
    pub fn playhead_marks(&self, v: Viewport, bounds: Rect, time: f64, _t: &Tokens) -> Vec<Mark> {
        let x = v.x(time);
        if !time.is_finite()
            || self.merged
            || x < v.layer_column + 35.
            || x >= v.width - 40.
        {
            return vec![];
        }
        let (top, bottom) = (bounds.y, bounds.y + bounds.height);
        let mut taken: Vec<f64> = vec![];
        let near = |taken: &[f64], y: f64| taken.iter().any(|t| (t - y).abs() < 13.);
        let mut marks = vec![];
        for curve in &self.curves {
            let Some(value) = curve.value_at(time) else {
                continue;
            };
            let y = bounds.y + bounds.height - (value-self.min)/(self.max-self.min)*bounds.height;
            if !y.is_finite() || y < top || y > bottom {
                continue;
            }
            let mut label = (y - 7.).clamp(top + 10., (bottom - 3.).max(top + 10.));
            while near(&taken, label) && label < bottom - 16. {
                label += 13.;
            }
            if near(&taken, label) {
                label = y - 20.;
                while near(&taken, label) {
                    label -= 13.;
                }
            }
            let label = (label >= top + 10.).then(|| {
                taken.push(label);
                label
            });
            marks.push(Mark {
                x,
                y,
                color: curve.color,
                text: value_text(value, 2),
                label,
            });
        }
        marks
    }
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
            merged: false,
            speed: false,
            view: String::new(),
            series: Vec::new(),
            picked: false,
        })
    }
    /// This graph drawn over the value range `min`..`max` instead of its own
    /// (a range held through a drag, or panned with the wheel).
    pub fn with_range(mut self, min: f64, max: f64) -> Self {
        if min.is_finite() && max.is_finite() && max > min {
            (self.min, self.max) = (min, max);
        }
        self
    }
    /// The box around two or more selected keys (v1's
    /// `graphSelectionBounds(points, 16, 8)`), in surface points.
    pub fn selection_box(&self, v: Viewport, region: Rect) -> Option<Rect> {
        if let Some(rect) = self.selection {
            return Some(rect);
        }
        if self.merged {
            return None;
        }
        let plot = plot(region);
        let y_of = |value: f64| {
            plot.y + plot.height - (value - self.min) / (self.max - self.min) * plot.height
        };
        let points: Vec<(f64, f64)> = self
            .curves
            .iter()
            .flat_map(|c| c.points.iter())
            .filter(|p| p.selected)
            .map(|p| (v.x(p.point.seconds), y_of(p.point.value)))
            .collect();
        if points.len() < 2 {
            return None;
        }
        let pad = 8.;
        let (mut x0, mut x1, mut y0, mut y1) = points.iter().fold(
            (
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
            ),
            |(a, b, c, d), (x, y)| (a.min(*x), b.max(*x), c.min(*y), d.max(*y)),
        );
        // Co-located keys (e.g. linked Scale X/Y at a single time & value)
        // have no box span; drawing 8 grips on top of one vertex obscures it.
        if (x1 - x0) < 1. && (y1 - y0) < 1. {
            return None;
        }
        (x0, x1, y0, y1) = (x0 - pad, x1 + pad, y0 - pad, y1 + pad);
        if x1 - x0 < 16. {
            let m = (x0 + x1) / 2.;
            (x0, x1) = (m - 8., m + 8.);
        }
        if y1 - y0 < 16. {
            let m = (y0 + y1) / 2.;
            (y0, y1) = (m - 8., m + 8.);
        }
        Some(Rect::new(x0, y0, x1 - x0, y1 - y0))
    }
    /// Whether the box scales: v1 reads values off the curve, which the
    /// Speed Graph and merged curves don't plot.
    pub fn transformable(&self) -> bool {
        !self.speed && !self.merged
    }
    /// The selected keys' span: first and last composition time, least and
    /// greatest value.
    pub fn selected_span(&self) -> Option<(f64, f64, f64, f64)> {
        let mut span: Option<(f64, f64, f64, f64)> = None;
        for p in self
            .curves
            .iter()
            .flat_map(|c| c.points.iter())
            .filter(|p| p.selected)
        {
            let (t, v) = (p.point.seconds, p.point.value);
            span = Some(match span {
                None => (t, t, v, v),
                Some((a, b, c, d)) => (a.min(t), b.max(t), c.min(v), d.max(v)),
            });
        }
        span
    }
    /// The value a curve has at `seconds` (its plotted samples).
    pub fn value_of(curve: &Curve, seconds: f64) -> Option<f64> { curve.value_at(seconds) }
    /// Whether this graph, shown over `min`..`max` (the range on screen),
    /// still reads well there: every value fits, and the values fill at
    /// least a quarter of it. Keeping wider hysteresis prevents sudden Y-axis
    /// jumps right after releasing a drag.
    pub fn fits_in(&self, min: f64, max: f64) -> bool {
        let span = max - min;
        self.min >= min - 1e-9
            && self.max <= max + 1e-9
            && span > 0.
            && (self.max - self.min) >= span * 0.25
    }
    /// Bounding rects of the quick controls in the legend strip: `(mode_pill, fit_button)`.
    pub fn legend_controls(&self, region: Rect) -> (Rect, Rect) {
        let right = region.x + region.width - 8.;
        let fit_w = 34.;
        let mode_w = 52.;
        let h = 18.;
        let y = region.y + (LEGEND - h) / 2.;
        let fit = Rect::new((right - fit_w).max(region.x), y, fit_w, h);
        let mode = Rect::new((fit.x - 6. - mode_w).max(region.x), y, mode_w, h);
        (mode, fit)
    }
    /// Single authoritative hit test shared by clicks, drags, hover highlights,
    /// and cursors.
    pub fn hit_test(&self, v: Viewport, region: Rect, x: f64, y: f64) -> Option<GraphHit> {
        if x < region.x || x > region.x + region.width || y < region.y || y > region.y + region.height
        {
            return None;
        }
        if y < region.y + LEGEND {
            let (mode, fit) = self.legend_controls(region);
            let inside = |r: Rect| x >= r.x && x <= r.x + r.width && y >= r.y && y <= r.y + r.height;
            if inside(fit) {
                return Some(GraphHit::LegendFit);
            }
            if inside(mode) {
                return Some(GraphHit::LegendMode);
            }
            return None;
        }
        let boxed = self.selection_box(v, region);
        if let Some(b) = boxed
            && self.transformable()
            && let Some(grip) = grip_at(b, x, y)
        {
            return Some(GraphHit::Grip(grip));
        }
        let plot = plot(region);
        let bottom = plot.y + plot.height;
        let y_of = |value: f64| bottom - (value - self.min) / (self.max - self.min) * plot.height;
        let from = v.scroll_time;
        let to = from + (v.width - v.layer_column) / v.pps;

        let mut best_handle: Option<(bool, f64, String, String, EaseSide)> = None;
        let mut best_key: Option<(bool, f64, String, String)> = None;

        for curve in &self.curves {
            let Some((layer, _)) = curve.id.rsplit_once(':') else {
                continue;
            };
            for key in &curve.points {
                let kx = v.x(key.point.seconds);
                // Check handles even if the key itself is slightly outside view.
                for (side, handle) in &key.handles {
                    let hx = v.x(handle.seconds);
                    let hy = y_of(handle.value);
                    let d = (hx - x).hypot(hy - y);
                    if d <= HANDLE_HIT {
                        let better = match &best_handle {
                            None => true,
                            // Prefer handles on selected keys, then nearest distance.
                            Some((sel, bd, _, _, _)) => {
                                (key.selected && !*sel) || (key.selected == *sel && d < *bd)
                            }
                        };
                        if better {
                            best_handle = Some((
                                key.selected,
                                d,
                                layer.to_owned(),
                                key.id.clone(),
                                *side,
                            ));
                        }
                    }
                }
                if key.point.seconds < from - 1e-6 || key.point.seconds > to + 1e-6 {
                    continue;
                }
                // In Speed Graph, a discontinuous key has two vertical anchors
                // (`in_spd` and `out_spd`); hit-test all of them.
                let mut kd = (kx - x).hypot(y_of(key.point.value) - y);
                if self.speed {
                    for (_, h) in &key.handles {
                        kd = kd.min((kx - x).hypot(y_of(h.value) - y));
                    }
                }
                if kd <= POINT_HIT {
                    let better = match &best_key {
                        None => true,
                        Some((sel, bd, _, _)) => {
                            (key.selected && !*sel) || (key.selected == *sel && kd < *bd)
                        }
                    };
                    if better {
                        best_key = Some((key.selected, kd, layer.to_owned(), key.id.clone()));
                    }
                }
            }
        }
        // Only let a key beat a handle when the pointer is directly on top of
        // the key vertex (< 5px) and closer to the key than the handle.
        let key_wins = match (&best_key, &best_handle) {
            (Some((_, kd, _, _)), Some((_, hd, _, _, _))) => *kd < 5. && *kd < *hd,
            (Some(_), None) => true,
            _ => false,
        };
        if !key_wins
            && let Some((_, _, layer, key, side)) = best_handle
        {
            return Some(GraphHit::Handle { layer, key, side });
        }
        if let Some((_, _, layer, key)) = best_key {
            return Some(GraphHit::Key { layer, key });
        }
        if boxed
            .is_some_and(|b| x >= b.x && x <= b.x + b.width && y >= b.y && y <= b.y + b.height)
        {
            return Some(GraphHit::SelectionBox);
        }
        None
    }
    /// Keys whose drawn centres the box covers, in curve and time order.
    pub fn marquee_keys(&self, v: Viewport, region: Rect, area: Rect) -> Vec<(String, String)> {
        let plot = plot(region);
        let from = v.scroll_time + (area.x.max(v.layer_column) - v.layer_column) / v.pps;
        let to = v.scroll_time
            + ((area.x + area.width).min(v.width) - v.layer_column) / v.pps;
        let mut keys = Vec::new();
        if from > to {
            return keys;
        }
        for curve in &self.curves {
            let Some((layer, _)) = curve.id.rsplit_once(':') else { continue };
            let start = curve.points.partition_point(|key| key.point.seconds < from);
            let end = curve.points.partition_point(|key| key.point.seconds <= to);
            for key in &curve.points[start..end] {
                let y = plot.y + plot.height
                    - (key.point.value - self.min) / (self.max - self.min) * plot.height;
                if y >= area.y && y <= area.y + area.height {
                    keys.push((layer.to_owned(), key.id.clone()));
                }
            }
        }
        keys
    }

    /// The graph drawn into `region` (the whole graph, legend included),
    /// v1's way, with each curve's value marked where the playhead crosses.
    pub fn prepare(
        &self,
        v: Viewport,
        region: Rect,
        t: &Tokens,
        playhead: Option<f64>,
    ) -> Result<Vec<Op>, Error> {
        self.prepare_with_selection(v, region, t, playhead, None)
    }

    pub fn prepare_with_selection(
        &self,
        v: Viewport,
        region: Rect,
        t: &Tokens,
        playhead: Option<f64>,
        selection: Option<&SceneSelection>,
    ) -> Result<Vec<Op>, Error> {
        self.prepare_with_state(v, region, t, playhead, selection, None)
    }

    pub fn prepare_with_state(
        &self,
        v: Viewport,
        region: Rect,
        t: &Tokens,
        playhead: Option<f64>,
        selection: Option<&SceneSelection>,
        decor: Option<&GraphDecor>,
    ) -> Result<Vec<Op>, Error> {
        if !v.valid()
            || ![region.x, region.y, region.width, region.height]
                .iter()
                .all(|x| x.is_finite())
            || region.width <= 0.
            || region.height <= 0.
        {
            return Err(Error::InvalidGeometry);
        }
        let c = &t.colors;
        let plot = plot(region);
        let (top, bottom) = (plot.y, plot.y + plot.height);
        let y_of = |value: f64| bottom - (value - self.min) / (self.max - self.min) * plot.height;
        let point = |p: Point| [v.x(p.seconds), y_of(p.value)];
        let right = region.x + region.width;
        // Curves and keys stay under the legend strip, inside the graph.
        let body = Rect::new(
            region.x,
            region.y + LEGEND,
            region.width,
            (region.height - LEGEND).max(0.),
        );
        let line_op = |id: String, rect: Rect, alpha: f64| {
            let mut op = Op::new(id, "rect", rect, c.text);
            op.opacity = alpha;
            op.clip = Some(region);
            op
        };
        let small = t.typography.minimum as f64;
        let text_height = small * t.typography.dense_leading as f64;
        let mut ops = vec![];
        // The graph covers what is under it: the tracks' own ground.
        let mut ground = Op::new("graph-ground", "rect", region, c.panel);
        ground.clip = Some(region);
        ops.push(ground);
        // Time lines on the ruler's steps.
        let step = time_step(v.pps);
        let mut time = (v.scroll_time / step).ceil() * step;
        while v.x(time) < right && ops.len() < 400 {
            let x = v.x(time).round() + 0.5;
            if x >= region.x {
                ops.push(line_op(
                    format!("graph-time:{time}"),
                    Rect::new(x - 0.5, region.y, 1., region.height),
                    0.05,
                ));
            }
            time += step;
        }
        // Value lines about every 44 points, zero stronger, each labelled.
        for value in value_ticks(self.min, self.max, plot.height) {
            let y = y_of(value).round() + 0.5;
            ops.push(line_op(
                format!("graph-grid:{value}"),
                Rect::new(region.x, y - 0.5, region.width, 1.),
                if value == 0. { 0.32 } else { 0.05 },
            ));
            let mut label = Op::new(
                format!("graph-value:{value}"),
                "text",
                Rect::new(
                    region.x + 5.,
                    y - 4. - text_height,
                    region.width,
                    text_height,
                ),
                c.muted,
            );
            label.text = Some(value_text(value, 1));
            label.font_size = small;
            label.detail = json!({"tabular":true});
            label.clip = Some(body);
            ops.push(label);
        }
        let from = v.scroll_time;
        let to = from + (v.width - v.layer_column) / v.pps;
        for curve in &self.curves {
            let ink = if self.merged { c.accent } else { curve.color };
            let first = curve.segments.partition_point(|s| s.to.seconds < from);
            let end = curve.segments.partition_point(|s| s.from.seconds <= to);
            let mut path = Op::new(format!("curve:{}", curve.id), "curve", region, ink);
            path.stroke = if self.merged { 1.8 } else { 1.35 };
            path.clip = Some(body);
            path.detail = json!({"segments":curve.segments[first..end].iter().map(|s|json!({"from":point(s.from),"to":point(s.to),"controls":s.controls.map(|points|points.map(point))})).collect::<Vec<_>>()});
            ops.push(path);
            let start = curve.points.partition_point(|key| key.point.seconds < from);
            let stop = curve.points.partition_point(|key| key.point.seconds <= to);
            let mut marks = vec![];
            let layer = curve.id.rsplit_once(':').map(|(layer, _)| layer);
            for key in &curve.points[start..stop] {
                let selected = selection.zip(layer)
                    .map_or(key.selected, |(selection, layer)| selection.key(layer, &key.id));
                let p = point(key.point);
                let key_hovered = decor.is_some_and(|d| {
                    matches!(&d.hover, Some(GraphHit::Key { key: hk, .. }) if hk == &key.id)
                });
                // In Speed Graph, a discontinuous key has distinct incoming &
                // outgoing speed anchors: draw a vertical dashed connector when
                // they differ so each handle stems cleanly from its own speed.
                if self.speed {
                    let in_y = key
                        .handles
                        .iter()
                        .find(|(s, _)| *s == EaseSide::In)
                        .map(|(_, h)| y_of(h.value));
                    let out_y = key
                        .handles
                        .iter()
                        .find(|(s, _)| *s == EaseSide::Out)
                        .map(|(_, h)| y_of(h.value));
                    if let (Some(iy), Some(oy)) = (in_y, out_y)
                        && (iy - oy).abs() > 1.
                    {
                        let mut jump = Op::new(
                            format!("graph-speed-jump:{}", key.id),
                            "line",
                            region,
                            ink,
                        );
                        jump.stroke = 1.;
                        jump.opacity = 0.55;
                        jump.detail = json!({"from":[p[0], iy],"to":[p[0], oy],"dash":[2.,2.]});
                        jump.clip = Some(body);
                        ops.push(jump);
                    }
                }
                // Tangents & handle knobs: hovered or active handles enlarge and
                // draw solid arms for crisp feedback when interactive decor is on.
                let interactive = decor.is_some();
                let handle_ink = if selected { t.graph_handle_selected() } else { ink };
                for (side, handle) in &key.handles {
                    let h = point(*handle);
                    let anchor = if self.speed {
                        [p[0], h[1]]
                    } else {
                        p
                    };
                    let side_name = side_name(*side);
                    let active = interactive
                        && (key.active_handle == Some(*side)
                            || decor.is_some_and(|d| {
                                d.active_handle
                                    .as_ref()
                                    .is_some_and(|(ak, as_)| ak == &key.id && as_ == side)
                            }));
                    let hovered = decor.is_some_and(|d| {
                        matches!(&d.hover, Some(GraphHit::Handle { key: hk, side: hs, .. }) if hk == &key.id && hs == side)
                    });
                    let emphasized = active || hovered;
                    let mut line = Op::new(
                        format!("graph-tangent:{}:{side_name}", key.id),
                        "line",
                        region,
                        if emphasized { t.graph_handle_selected() } else { handle_ink },
                    );
                    line.stroke = if emphasized { 1.6 } else { 1. };
                    line.detail = if emphasized {
                        json!({"from":anchor,"to":h})
                    } else {
                        json!({"from":anchor,"to":h,"dash":[2.,2.]})
                    };
                    line.clip = Some(body);
                    ops.push(line);
                    let knob = if emphasized {
                        9.
                    } else if interactive {
                        7.
                    } else {
                        6.
                    };
                    let knob_ink = if emphasized { t.graph_handle_selected() } else { handle_ink };
                    let mut dot = Op::new(
                        format!("graph-handle:{}:{side_name}", key.id),
                        "circle",
                        Rect::new(h[0] - knob / 2., h[1] - knob / 2., knob, knob),
                        if selected || emphasized { knob_ink } else { c.panel },
                    );
                    dot.detail = json!({
                        "keyline": if active { c.text } else { knob_ink },
                        "keylineWidth": if emphasized { 1.5 } else if interactive { 1.2 } else { 1. },
                    });
                    dot.clip = Some(body);
                    ops.push(dot);
                }
                let side_len = if key_hovered { VERTEX + 1.5 } else { VERTEX };
                let half = side_len / 2.;
                let ring = if selected {
                    Some(t.graph_key_selected())
                } else if key_hovered {
                    Some(c.text)
                } else {
                    None
                };
                let mut mark = Op::new(
                    format!("graph-key:{}", key.id),
                    "graph-vertex",
                    Rect::new(p[0] - half, p[1] - half, side_len, side_len),
                    ink,
                );
                mark.detail = json!({"kind":key.shape.name(),"ring":ring});
                mark.clip = Some(body);
                marks.push(mark);
                // Secondary vertex marker at the split incoming speed anchor.
                if self.speed
                    && let Some((_, in_h)) = key.handles.iter().find(|(s, _)| *s == EaseSide::In)
                    && (y_of(in_h.value) - p[1]).abs() > 2.
                {
                    let iy = y_of(in_h.value);
                    let sub = 5.5;
                    let mut sub_mark = Op::new(
                        format!("graph-key-split:{}", key.id),
                        "circle",
                        Rect::new(p[0] - sub / 2., iy - sub / 2., sub, sub),
                        c.panel,
                    );
                    sub_mark.detail = json!({"keyline": if selected { t.graph_key_selected() } else { ink }, "keylineWidth": 1.3});
                    sub_mark.clip = Some(body);
                    marks.push(sub_mark);
                }
            }
            // Keys over every tangent.
            ops.extend(marks);
            // The value where the curve leaves the view, at the right edge.
            if let Some(last) = curve.points.last()
                && v.x(last.point.seconds) < right - 55.
                && let Some(value) =
                    Graph::value_of(curve, from + (right - 16. - v.layer_column) / v.pps)
            {
                let y = y_of(value);
                if (top..=bottom).contains(&y) {
                    let mut label = Op::new(
                        format!("graph-end:{}", curve.id),
                        "text",
                        Rect::new(
                            region.x,
                            y - 6. - text_height,
                            right - 16. - region.x,
                            text_height,
                        ),
                        ink,
                    );
                    label.text = Some(value_text(value, 2));
                    label.font_size = small;
                    label.detail = json!({"align":"right","tabular":true});
                    label.clip = Some(body);
                    ops.push(label);
                }
            }
        }
        // Where the playhead crosses each curve: a dot and its value.
        if let Some(time) = playhead.filter(|_| !self.merged) {
            let x = v.x(time);
            if x >= region.x + 35. && x < right - 40. {
                let mut taken: Vec<f64> = vec![];
                for curve in &self.curves {
                    let Some(value) = Graph::value_of(curve, time) else {
                        continue;
                    };
                    let y = y_of(value);
                    if !(top..=bottom).contains(&y) {
                        continue;
                    }
                    let mut dot = Op::new(
                        format!("graph-now:{}", curve.id),
                        "circle",
                        Rect::new(x - 3., y - 3., 6., 6.),
                        curve.color,
                    );
                    dot.clip = Some(body);
                    ops.push(dot);
                    let mut at = (y - 7.).clamp(top + 10., bottom - 3.);
                    while taken.iter().any(|y| (y - at).abs() < 13.) && at < bottom - 16. {
                        at += 13.;
                    }
                    if taken.iter().any(|y| (y - at).abs() < 13.) {
                        at = y - 20.;
                        while taken.iter().any(|y| (y - at).abs() < 13.) {
                            at -= 13.;
                        }
                    }
                    if at >= top + 10. {
                        taken.push(at);
                        let mut label = Op::new(
                            format!("graph-now-value:{}", curve.id),
                            "text",
                            Rect::new(x + 7., at - text_height + 2., 120., text_height),
                            curve.color,
                        );
                        label.text = Some(value_text(value, 2));
                        label.font_size = small;
                        label.detail = json!({"tabular":true});
                        label.clip = Some(body);
                        ops.push(label);
                    }
                }
            }
        }
        if let Some(rect) = self.selection_box(v, region) {
            if ![rect.x, rect.y, rect.width, rect.height]
                .iter()
                .all(|v| v.is_finite())
                || rect.width < 0.
                || rect.height < 0.
            {
                return Err(Error::InvalidGeometry);
            }
            let mut fill = Op::new("graph-selection-fill", "rect", rect, c.accent);
            fill.opacity = 0.12;
            fill.clip = Some(body);
            ops.push(fill);
            let mut op = Op::new("graph-selection", "ring", rect, c.accent);
            op.stroke = 1.;
            op.detail = json!({"dash":[3.,3.]});
            op.clip = Some(body);
            ops.push(op);
            let shown = if self.transformable() {
                grips(rect).to_vec()
            } else {
                vec![]
            };
            for (i, (x, y)) in shown.into_iter().enumerate() {
                let mut op = Op::new(
                    format!("graph-transform-handle:{i}"),
                    "rect",
                    Rect::new(x.round() - 3., y.round() - 3., 6., 6.),
                    c.accent,
                );
                op.clip = Some(body);
                ops.push(op);
            }
        }
        // The legend: its own strip, clear of curves and value lines.
        let strip = Rect::new(region.x, region.y, region.width, LEGEND);
        let mut ground = Op::new("graph-legend", "rect", strip, c.panel);
        ground.clip = Some(region);
        ops.push(ground);
        let (edge, alpha) = t.line();
        let mut rule = Op::new(
            "graph-legend-rule",
            "rect",
            Rect::new(region.x, region.y, region.width, 1.),
            edge,
        );
        rule.opacity = alpha as f64;
        rule.clip = Some(region);
        ops.push(rule);
        let (mode_rect, fit_rect) = self.legend_controls(region);
        let show_pills = decor.is_some() && region.width >= 180.;
        let legend_limit = if show_pills {
            (mode_rect.x - 8.).max(region.x + 12.)
        } else {
            right - 8.
        };
        let mut x = region.x + 12.;
        let legend: Vec<(String, u32)> = if self.merged {
            vec![("Merged curves".to_owned(), c.accent)]
        } else {
            self.curves
                .iter()
                .map(|c| (c.name.clone(), c.color))
                .collect()
        };
        for (name, ink) in legend {
            let width = name.chars().count() as f64 * small * 0.58 + 25.;
            if x + width > legend_limit {
                let mut more = Op::new(
                    "graph-legend-more",
                    "text",
                    Rect::new(x, region.y + 17. - text_height + 2., 20., text_height),
                    c.muted,
                );
                more.text = Some("…".into());
                more.font_size = small;
                more.clip = Some(strip);
                ops.push(more);
                break;
            }
            let mut swatch = Op::new(
                format!("graph-legend-swatch:{name}"),
                "rect",
                Rect::new(x, region.y + 11., 10., 2.),
                ink,
            );
            swatch.clip = Some(strip);
            ops.push(swatch);
            let mut label = Op::new(
                format!("graph-legend:{name}"),
                "text",
                Rect::new(x + 15., region.y + 8., width, text_height),
                c.secondary,
            );
            label.text = Some(name);
            label.font_size = small;
            label.clip = Some(strip);
            ops.push(label);
            x += width;
        }
        // Quick Value/Speed toggle & Fit controls on the right of the legend strip.
        let mode_hover = decor.is_some_and(|d| d.hover == Some(GraphHit::LegendMode));
        let fit_hover = decor.is_some_and(|d| d.hover == Some(GraphHit::LegendFit));
        let mut draw_pill = |id: &str, rect: Rect, text: &str, active: bool, hovered: bool| {
            let mut bg = Op::new(
                format!("graph-pill-bg:{id}"),
                "rounded-rect",
                rect,
                if active {
                    c.selected_fill
                } else if hovered {
                    c.hover
                } else {
                    c.field
                },
            );
            bg.radius = 4.;
            bg.clip = Some(strip);
            ops.push(bg);
            let mut lbl = Op::new(
                format!("graph-pill:{id}"),
                "text",
                rect,
                if active || hovered { c.text } else { c.secondary },
            );
            lbl.text = Some(text.to_owned());
            lbl.font_size = small;
            lbl.detail = json!({"align":"center"});
            lbl.clip = Some(strip);
            ops.push(lbl);
        };
        if show_pills {
            draw_pill(
                "mode",
                mode_rect,
                if self.speed { "Speed" } else { "Value" },
                self.speed,
                mode_hover,
            );
            draw_pill("fit", fit_rect, "Fit", false, fit_hover);
        }
        Ok(ops)
    }
}
/// A transform box's grips (v1's `graphTransformHandles`), in `GRIPS` order.
pub const GRIPS: [&str; 8] = ["nw", "n", "ne", "e", "se", "s", "sw", "w"];
pub fn grips(b: Rect) -> [(f64, f64); 8] {
    let (x0, y0, x1, y1) = (b.x, b.y, b.x + b.width, b.y + b.height);
    let (mx, my) = ((x0 + x1) / 2., (y0 + y1) / 2.);
    [
        (x0, y0),
        (mx, y0),
        (x1, y0),
        (x1, my),
        (x1, y1),
        (mx, y1),
        (x0, y1),
        (x0, my),
    ]
}
/// The grip within `GRIP_HIT` points of `(x, y)`, the nearest.
pub fn grip_at(b: Rect, x: f64, y: f64) -> Option<usize> {
    let mut best = None;
    let mut reach = GRIP_HIT;
    for (i, (gx, gy)) in grips(b).into_iter().enumerate() {
        let d = (x - gx).hypot(y - gy);
        if d <= reach {
            (best, reach) = (Some(i), d);
        }
    }
    best
}
/// v1's drag readout (`drawGraphReadout`): the numbers under the pointer in
/// a small accent bubble.
pub fn readout(text: &str, x: f64, y: f64, region: Rect, t: &Tokens) -> Vec<Op> {
    let small = t.typography.minimum as f64;
    let width = text.chars().count() as f64 * small * 0.62 + 12.;
    let height = 18.;
    let right = region.x + region.width;
    let bottom = region.y + region.height;
    let bx = (x + 14.).clamp(region.x + 4., (right - width - 4.).max(region.x + 4.));
    let by = (y - height - 8.).clamp(region.y + 32., (bottom - height - 4.).max(region.y + 32.));
    let mut bubble = Op::new(
        "graph-readout",
        "rounded-rect",
        Rect::new(bx, by, width, height),
        t.colors.accent,
    );
    bubble.radius = 3.;
    let mut label = Op::new(
        "graph-readout-text",
        "text",
        Rect::new(bx + 6., by, width - 6., height),
        0xffffff,
    );
    label.text = Some(text.to_owned());
    label.font_size = small;
    label.detail = json!({"tabular":true});
    vec![bubble, label]
}
/// The handle side as it appears in op ids: `graph-handle:<key>:<in|out>`.
pub fn side_name(side: EaseSide) -> &'static str {
    match side {
        EaseSide::In => "in",
        _ => "out",
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn value_lines_fall_about_every_44_points_on_round_steps() {
        assert_eq!(value_ticks(0., 100., 220.), [0., 20., 40., 60., 80., 100.]);
        assert_eq!(value_ticks(-0.3, 0.9, 132.), [0., 0.5]);
        assert_eq!(value_ticks(-0.3, 0.9, 400.), [-0.2, 0., 0.2, 0.4, 0.6, 0.8]);
        assert!(value_ticks(1., 1., 100.).is_empty());
    }
    #[test]
    fn values_read_as_v1_prints_them() {
        assert_eq!(value_text(150., 1), "150");
        assert_eq!(value_text(0.25, 1), "0.3");
        assert_eq!(value_text(-20., 1), "-20");
        assert_eq!(value_text(194.8149, 2), "194.81");
        assert_eq!(value_text(-0.0001, 2), "0");
    }
    #[test]
    fn axes_keep_their_colours_and_keys_their_shapes() {
        // X, Y and Z by axis, others in turn, from the appearance's tokens:
        // each reads apart from its neighbours and from the plot's ground.
        for scheme in [pm_design::Scheme::Light, pm_design::Scheme::Dark] {
            let t = Tokens::new(scheme, Default::default());
            let inks = [
                t.graph_curve("position.x", 3),
                t.graph_curve("position.y", 0),
                t.graph_curve("position.z", 0),
                t.graph_curve("opacity", 1),
            ];
            for (i, a) in inks.iter().enumerate() {
                assert_ne!(*a, t.colors.panel, "{scheme:?}");
                for b in &inks[i + 1..] {
                    assert_ne!(a, b, "{scheme:?}");
                }
            }
            assert_ne!(t.graph_key_selected(), t.graph_handle_selected());
        }
        let key = |json: &str| KeyShape::of(&pm_model::json::parse(json).unwrap());
        assert_eq!(key(r#"{"outInterp":"hold"}"#), KeyShape::Hold);
        assert_eq!(key(r#"{"hold":true,"outInterp":"bezier"}"#), KeyShape::Hold);
        assert_eq!(
            key(r#"{"inInterp":"bezier","outInterp":"linear"}"#),
            KeyShape::Bezier
        );
        assert_eq!(
            key(r#"{"inInterp":"linear","outInterp":"linear"}"#),
            KeyShape::Linear
        );
        assert_eq!(time_step(100.), 1.);
        assert_eq!(time_step(1000.), 0.1);
    }

    #[test]
    fn linked_scale_and_combined_position_collapse_xy_until_unlinked_or_separated() {
        use pm_kernel::DocumentStore;
        use std::sync::Arc;
        let make = |extra: &str, key: &str| {
            let project = pm_model::json::parse(&format!(
                r#"{{
                    "id": "p", "name": "GraphXY", "w": 1920, "h": 1080, "fps": 30, "dur": 5,
                    "layers": [{{
                        "id": "L1", "type": "solid", "name": "Box", "from": 0, "dur": 5,
                        {extra}
                        "p": {{
                            "scale.x": {{"v": 100, "kf": [{{"t": 0, "v": 100, "i": "sx0"}}, {{"t": 2, "v": 200, "i": "sx1"}}]}},
                            "scale.y": {{"v": 100, "kf": [{{"t": 0, "v": 100, "i": "sy0"}}, {{"t": 2, "v": 200, "i": "sy1"}}]}},
                            "position.x": {{"v": 100, "kf": [{{"t": 0, "v": 100, "i": "px0"}}, {{"t": 2, "v": 400, "i": "px1"}}]}},
                            "position.y": {{"v": 100, "kf": [{{"t": 0, "v": 100, "i": "py0"}}, {{"t": 2, "v": 500, "i": "py1"}}]}}
                        }}
                    }}]
                }}"#
            ))
            .unwrap();
            let mut store = DocumentStore::new_untitled(project);
            store.select_layers(["L1"], false);
            store.select_keys([key]);
            let mut m = crate::TimelineModel::default();
            m.index = Arc::new(crate::TimelineIndex::from_snapshot(&store.snapshot()));
            m.selection = store.selection().clone();
            m.rebuild_rows();
            m
        };
        let eval = |_: &str, ch: &str, t: f64| -> Option<f64> {
            let u = (t / 2.).clamp(0., 1.);
            Some(match ch {
                "scale.x" | "scale.y" => 100. + 100. * u,
                "position.x" => 100. + 300. * u,
                "position.y" => 100. + 400. * u,
                _ => 0.,
            })
        };
        let tokens = Tokens::new(pm_design::Scheme::Dark, Default::default());

        // Default linked scale + combined position:
        // - Scale collapses to 1 curve named "Scale"
        // - Position collapses to 1 Speed curve named "Position (Speed)" with handles
        let m = make("", "sx0");
        let g = sample(&m, &tokens, eval).expect("scale graph");
        assert_eq!(g.curves().len(), 1);
        assert_eq!(g.curves()[0].name, "Scale");
        assert!(!g.speed);

        let m = make("", "px0");
        let g = sample(&m, &tokens, eval).expect("position speed graph");
        assert_eq!(g.curves().len(), 1);
        assert_eq!(g.curves()[0].name, "Position (Speed)");
        assert!(g.speed);
        // Velocity magnitude of (300/2, 400/2) = (150, 200) is 250 px/s.
        assert!((g.curves()[0].points[0].point.value - 250.).abs() < 1e-3);
        assert!(!g.curves()[0].points[0].handles.is_empty(), "speed handle shown");

        // Unlinked scale + separated position: X and Y plot separately again.
        let m = make(r#""scaleLinked": false, "separateDimensions": true,"#, "sx0");
        let g = sample(&m, &tokens, eval).expect("unlinked scale graph");
        assert_eq!(g.curves().len(), 2);

        let m = make(r#""scaleLinked": false, "separateDimensions": true,"#, "px0");
        let g = sample(&m, &tokens, eval).expect("separated position graph");
        assert_eq!(g.curves().len(), 2);
        assert!(!g.speed);
    }
}
