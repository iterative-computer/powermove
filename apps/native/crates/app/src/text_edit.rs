//! On-canvas text editing (the shipped viewer's `text-editor.ts`).
//!
//! The engine keeps painting the layer while it is edited, so what the user
//! sees is exactly what renders and exports. An invisible `NSTextView` (P-TEXT
//! canvas slot) owns typing, IME composition, dictation, emoji, spelling and
//! the clipboard; every change is written through one kernel gesture, so the
//! whole session is one undo step. Frame, caret, selection and misspellings
//! are drawn here from the engine's own caret geometry (`textLayout`).
//!
//! Contract carried over from the shipped app:
//! - the Text tool's click makes point text, a drag at least 8 px wide makes
//!   paragraph text with automatic height; both edit at once;
//! - Escape, Command-Return and clicking elsewhere commit; nothing typed is
//!   ever discarded by leaving;
//! - an empty result removes the layer (a fresh one leaves no history);
//! - afterwards the tool returns to Select;
//! - clicks inside the text place the caret, drags select, double-click picks
//!   a word, triple-click a paragraph;
//! - Command-B / I, Command-Shift-> / < (Option: by ten), Command-Option-L/T/R;
//! - Return with one text layer selected, or a double-click on one with the
//!   Select tool, edits it.
use super::{GizmoSurface, inverse_point, transform};
use gpui::{prelude::*, *};
use gpui_macos::native_text::TextEvent;
use pm_kernel::{
    Call, Caller, Command, ContentCommand, GestureCall, LayerCommand, PropertyCommand, Reply,
    Request, SelectionCall, TextCommand, WriteBody,
    commands::{
        layers::{AddLayer, LayerPatch},
        properties::SetProperty,
    },
};
use pm_model::{Map, Value};
use std::{collections::VecDeque, rc::Rc};

/// The native editor a session types into. The host makes it; tests fake it.
pub trait TextInput {
    fn value(&self) -> String;
    fn selection(&self) -> (usize, usize);
    fn set_selection(&self, start: usize, end: usize);
    /// An outside change while editing; never interrupts a composition.
    fn replace_value(&self, value: &str);
    fn composing(&self) -> bool;
    fn focused(&self) -> bool;
    fn focus(&self);
    fn set_font(&self, family: &str, points: f64);
    fn misspelled(&self) -> Vec<(usize, usize)>;
    fn context_menu(&self, word: (usize, usize));
    /// Window-point placement over the text, clipped to the viewer.
    fn place(&self, bounds: Bounds<Pixels>, clip: Bounds<Pixels>);
    /// Where the system insertion point stands, clipped; `None` hides it.
    fn stage_caret(&self, caret: Option<(Bounds<Pixels>, Bounds<Pixels>)>);
    /// Where input method candidates open, window points.
    fn stage_input_rect(&self, rect: Option<Bounds<Pixels>>);
    /// The composing range an input method holds.
    fn marked_range(&self) -> Option<(usize, usize)>;
    fn take_events(&self) -> Vec<TextEvent>;
    fn on_event(&self, wake: Rc<dyn Fn()>);
}

/// A press this close to the frame (display points) still edits.
const HIT_SLACK: f64 = 6.;
/// Narrower Text tool drags make point text.
const PARAGRAPH_MIN: f64 = 8.;

