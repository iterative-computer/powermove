//! Native timeline paint plan. Preparation is worker-only; playback changes only
//! the overlay. Coordinates are points; authored seconds are never frame-rounded.
//! Spec: docs/native/design/panels/timeline.md §2–3. No platform/JS dependency.
use pm_design::{Scheme, Tokens};
use serde_json::{Value, json};
use std::{collections::HashMap, ops::Range};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl Rect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
    fn wire(self) -> Value {
        json!([self.x, self.y, self.width, self.height])
    }
    fn contains(self, x: f64, y: f64) -> bool {
        x >= self.x && x <= self.x + self.width && y >= self.y && y < self.y + self.height
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Op {
    pub id: String,
    /// Shape names are spec/host symbols, never browser/CSS paint commands.
    pub shape: &'static str,
    pub bounds: Rect,
    pub color: u32,
    pub opacity: f64,
    pub radius: f64,
    pub stroke: f64,
    pub text: Option<String>,
    pub font_size: f64,
    pub detail: Value,
    pub clip: Option<Rect>,
}
impl Op {
    pub(crate) fn new(
        id: impl Into<String>,
        shape: &'static str,
        bounds: Rect,
        color: u32,
    ) -> Self {
        Self {
            id: id.into(),
            shape,
            bounds,
            color,
            opacity: 1.,
            radius: 0.,
            stroke: 0.,
            text: None,
            font_size: 0.,
            detail: Value::Null,
            clip: None,
        }
    }
    pub fn wire(&self) -> Value {
        json!({"id":self.id,"shape":self.shape,"bounds":self.bounds.wire(),"color":self.color,"opacity":self.opacity,"radius":self.radius,"stroke":self.stroke,"text":self.text,"fontSize":self.font_size,"detail":self.detail,"clip":self.clip.map(Rect::wire)})
    }
}
/// GPUI/macOS implements this; Linux's recorder is an exact inspectable double.
/// DESIGN-TODO(opus): supply native marker/keyframe.hold paths and Rendering
/// stripe pitch/period. Named requests below do not invent art or cadence.
/// The host clips all operations to the supplied viewport before rasterisation.
pub trait Painter {
    fn paint(&mut self, viewport: Rect, ops: &[Op]);
}
#[derive(Default)]
pub struct Recorder {
    pub frames: Vec<Value>,
}
impl Painter for Recorder {
    fn paint(&mut self, viewport: Rect, ops: &[Op]) {
        self.frames.push(
            json!({"clip":viewport.wire(),"ops":ops.iter().map(Op::wire).collect::<Vec<_>>()}),
        );
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyState {
    Static,
    Animated,
    AtPlayhead,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    Linear,
    Ease,
    Hold,
}
#[derive(Clone, Debug)]
pub struct Key {
    pub id: String,
    pub seconds: f64,
    pub selected: bool,
    pub focused: bool,
    pub interpolation: Interpolation,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Media {
    Ready,
    Loading,
    Offline,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Label {
    Blue,
    Violet,
    Orange,
    Red,
    Yellow,
    Green,
    Teal,
    Gray,
}
impl Label {
    fn color(self, t: &Tokens) -> u32 {
        let l = t.colors.labels;
        match self {
            Self::Blue => l.blue,
            Self::Violet => l.violet,
            Self::Orange => l.orange,
            Self::Red => l.red,
            Self::Yellow => l.yellow,
            Self::Green => l.green,
            Self::Teal => l.teal,
            Self::Gray => l.gray,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Clip {
    pub id: String,
    pub name: String,
    pub from: f64,
    pub duration: f64,
    pub label: Label,
    pub selected: bool,
    pub locked: bool,
    pub media: Media,
    /// Already decoded/downsampled for this visible window, peak values in [-1,1].
    pub waveform: Option<Vec<f64>>,
    pub trim: Option<Edge>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    In,
    Out,
}
#[derive(Clone, Debug)]
pub enum RowContent {
    Layer {
        clip: Option<Clip>,
        summary: Vec<Key>,
    },
    Property {
        state: KeyState,
        keys: Vec<Key>,
    },
    Track {
        clips: Vec<Clip>,
    },
}
#[derive(Clone, Debug)]
pub struct PaintRow {
    pub id: String,
    pub accessibility_label: String,
    pub depth: usize,
    pub selected: bool,
    pub hovered: bool,
    pub focused: bool,
    pub content: RowContent,
}
impl PaintRow {
    fn height(&self, t: &Tokens) -> f64 {
        if matches!(self.content, RowContent::Property { .. }) {
            t.metrics.property_row as f64
        } else {
            t.metrics.track_row as f64
        }
    }
}
#[derive(Clone, Debug)]
pub struct Marker {
    pub id: String,
    pub seconds: f64,
    pub name: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarState {
    Missing,
    Rendering,
    Rendered,
    Kept,
    Stale,
}
#[derive(Clone, Debug)]
pub struct Bar {
    pub from: f64,
    pub to: f64,
    pub state: BarState,
}
#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    pub width: f64,
    pub height: f64,
    pub scroll_y: f64,
    pub scroll_time: f64,
    pub pps: f64,
    pub layer_column: f64,
}
impl Viewport {
    pub(crate) fn x(self, seconds: f64) -> f64 {
        self.layer_column + (seconds - self.scroll_time) * self.pps
    }
    fn rect(self) -> Rect {
        Rect::new(0., 0., self.width, self.height)
    }
    pub fn seconds(self, x: f64) -> Option<f64> {
        (x >= self.layer_column && x <= self.width)
            .then_some(self.scroll_time + (x - self.layer_column) / self.pps)
    }
    pub(crate) fn valid(self) -> bool {
        [
            self.width,
            self.height,
            self.scroll_y,
            self.scroll_time,
            self.pps,
            self.layer_column,
        ]
        .iter()
        .all(|v| v.is_finite())
            && self.width > 0.
            && self.height > 0.
            && self.scroll_y >= 0.
            && (4. ..=4000.).contains(&self.pps)
            && self.layer_column >= 0.
            && self.layer_column < self.width
    }
}
#[derive(Clone, Debug)]
pub struct Scene {
    rows: Vec<PaintRow>,
    tops: Vec<f64>,
    positions: HashMap<String, usize>,
    control_times: HashMap<String, std::sync::Arc<[f64]>>,
    track_ends: HashMap<String, Vec<f64>>,
    pub duration: f64,
    pub fps: f64,
    pub work_area: (f64, f64),
    pub markers: Vec<Marker>,
    pub bars: Vec<Bar>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidGeometry,
    InvalidTime,
    DuplicateId,
    MissingTrackPresentation,
}
/// Schema/media presentation facts supplied by native services on the worker.
/// Authored timing, lock and selection are always taken from TimelineModel.
pub trait ModelPresenter {
    fn accessibility_label(&self, row: &crate::RowId) -> String;
    fn layer_label(&self, layer: &crate::Layer) -> Label;
    fn media(&self, layer: &crate::Layer) -> Media;
    fn interpolation(&self, key: &crate::Key) -> Interpolation;
    /// Measured/downsampled audio peaks supplied by native media services.
    fn waveform(&self, _layer: &crate::Layer) -> Option<Vec<f64>> {
        None
    }
    /// Native schema supplies track names/AX copy; no renderer-invented labels.
    fn track_accessibility_label(
        &self,
        _area: crate::tracks::Area,
        _lane: usize,
    ) -> Option<String> {
        None
    }
}
impl Scene {
    pub fn from_model(
        model: &crate::TimelineModel,
        p: &dyn ModelPresenter,
        t: &Tokens,
        time: f64,
    ) -> Result<Self, Error> {
        use crate::RowId;
        use std::collections::HashSet;
        if !time.is_finite() {
            return Err(Error::InvalidTime);
        }
        let selected: HashSet<_> = model
            .selection
            .layers()
            .iter()
            .map(String::as_str)
            .collect();
        let selected_keys: HashSet<_> = model.selection.keys().iter().map(String::as_str).collect();
        let keys = |layer: &crate::Layer, channels: Vec<&crate::Channel>| {
            channels
                .into_iter()
                .flat_map(|ch| ch.keys.iter())
                .map(|k| Key {
                    id: k.id.clone(),
                    seconds: layer.from + k.time,
                    selected: selected_keys.contains(k.id.as_str()),
                    focused: false,
                    interpolation: p.interpolation(k),
                })
                .collect::<Vec<_>>()
        };
        let mut summaries = HashMap::new();
        let rows = model
            .rows()
            .iter()
            .map(|row| {
                let (content, is_selected) = match &row.id {
                    RowId::Layer(id) => {
                        let layer = model.index.layer(id).ok_or(Error::DuplicateId)?;
                        let summary = summaries
                            .entry(id.clone())
                            .or_insert_with(|| keys(layer, layer.channels.iter().collect()))
                            .clone();
                        (
                            RowContent::Layer {
                                clip: Some(Clip {
                                    id: id.clone(),
                                    name: layer.name.to_string_lossy(),
                                    from: layer.from,
                                    duration: layer.duration,
                                    label: p.layer_label(layer),
                                    selected: selected.contains(id.as_str()),
                                    locked: layer.effectively_locked,
                                    media: p.media(layer),
                                    waveform: p.waveform(layer),
                                    trim: None,
                                }),
                                summary,
                            },
                            selected.contains(id.as_str()),
                        )
                    }
                    RowId::Property { layer_id, channel } => {
                        let layer = model.index.layer(layer_id).ok_or(Error::DuplicateId)?;
                        let keys = keys(layer, model.track_channels(layer_id, channel));
                        let state = if keys.is_empty() {
                            KeyState::Static
                        } else if keys.iter().any(|k| (k.seconds - time).abs() <= 1e-5) {
                            KeyState::AtPlayhead
                        } else {
                            KeyState::Animated
                        };
                        (RowContent::Property { state, keys }, false)
                    }
                };
                Ok(PaintRow {
                    id: crate::v4::row_id(&row.id),
                    accessibility_label: p.accessibility_label(&row.id),
                    depth: row.depth,
                    selected: is_selected,
                    hovered: false,
                    focused: false,
                    content,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let rows = if model.mode == crate::TimelineMode::Tracks {
            use crate::tracks::Area;
            let layout = crate::tracks::layout(&model.index, &model.reveal);
            let mut clips: HashMap<(Area, usize), Vec<Clip>> = HashMap::new();
            let mut lane_by_layer = HashMap::new();
            for item in &layout.items {
                let layer = model.index.layer(&item.id).ok_or(Error::DuplicateId)?;
                lane_by_layer.insert(item.id.as_str(), (item.area, item.lane));
                clips.entry((item.area, item.lane)).or_default().push(Clip {
                    id: item.id.clone(),
                    name: layer.name.to_string_lossy(),
                    from: item.from,
                    duration: item.duration,
                    label: p.layer_label(layer),
                    selected: selected.contains(item.id.as_str()),
                    locked: layer.effectively_locked,
                    media: p.media(layer),
                    waveform: p.waveform(layer),
                    trim: None,
                });
            }
            let mut properties: HashMap<(Area, usize), Vec<PaintRow>> = HashMap::new();
            for row in rows {
                if let Ok(crate::RowId::Property { layer_id, .. }) = crate::v4::parse_row(&row.id)
                    && let Some(lane) = lane_by_layer.get(layer_id.as_str())
                {
                    properties.entry(*lane).or_default().push(row);
                }
            }
            let mut tracks = vec![];
            for (area, count) in [
                (Area::Video, layout.video),
                (Area::Audio, layout.audio),
                (Area::Captions, layout.captions),
            ] {
                for lane in 0..count {
                    tracks.push(PaintRow {
                        id: json!(["track", format!("{area:?}"), lane]).to_string(),
                        accessibility_label: p
                            .track_accessibility_label(area, lane)
                            .ok_or(Error::MissingTrackPresentation)?,
                        depth: 0,
                        selected: false,
                        hovered: false,
                        focused: false,
                        content: RowContent::Track {
                            clips: clips.remove(&(area, lane)).unwrap_or_default(),
                        },
                    });
                    tracks.extend(properties.remove(&(area, lane)).unwrap_or_default());
                }
            }
            tracks
        } else {
            rows
        };
        let mut scene = Self::new(rows, model.index.duration, model.index.fps, t)?;
        scene.work_area = (model.index.work[0], model.index.work[1]);
        scene.markers = model
            .index
            .markers
            .iter()
            .enumerate()
            .map(|(i, m)| Marker {
                id: m.id.clone().unwrap_or_else(|| format!("marker-{i}")),
                seconds: m.time,
                name: m.name.to_string_lossy(),
            })
            .collect();
        Ok(scene)
    }
    /// O(rows + keys log keys) on publication, never on a display callback.
    /// Sort presentation indexes only; the source model's ordering is untouched.
    pub fn new(
        mut rows: Vec<PaintRow>,
        duration: f64,
        fps: f64,
        t: &Tokens,
    ) -> Result<Self, Error> {
        if !duration.is_finite() || duration < 0. || !fps.is_finite() || fps <= 0. {
            return Err(Error::InvalidTime);
        }
        let mut tops = vec![0.];
        let mut positions = HashMap::new();
        for (i, row) in rows.iter_mut().enumerate() {
            if row.id.is_empty()
                || row.accessibility_label.is_empty()
                || positions.insert(row.id.clone(), i).is_some()
            {
                return Err(Error::DuplicateId);
            }
            let (clips, keys): (Vec<&Clip>, &mut Vec<Key>) = match &mut row.content {
                RowContent::Layer { clip, summary } => (clip.iter().collect(), summary),
                RowContent::Property { keys, .. } => (vec![], keys),
                RowContent::Track { clips } => {
                    if clips.iter().any(|c| !valid_clip(c)) {
                        return Err(Error::InvalidTime);
                    }
                    clips.sort_by(|a, b| a.from.total_cmp(&b.from));
                    tops.push(tops.last().unwrap() + row.height(t));
                    continue;
                }
            };
            if clips.iter().any(|c| !valid_clip(c)) || keys.iter().any(|k| !k.seconds.is_finite()) {
                return Err(Error::InvalidTime);
            }
            keys.sort_by(|a, b| a.seconds.total_cmp(&b.seconds));
            tops.push(tops.last().unwrap() + row.height(t));
        }
        let control_times = rows
            .iter()
            .filter_map(|row| match &row.content {
                RowContent::Property { keys, .. } => Some((
                    row.id.clone(),
                    keys.iter().map(|k| k.seconds).collect::<Vec<_>>().into(),
                )),
                _ => None,
            })
            .collect();
        let track_ends = rows
            .iter()
            .filter_map(|row| match &row.content {
                RowContent::Track { clips } => {
                    let mut end = f64::NEG_INFINITY;
                    Some((
                        row.id.clone(),
                        clips
                            .iter()
                            .map(|c| {
                                end = end.max(c.from + c.duration);
                                end
                            })
                            .collect(),
                    ))
                }
                _ => None,
            })
            .collect();
        Ok(Self {
            rows,
            tops,
            positions,
            control_times,
            track_ends,
            duration,
            fps,
            work_area: (0., duration),
            markers: vec![],
            bars: vec![],
        })
    }
    pub fn total(&self) -> usize {
        self.rows.len()
    }
    pub fn row(&self, id: &str) -> Option<&PaintRow> {
        self.positions.get(id).map(|i| &self.rows[*i])
    }
    /// AX virtual navigation resolves any ID/ordinal without mounting 60k nodes.
    pub fn navigate(&self, id: &str, delta: isize) -> Option<&PaintRow> {
        let i = self.positions.get(id)?.checked_add_signed(delta)?;
        self.rows.get(i)
    }
    pub fn navigate_first(&self, ordinal: usize) -> Option<String> {
        self.rows.get(ordinal).map(|r| r.id.clone())
    }
    pub fn scroll_for(&self, id: &str) -> Option<f64> {
        self.positions.get(id).map(|i| self.tops[*i])
    }
    /// Content height of all rows, for scroll clamping.
    pub fn content_height(&self) -> f64 {
        self.tops.last().copied().unwrap_or(0.)
    }
    /// The row at a content-space y. Variable heights use the same index as paint.
    pub fn row_at(&self, y: f64) -> Option<&PaintRow> {
        if y < 0. {
            return None;
        }
        self.rows
            .get(self.tops.partition_point(|top| *top <= y).checked_sub(1)?)
    }
    /// Ordinal window covering a scroll offset, for the outline's row request.
    pub fn visible_range(&self, scroll_y: f64, height: f64) -> Range<usize> {
        let first = self
            .tops
            .partition_point(|y| *y <= scroll_y)
            .saturating_sub(1)
            .min(self.total());
        let end = self
            .tops
            .partition_point(|y| *y < scroll_y + height)
            .min(self.total());
        first..end.max(first)
    }
    pub fn anchor(&self, v: Viewport) -> Option<(&str, f64)> {
        let i = self
            .tops
            .partition_point(|y| *y <= v.scroll_y)
            .saturating_sub(1)
            .min(self.total().saturating_sub(1));
        self.rows
            .get(i)
            .map(|r| (r.id.as_str(), v.scroll_y - self.tops[i]))
    }
    pub fn reanchor(&self, id: &str, offset: f64) -> Option<f64> {
        self.scroll_for(id).map(|y| (y + offset).max(0.))
    }
    fn range(&self, v: Viewport, t: &Tokens) -> Range<usize> {
        let height = (v.height - t.metrics.ruler as f64).max(0.);
        let first = self
            .tops
            .partition_point(|y| *y < (v.scroll_y - height).max(0.))
            .saturating_sub(1)
            .min(self.total());
        let end = self
            .tops
            .partition_point(|y| *y <= v.scroll_y + 2. * height)
            .min(self.total());
        first..end
    }
    /// One viewport of overscan on each side; time keys use binary search.
    pub fn prepare(&self, v: Viewport, t: &Tokens) -> Result<Frame, Error> {
        if !v.valid() {
            return Err(Error::InvalidGeometry);
        }
        if !self.work_area.0.is_finite()
            || !self.work_area.1.is_finite()
            || self.work_area.0 < 0.
            || self.work_area.0 > self.work_area.1
            || self.work_area.1 > self.duration
            || self.markers.iter().any(|m| !m.seconds.is_finite())
            || self
                .bars
                .iter()
                .any(|b| !b.from.is_finite() || !b.to.is_finite() || b.to < b.from)
        {
            return Err(Error::InvalidTime);
        }
        let mut ops = vec![];
        let m = t.metrics;
        let c = &t.colors;
        // The track ground is the panel material (a theme's panel colour at
        // the material alpha; opaque under Reduce Transparency). The shell
        // leaves this panel's body to it.
        let mut ground = Op::new("ground", "rect", v.rect(), c.panel);
        ground.opacity = if t.translucent() {
            t.translucency.panel as f64
        } else {
            1.
        };
        ops.push(ground);
        ops.push(Op::new(
            "gutter",
            "rect",
            Rect::new(0., 0., v.layer_column, v.height),
            c.panel_secondary,
        ));
        self.ruler(v, t, &mut ops);
        let range = self.range(v, t);
        let mut ax = vec![];
        let mut controls = vec![];
        for i in range.clone() {
            let op_start = ops.len();
            let row = &self.rows[i];
            let y = m.ruler as f64 + self.tops[i] - v.scroll_y;
            let h = row.height(t);
            // Selection and hover fill the outline column only; in the tracks
            // the clip's accent ring marks the selection (the owner's demo).
            let rect = Rect::new(0., y, v.layer_column, h);
            if row.selected || row.hovered {
                ops.push(Op::new(
                    format!("row:{}", row.id),
                    "rect",
                    rect,
                    if row.selected {
                        c.selected_fill
                    } else {
                        c.hover
                    },
                ));
            }
            if row.focused {
                let mut o = Op::new(
                    format!("focus:{}", row.id),
                    "ring",
                    Rect::new(1., y + 1., v.width - 2., h - 2.),
                    c.focus,
                );
                o.stroke = 2.;
                ops.push(o);
            }
            if t.accessibility.increase_contrast
                && !matches!(row.content, RowContent::Property { .. })
            {
                ops.push(Op::new(
                    format!("separator:{}", row.id),
                    "rect",
                    Rect::new(0., y + h - m.border as f64, v.width, m.border as f64),
                    c.separator,
                ));
            }
            match &row.content {
                RowContent::Layer { clip, summary } => {
                    if let Some(clip) = clip {
                        paint_clip(clip, y, h, v, t, &mut ops);
                    }
                    paint_keys(
                        &row.id,
                        summary,
                        true,
                        y + h - m.inner_radius as f64,
                        v,
                        t,
                        &mut ops,
                    );
                }
                RowContent::Track { clips } => {
                    let first =
                        self.track_ends[&row.id].partition_point(|end| *end < v.scroll_time);
                    let last = clips.partition_point(|clip| {
                        clip.from <= v.scroll_time + (v.width - v.layer_column) / v.pps
                    });
                    for clip in &clips[first.min(last)..last] {
                        paint_clip(clip, y, h, v, t, &mut ops);
                    }
                }
                RowContent::Property { state, keys } => {
                    let mut o = Op::new(
                        format!("toggle:{}", row.id),
                        match state {
                            KeyState::Static => "diamond",
                            KeyState::Animated => "diamond.lefthalf.filled",
                            KeyState::AtPlayhead => "diamond.fill",
                        },
                        Rect::new(
                            m.inset as f64 + m.hit as f64 + (m.hit - m.keyframe) as f64 / 2.,
                            y + (h - m.keyframe as f64) / 2.,
                            m.keyframe as f64,
                            m.keyframe as f64,
                        ),
                        if *state == KeyState::Static {
                            c.muted
                        } else {
                            c.accent
                        },
                    );
                    o.detail = json!({"navigationVisible":*state!=KeyState::Static,"labelX":m.inset+3.*m.hit+m.gap});
                    controls.push((o.clone(), self.control_times[&row.id].clone()));
                    ops.push(o);
                    paint_keys(&row.id, keys, false, y + h / 2., v, t, &mut ops);
                }
            }
            for op in &mut ops[op_start..] {
                let track = op.id.starts_with("clip")
                    || op.id.starts_with("key:")
                    || op.id.starts_with("key-focus:")
                    || op.id.starts_with("trim:");
                let x = if track { v.layer_column } else { 0. };
                op.clip = Some(Rect::new(
                    x,
                    m.ruler as f64,
                    v.width - x,
                    (v.height - m.ruler as f64).max(0.),
                ));
            }
            ax.push(json!({"id":row.id,"role":"row","label":row.accessibility_label,"rowIndex":i,"rowCount":self.total(),"selected":row.selected,"depth":row.depth,"focusable":false}));
        }
        // Outside the work area: veil track pixels only, after clips and keys.
        for (side, (a, b)) in [
            (v.layer_column, v.x(self.work_area.0)),
            (v.x(self.work_area.1), v.width),
        ]
        .into_iter()
        .enumerate()
        {
            if let Some(r) = span(a, b, m.ruler as f64, v.height - m.ruler as f64, v) {
                let mut o = Op::new(
                    format!("work-area-veil:{side}"),
                    "rect",
                    r,
                    c.panel_secondary,
                );
                o.opacity = 0.3;
                ops.push(o);
            }
        }
        // The layer column's edge, over row fills and clips: a hairline (the
        // resize handle's mark).
        let (edge, edge_alpha) = t.line();
        let mut o = Op::new(
            "column-edge",
            "rect",
            Rect::new(
                v.layer_column - m.border as f64,
                0.,
                m.border as f64,
                v.height,
            ),
            edge,
        );
        o.opacity = edge_alpha as f64;
        ops.push(o);
        Ok(Frame {
            viewport: v,
            range,
            total: self.total(),
            ops,
            accessibility: ax,
            controls,
        })
    }
    fn ruler(&self, v: Viewport, t: &Tokens, ops: &mut Vec<Op>) {
        let (m, c) = (t.metrics, &t.colors);
        let h = m.ruler as f64;
        ops.push(Op::new(
            "ruler-border",
            "rect",
            Rect::new(
                v.layer_column,
                h - m.border as f64,
                v.width - v.layer_column,
                m.border as f64,
            ),
            c.separator,
        ));
        if self.work_area != (0., self.duration) {
            if let Some(r) = span(v.x(self.work_area.0), v.x(self.work_area.1), 0., h, v) {
                ops.push(Op::new("work-area", "rect", r, c.accent_wash));
            }
            for (edge, time) in [("in", self.work_area.0), ("out", self.work_area.1)] {
                let x = v.x(time);
                if x >= v.layer_column && x <= v.width {
                    let mut o = Op::new(
                        format!("work-{edge}"),
                        "work-area-handle",
                        Rect::new(x - 3., 0., 6., h),
                        c.accent,
                    );
                    o.radius = m.inner_radius as f64;
                    o.detail = json!({"outerEdge":edge});
                    ops.push(o);
                }
            }
        }
        // Ticks are `text` ink at low alpha, so they sit in the material like
        // the panel hairlines; Increase Contrast makes them solid.
        let (tick, major) = if t.accessibility.increase_contrast {
            ((c.separator, 1.), (c.secondary, 1.))
        } else {
            (
                (c.text, t.translucency.tick_minor as f64),
                (c.text, t.translucency.tick_major as f64),
            )
        };
        // Literal spec chooses the largest candidate retaining >=8pt spacing.
        let step = [1. / self.fps, 0.25, 1., 5.]
            .into_iter()
            .filter(|s| s * v.pps >= 8.)
            .max_by(f64::total_cmp)
            .unwrap_or(1. / self.fps);
        let label_step = if v.pps / self.fps >= 40. {
            (60. / (v.pps / self.fps)).ceil() / self.fps
        } else {
            [1., 5., 10., 15., 30., 60., 120., 300., 600., 1800., 3600.]
                .into_iter()
                .find(|s| s * v.pps >= 60.)
                .unwrap_or(3600.)
        };
        let right = v.scroll_time + (v.width - v.layer_column) / v.pps;
        let mut i = (v.scroll_time / step).ceil().max(0.) as u64;
        while (i as f64) * step <= right && ops.len() < 10_000 {
            let time = i as f64 * step;
            let x = v.x(time);
            let mut o = Op::new(
                format!("minor:{i}"),
                "rect",
                Rect::new(x, h - 3., m.border as f64, 3.),
                tick.0,
            );
            o.opacity = tick.1;
            ops.push(o);
            let Some(next) = i.checked_add(1) else {
                break;
            };
            i = next;
        }
        let mut i = (v.scroll_time / label_step).ceil().max(0.) as u64;
        while i as f64 * label_step <= right && ops.len() < 10_000 {
            let time = i as f64 * label_step;
            let x = v.x(time);
            let mut o = Op::new(
                format!("major:{i}"),
                "rect",
                Rect::new(x, h - 7., m.border as f64, 7.),
                major.0,
            );
            o.opacity = major.1;
            ops.push(o);
            // A faint second grid under the tracks (the owner's demo).
            if !t.accessibility.increase_contrast {
                let mut o = Op::new(
                    format!("grid:{i}"),
                    "rect",
                    Rect::new(x, h, m.border as f64, (v.height - h).max(0.)),
                    c.text,
                );
                o.opacity = t.translucency.grid as f64;
                ops.push(o);
            }
            let mut o = Op::new(
                format!("time:{i}"),
                "text",
                Rect::new(
                    x + 4.,
                    4.,
                    label_step * v.pps - 4.,
                    t.typography.minimum as f64,
                ),
                c.muted,
            );
            let seconds = time.floor() as u64;
            o.text = Some(if v.pps / self.fps >= 40. {
                format!(
                    "{}:{:02}:{:02}",
                    seconds / 60,
                    seconds % 60,
                    ((time - seconds as f64) * self.fps).round() as u64
                )
            } else {
                format!("{}:{:02}", seconds / 60, seconds % 60)
            });
            o.font_size = t.typography.minimum as f64;
            o.detail = json!({"tabular":true,"baseline":"top + 4 + capHeight"});
            ops.push(o);
            let Some(next) = i.checked_add(1) else {
                break;
            };
            i = next;
        }
        for marker in &self.markers {
            let x = v.x(marker.seconds);
            if x >= v.layer_column && x <= v.width {
                ops.push(Op::new(
                    format!("marker-guide:{}", marker.id),
                    "rect",
                    Rect::new(x, h, m.border as f64, v.height - h),
                    c.separator,
                ));
                let mut o = Op::new(
                    format!("marker:{}", marker.id),
                    "marker",
                    Rect::new(x - 4.5, h - 9., 9., 9.),
                    c.secondary,
                );
                o.detail = json!({"name":marker.name,"seconds":marker.seconds});
                ops.push(o);
            }
        }
        for (i, b) in self.bars.iter().enumerate() {
            if matches!(b.state, BarState::Missing | BarState::Stale) {
                continue;
            }
            if let Some(r) = span(v.x(b.from), v.x(b.to), 0., 3., v) {
                let mut o = Op::new(
                    format!("render:{i}"),
                    if b.state == BarState::Rendering && !t.accessibility.reduce_motion {
                        "stripes-45"
                    } else {
                        "rect"
                    },
                    r,
                    if b.state == BarState::Rendering {
                        c.muted
                    } else {
                        c.success
                    },
                );
                o.opacity = match b.state {
                    BarState::Rendering if t.accessibility.reduce_motion => 0.5,
                    BarState::Rendered => 0.7,
                    _ => 1.,
                };
                ops.push(o);
                if b.state == BarState::Kept {
                    ops.push(Op::new(
                        format!("kept:{i}"),
                        "rect",
                        Rect::new(r.x, r.y, r.width, 1.),
                        c.text,
                    ));
                }
            }
        }
    }
}
fn valid_clip(c: &Clip) -> bool {
    c.from.is_finite()
        && c.duration.is_finite()
        && c.duration >= 0.
        && (c.from + c.duration).is_finite()
        && c.waveform
            .as_ref()
            .is_none_or(|w| w.iter().all(|p| p.is_finite() && (-1. ..=1.).contains(p)))
}
fn span(a: f64, b: f64, y: f64, h: f64, v: Viewport) -> Option<Rect> {
    let x = a.max(v.layer_column);
    let end = b.min(v.width);
    (end > x).then_some(Rect::new(x, y, end - x, h.max(0.)))
}
fn mix(a: u32, b: u32, amount: f64) -> u32 {
    [0, 8, 16].into_iter().fold(0, |out, s| {
        out | (((((a >> s) & 255) as f64 * (1. - amount) + ((b >> s) & 255) as f64 * amount).round()
            as u32)
            << s)
    })
}
fn paint_clip(c: &Clip, y: f64, h: f64, v: Viewport, t: &Tokens, ops: &mut Vec<Op>) {
    let m = t.metrics;
    let colors = &t.colors;
    let Some(r) = span(
        v.x(c.from),
        v.x(c.from + c.duration),
        y + m.inner_radius as f64,
        h - 2. * m.inner_radius as f64,
        v,
    ) else {
        return;
    };
    let fill = if c.media == Media::Offline {
        colors.labels.gray
    } else {
        c.label.color(t)
    };
    let mut o = Op::new(
        format!("clip:{}", c.id),
        "rounded-rect",
        r,
        if c.selected {
            mix(
                fill,
                colors.text,
                if t.scheme == Scheme::Dark { 0.12 } else { 0.06 },
            )
        } else {
            fill
        },
    );
    o.radius = m.clip_radius as f64;
    o.detail = json!({"from":c.from,"duration":c.duration,"cursor":if c.locked {"notAllowed"}else{"openHand"},"sourceEdges":[v.x(c.from),v.x(c.from+c.duration)]});
    ops.push(o);
    if !c.selected {
        // A hairline keyline gives the muted fill an edge against the tracks.
        let (edge, alpha) = t.edge();
        let mut o = Op::new(format!("clip-edge:{}", c.id), "ring", r, edge);
        o.opacity = if t.accessibility.increase_contrast {
            1.
        } else {
            (alpha * 2.) as f64
        };
        o.stroke = m.border as f64;
        o.radius = m.inner_radius as f64;
        ops.push(o);
    }
    if c.selected {
        let mut o = Op::new(
            format!("clip-ring:{}", c.id),
            "outside-ring",
            r,
            colors.accent,
        );
        o.stroke = m.playhead as f64;
        o.radius = m.clip_radius as f64;
        ops.push(o);
    }
    let mut o = Op::new(
        format!("clip-content:{}", c.id),
        if c.waveform.is_some() {
            "waveform"
        } else {
            "text"
        },
        Rect::new(
            r.x + (m.inset - m.inner_radius) as f64,
            r.y,
            (r.width - (m.inset - m.inner_radius) as f64).max(0.),
            r.height,
        ),
        if c.media == Media::Offline {
            colors.danger
        } else if c.media == Media::Loading {
            colors.muted
        } else {
            colors.text
        },
    );
    o.text = Some(if c.media == Media::Loading {
        "Loading…".into()
    } else {
        c.name.clone()
    });
    o.font_size = if c.media == Media::Loading {
        t.typography.minimum
    } else {
        t.typography.body
    } as f64;
    o.detail = json!({"truncate":"tail","offlineSymbol":if c.media==Media::Offline {Some("exclamationmark.triangle.fill")}else{None},"lockSymbol":if c.locked {Some("lock")}else{None},"waveform":c.waveform,"maxPeakToPeak":h-4.*m.inner_radius as f64});
    if c.waveform.is_some() {
        o.opacity = 0.75;
    }
    ops.push(o);
    if let Some(edge) = c.trim {
        let x = match edge {
            Edge::In => v.x(c.from),
            Edge::Out => v.x(c.from + c.duration) - 2.,
        };
        if x >= v.layer_column && x + 2. <= v.width && !c.locked {
            ops.push(Op::new(
                format!("trim:{}", c.id),
                "rect",
                Rect::new(x, r.y, 2., r.height),
                colors.text,
            ));
        }
    }
}
fn paint_keys(
    row: &str,
    keys: &[Key],
    summary: bool,
    y: f64,
    v: Viewport,
    t: &Tokens,
    ops: &mut Vec<Op>,
) {
    let size = t.metrics.keyframe as f64 - if summary { 2. } else { 0. };
    let from = v.scroll_time - size / v.pps;
    let to = v.scroll_time + (v.width - v.layer_column + size) / v.pps;
    let start = keys.partition_point(|k| k.seconds < from);
    let end = keys.partition_point(|k| k.seconds <= to);
    for key in &keys[start..end] {
        let x = v.x(key.seconds);
        let mut o = Op::new(
            format!("key:{}", json!([row, key.id])),
            if summary {
                "diamond"
            } else {
                match key.interpolation {
                    Interpolation::Linear => "diamond",
                    Interpolation::Ease => "circle",
                    Interpolation::Hold => "keyframe.hold",
                }
            },
            Rect::new(x - size / 2., y - size / 2., size, size),
            if key.selected {
                t.colors.accent
            } else {
                t.colors.text
            },
        );
        o.opacity = if key.selected { 1. } else { 0.9 };
        o.detail = json!({"keyline":if summary || key.selected {Some(t.colors.panel)}else{None},"keylineWidth":1.,"seconds":key.seconds});
        ops.push(o);
        if key.focused {
            let mut o = Op::new(
                format!("key-focus:{}", json!([row, key.id])),
                "outside-ring",
                Rect::new(x - size / 2., y - size / 2., size, size),
                t.colors.focus,
            );
            o.stroke = 2.;
            ops.push(o);
        }
    }
}
fn key_identity(op: &Op) -> Option<(String, String)> {
    serde_json::from_str(op.id.strip_prefix("key:")?).ok()
}
fn intersects(a: Rect, b: Rect) -> bool {
    a.x <= b.x + b.width && b.x <= a.x + a.width && a.y <= b.y + b.height && b.y <= a.y + a.height
}
#[derive(Clone, Debug)]
pub struct Frame {
    pub viewport: Viewport,
    pub range: Range<usize>,
    pub total: usize,
    pub ops: Vec<Op>,
    pub accessibility: Vec<Value>,
    controls: Vec<(Op, std::sync::Arc<[f64]>)>,
}
impl Frame {
    pub fn wire(&self) -> Value {
        json!({"range":[self.range.start,self.range.end],"rowCount":self.total,"ops":self.ops.iter().map(Op::wire).collect::<Vec<_>>(),"accessibility":self.accessibility})
    }
    /// Host persists the semantic-ID -> NodeId mapping across publications.
    /// Unmounted rows stay addressable through Scene::navigate/scroll_for.
    pub fn accessibility_tree(
        &self,
        root: accesskit::NodeId,
        row_ids: &HashMap<String, accesskit::NodeId>,
        focused: Option<&str>,
    ) -> Result<accesskit::TreeUpdate, Error> {
        use accesskit::{Node, Role, Tree, TreeUpdate};
        let mut outline = Node::new(Role::Tree);
        outline.set_label("Timeline");
        outline.set_row_count(self.total);
        outline.set_multiselectable();
        let mut nodes = vec![];
        let mut children = vec![];
        for row in &self.accessibility {
            let id = *row_ids
                .get(row["id"].as_str().ok_or(Error::DuplicateId)?)
                .ok_or(Error::DuplicateId)?;
            if id == root || children.contains(&id) {
                return Err(Error::DuplicateId);
            }
            let mut node = Node::new(Role::TreeItem);
            node.set_label(row["label"].as_str().unwrap_or_default());
            node.set_row_index(row["rowIndex"].as_u64().unwrap_or(0) as usize);
            node.set_selected(row["selected"].as_bool().unwrap_or(false));
            node.set_level(row["depth"].as_u64().unwrap_or(0) as usize + 1);
            if focused == row["id"].as_str() {
                outline.set_active_descendant(id);
            }
            children.push(id);
            nodes.push((id, node));
        }
        outline.set_children(children);
        nodes.push((root, outline));
        Ok(TreeUpdate {
            nodes,
            tree: Some(Tree::new(root)),
            focus: root,
            tree_id: accesskit::TreeId::ROOT,
        })
    }
    pub fn paint(&self, p: &mut dyn Painter) {
        p.paint(self.viewport.rect(), &self.ops);
    }
    /// Property diamonds follow transport without re-laying out rows or
    /// re-enumerating channels. Key times are shared worker-built indexes.
    pub fn transport(&self, seconds: f64, t: &Tokens) -> Result<Vec<Op>, Error> {
        let mut ops = self.playhead(seconds, t)?;
        for (base, times) in &self.controls {
            let i = times.partition_point(|time| *time < seconds - 1e-5);
            let at = times
                .get(i)
                .is_some_and(|time| (*time - seconds).abs() <= 1e-5);
            let mut op = base.clone();
            op.shape = if times.is_empty() {
                "diamond"
            } else if at {
                "diamond.fill"
            } else {
                "diamond.lefthalf.filled"
            };
            op.color = if times.is_empty() {
                t.colors.muted
            } else {
                t.colors.accent
            };
            ops.push(op);
        }
        Ok(ops)
    }
    /// Retained overlay: no row layout, model scan, JSON or JS on playback tick.
    pub fn playhead(&self, seconds: f64, t: &Tokens) -> Result<Vec<Op>, Error> {
        if !seconds.is_finite() {
            return Err(Error::InvalidTime);
        }
        let v = self.viewport;
        let x = v.x(seconds);
        if x < v.layer_column || x > v.width {
            return Ok(vec![]);
        }
        let mut head = Op::new(
            "playhead-head",
            "ph-head",
            Rect::new(x - 5.5, 0., 11., t.metrics.ruler as f64 - 4.),
            t.colors.accent,
        );
        head.detail = json!({"topRadius":3.,"bottomRadius":6.});
        Ok(vec![
            Op::new(
                "playhead-line",
                "rect",
                Rect::new(
                    x - t.metrics.playhead as f64 / 2.,
                    t.metrics.ruler as f64 - 4.,
                    t.metrics.playhead as f64,
                    (v.height - t.metrics.ruler as f64 + 4.).max(0.),
                ),
                t.colors.accent,
            ),
            head,
        ])
    }
    /// Topmost op of a family under the point. Native hit testing follows paint order.
    pub fn hit_op(&self, prefix: &str, x: f64, y: f64, slop: f64) -> Option<&Op> {
        self.ops.iter().rev().find(|o| {
            let b = o.bounds;
            o.id.starts_with(prefix)
                && Rect::new(
                    b.x - slop,
                    b.y - slop,
                    b.width + 2. * slop,
                    b.height + 2. * slop,
                )
                .contains(x, y)
                && o.clip.is_none_or(|clip| clip.contains(x, y))
        })
    }
    /// `(row id, key id)` of the key under the point, with a hit target padded
    /// to the design's minimum; the painted diamond stays `metrics.keyframe`.
    pub fn hit_key(&self, x: f64, y: f64, slop: f64) -> Option<(String, String)> {
        key_identity(self.hit_op("key:", x, y, slop)?)
    }
    /// Every key whose diamond meets the rectangle, for marquee selection.
    pub fn keys_in(&self, area: Rect) -> Vec<String> {
        self.ops
            .iter()
            .filter(|o| o.id.starts_with("key:") && intersects(o.bounds, area))
            .filter_map(key_identity)
            .map(|(_, key)| key)
            .collect()
    }
    /// Every clip whose painted body meets the rectangle.
    pub fn clips_in(&self, area: Rect) -> Vec<String> {
        self.ops
            .iter()
            .filter(|o| o.id.starts_with("clip:") && intersects(o.bounds, area))
            .map(|o| o.id[5..].to_owned())
            .collect()
    }
    pub fn hit_clip(&self, x: f64, y: f64) -> Option<(&str, Option<Edge>)> {
        if y < 0.
            || y >= self.viewport.height
            || x < self.viewport.layer_column
            || x > self.viewport.width
        {
            return None;
        }
        self.ops.iter().rev().find_map(|o| {
            if !o.id.starts_with("clip:")
                || !o.bounds.contains(x, y)
                || o.clip.is_some_and(|clip| !clip.contains(x, y))
            {
                return None;
            }
            let edges = o.detail["sourceEdges"].as_array()?;
            let edge = if (x - edges[0].as_f64()?).abs() <= 5. {
                Some(Edge::In)
            } else if (x - edges[1].as_f64()?).abs() <= 5. {
                Some(Edge::Out)
            } else {
                None
            };
            Some((&o.id[5..], edge))
        })
    }
}

/// One extension shape on a clip (`timeline.paint`), already validated:
/// finite and bounded, in points from the clip's top-left corner.
#[derive(Clone, Debug, PartialEq)]
pub enum Decoration {
    Rect {
        rect: [f64; 4],
        color: u32,
        opacity: f64,
        radius: f64,
    },
    Text {
        x: f64,
        y: f64,
        text: String,
        size: f64,
        color: u32,
        opacity: f64,
    },
}
/// Shapes by clip id.
pub type Decorations = HashMap<String, Vec<Decoration>>;
/// Shapes one clip may carry, from every painter together.
pub const DECORATIONS_PER_CLIP: usize = 64;
/// Characters one text shape may show.
pub const DECORATION_TEXT: usize = 200;
/// How far (points) a shape may sit from its clip; larger is clamped.
const DECORATION_REACH: f64 = 4096.;

/// Adds one painter's answer (`{clipId: [shape, …]}`) to `into`, keeping only
/// the `visible` clips, at most [`DECORATIONS_PER_CLIP`] shapes per clip, finite
/// bounded numbers and [`DECORATION_TEXT`] characters of text. Runs on the
/// worker that received the answer, before anything reaches the main thread.
pub fn add_decorations(
    into: &mut Decorations,
    answer: &Value,
    visible: &std::collections::HashSet<String>,
) {
    let Some(answer) = answer.as_object() else {
        return;
    };
    let number = |v: &Value, low: f64, high: f64| {
        v.as_f64()
            .filter(|n| n.is_finite())
            .map(|n| n.clamp(low, high))
    };
    let colour = |v: &Value| {
        v.as_str()
            .and_then(|h| h.strip_prefix('#'))
            .filter(|h| h.len() == 6)
            .and_then(|h| u32::from_str_radix(h, 16).ok())
    };
    let reach = DECORATION_REACH;
    for (id, shapes) in answer {
        let (Some(shapes), true) = (shapes.as_array(), visible.contains(id)) else {
            continue;
        };
        let list = into.entry(id.clone()).or_default();
        for shape in shapes {
            if list.len() >= DECORATIONS_PER_CLIP {
                break;
            }
            let Some(color) = colour(&shape["color"]) else {
                continue;
            };
            let opacity = number(&shape["opacity"], 0., 1.).unwrap_or(1.);
            if let Some(r) = shape["rect"].as_array().filter(|r| r.len() == 4) {
                let (Some(x), Some(y), Some(w), Some(h)) = (
                    number(&r[0], -reach, reach),
                    number(&r[1], -reach, reach),
                    number(&r[2], 0., reach),
                    number(&r[3], 0., reach),
                ) else {
                    continue;
                };
                list.push(Decoration::Rect {
                    rect: [x, y, w, h],
                    color,
                    opacity,
                    radius: number(&shape["radius"], 0., 64.).unwrap_or(0.),
                });
            } else if let Some(text) = shape["text"].as_str() {
                list.push(Decoration::Text {
                    x: number(&shape["x"], -reach, reach).unwrap_or(0.),
                    y: number(&shape["y"], -reach, reach).unwrap_or(0.),
                    text: text.chars().take(DECORATION_TEXT).collect(),
                    size: number(&shape["size"], 6., 24.).unwrap_or(10.),
                    color,
                    opacity,
                });
            }
        }
        if list.is_empty() {
            into.remove(id);
        }
    }
}

/// The frame's clips' extension decorations as ops, clipped to each clip's
/// visible part. Main-thread cost (TL-PAINT-1): ≤ 2 ms for 1,000 clips.
pub fn decoration_ops(ops: &[Op], decorations: &Decorations) -> Vec<Op> {
    if decorations.is_empty() {
        return vec![];
    }
    let mut out = vec![];
    for clip in ops {
        let Some(id) = clip.id.strip_prefix("clip:") else {
            continue;
        };
        let Some(shapes) = decorations.get(id) else {
            continue;
        };
        let b = clip.bounds;
        let within = Some(match clip.clip {
            Some(c) => {
                let (x0, y0) = (b.x.max(c.x), b.y.max(c.y));
                let (x1, y1) = (
                    (b.x + b.width).min(c.x + c.width),
                    (b.y + b.height).min(c.y + c.height),
                );
                Rect::new(x0, y0, (x1 - x0).max(0.), (y1 - y0).max(0.))
            }
            None => b,
        });
        for (i, shape) in shapes.iter().enumerate() {
            let base = Op {
                id: format!("deco:{id}:{i}"),
                shape: "rect",
                bounds: b,
                color: 0,
                opacity: 1.,
                radius: 0.,
                stroke: 0.,
                text: None,
                font_size: 0.,
                detail: Value::Null,
                clip: within,
            };
            out.push(match shape {
                Decoration::Rect {
                    rect: [x, y, w, h],
                    color,
                    opacity,
                    radius,
                } => Op {
                    shape: if *radius > 0. { "rounded-rect" } else { "rect" },
                    bounds: Rect::new(b.x + x, b.y + y, *w, *h),
                    color: *color,
                    opacity: *opacity,
                    radius: *radius,
                    ..base
                },
                Decoration::Text {
                    x,
                    y,
                    text,
                    size,
                    color,
                    opacity,
                } => Op {
                    shape: "text",
                    bounds: Rect::new(b.x + x, b.y + y, b.width, size * 1.4),
                    text: Some(text.clone()),
                    font_size: *size,
                    color: *color,
                    opacity: *opacity,
                    ..base
                },
            });
        }
    }
    out
}

#[cfg(test)]
mod decoration_tests {
    use super::*;
    fn clips(n: usize) -> Vec<Op> {
        (0..n)
            .map(|i| {
                let mut o = Op::new(
                    format!("clip:L{i}"),
                    "rounded-rect",
                    Rect::new(
                        300. + (i % 10) as f64 * 80.,
                        (i / 10) as f64 * 22.,
                        76.,
                        20.,
                    ),
                    0x1f4b52,
                );
                o.clip = Some(Rect::new(266., 28., 1200., 2200.));
                o
            })
            .collect()
    }
    fn visible(n: usize) -> std::collections::HashSet<String> {
        (0..n).map(|i| format!("L{i}")).collect()
    }
    /// TL-PAINT-1: extension shapes for 1,000 clips become ops in ≤ 2 ms on the
    /// main thread, at the 95th percentile of 40 runs. The gate is defined for
    /// an optimized build: `cargo test --release -p pm-panel-timeline`.
    #[test]
    #[cfg_attr(
        debug_assertions,
        ignore = "TL-PAINT-1 is measured in an optimized build (cargo test --release)"
    )]
    fn a_thousand_painted_clips_cost_under_two_milliseconds() {
        let ops = clips(1000);
        let answer: serde_json::Map<String, Value> = (0..1000)
            .map(|i| {
                (
                    format!("L{i}"),
                    json!([{"rect":[0,0,76,20],"color":"#5a1f8c","opacity":0.5},
                           {"text":"take 3","x":4,"y":4,"color":"#ffffff","size":10}]),
                )
            })
            .collect();
        let mut shapes = Decorations::new();
        add_decorations(&mut shapes, &Value::Object(answer), &visible(1000));
        assert_eq!(decoration_ops(&ops, &shapes).len(), 2000);
        let mut runs: Vec<std::time::Duration> = (0..40)
            .map(|_| {
                let start = std::time::Instant::now();
                std::hint::black_box(decoration_ops(&ops, &shapes));
                start.elapsed()
            })
            .collect();
        runs.sort();
        let p95 = runs[37];
        eprintln!("TL-PAINT-1 decoration ops for 1,000 clips: p95 {p95:?}");
        assert!(p95 <= std::time::Duration::from_millis(2), "{p95:?}");
    }
    #[test]
    fn painter_output_is_bounded_before_it_is_kept() {
        let long = "x".repeat(10_000);
        let many: Vec<Value> = (0..500)
            .map(|_| json!({"rect":[0,0,4,4],"color":"#ffffff"}))
            .collect();
        let answer = json!({
            "L0": [{"rect":[1e300,-1e300,1e300,5],"radius":1e300,"color":"#ff0000"},
                   {"rect":[0,0,"wide",5],"color":"#ff0000"},
                   {"rect":[0,0,5,5],"color":"red"},
                   {"text":long,"x":1e300,"size":1e300,"color":"#00ff00"}],
            "L1": many,
            "gone": [{"rect":[0,0,4,4],"color":"#ffffff"}],
        });
        let mut shapes = Decorations::new();
        add_decorations(&mut shapes, &answer, &visible(2));
        assert!(!shapes.contains_key("gone"), "only visible clips are kept");
        assert_eq!(shapes["L1"].len(), DECORATIONS_PER_CLIP);
        let l0 = &shapes["L0"];
        assert_eq!(l0.len(), 2, "unparseable and uncoloured shapes are dropped");
        let Decoration::Rect { rect, radius, .. } = &l0[0] else {
            panic!("{l0:?}")
        };
        assert_eq!(*rect, [4096., -4096., 4096., 5.]);
        assert_eq!(*radius, 64.);
        let Decoration::Text { text, x, size, .. } = &l0[1] else {
            panic!("{l0:?}")
        };
        assert_eq!(text.chars().count(), DECORATION_TEXT);
        assert_eq!((*x, *size), (4096., 24.));
        for op in decoration_ops(&clips(2), &shapes) {
            let b = op.bounds;
            assert!(
                [b.x, b.y, b.width, b.height]
                    .iter()
                    .all(|v| (*v as f32).is_finite())
            );
        }
        // A second painter adds to the same clip only up to the shared limit.
        add_decorations(
            &mut shapes,
            &json!({"L1":[{"rect":[0,0,4,4],"color":"#000000"}]}),
            &visible(2),
        );
        assert_eq!(shapes["L1"].len(), DECORATIONS_PER_CLIP);
    }
}
