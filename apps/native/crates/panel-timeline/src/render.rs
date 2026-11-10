//! Native timeline paint plan. Preparation is worker-only; playback changes only
//! the overlay. Coordinates are points; authored seconds are never frame-rounded.
//! Spec: docs/native/design/panels/timeline.md §2–3. No platform/JS dependency.
use pm_design::Tokens;
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
    /// A keyframe glyph identical to the key op before it at the same device
    /// half-pixel: painting it changes nothing. It stays for hit testing and
    /// marquee selection.
    pub covered: bool,
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
            covered: false,
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
    /// A clip's material by layer kind, not by the layer's own swatch, so every
    /// text clip is the same slate and every audio clip the same green (v1's
    /// type-keyed clip palette). The outline's kind badge shares it.
    pub fn for_kind(kind: &str) -> Self {
        match kind {
            "text" => Self::Blue,
            "shape" => Self::Red,
            "video" | "sequence" => Self::Teal,
            "image" | "captions" => Self::Yellow,
            "audio" => Self::Green,
            "solid" | "adjustment" | "effect" | "shader" | "extension" => Self::Violet,
            _ => Self::Gray,
        }
    }
    /// The `colors.labels.*` field name.
    pub fn name(self) -> &'static str {
        pm_design::Labels::NAMES[self as usize]
    }
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
    /// The layer is switched off: the clip draws at half strength.
    pub hidden: bool,
    pub media: Media,
    /// An audio clip draws its waveform instead of a name; `None` while the
    /// peaks are still being read (a hairline then marks the clip's middle).
    pub audio: Option<Option<Waveform>>,
    pub trim: Option<Edge>,
    /// The first missing extension effect: the clip carries a quiet badge.
    pub missing_effect: Option<String>,
    /// A captions clip's cues (S089), drawn in place of its name.
    pub cues: Vec<CueMark>,
}
/// One caption cue on its clip, in composition seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct CueMark {
    pub id: String,
    pub start: f64,
    pub end: f64,
    pub text: String,
}
/// The op id of a cue: `cue:` then the layer and cue ids, unit-separated.
pub fn cue_op(layer: &str, cue: &str) -> String {
    format!("cue:{layer}\u{1f}{cue}")
}
/// The layer and cue an op id names.
pub fn parse_cue_op(id: &str) -> Option<(&str, &str)> {
    id.strip_prefix("cue:")?.split_once('\u{1f}')
}
/// An audio file's peak envelope, read once per file off the main thread.
#[derive(Clone, Debug, PartialEq)]
pub struct Waveform {
    /// Largest absolute sample of any channel per bucket, 0–1.
    pub peaks: std::sync::Arc<[f32]>,
    /// Buckets per second of media.
    pub rate: f64,
    /// Media seconds skipped before the clip starts.
    pub trim: f64,
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
        summary: std::sync::Arc<[Key]>,
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
    /// An audio layer's waveform: `Some(None)` while its peaks are read,
    /// `None` for layers that draw a name.
    fn waveform(&self, _layer: &crate::Layer) -> Option<Option<Waveform>> {
        None
    }
    /// A captions layer's cues; none for other layers.
    fn cues(&self, _layer: &crate::Layer) -> Vec<CueMark> {
        vec![]
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
        // Selected keys by (layer, key): key ids may repeat across layers.
        let selected_keys: HashSet<(&str, &str)> = model
            .index
            .selected_keys(&model.selection)
            .into_iter()
            .map(|(l, _, k)| (l.id.as_str(), k.id.as_str()))
            .collect();
        let keys = |layer: &crate::Layer, channels: Vec<&crate::Channel>| {
            channels
                .into_iter()
                .flat_map(|ch| ch.keys.iter())
                .map(|k| Key {
                    id: k.id.clone(),
                    seconds: layer.from + k.time,
                    selected: selected_keys.contains(&(layer.id.as_str(), k.id.as_str())),
                    focused: false,
                    interpolation: p.interpolation(k),
                })
                .collect::<Vec<_>>()
        };
        // Layers with a selected key: only their marks are copied to show it.
        let marked: HashSet<&str> = selected_keys.iter().map(|(layer, _)| *layer).collect();
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
                            .or_insert_with(|| {
                                let shared = layer.summary(|layer| {
                                    keys(layer, layer.channels.iter().collect())
                                        .into_iter()
                                        .map(|k| Key {
                                            selected: false,
                                            ..k
                                        })
                                        .collect()
                                });
                                if marked.contains(id.as_str()) {
                                    shared
                                        .iter()
                                        .map(|k| Key {
                                            selected: selected_keys
                                                .contains(&(id.as_str(), k.id.as_str())),
                                            ..k.clone()
                                        })
                                        .collect()
                                } else {
                                    shared
                                }
                            })
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
                                    hidden: !layer.visible,
                                    media: p.media(layer),
                                    audio: p.waveform(layer),
                                    trim: None,
                                    missing_effect: layer.missing_effects.first().cloned(),
                                    cues: p.cues(layer),
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
                    hidden: !layer.visible,
                    media: p.media(layer),
                    audio: p.waveform(layer),
                    trim: None,
                    missing_effect: layer.missing_effects.first().cloned(),
                    cues: p.cues(layer),
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
            for (area, lane) in layout.display_order() {
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
            let sorted = |keys: &[Key]| keys.is_sorted_by(|a, b| a.seconds <= b.seconds);
            let (clips, keys): (Vec<&Clip>, &[Key]) = match &mut row.content {
                RowContent::Layer { clip, summary } => {
                    if !sorted(summary) {
                        let mut keys = summary.to_vec();
                        keys.sort_by(|a, b| a.seconds.total_cmp(&b.seconds));
                        *summary = keys.into();
                    }
                    (clip.iter().collect(), summary)
                }
                RowContent::Property { keys, .. } => {
                    keys.sort_by(|a, b| a.seconds.total_cmp(&b.seconds));
                    (vec![], keys)
                }
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
    /// Row `i` and its top and bottom in content space.
    pub fn span(&self, i: usize) -> Option<(&PaintRow, f64, f64)> {
        let row = self.rows.get(i)?;
        let top = self.tops[i];
        Some((row, top, self.tops.get(i + 1).copied().unwrap_or(top)))
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
    /// The rows on screen. Every scroll prepares a new frame, so rows past
    /// the viewport would only cost time (a project with baked animation
    /// puts tens of thousands of keys in them).
    fn range(&self, v: Viewport, t: &Tokens) -> Range<usize> {
        let height = (v.height - t.metrics.ruler as f64).max(0.);
        let first = self
            .tops
            .partition_point(|y| *y < v.scroll_y.max(0.))
            .saturating_sub(1)
            .min(self.total());
        let end = self
            .tops
            .partition_point(|y| *y < v.scroll_y + height)
            .min(self.total());
        first..end
    }
    /// The rows on screen; time keys use binary search.
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
            self.row_ops(i, v, t, &mut ops, Some(&mut controls));
            for op in &mut ops[op_start..] {
                let track = op.id.starts_with("clip")
                    || op.id.starts_with("lane:")
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
    /// What a marquee rectangle (panel coordinates of viewport `v`) covers:
    /// keys as `(layer, key)` and clips by id. It reads every row under the
    /// rectangle, built or not: a frame holds only the rows on screen, and a
    /// marquee dragged past the panel's edge covers rows outside it.
    pub fn marquee(&self, v: Viewport, t: &Tokens, area: Rect) -> Marquee {
        let ruler = t.metrics.ruler as f64;
        let (top, bottom) = (
            area.y - ruler + v.scroll_y,
            area.y + area.height - ruler + v.scroll_y,
        );
        let first = self
            .tops
            .partition_point(|y| *y <= top)
            .saturating_sub(1)
            .min(self.total());
        let end = self
            .tops
            .partition_point(|y| *y <= bottom)
            .min(self.total());
        let (mut covered, mut ops) = (Marquee::default(), vec![]);
        for i in first..end {
            ops.clear();
            self.row_ops(i, v, t, &mut ops, None);
            for o in ops.iter().filter(|o| intersects(o.bounds, area)) {
                if let Some(id) = o.id.strip_prefix("clip:") {
                    covered.clips.push(id.to_owned());
                } else if o.id.starts_with("key:") {
                    covered.keys.extend(key_identity(o).and_then(on_layer));
                }
            }
        }
        covered
    }
    /// Row `i`'s ops in panel space (unclipped), and its keyframe toggle
    /// with its key times when `controls` is given.
    fn row_ops(
        &self,
        i: usize,
        v: Viewport,
        t: &Tokens,
        ops: &mut Vec<Op>,
        mut controls: Option<&mut Vec<(Op, std::sync::Arc<[f64]>)>>,
    ) {
        let (m, c) = (t.metrics, &t.colors);
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
        if t.accessibility.increase_contrast && !matches!(row.content, RowContent::Property { .. })
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
                    paint_clip(clip, y, h, v, t, ops);
                }
                // Summary marks only while the layer is collapsed: open, its
                // property rows show the keys themselves.
                let open = self
                    .rows
                    .get(i + 1)
                    .is_some_and(|next| matches!(next.content, RowContent::Property { .. }));
                if !open {
                    let ink = clip.as_ref().map_or(c.text, |clip| {
                        t.clip_material(clip.label.color(t)).foreground
                    });
                    paint_keys(
                        &row.id,
                        summary,
                        Some(ink),
                        y + h - m.border as f64 - m.inner_radius as f64,
                        v,
                        t,
                        ops,
                    );
                }
            }
            RowContent::Track { clips } => {
                let first = self.track_ends[&row.id].partition_point(|end| *end < v.scroll_time);
                let last = clips.partition_point(|clip| {
                    clip.from <= v.scroll_time + (v.width - v.layer_column) / v.pps
                });
                for clip in &clips[first.min(last)..last] {
                    paint_clip(clip, y, h, v, t, ops);
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
                if let Some(controls) = controls.as_deref_mut() {
                    controls.push((o.clone(), self.control_times[&row.id].clone()));
                }
                ops.push(o);
                // The lane the keys sit on (v1): a faint line across the tracks.
                if !keys.is_empty() {
                    let mut lane = Op::new(
                        format!("lane:{}", row.id),
                        "rect",
                        Rect::new(
                            v.layer_column,
                            y + (h - m.border as f64) / 2.,
                            v.width - v.layer_column,
                            m.border as f64,
                        ),
                        c.text,
                    );
                    lane.opacity = t.translucency.grid as f64;
                    ops.push(lane);
                }
                paint_keys(&row.id, keys, None, y + h / 2., v, t, ops);
            }
        }
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
        // Labels at the first nice step at least 60 pt apart (frames once a
        // frame is 40 pt wide); minor ticks split each labelled step in four,
        // or into frames when the labels count frames.
        let frames = v.pps / self.fps >= 40.;
        let label_step = if frames {
            (60. / (v.pps / self.fps)).ceil() / self.fps
        } else {
            [1., 5., 10., 15., 30., 60., 120., 300., 600., 1800., 3600.]
                .into_iter()
                .find(|s| s * v.pps >= 60.)
                .unwrap_or(3600.)
        };
        let step = if frames {
            1. / self.fps
        } else {
            label_step / 4.
        };
        let right = v.scroll_time + (v.width - v.layer_column) / v.pps;
        if step * v.pps >= 6. {
            let mut i = (v.scroll_time / step).ceil().max(0.) as u64;
            while (i as f64) * step <= right && ops.len() < 10_000 {
                let x = v.x(i as f64 * step);
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
        }
        let mut i = (v.scroll_time / label_step).ceil().max(0.) as u64;
        while i as f64 * label_step <= right && ops.len() < 10_000 {
            let time = i as f64 * label_step;
            let x = v.x(time);
            let mut o = Op::new(
                format!("major:{i}"),
                "rect",
                Rect::new(x, h - 6., m.border as f64, 6.),
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
            // The label centres over its tick, so the numbers read as a
            // scale; one that would cross the layer column slides right.
            let width = label_step * v.pps;
            let mut o = Op::new(
                format!("time:{i}"),
                "text",
                Rect::new(x - width / 2., 2., width, t.typography.minimum as f64),
                c.muted,
            );
            let seconds = time.floor() as u64;
            o.text = Some(if frames {
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
            o.detail =
                json!({"tabular":true,"align":"center","minX":v.layer_column + m.gap as f64});
            ops.push(o);
            let Some(next) = i.checked_add(1) else {
                break;
            };
            i = next;
        }
        // Marker pins sit on the ruler's foot; a guide drops through the tracks.
        for marker in &self.markers {
            let x = v.x(marker.seconds);
            if x >= v.layer_column && x <= v.width {
                let mut o = Op::new(
                    format!("marker-guide:{}", marker.id),
                    "rect",
                    Rect::new(x, h, m.border as f64, v.height - h),
                    c.secondary,
                );
                o.opacity = 0.35;
                ops.push(o);
                let mut o = Op::new(
                    format!("marker:{}", marker.id),
                    "marker",
                    Rect::new(x - 4.5, h - 11., 9., 10.),
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
        && c.audio
            .iter()
            .flatten()
            .all(|w| w.rate.is_finite() && w.rate > 0. && w.trim.is_finite() && w.trim >= 0.)
}
fn span(a: f64, b: f64, y: f64, h: f64, v: Viewport) -> Option<Rect> {
    let x = a.max(v.layer_column);
    let end = b.min(v.width);
    (end > x).then_some(Rect::new(x, y, end - x, h.max(0.)))
}
/// Width of an offline clip's "Offline" lead, in `minimum` ems.
const OFFLINE_ADVANCE: f64 = 3.6;
/// Width of the "Missing effect" badge, in `minimum` ems.
const MISSING_EFFECT_ADVANCE: f64 = 6.6;
fn paint_clip(c: &Clip, y: f64, h: f64, v: Viewport, t: &Tokens, ops: &mut Vec<Op>) {
    let m = t.metrics;
    let colors = &t.colors;
    // Clips fill their row with a hairline breath between neighbours (v1).
    let Some(r) = span(
        v.x(c.from),
        v.x(c.from + c.duration),
        y + m.border as f64,
        h - 2. * m.border as f64,
        v,
    ) else {
        return;
    };
    // Media Offline (docs/native/design/media-offline.md): the viewer's
    // ground with the danger hatch, as the engine's card.
    let offline = c.media == Media::Offline;
    let fill = if offline {
        colors.viewer
    } else {
        c.label.color(t)
    };
    // The tactile chip (pm-design `clip_material`): gradient body, top
    // highlight, a seam one shade under the body, a soft drop shadow. A hidden
    // layer's clip draws at half strength.
    let material = t.clip_material(fill);
    let strength = if c.hidden { 0.5 } else { 1. };
    let mut o = Op::new(format!("clip:{}", c.id), "clip", r, fill);
    o.radius = m.clip_radius as f64;
    o.opacity = strength;
    o.detail = json!({
        "from":c.from,"duration":c.duration,
        "cursor":if c.locked {"notAllowed"}else{"openHand"},
        "sourceEdges":[v.x(c.from),v.x(c.from+c.duration)],
        "top":material.top,"bottom":material.bottom,
        "highlight":material.highlight.0,"highlightAlpha":material.highlight.1,
        "seam":material.seam,"seamWidth":m.border,
        "shadow":material.shadow.0,"shadowAlpha":material.shadow.1,
    });
    ops.push(o);
    if offline {
        let mut o = Op::new(
            format!("clip-hatch:{}", c.id),
            "stripes-45",
            r,
            colors.danger,
        );
        o.opacity = t.translucency.offline_hatch as f64 * strength;
        o.radius = m.clip_radius as f64;
        o.detail = json!({"period": (h * 0.45).max(6.), "stripe": (h * 0.18).max(2.)});
        ops.push(o);
    }
    // "Offline" leads in `danger`; the name follows in the clip's foreground.
    let lead = (m.inset - m.inner_radius) as f64;
    let offline_w = if offline {
        let w = t.typography.minimum as f64 * OFFLINE_ADVANCE;
        let mut o = Op::new(
            format!("clip-offline:{}", c.id),
            "text",
            Rect::new(r.x + lead, r.y, w.min((r.width - lead).max(0.)), r.height),
            colors.danger,
        );
        o.text = Some("Offline".into());
        o.font_size = t.typography.minimum as f64;
        o.opacity = strength;
        o.detail = json!({"truncate":"tail"});
        ops.push(o);
        w + m.gap as f64
    } else {
        0.
    };
    // A missing extension effect: a quiet trailing badge in `danger`; the name
    // truncates before it (media-offline.md). Too narrow a clip drops it.
    let badge_w = t.typography.minimum as f64 * MISSING_EFFECT_ADVANCE;
    let badge = c
        .missing_effect
        .as_ref()
        .filter(|_| r.width - lead - offline_w > badge_w + 3. * lead);
    let reserve = if badge.is_some() { badge_w + lead } else { 0. };
    let ink = if offline {
        colors.text
    } else {
        material.foreground
    };
    // A readable audio clip shows its waveform in place of the name (v1): the
    // bars stand on the clip's floor in the body's bright tint, one column per
    // point summarising every peak under it.
    if !c.cues.is_empty() {
        // Captions (S089): each cue a quiet chip with its text, in place of
        // the clip's name; a cue is clicked, double-clicked and read by itself.
        for cue in &c.cues {
            let x0 = v.x(cue.start).max(r.x + m.border as f64);
            let x1 = v.x(cue.end).min(r.x + r.width - m.border as f64);
            if x1 - x0 < 2. {
                continue;
            }
            let chip = Rect::new(
                x0,
                r.y + 2. * m.border as f64,
                x1 - x0,
                (r.height - 4. * m.border as f64).max(0.),
            );
            let mut o = Op::new(cue_op(&c.id, &cue.id), "rect", chip, material.primary);
            o.radius = (m.clip_radius as f64 - m.border as f64).max(0.);
            o.opacity = 0.35 * strength;
            o.detail = json!({"from":cue.start,"to":cue.end,"text":cue.text});
            ops.push(o);
            let mut o = Op::new(
                format!("cue-text:{}\u{1f}{}", c.id, cue.id),
                "text",
                Rect::new(
                    x0 + lead,
                    chip.y,
                    (x1 - x0 - 2. * lead).max(0.),
                    chip.height,
                ),
                ink,
            );
            o.text = Some(cue.text.replace('\n', " "));
            o.font_size = t.typography.minimum as f64;
            o.opacity = strength;
            o.detail = json!({"truncate":"tail"});
            ops.push(o);
        }
    } else if let (Some(audio), false) = (&c.audio, offline) {
        // Bars keep clear of the seam and the rounded corners on every side.
        let inset = 2. * m.border as f64;
        let r = Rect::new(r.x + inset, r.y, (r.width - 2. * inset).max(0.), r.height);
        let mut o = Op::new(
            format!("clip-content:{}", c.id),
            "waveform",
            r,
            material.primary,
        );
        o.text = Some(c.name.clone());
        o.opacity = strength;
        let left = v.x(c.from);
        let bars: Vec<f32> = audio
            .as_ref()
            .map(|w| {
                let count = r.width.floor().max(0.) as usize;
                (0..count)
                    .map(|i| {
                        let at = w.trim + (r.x + i as f64 - left) / v.pps;
                        let from = (at * w.rate).floor().max(0.) as usize;
                        let to = (((at + 1. / v.pps) * w.rate).floor() as usize).max(from + 1);
                        w.peaks
                            .get(from.min(w.peaks.len())..to.min(w.peaks.len()))
                            .map_or(0., |s| s.iter().copied().fold(0., f32::max))
                    })
                    .collect()
            })
            .unwrap_or_default();
        o.detail = json!({"bars": bars, "inset": m.border * 2., "minBar": m.border});
        ops.push(o);
    } else {
        let mut o = Op::new(
            format!("clip-content:{}", c.id),
            "text",
            Rect::new(
                r.x + lead + offline_w,
                r.y,
                (r.width - lead - offline_w - reserve).max(0.),
                r.height,
            ),
            ink,
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
        o.opacity = strength * if c.media == Media::Loading { 0.7 } else { 1. };
        o.detail = json!({"truncate":"tail","offlineSymbol":if offline {Some("exclamationmark.triangle.fill")}else{None},"lockSymbol":if c.locked {Some("lock")}else{None}});
        ops.push(o);
    }
    if let Some(effect) = badge {
        let mut o = Op::new(
            format!("clip-missing-effect:{}", c.id),
            "text",
            Rect::new(r.x + r.width - badge_w - lead, r.y, badge_w, r.height),
            colors.danger,
        );
        o.text = Some("Missing effect".into());
        o.font_size = t.typography.minimum as f64;
        o.opacity = strength;
        o.detail = json!({"truncate":"tail","tooltip":format!("Missing effect: {effect}. The layer draws without it.")});
        ops.push(o);
    }
    // Selection: an accent ring inside the clip's edge, so a selected clip
    // never grows into its neighbours (v1's inset ring).
    if c.selected {
        let mut o = Op::new(format!("clip-ring:{}", c.id), "ring", r, colors.accent);
        o.stroke = 2. * m.border as f64;
        o.radius = m.clip_radius as f64;
        ops.push(o);
    }
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
/// Keys on a property row (v1's marks): linear diamond, ease circle, hold
/// square; the first and last keys of an animated span are half filled toward
/// it, so a curve's ends read at a glance; selected keys are `accent` with a
/// `panel` keyline. A collapsed layer's summary marks (`summary` = the clip's
/// foreground) are small quiet diamonds inside the clip's foot.
/// A key glyph's look: shape, colour, selected (opacity and keyline), half.
type Glyph<'a> = (&'a str, u32, bool, Option<&'a str>);
fn paint_keys(
    row: &str,
    keys: &[Key],
    summary: Option<u32>,
    y: f64,
    v: Viewport,
    t: &Tokens,
    ops: &mut Vec<Op>,
) {
    let size = t.metrics.keyframe as f64 - if summary.is_some() { 4. } else { 0. };
    let from = v.scroll_time - size / v.pps;
    let to = v.scroll_time + (v.width - v.layer_column + size) / v.pps;
    let start = keys.partition_point(|k| k.seconds < from);
    let end = keys.partition_point(|k| k.seconds <= to);
    let (first, last) = (
        keys.first().map(|k| k.seconds),
        keys.last().map(|k| k.seconds),
    );
    // The glyph drawn last and its device half-pixel: keys are in time
    // order, so an identical glyph on the same half-pixel follows it directly.
    let mut on_top: Option<(i64, Glyph)> = None;
    // `key:["row","key"]`, the row's part escaped once.
    let prefix = format!("key:[{},", serde_json::to_string(row).unwrap_or_default());
    for key in &keys[start..end] {
        let x = v.x(key.seconds);
        let shape = if summary.is_some() {
            "diamond"
        } else {
            match key.interpolation {
                Interpolation::Linear => "diamond",
                Interpolation::Ease => "circle",
                Interpolation::Hold => "square",
            }
        };
        let color = match summary {
            _ if key.selected => t.colors.accent,
            Some(ink) => ink,
            None => t.colors.text,
        };
        let mut id = prefix.clone();
        id.push_str(&serde_json::to_string(&key.id).unwrap_or_default());
        id.push(']');
        let mut o = Op::new(
            id,
            shape,
            Rect::new(x - size / 2., y - size / 2., size, size),
            color,
        );
        o.opacity = match summary {
            _ if key.selected => 1.,
            Some(_) => 0.6,
            None => 0.85,
        };
        // A lone key is solid; the ends of a span are half filled toward it.
        let half = (summary.is_none() && first != last && shape == "diamond")
            .then(|| {
                if Some(key.seconds) == first {
                    Some("right")
                } else if Some(key.seconds) == last {
                    Some("left")
                } else {
                    None
                }
            })
            .flatten();
        o.detail = json!({"keyline":if key.selected {Some(t.colors.panel)}else{None},"keylineWidth":t.metrics.border,"half":half,"seconds":key.seconds});
        let glyph = ((x * 2.).round() as i64, (shape, color, key.selected, half));
        o.covered = on_top == Some(glyph);
        on_top = Some(glyph);
        ops.push(o);
        if key.focused {
            // The ring may cover keys under it: the next one draws again.
            on_top = None;
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
/// A key op's `(row, key)` as `(layer, key)`.
fn on_layer((row, key): (String, String)) -> Option<(String, String)> {
    match crate::v4::parse_row(&row).ok()? {
        crate::RowId::Layer(layer)
        | crate::RowId::Property {
            layer_id: layer, ..
        } => Some((layer, key)),
    }
}
fn key_identity(op: &Op) -> Option<(String, String)> {
    serde_json::from_str(op.id.strip_prefix("key:")?).ok()
}
fn intersection(a: Rect, b: Rect) -> Option<Rect> {
    let (x, y) = (a.x.max(b.x), a.y.max(b.y));
    let (right, bottom) = (
        (a.x + a.width).min(b.x + b.width),
        (a.y + a.height).min(b.y + b.height),
    );
    (right > x && bottom > y).then(|| Rect::new(x, y, right - x, bottom - y))
}
/// A clip as assistive technology sees it. See [`Frame::clip_nodes`].
#[derive(Clone, Debug, PartialEq)]
pub struct ClipNode {
    pub id: String,
    pub name: String,
    pub from: f64,
    pub duration: f64,
    pub selected: bool,
    /// Panel coordinates of its visible body.
    pub bounds: Rect,
}
impl ClipNode {
    /// "Interview, 2 to 7.5 seconds", for VoiceOver and agents.
    pub fn label(&self) -> String {
        let seconds = |s: f64| {
            let s = (s * 100.).round() / 100.;
            if s.fract() == 0. {
                format!("{s:.0}")
            } else {
                format!("{s}")
            }
        };
        let name = if self.name.is_empty() {
            "Clip"
        } else {
            &self.name
        };
        format!(
            "{name}, {} to {} seconds",
            seconds(self.from),
            seconds(self.from + self.duration)
        )
    }
}
fn intersects(a: Rect, b: Rect) -> bool {
    a.x <= b.x + b.width && b.x <= a.x + a.width && a.y <= b.y + b.height && b.y <= a.y + a.height
}
/// What a marquee covers. See [`Scene::marquee`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Marquee {
    /// `(layer, key)`, in row and time order.
    pub keys: Vec<(String, String)>,
    /// Clip ids, in row and time order.
    pub clips: Vec<String>,
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
    /// The tooltip of the op under the point (a clip's Missing effect badge).
    pub fn tooltip_at(&self, x: f64, y: f64) -> Option<String> {
        self.hit_op("clip-missing-effect:", x, y, 0.)
            .and_then(|o| o.detail["tooltip"].as_str())
            .map(str::to_owned)
    }
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
    /// The key under a point as `(layer, key)`: key ids may repeat across
    /// layers, so the row it sits on names its layer.
    pub fn hit_layer_key(&self, x: f64, y: f64, slop: f64) -> Option<(String, String)> {
        self.hit_key(x, y, slop).and_then(on_layer)
    }
    /// One accessible element per painted clip, in paint order: its name and
    /// times, whether it is selected, and its visible body within the tracks.
    pub fn clip_nodes(&self) -> Vec<ClipNode> {
        // One pass indexes each clip's name and selection ring.
        let mut names = HashMap::new();
        let mut rings = std::collections::HashSet::new();
        for o in &self.ops {
            if let Some(id) = o.id.strip_prefix("clip-content:") {
                names.insert(id, o.text.as_deref().unwrap_or_default());
            } else if let Some(id) = o.id.strip_prefix("clip-ring:") {
                rings.insert(id);
            }
        }
        self.ops
            .iter()
            .filter_map(|o| {
                let id = o.id.strip_prefix("clip:")?;
                let mut bounds = match o.clip {
                    Some(c) => intersection(o.bounds, c)?,
                    None => o.bounds,
                };
                let bottom = (bounds.y + bounds.height).min(self.viewport.height);
                bounds.y = bounds.y.max(0.);
                bounds.height = bottom - bounds.y;
                if bounds.width <= 0. || bounds.height <= 0. {
                    return None;
                }
                Some(ClipNode {
                    id: id.to_owned(),
                    name: names.get(id).copied().unwrap_or_default().to_owned(),
                    from: o.detail["from"].as_f64().unwrap_or(0.),
                    duration: o.detail["duration"].as_f64().unwrap_or(0.),
                    selected: rings.contains(id),
                    bounds,
                })
            })
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
                covered: false,
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

#[cfg(test)]
mod key_cover_tests {
    use super::*;

    fn key(id: &str, seconds: f64, selected: bool) -> Key {
        Key {
            id: id.into(),
            seconds,
            selected,
            focused: false,
            interpolation: Interpolation::Linear,
        }
    }

    /// Dense keys: a glyph identical to the one before it on the same device
    /// half-pixel is marked covered (paint leaves it out); every key keeps
    /// its op for hit testing and marquee selection.
    #[test]
    fn identical_keys_on_one_half_pixel_are_covered_not_dropped() {
        let t = Tokens::new(pm_design::Scheme::Dark, Default::default());
        let v = Viewport {
            width: 1000.,
            height: 300.,
            scroll_y: 0.,
            scroll_time: 0.,
            pps: 10.,
            layer_column: 200.,
        };
        // A span's ends are half filled (other glyphs); between them, keys
        // a tenth of a pixel apart at 10 px/s.
        let keys = vec![
            key("start", 0., false),
            key("a", 1., false),
            key("b", 1.01, false),
            key("c", 1.02, true),
            key("d", 1.024, false),
            key("e", 1.2, false),
            key("f", 1.21, false),
            key("end", 9., false),
        ];
        let mut ops = vec![];
        paint_keys("L1", &keys, None, 50., v, &t, &mut ops);
        let covered: Vec<bool> = ops.iter().map(|o| o.covered).collect();
        // b repeats a on its half-pixel; c is selected (another glyph) and d
        // is on top of it again; f repeats e.
        assert_eq!(
            covered,
            [false, false, true, false, false, false, true, false]
        );
        assert_eq!(ops.len(), keys.len());
        assert_eq!(ops[1].id, format!("key:{}", json!(["L1", "a"])));
    }
}