// ── caret geometry (text-geometry.ts) ─────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Line {
    /// UTF-16 length of the line's text.
    pub len: usize,
    pub start: usize,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    /// UTF-16 index and x of each caret stop.
    pub boundaries: Vec<(usize, f64)>,
}
/// raster.ts `textCaretLayout`, in the text layer's local space.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    pub lines: Vec<Line>,
    pub line_height: f64,
    pub size: f64,
    pub length: usize,
    pub align: Align,
    pub box_width: f64,
    pub box_height: f64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Caret {
    pub x: f64,
    pub top: f64,
    pub height: f64,
    pub line: usize,
}
fn num(v: Option<&Value>) -> f64 {
    v.and_then(Value::as_f64).unwrap_or(0.)
}
impl Layout {
    pub fn parse(v: &Value) -> Option<Self> {
        let lines = v
            .get("lines")?
            .as_array()?
            .iter()
            .map(|l| Line {
                len: l
                    .get("text")
                    .and_then(Value::as_str)
                    .map_or(0, |t| t.encode_utf16().count()),
                start: num(l.get("start")) as usize,
                x: num(l.get("x")),
                y: num(l.get("y")),
                width: num(l.get("width")),
                boundaries: l
                    .get("boundaries")
                    .and_then(Value::as_array)
                    .map(|b| {
                        b.iter()
                            .map(|b| (num(b.get("index")) as usize, num(b.get("x"))))
                            .collect()
                    })
                    .unwrap_or_default(),
            })
            .collect();
        Some(Self {
            lines,
            line_height: num(v.get("lineHeight")),
            size: num(v.get("size")),
            length: num(v.get("length")) as usize,
            align: match v.get("align").and_then(Value::as_str) {
                Some("center") => Align::Center,
                Some("right") => Align::Right,
                _ => Align::Left,
            },
            box_width: num(v.get("boxWidth")),
            box_height: num(v.get("boxHeight")),
        })
    }
    fn range_end(&self, row: usize) -> usize {
        self.lines
            .get(row + 1)
            .map_or(self.length + 1, |next| next.start)
    }
    /// The display line owning an index; whitespace dropped at a wrap belongs
    /// to the line before it.
    pub fn line_for(&self, index: usize) -> usize {
        let index = index.min(self.length);
        (0..self.lines.len())
            .find(|&row| index < self.range_end(row))
            .unwrap_or(self.lines.len().saturating_sub(1))
    }
    /// One line pitch, centred on the glyph box so lines abut.
    fn line_box(&self, line: &Line) -> (f64, f64) {
        let height = self.size.max(self.line_height);
        (line.y + self.size / 2. - height / 2., height)
    }
    fn boundary_x(line: &Line, index: usize) -> f64 {
        let b = &line.boundaries;
        let (Some(first), Some(last)) = (b.first(), b.last()) else {
            return line.x;
        };
        if index <= first.0 {
            return first.1;
        }
        if index >= last.0 {
            return last.1;
        }
        // Inside a grapheme cluster: the nearer edge.
        let mut previous = *first;
        for &boundary in b {
            if boundary.0 == index {
                return boundary.1;
            }
            if boundary.0 > index {
                return if index - previous.0 < boundary.0 - index {
                    previous.1
                } else {
                    boundary.1
                };
            }
            previous = boundary;
        }
        last.1
    }
    pub fn caret(&self, index: usize) -> Caret {
        if self.lines.is_empty() {
            return Caret {
                x: 0.,
                top: 0.,
                height: self.size,
                line: 0,
            };
        }
        let row = self.line_for(index);
        let line = &self.lines[row];
        let (top, height) = self.line_box(line);
        Caret {
            x: Self::boundary_x(line, index),
            top,
            height,
            line: row,
        }
    }
    /// The nearest index to a local point, clamped to the lines.
    pub fn index_at(&self, x: f64, y: f64) -> usize {
        if self.lines.is_empty() {
            return 0;
        }
        let row = (0..self.lines.len())
            .find(|&row| {
                let (top, height) = self.line_box(&self.lines[row]);
                y < top + height
            })
            .unwrap_or(self.lines.len() - 1);
        let line = &self.lines[row];
        let mut best = line
            .boundaries
            .first()
            .copied()
            .unwrap_or((line.start, line.x));
        for &boundary in &line.boundaries {
            if (boundary.1 - x).abs() < (best.1 - x).abs() {
                best = boundary;
            }
        }
        best.0.min(self.length)
    }
    /// One highlight per display line; a line the range continues past
    /// reaches a little into its trailing space.
    pub fn selection_rects(&self, start: usize, end: usize) -> Vec<Rect> {
        let (from, to) = (start.min(end), start.max(end).min(self.length));
        if to <= from || self.lines.is_empty() {
            return vec![];
        }
        let (first, last) = (self.line_for(from), self.line_for(to));
        (first..=last)
            .map(|row| {
                let line = &self.lines[row];
                let (top, height) = self.line_box(line);
                let line_end = line.start + line.len;
                let x0 = Self::boundary_x(line, if row == first { from } else { line.start });
                let x1 = if row < last {
                    Self::boundary_x(line, line_end) + self.size * 0.25
                } else {
                    Self::boundary_x(line, to.min(line_end))
                };
                Rect {
                    x: x0.min(x1),
                    y: top,
                    w: (x1 - x0).abs(),
                    h: height,
                }
            })
            .collect()
    }
    /// One display line up or down keeping the sticky column; `None` at the
    /// first or last line.
    pub fn vertical(&self, index: usize, down: bool, sticky: f64) -> Option<usize> {
        let current = self.caret(index);
        let row = if down {
            current.line + 1
        } else {
            current.line.checked_sub(1)?
        };
        let line = self.lines.get(row)?;
        let (top, height) = self.line_box(line);
        Some(self.index_at(sticky, top + height / 2.))
    }
    /// The authored box for paragraph text, else the lines' union.
    pub fn frame(&self) -> Rect {
        let (Some(first), Some(last)) = (self.lines.first(), self.lines.last()) else {
            return Rect {
                x: 0.,
                y: 0.,
                w: 0.,
                h: self.size,
            };
        };
        let (top, _) = self.line_box(first);
        let (last_top, last_height) = self.line_box(last);
        let bottom = last_top + last_height;
        if self.box_width > 0. {
            let x = match self.align {
                Align::Center => -self.box_width / 2.,
                Align::Right => -self.box_width,
                Align::Left => 0.,
            };
            let h = if self.box_height > 0. {
                self.box_height.max(bottom - top)
            } else {
                bottom - top
            };
            return Rect {
                x,
                y: top,
                w: self.box_width,
                h,
            };
        }
        let x0 = self.lines.iter().map(|l| l.x).fold(f64::INFINITY, f64::min);
        let x1 = self
            .lines
            .iter()
            .map(|l| l.x + l.width)
            .fold(f64::NEG_INFINITY, f64::max);
        let (x0, x1) = if x0.is_finite() && x1 > x0 {
            (x0, x1)
        } else {
            (first.x, first.x)
        };
        Rect {
            x: x0,
            y: top,
            w: x1 - x0,
            h: bottom - top,
        }
    }
}
fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}
fn unit_char(u: u16) -> char {
    char::from_u32(u as u32).unwrap_or('a')
}
/// Double-click: a word, a whitespace run or a punctuation run.
pub fn word_range(text: &str, index: usize) -> (usize, usize) {
    let t = units(text);
    if t.is_empty() {
        return (0, 0);
    }
    let at = index.min(t.len() - 1);
    let space = |u: u16| unit_char(u).is_whitespace();
    let word = |u: u16| {
        matches!(unit_char(u), c if c.is_alphanumeric() || c == '_' || c == '\'' || c == '’')
            || (0xD800..0xE000).contains(&u)
    };
    let probe = t[at];
    let kind = |u: u16| {
        if space(probe) {
            space(u)
        } else if word(probe) {
            word(u)
        } else {
            !space(u) && !word(u)
        }
    };
    let newline = u16::from(b'\n');
    let (mut start, mut end) = (at, at + 1);
    while start > 0 && kind(t[start - 1]) && t[start - 1] != newline {
        start -= 1;
    }
    while end < t.len() && kind(t[end]) && t[end] != newline {
        end += 1;
    }
    (start, end)
}
/// Triple-click: the paragraph around an index.
pub fn paragraph_range(text: &str, index: usize) -> (usize, usize) {
    let t = units(text);
    let at = index.min(t.len());
    let newline = u16::from(b'\n');
    let start = t[..at]
        .iter()
        .rposition(|&u| u == newline)
        .map_or(0, |i| i + 1);
    let end = t[at..]
        .iter()
        .position(|&u| u == newline)
        .map_or(t.len(), |i| at + i);
    (start, end)
}
/// A fresh layer is named after its first line, up to 40 characters.
pub fn first_line(text: &str) -> String {
    let line = text
        .split('\n')
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    if line.chars().count() > 40 {
        let cut: String = line.chars().take(39).collect();
        format!("{}…", cut.trim_end())
    } else {
        line.to_owned()
    }
}

// ── session ───────────────────────────────────────────────────────────────

/// The edited layer as the kernel has it, read off the main thread.
#[derive(Clone, Debug)]
pub struct TextPose {
    pub layout: Layout,
    pub matrix: [f64; 6],
    pub text: String,
    pub font: String,
    pub weight: f64,
    pub size: f64,
    pub italic: bool,
    pub name: String,
}
/// The layer's stored text (`d.text`).
fn model_text(p: &pm_kernel::Published, id: &str) -> String {
    p.snapshot
        .root()
        .and_then(|root| {
            let at = pm_kernel::model::find_layer(root, id)?;
            let text = pm_kernel::model::layer_at(root, &at)?
                .get("d")?
                .get("text")?;
            text.to_value().as_str().map(str::to_owned)
        })
        .unwrap_or_default()
}
pub fn read_text(p: &pm_kernel::Published, id: &str, time: f64) -> Option<TextPose> {
    let root = p.snapshot.root()?;
    let layer = pm_kernel::model::find_layer(root, id)
        .and_then(|at| pm_kernel::model::layer_at(root, &at))?;
    let name = layer
        .get("name")
        .and_then(|n| n.to_value().as_str().map(str::to_owned))
        .unwrap_or_default();
    // Fonts come from the registry when the layout needs them; the caret
    // follows typography anchoring as the painted glyphs do.
    let (layout, matrix, values) = crate::text_host::query(p, |q| {
        let layout = q.text_layout(None, id, time)?;
        let mut matrix = q.world_matrix(None, id, time)?;
        let [x, y] = q.text_font_offset(None, id, time).unwrap_or([0., 0.]);
        matrix[4] += matrix[0] * x + matrix[2] * y;
        matrix[5] += matrix[1] * x + matrix[3] * y;
        let values: Vec<Option<Value>> = ["c.text", "c.font", "c.weight", "c.size", "c.italic"]
            .iter()
            .map(|key| q.evaluate(None, id, key, time).ok())
            .collect();
        Ok((layout, matrix, values))
    })
    .ok()?;
    let layout = Layout::parse(&layout)?;
    let value = |key: &str| {
        let at = ["c.text", "c.font", "c.weight", "c.size", "c.italic"]
            .iter()
            .position(|k| *k == key)?;
        values[at].clone()
    };
    let text = value("c.text")
        .and_then(|v| v.as_str().map(str::to_owned))
        .or_else(|| {
            layer
                .get("d")
                .and_then(|d| d.get("text"))
                .and_then(|t| t.to_value().as_str().map(str::to_owned))
        })
        .unwrap_or_default();
    let font = value("c.font")
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    Some(TextPose {
        layout,
        matrix,
        text,
        font,
        weight: value("c.weight").and_then(|v| v.as_f64()).unwrap_or(400.),
        size: value("c.size").and_then(|v| v.as_f64()).unwrap_or(16.),
        italic: value("c.italic").is_some_and(|v| v == Value::Bool(true)),
        name,
    })
}

/// Where a session starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Entry {
    /// Caret at a composition point (Text tool click on existing text).
    Caret([f64; 2]),
    /// Everything selected (double-click with the Select tool).
    All,
    /// Caret at the end (Return).
    End,
}
enum Job {
    Begin(&'static str),
    /// The fresh layer (and its paragraph box), inside the gesture.
    Add {
        at: [f64; 2],
        width: Option<f64>,
        time: f64,
    },
    Step {
        commands: Vec<Command>,
        replace: bool,
        text: bool,
    },
    Select(String),
    Finish {
        text: String,
        fresh: bool,
        rename: bool,
    },
}
pub struct TextSession {
    pub layer: String,
    fresh: bool,
    time: f64,
    input: Option<Rc<dyn TextInput>>,
    pub pose: Option<TextPose>,
    entry: Option<Entry>,
    jobs: VecDeque<Job>,
    busy: bool,
    gesture: Option<u64>,
    /// The newest queued or written step is a text step a later one may supersede.
    text_step: bool,
    written: String,
    selection: (usize, usize),
    /// Drag-select anchor while the press is held.
    dragging: Option<usize>,
    sticky: Option<f64>,
    misspelled: Vec<(usize, usize)>,
    /// A press inside the text took first responder for a moment; the editor
    /// asked for it back, so the blur it caused does not end the session.
    refocus: bool,
    finishing: bool,
    reading: u64,
}
impl TextSession {
    pub fn active(&self) -> bool {
        !self.finishing
    }
    /// Composition point → the text's local space.
    fn local(&self, at: [f64; 2]) -> Option<[f64; 2]> {
        inverse_point(&self.pose.as_ref()?.matrix, at)
    }
    fn index_at(&self, at: [f64; 2]) -> Option<usize> {
        let local = self.local(at)?;
        Some(self.pose.as_ref()?.layout.index_at(local[0], local[1]))
    }
    /// Within the frame, with a display-sized slack.
    pub fn contains(&self, at: [f64; 2], scale: f64) -> bool {
        let (Some(pose), Some(local)) = (&self.pose, self.local(at)) else {
            return false;
        };
        let f = pose.layout.frame();
        let linear = (pose.matrix[0].hypot(pose.matrix[1])).max(1e-6);
        let slack = HIT_SLACK / scale.max(0.02) / linear;
        local[0] >= f.x - slack
            && local[0] <= f.x + f.w + slack
            && local[1] >= f.y - slack
            && local[1] <= f.y + f.h + slack
    }
}

impl GizmoSurface {
    pub fn text_session(&self) -> Option<&TextSession> {
        self.text.as_ref().filter(|s| s.active())
    }
    /// Begins editing `layer` (existing text). The pose is read first so the
    /// editor starts with the layer's own text.
    pub fn edit_text(
        &mut self,
        layer: String,
        entry: Entry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.text.is_some() {
            return;
        }
        let time = self.host.time(cx);
        self.host.pause(cx);
        let (kernel, project) = (self.kernel.clone(), self.project.clone());
        let handle = window.window_handle();
        let id = layer.clone();
        cx.spawn(async move |this, cx| {
            let pose = cx
                .background_executor()
                .spawn(async move {
                    let p = kernel.published(&project)?;
                    let root = p.snapshot.root()?;
                    let info = pm_kernel::model::list_layers(root, None)
                        .into_iter()
                        .find(|l| l.id == id)?;
                    if info.kind != "text" || info.effectively_locked {
                        return None;
                    }
                    // Without fonts there is no caret geometry yet, but the
                    // text still edits; the frame appears once it measures.
                    let text = model_text(&p, &id);
                    Some((read_text(&p, &id, time), text))
                })
                .await;
            let Some((pose, text)) = pose else { return };
            let _ = handle.update(cx, |_, window, cx| {
                let _ = this.update(cx, |s, cx| {
                    s.open_session(layer, false, time, pose, text, Some(entry), window, cx)
                });
            });
        })
        .detach();
    }
    /// The Text tool's release on empty canvas: a new layer inside the
    /// editing gesture, so an empty result leaves nothing behind.
    pub fn new_text(
        &mut self,
        at: [f64; 2],
        width: Option<f64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.text.is_some() {
            return;
        }
        let time = self.host.time(cx);
        self.host.pause(cx);
        let id = pm_kernel::new_id("L");
        self.open_session(
            id.clone(),
            true,
            time,
            None,
            String::new(),
            None,
            window,
            cx,
        );
        if let Some(s) = &mut self.text {
            s.jobs.push_back(Job::Begin("Add Text"));
            // Planned off main: the alignment it inherits is evaluated.
            s.jobs.push_back(Job::Add { at, width, time });
            s.jobs.push_back(Job::Select(id));
        }
        self.text_pump(cx);
    }
    #[allow(clippy::too_many_arguments)]
    fn open_session(
        &mut self,
        layer: String,
        fresh: bool,
        time: f64,
        pose: Option<TextPose>,
        initial: String,
        entry: Option<Entry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.text.is_some() {
            return;
        }
        let input = self.host.text_input(&initial, window, cx);
        if let Some(input) = &input {
            let (tx, rx) = async_channel::unbounded::<()>();
            input.on_event(Rc::new(move || {
                let _ = tx.try_send(());
            }));
            cx.spawn(async move |this, cx| {
                while rx.recv().await.is_ok() {
                    if this.update(cx, |s, cx| s.text_events(cx)).is_err() {
                        break;
                    }
                }
            })
            .detach();
            input.focus();
        }
        let label = if fresh { None } else { Some("Edit Text") };
        let mut session = TextSession {
            layer: layer.clone(),
            fresh,
            time,
            input,
            pose: None,
            entry,
            jobs: VecDeque::new(),
            busy: false,
            gesture: None,
            text_step: false,
            written: initial,
            selection: (0, 0),
            dragging: None,
            sticky: None,
            misspelled: vec![],
            refocus: false,
            finishing: false,
            reading: 0,
        };
        if let Some(label) = label {
            session.jobs.push_back(Job::Begin(label));
            session.jobs.push_back(Job::Select(layer));
        }
        self.text = Some(session);
        if let Some(pose) = pose {
            self.adopt_text_pose(pose, cx);
        }
        // The gizmo box gives way to the text frame.
        self.draw(cx);
        self.text_pump(cx);
        cx.notify();
    }
    fn adopt_text_pose(&mut self, pose: TextPose, cx: &mut Context<Self>) {
        let Some(s) = self.text.as_mut() else { return };
        let entry = s.entry.take();
        s.pose = Some(pose.clone());
        if let Some(input) = s.input.clone() {
            // Outside edits (undo, the inspector) flow back into the editor.
            let pending = s.busy || !s.jobs.is_empty();
            if !pending && !input.composing() && pose.text != input.value() {
                input.replace_value(&pose.text);
                s.written = pose.text.clone();
            }
            match entry {
                Some(Entry::Caret(at)) => {
                    let index = s.index_at(at).unwrap_or(pose.layout.length);
                    input.set_selection(index, index);
                }
                Some(Entry::All) => input.set_selection(0, usize::MAX),
                Some(Entry::End) => input.set_selection(usize::MAX, usize::MAX),
                None => {}
            }
            s.selection = input.selection();
            s.misspelled = input.misspelled();
        }
        cx.notify();
    }
    /// Reads the edited layer after a write or a publication.
    pub(super) fn text_reread(&mut self, cx: &mut Context<Self>) {
        let Some(s) = self.text.as_mut().filter(|s| s.active()) else {
            return;
        };
        s.reading += 1;
        let ticket = s.reading;
        let (kernel, project, id, time) = (
            self.kernel.clone(),
            self.project.clone(),
            s.layer.clone(),
            s.time,
        );
        cx.spawn(async move |this, cx| {
            let pose = cx
                .background_executor()
                .spawn(async move { read_text(kernel.published(&project)?.as_ref(), &id, time) })
                .await;
            let _ = this.update(cx, |s, cx| {
                if let (Some(pose), Some(true)) =
                    (pose, s.text.as_ref().map(|t| t.reading == ticket))
                {
                    s.adopt_text_pose(pose, cx);
                }
            });
        })
        .detach();
    }
    fn text_events(&mut self, cx: &mut Context<Self>) {
        let Some(input) = self.text.as_ref().and_then(|s| s.input.clone()) else {
            return;
        };
        for event in input.take_events() {
            let Some(s) = self.text.as_mut().filter(|s| s.active()) else {
                return;
            };
            match event {
                TextEvent::Changed(value) => {
                    let value = value.replace('\r', "");
                    if value != s.written {
                        s.written = value.clone();
                        s.push_text(value);
                        s.sticky = None;
                        s.misspelled = input.misspelled();
                        self.text_pump(cx);
                    }
                }
                TextEvent::Selection { start, end } => {
                    s.selection = (start, end);
                    cx.notify();
                }
                TextEvent::Key(key) => self.text_key(key, cx),
                TextEvent::Blurred => {
                    if std::mem::take(&mut s.refocus) {
                        continue;
                    }
                    if !input.focused() {
                        self.finish_text(false, cx);
                    }
                }
                TextEvent::Committed(_) | TextEvent::Cancelled | TextEvent::Submitted(_) => {}
            }
        }
    }
    fn text_key(&mut self, key: &str, cx: &mut Context<Self>) {
        if key == "commit" {
            self.finish_text(false, cx);
            return;
        }
        let Some(s) = self.text.as_mut() else { return };
        let Some(pose) = s.pose.clone() else { return };
        let (id, time) = (s.layer.clone(), s.time);
        let property = |key: &str, value: f64| -> Command {
            let mut set = SetProperty::new(&id, key, Value::Number(value), time);
            set.preserve_hand_edits = false;
            PropertyCommand::Set(set).into()
        };
        let content = |key: &str, value: Value| -> Command {
            let mut patch = Map::new();
            patch.insert(key, value);
            ContentCommand::Set {
                layer_id: id.clone(),
                patch,
                time,
            }
            .into()
        };
        let step = match key {
            "bold" => property("c.weight", if pose.weight >= 600. { 400. } else { 700. }),
            "italic" => content("italic", Value::Bool(!pose.italic)),
            "larger" | "smaller" | "larger10" | "smaller10" => {
                let step = match key {
                    "larger" => 1.,
                    "smaller" => -1.,
                    "larger10" => 10.,
                    _ => -10.,
                };
                property("c.size", (pose.size + step).round().max(4.))
            }
            "alignLeft" => content("align", Value::from("left")),
            "alignCenter" => content("align", Value::from("center")),
            "alignRight" => content("align", Value::from("right")),
            "up" | "down" | "shiftUp" | "shiftDown" => {
                let Some(input) = s.input.clone() else { return };
                let (start, end) = s.selection;
                let down = key.ends_with("own");
                let extend = key.starts_with("shift");
                // The anchor is the end the caret did not move from.
                let (anchor, focus) = if s.dragging.is_none() && extend && start != end {
                    if down { (start, end) } else { (end, start) }
                } else {
                    (start, end)
                };
                let focus = if extend {
                    focus
                } else if down {
                    end
                } else {
                    start
                };
                let from = pose.layout.caret(focus);
                let sticky = *s.sticky.get_or_insert(from.x);
                let next = pose
                    .layout
                    .vertical(focus, down, sticky)
                    .unwrap_or(if down { pose.layout.length } else { 0 });
                if extend {
                    input.set_selection(anchor.min(next), anchor.max(next));
                } else if start != end {
                    let at = if down { end } else { start };
                    input.set_selection(at, at);
                } else {
                    input.set_selection(next, next);
                }
                return;
            }
            _ => return,
        };
        s.jobs.push_back(Job::Step {
            commands: vec![step],
            replace: false,
            text: false,
        });
        s.text_step = false;
        self.text_pump(cx);
    }
    /// Commits the session (`keep_tool`: a toolbar choice already set the tool).
    pub fn finish_text(&mut self, keep_tool: bool, cx: &mut Context<Self>) -> bool {
        let Some(s) = self.text.as_mut().filter(|s| s.active()) else {
            return false;
        };
        s.finishing = true;
        if let Some(input) = s.input.take() {
            let value = input.value().replace('\r', "");
            if value != s.written {
                s.written = value.clone();
                s.push_text(value);
            }
        }
        let rename = s.fresh && s.pose.as_ref().is_none_or(|p| p.name == "Text");
        s.jobs.push_back(Job::Finish {
            text: s.written.clone(),
            fresh: s.fresh,
            rename,
        });
        if !keep_tool && self.host.tool(cx) == super::Tool::Text {
            self.host.command("toolSelect", cx);
        }
        self.text_pump(cx);
        self.refresh(cx);
        cx.notify();
        true
    }
    /// A press while editing. `true` when the text took it.
    pub(super) fn text_press(&mut self, event: &MouseDownEvent, at: [f64; 2], scale: f64) -> bool {
        let Some(s) = self.text.as_mut().filter(|s| s.active()) else {
            return false;
        };
        if !s.contains(at, scale) {
            return false;
        }
        let (Some(input), Some(index)) = (s.input.clone(), s.index_at(at)) else {
            return true;
        };
        s.refocus = !input.focused();
        input.focus();
        let text = input.value();
        s.sticky = None;
        match event.click_count {
            n if n >= 3 => {
                let (a, b) = paragraph_range(&text, index);
                input.set_selection(a, b);
            }
            2 => {
                let (a, b) = word_range(&text, index);
                input.set_selection(a, b);
            }
            _ => {
                let (start, end) = input.selection();
                let anchor = if event.modifiers.shift {
                    if index < start { end } else { start }
                } else {
                    index
                };
                input.set_selection(anchor.min(index), anchor.max(index));
                s.dragging = Some(anchor);
            }
        }
        true
    }
    pub(super) fn text_drag(&mut self, at: [f64; 2]) -> bool {
        let Some(s) = self.text.as_mut().filter(|s| s.active()) else {
            return false;
        };
        let Some(anchor) = s.dragging else {
            return false;
        };
        if let (Some(index), Some(input)) = (s.index_at(at), s.input.clone()) {
            input.set_selection(anchor.min(index), anchor.max(index));
        }
        true
    }
    pub(super) fn text_release(&mut self) -> bool {
        self.text.as_mut().and_then(|s| s.dragging.take()).is_some()
    }
    /// Secondary press inside the text: the editor's own menu on the word.
    pub(super) fn text_context(&mut self, at: [f64; 2], scale: f64) -> bool {
        let Some(s) = self.text.as_mut().filter(|s| s.active()) else {
            return false;
        };
        if !s.contains(at, scale) {
            return false;
        }
        if let (Some(input), Some(index)) = (s.input.clone(), s.index_at(at)) {
            let (start, end) = s.selection;
            let word = if start != end && (start..=end).contains(&index) {
                (start, end)
            } else {
                word_range(&input.value(), index)
            };
            s.refocus = !input.focused();
            input.focus();
            input.context_menu(word);
        }
        true
    }
    fn text_pump(&mut self, cx: &mut Context<Self>) {
        let Some(s) = self.text.as_mut() else { return };
        if s.busy {
            return;
        }
        let Some(job) = s.jobs.pop_front() else {
            if s.finishing {
                self.text = None;
                cx.notify();
            }
            return;
        };
        s.busy = true;
        let finishing_job = matches!(job, Job::Finish { .. });
        let (kernel, project, gesture, layer) = (
            self.kernel.clone(),
            self.project.clone(),
            s.gesture,
            s.layer.clone(),
        );
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { run_job(&kernel, &project, gesture, &layer, job).await })
                .await;
            let _ = this.update(cx, |g, cx| {
                let Some(s) = g.text.as_mut() else { return };
                s.busy = false;
                match result {
                    Ok(Some(id)) => s.gesture = Some(id),
                    Ok(None) => {}
                    Err(error) => {
                        // A refused step ends the session rather than typing
                        // into nothing.
                        g.host.report(error, cx);
                        if !finishing_job {
                            g.finish_text(true, cx);
                        }
                    }
                }
                if !finishing_job {
                    g.text_reread(cx);
                }
                g.text_pump(cx);
            });
        })
        .detach();
    }
    /// Frame, selection or caret, and misspellings, in window points; then
    /// the editor is placed over the text for input method candidates.
    pub(super) fn text_paint(&self, frame: super::ViewFrame, window: &mut Window) {
        let Some(s) = self.text.as_ref().filter(|s| s.active()) else {
            return;
        };
        let Some(pose) = &s.pose else { return };
        let to_window = |p: [f64; 2]| {
            let c = transform(&pose.matrix, p);
            point(
                frame.origin.x + px((c[0] * frame.scale) as f32),
                frame.origin.y + px((c[1] * frame.scale) as f32),
            )
        };
        let quad = |r: Rect| {
            [
                to_window([r.x, r.y]),
                to_window([r.x + r.w, r.y]),
                to_window([r.x + r.w, r.y + r.h]),
                to_window([r.x, r.y + r.h]),
            ]
        };
        let fill = |corners: [Point<Pixels>; 4], color: Rgba, window: &mut Window| {
            let mut path = PathBuilder::fill();
            path.move_to(corners[0]);
            for c in &corners[1..] {
                path.line_to(*c);
            }
            path.close();
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        };
        let t = &self.tokens;
        let layout = &pose.layout;
        let linear = pose.matrix[0].hypot(pose.matrix[1]).max(1e-6);
        let unit = 1. / (frame.scale * linear);
        // editing.md: the bounds as a 1 pt secondary dashed line, selection in
        // accent at 30 %, the system's own insertion point.
        let corners = quad(layout.frame());
        let mut outline = PathBuilder::stroke(px(1.)).dash_array(&[px(4.), px(3.)]);
        outline.add_polygon(&corners, true);
        if let Ok(path) = outline.build() {
            window.paint_path(path, rgb(t.colors.secondary));
        }
        let (start, end) = s.selection;
        let wash = rgba((t.colors.accent << 8) | 0x4D);
        for r in layout.selection_rects(start, end) {
            fill(quad(r), wash, window);
        }
        let underline = |a: usize, b: usize, color: Rgba, window: &mut Window| {
            for r in layout.selection_rects(a, b) {
                let rule = Rect {
                    x: r.x,
                    y: r.y + r.h - 2. * unit,
                    w: r.w,
                    h: unit,
                };
                fill(quad(rule), color, window);
            }
        };
        for &(a, b) in &s.misspelled {
            underline(a, b, rgb(t.colors.danger), window);
        }
        if let Some((a, b)) = s.input.as_ref().and_then(|i| i.marked_range()) {
            underline(a, b, rgb(t.colors.text), window);
        }
        let window_box = |points: &[Point<Pixels>]| {
            let xs = points.iter().map(|c| f32::from(c.x));
            let ys = points.iter().map(|c| f32::from(c.y));
            let (x0, x1) = (
                xs.clone().fold(f32::MAX, f32::min),
                xs.fold(f32::MIN, f32::max),
            );
            let (y0, y1) = (
                ys.clone().fold(f32::MAX, f32::min),
                ys.fold(f32::MIN, f32::max),
            );
            Bounds::new(
                point(px(x0), px(y0)),
                size(px((x1 - x0).max(1.)), px((y1 - y0).max(1.))),
            )
        };
        if let Some(input) = &s.input {
            let stem = |index: usize| {
                let c = layout.caret(index);
                (
                    to_window([c.x, c.top]),
                    to_window([c.x, c.top + c.height]),
                    c,
                )
            };
            // Candidates open at the composition's start, else the caret.
            let (top, bottom, _) = stem(input.marked_range().map_or(start, |m| m.0));
            input.stage_input_rect(Some(window_box(&[top, bottom])));
            let (top, bottom, c) = stem(start);
            // The system insertion point stands upright; text at an angle gets
            // its caret drawn along the glyphs instead.
            let upright = (f32::from(top.x) - f32::from(bottom.x)).abs() < 0.5;
            let caret = (start == end && upright).then(|| {
                let b = window_box(&[top, bottom]);
                // The indicator draws its own width inside a slim frame.
                Bounds::new(
                    point(b.center().x - px(1.5), b.origin.y),
                    size(px(3.), b.size.height),
                )
            });
            input.stage_caret(caret.map(|c| (c, self.bounds)));
            if start == end && !upright {
                let width = 1.5 * unit;
                let rect = Rect {
                    x: c.x - width / 2.,
                    y: c.top,
                    w: width,
                    h: c.height,
                };
                fill(quad(rect), rgb(t.colors.accent), window);
            }
        }
        if let Some(input) = &s.input {
            input.place(window_box(&corners), self.bounds);
            input.set_font(&pose.font, layout.size * frame.scale * linear);
        }
    }
}
/// The new layer and, for a paragraph, its box (viewer.ts `startText`).
fn add_text(
    published: &pm_kernel::Published,
    id: &str,
    at: [f64; 2],
    width: Option<f64>,
    time: f64,
) -> Vec<Command> {
    // New text keeps the alignment of the selected or the newest text layer.
    let align = published.snapshot.root().and_then(|root| {
        let layers = pm_kernel::model::list_layers(root, None);
        let texts: Vec<_> = layers.iter().filter(|l| l.kind == "text").collect();
        let source = published
            .selection
            .layers()
            .iter()
            .find_map(|s| texts.iter().find(|l| &l.id == s))
            .or(texts.last())?;
        let value =
            crate::text_host::query(published, |q| q.evaluate(None, &source.id, "c.align", time))
                .ok()?;
        value.as_str().map(str::to_owned)
    });
    let align = match align.as_deref() {
        Some("center") => "center",
        Some("right") => "right",
        _ => "left",
    };
    let mut content = Map::new();
    content.insert("text", Value::from(""));
    content.insert("align", Value::from(align));
    let mut properties = Map::new();
    properties.insert("position.x", Value::Number(at[0]));
    properties.insert("position.y", Value::Number(at[1]));
    let add = AddLayer {
        id: id.into(),
        layer_type: "text".into(),
        comp: None,
        name: Some("Text".into()),
        from: None,
        duration: None,
        content,
        properties,
        color: None,
        index: None,
        parent: None,
        blend: None,
        motion_blur: None,
        visible: None,
        shy: None,
        collapsed: None,
        time,
    };
    let mut commands: Vec<Command> = vec![LayerCommand::Add(Box::new(add)).into()];
    if let Some(width) = width {
        commands.push(
            TextCommand::SetBox {
                layer_id: id.into(),
                width: Some((width * 1000.).round() / 1000.),
                height: None,
                time,
            }
            .into(),
        );
    }
    commands
}
impl TextSession {
    /// Queues a text write; a run of them supersedes itself in the kernel.
    fn push_text(&mut self, text: String) {
        let set: Command = TextCommand::SetText {
            layer_id: self.layer.clone(),
            text,
            time: self.time,
        }
        .into();
        if let Some(Job::Step {
            commands,
            text: true,
            ..
        }) = self.jobs.back_mut()
        {
            *commands = vec![set];
            return;
        }
        self.jobs.push_back(Job::Step {
            commands: vec![set],
            replace: self.text_step,
            text: true,
        });
        self.text_step = true;
    }
}
async fn call(
    kernel: &pm_kernel::KernelHandle,
    project: &str,
    call: Call,
) -> Result<Reply, String> {
    let ack = kernel
        .call(project, Request::new(Caller::native(), call))
        .await
        .ok_or("Kernel closed")?
        .map_err(|e| format!("{e:?}"))?;
    match ack.reply {
        Reply::Throw(error) => Err(error.to_string()),
        reply => Ok(reply),
    }
}
/// One job's kernel calls. `Ok(Some(id))` names a gesture it began.
async fn run_job(
    kernel: &pm_kernel::KernelHandle,
    project: &str,
    gesture: Option<u64>,
    layer: &str,
    job: Job,
) -> Result<Option<u64>, String> {
    let step = |gesture: u64, commands: Vec<Command>, replace: bool| -> Call {
        GestureCall::Step {
            gesture,
            body: WriteBody::Commands(commands),
            replace,
        }
        .into()
    };
    match job {
        Job::Begin(label) => {
            match call(
                kernel,
                project,
                GestureCall::Begin {
                    label: label.into(),
                }
                .into(),
            )
            .await?
            {
                Reply::Gesture(id) => Ok(Some(id)),
                _ => Err("The edit could not start".into()),
            }
        }
        Job::Add { at, width, time } => {
            let gesture = gesture.ok_or("The edit could not start")?;
            let published = kernel.published(project).ok_or("No open project")?;
            let commands = add_text(&published, layer, at, width, time);
            call(kernel, project, step(gesture, commands, false)).await?;
            Ok(None)
        }
        Job::Step {
            commands, replace, ..
        } => {
            let gesture = gesture.ok_or("The edit could not start")?;
            call(kernel, project, step(gesture, commands, replace)).await?;
            Ok(None)
        }
        Job::Select(id) => {
            let select = SelectionCall::Layers {
                ids: vec![id],
                add: false,
            };
            call(kernel, project, select.into()).await?;
            Ok(None)
        }
        Job::Finish {
            text,
            fresh,
            rename,
        } => {
            let Some(gesture) = gesture else {
                return Ok(None);
            };
            if text.trim().is_empty() {
                call(kernel, project, GestureCall::Cancel { gesture }.into()).await?;
                if !fresh {
                    let delete = Call::Transact {
                        label: Some("Delete Empty Text".into()),
                        body: WriteBody::Commands(vec![
                            LayerCommand::Delete {
                                layer_ids: vec![layer.into()],
                            }
                            .into(),
                        ]),
                        expected_revision: None,
                    };
                    call(kernel, project, delete).await?;
                }
                return Ok(None);
            }
            if rename {
                let patch = LayerPatch {
                    name: Some(first_line(&text)),
                    ..Default::default()
                };
                let set = LayerCommand::Set {
                    layer_id: layer.into(),
                    patch,
                };
                call(kernel, project, step(gesture, vec![set.into()], false)).await?;
            }
            call(kernel, project, GestureCall::Commit { gesture }.into()).await?;
            Ok(None)
        }
    }
}
/// The press that began a Text tool gesture and the box dragged so far.
pub struct TextSketch {
    pub origin: Point<Pixels>,
    pub start: [f64; 2],
    pub shift: bool,
    pub shape: Option<crate::shape_tool::ShapeBox>,
}
impl GizmoSurface {
    /// The Text tool's release: existing text under a click is edited at the
    /// pointer, otherwise new point or paragraph text.
    pub(super) fn text_tool_release(
        &mut self,
        sketch: TextSketch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paragraph = sketch.shape.filter(|b| b.w() >= PARAGRAPH_MIN);
        if let Some(b) = paragraph {
            self.new_text([b.x0, b.y0], Some(b.w()), window, cx);
            return;
        }
        let at = sketch.start;
        let time = self.host.time(cx);
        let (kernel, project) = (self.kernel.clone(), self.project.clone());
        let handle = window.window_handle();
        let shift = sketch.shift;
        cx.spawn(async move |this, cx| {
            let hit = cx
                .background_executor()
                .spawn(async move { text_under(&kernel, &project, at, time) })
                .await;
            let _ = handle.update(cx, |_, window, cx| {
                let _ = this.update(cx, |s, cx| match hit.filter(|_| !shift) {
                    Some(id) => s.edit_text(id, Entry::Caret(at), window, cx),
                    None => s.new_text(at, None, window, cx),
                });
            });
        })
        .detach();
    }
    /// A double-click's text edit: the layer's text, all of it selected.
    pub(super) fn edit_text_all(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_text(id, Entry::All, window, cx);
    }
    /// Return with one text layer selected edits it (Edit ▸ Edit Text).
    pub fn edit_selected_text(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(p) = self.kernel.published(&self.project) else {
            return false;
        };
        let [id] = p.selection.layers() else {
            return false;
        };
        let is_text = p.snapshot.root().is_some_and(|root| {
            pm_kernel::model::list_layers(root, None)
                .iter()
                .any(|l| &l.id == id && l.kind == "text" && !l.effectively_locked)
        });
        if !is_text {
            return false;
        }
        self.edit_text(id.clone(), Entry::End, window, cx);
        true
    }
}
/// The unlocked, active text layer the pointer is over, deepest first.
fn text_under(
    kernel: &pm_kernel::KernelHandle,
    project: &str,
    at: [f64; 2],
    time: f64,
) -> Option<String> {
    let p = kernel.published(project)?;
    let root = p.snapshot.root()?;
    let id = crate::text_host::query(&p, |q| {
        q.pick(None, at[0], at[1], time, &Default::default())
    })
    .ok()?
    .layer_id?;
    pm_kernel::model::list_layers(root, None)
        .into_iter()
        .find(|l| l.id == id && l.kind == "text" && !l.effectively_locked)
        .map(|l| l.id)
}

fn ns_rect(b: Bounds<Pixels>) -> objc2_foundation::NSRect {
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    NSRect::new(
        NSPoint::new(f32::from(b.origin.x) as f64, f32::from(b.origin.y) as f64),
        NSSize::new(
            f32::from(b.size.width) as f64,
            f32::from(b.size.height) as f64,
        ),
    )
}
/// The P-TEXT canvas slot over the viewer.
struct NativeInput(gpui_macos::native_text::NativeTextSlot);
impl TextInput for NativeInput {
    fn value(&self) -> String {
        self.0.value()
    }
    fn selection(&self) -> (usize, usize) {
        self.0.selection()
    }
    fn set_selection(&self, start: usize, end: usize) {
        self.0.set_selection(start, end);
    }
    fn replace_value(&self, value: &str) {
        self.0.replace_value(value);
    }
    fn composing(&self) -> bool {
        self.0.has_marked_text()
    }
    fn focused(&self) -> bool {
        self.0.is_focused()
    }
    fn focus(&self) {
        self.0.begin_editing();
    }
    fn set_font(&self, family: &str, points: f64) {
        self.0.set_font(family, points);
    }
    fn misspelled(&self) -> Vec<(usize, usize)> {
        self.0.misspelled()
    }
    fn context_menu(&self, word: (usize, usize)) {
        self.0.context_menu(word);
    }
    fn place(&self, bounds: Bounds<Pixels>, clip: Bounds<Pixels>) {
        self.0.stage(ns_rect(bounds), ns_rect(clip));
    }
    fn stage_caret(&self, caret: Option<(Bounds<Pixels>, Bounds<Pixels>)>) {
        self.0
            .stage_caret(caret.map(|(c, clip)| (ns_rect(c), ns_rect(clip))));
    }
    fn marked_range(&self) -> Option<(usize, usize)> {
        self.0.marked_range()
    }
    fn stage_input_rect(&self, rect: Option<Bounds<Pixels>>) {
        self.0.stage_input_rect(rect.map(ns_rect));
    }
    fn take_events(&self) -> Vec<TextEvent> {
        self.0.take_events()
    }
    fn on_event(&self, wake: Rc<dyn Fn()>) {
        self.0.on_event(wake);
    }
}
pub fn native_input(initial: &str, window: &Window) -> Option<Rc<dyn TextInput>> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
    else {
        return None;
    };
    let view: objc2::rc::Retained<objc2_app_kit::NSView> =
        unsafe { objc2::rc::Retained::retain(handle.ns_view.as_ptr().cast())? };
    let slot = gpui_macos::native_text::NativeTextSlot::new(
        &view,
        gpui_macos::native_text::TextKind::Canvas,
        "Edit text on canvas",
        initial,
    );
    Some(Rc::new(NativeInput(slot)))
}

#[cfg(test)]
mod tests {
    // Not a glob: gpui's own `test` attribute would shadow the standard one.
    use super::{Align, Layout, Line, first_line, paragraph_range, word_range};
    fn layout() -> Layout {
        // Two lines: "ab" at y 0 and "cd" at y 20; 10 px per character.
        let line = |start: usize, y: f64| Line {
            len: 2,
            start,
            x: 0.,
            y,
            width: 20.,
            boundaries: vec![(start, 0.), (start + 1, 10.), (start + 2, 20.)],
        };
        Layout {
            lines: vec![line(0, 0.), line(3, 20.)],
            line_height: 20.,
            size: 16.,
            length: 5,
            align: Align::Left,
            box_width: 0.,
            box_height: 0.,
        }
    }
    #[test]
    fn carets_and_hits_follow_the_engine_lines() {
        let l = layout();
        assert_eq!(l.caret(1).x, 10.);
        assert_eq!(l.caret(4).line, 1);
        // The newline belongs to the first line's end.
        assert_eq!(l.caret(2).line, 0);
        assert_eq!(l.index_at(14., 25.), 4);
        assert_eq!(l.index_at(-50., -50.), 0);
        assert_eq!(l.index_at(500., 500.), 5);
    }
    #[test]
    fn vertical_moves_keep_the_column() {
        let l = layout();
        assert_eq!(l.vertical(1, true, 10.), Some(4));
        assert_eq!(l.vertical(4, false, 10.), Some(1));
        assert_eq!(l.vertical(4, true, 10.), None);
    }
    #[test]
    fn a_selection_across_lines_has_one_rect_per_line() {
        let r = layout().selection_rects(1, 4);
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].x, r[0].w), (10., 14.));
        assert_eq!((r[1].x, r[1].w), (0., 10.));
    }
    #[test]
    fn point_text_frames_its_lines_and_paragraphs_their_box() {
        let mut l = layout();
        let f = l.frame();
        assert_eq!((f.x, f.w), (0., 20.));
        l.box_width = 300.;
        l.align = Align::Center;
        assert_eq!(l.frame().x, -150.);
    }
    #[test]
    fn word_and_paragraph_ranges() {
        assert_eq!(word_range("hello world", 7), (6, 11));
        assert_eq!(word_range("hello world", 5), (5, 6));
        assert_eq!(paragraph_range("ab\ncd\nef", 4), (3, 5));
        assert_eq!(first_line("\n  Hello  \nthere"), "Hello");
        let long = "x".repeat(50);
        assert_eq!(first_line(&long).chars().count(), 40);
    }
}
