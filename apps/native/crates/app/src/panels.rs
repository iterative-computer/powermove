//! Temporary P1 controls. P3 replaces these through the same shell slots.
//! Visuals follow docs/native/design/language.md §9–§11 with pm-design tokens.
use crate::{platform, session::Controller};
use gpui::{prelude::*, *};
use pm_kernel::{
    commands::{layers::LayerPatch, properties::SetProperty},
    *,
};
use pm_shell::layout::Rect;
use std::{ops::Range, sync::Arc};

pub struct NumericDrag {
    pub channel: String,
    pub layer: String,
    pub start: Point<Pixels>,
    pub value: f64,
    pub latest: f64,
    pub gesture: Option<u64>,
    pub finished: bool,
    pub moved: bool,
}
impl Controller {
    fn number(&mut self, channel: &str) -> f64 {
        let t = self.time();
        self.selected
            .clone()
            .and_then(|id| {
                self.mirror
                    .as_mut()?
                    .queries()
                    .evaluate(None, &id, channel, t)
                    .ok()
            })
            .and_then(|v| v.as_f64())
            .unwrap_or(if channel == "opacity" { 100. } else { 0. })
    }
    fn property(&self, layer: String, channel: String, value: f64) -> Command {
        Command::Property(PropertyCommand::Set(SetProperty::new(
            layer,
            channel,
            value,
            self.time(),
        )))
    }
    fn start_number(&mut self, channel: String, start: Point<Pixels>, cx: &mut Context<Self>) {
        if self.drag.is_some() {
            return;
        }
        let Some(layer) = self.selected.clone() else {
            return;
        };
        let value = self.number(&channel);
        self.drag = Some(NumericDrag {
            channel: channel.clone(),
            layer,
            start,
            value,
            latest: value,
            gesture: None,
            finished: false,
            moved: false,
        });
        let Some(s) = &self.session else { return };
        let pending = self.kernel.call(
            &s.project,
            Request::new(
                Caller::native(),
                GestureCall::Begin {
                    label: format!("Change {channel}"),
                },
            ),
        );
        cx.spawn(async move |this, cx| {
            let reply = pending.await;
            let _ = this.update(cx, |v, cx| {
                if let Some(Ok(Ack {
                    reply: Reply::Gesture(id),
                    ..
                })) = reply
                {
                    if let Some(d) = &mut v.drag {
                        d.gesture = Some(id);
                    }
                    v.step_number(cx);
                    if v.drag.as_ref().is_some_and(|d| d.finished) {
                        v.finish_number(cx);
                    }
                } else {
                    v.drag = None;
                    v.message = format!("Could not begin field gesture: {reply:?}");
                }
            });
        })
        .detach();
    }
    fn step_number(&mut self, cx: &mut Context<Self>) {
        if let Some(d) = &self.drag
            && let Some(id) = d.gesture
            && d.moved
        {
            let command = self.property(d.layer.clone(), d.channel.clone(), d.latest);
            self.call(
                Call::Gesture(GestureCall::Step {
                    gesture: id,
                    body: WriteBody::Commands(vec![command]),
                    replace: true,
                }),
                cx,
            );
        }
    }
    fn finish_number(&mut self, cx: &mut Context<Self>) {
        let Some(d) = &mut self.drag else { return };
        d.finished = true;
        if d.gesture.is_none() {
            return;
        }
        let d = self.drag.take().unwrap();
        self.call(
            Call::Gesture(GestureCall::Commit {
                gesture: d.gesture.unwrap(),
            }),
            cx,
        );
        if !d.moved {
            self.edit_field(d.channel, cx);
        }
    }
    fn edit_field(&mut self, channel: String, cx: &mut Context<Self>) {
        let Some(layer) = self.selected.clone() else {
            return;
        };
        let value = if channel == "name" {
            self.layers()
                .iter()
                .find(|l| l.id == layer)
                .map(|l| l.name.clone())
                .unwrap_or_default()
        } else {
            format!("{}", self.number(&channel))
        };
        let title = if channel == "name" {
            "Rename Layer"
        } else {
            channel_label(&channel)
        };
        let rx = platform::field(&self.host.borrow().window, title, &value);
        cx.spawn(async move |this, cx| {
            if let Ok(Some(text)) = rx.recv().await {
                let _ = this.update(cx, |v, cx| {
                    if channel == "name" {
                        v.edit(
                            Command::Layer(LayerCommand::Set {
                                layer_id: layer,
                                patch: LayerPatch {
                                    name: Some(text),
                                    ..Default::default()
                                },
                            }),
                            cx,
                        );
                    } else if let Ok(n) = text.parse::<f64>() {
                        if n.is_finite() && (channel != "opacity" || (0. ..=100.).contains(&n)) {
                            let command = v.property(layer, channel, n);
                            v.edit(command, cx);
                        } else {
                            v.message = "Enter a finite value (opacity: 0–100)".into();
                            cx.notify();
                        }
                    } else {
                        v.message = "Enter a number".into();
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }
}

/// Inspector channels shown by the temporary Properties panel.
const CHANNELS: [&str; 3] = ["opacity", "position.x", "position.y"];
const VALUE_TIP: &str = "Drag to adjust. Click to type.";
/// Viewer zoom pop-up choices; `None` is Fit.
const ZOOMS: [Option<f64>; 6] = [None, Some(0.25), Some(0.5), Some(1.), Some(2.), Some(4.)];

/// Title Case label without its unit (units belong to values).
pub fn channel_label(channel: &str) -> &'static str {
    match channel {
        "opacity" => "Opacity",
        "position.x" => "Position X",
        "position.y" => "Position Y",
        _ => "Value",
    }
}

/// Value with its unit at the property's display precision: percentages to
/// one decimal, pixels to two, trailing zeros trimmed (`100 %`, `12.5 px`).
pub fn display_value(channel: &str, value: f64) -> String {
    let (decimals, unit) = match channel {
        "opacity" => (1, "%"),
        _ => (2, "px"),
    };
    let mut text = format!("{value:.decimals$}");
    if text.contains('.') {
        text = text.trim_end_matches('0').trim_end_matches('.').to_owned();
    }
    if text == "-0" {
        text = "0".into();
    }
    format!("{text} {unit}")
}

/// Zoom as a whole percentage (`54 %`).
pub fn percent(zoom: f64) -> String {
    format!("{} %", (zoom * 100.).round() as i64)
}

/// The zoom pop-up title: `Fit (54 %)` or `100 %`.
pub fn zoom_title(zoom: Option<f64>, fit: f64) -> String {
    match zoom {
        None => format!("Fit ({})", percent(fit)),
        Some(z) => percent(z),
    }
}

/// Fit scale for the body below the viewer bar (the shell's header): the
/// image is centred there with `margin` on every side.
pub fn fit_scale(body_w: f32, body_h: f32, comp: [f64; 2], margin: f32) -> f64 {
    let w = (body_w - margin * 2.).max(1.) as f64 / comp[0].max(1.);
    let h = (body_h - margin * 2.).max(1.) as f64 / comp[1].max(1.);
    w.min(h).max(0.01)
}

/// Clip label colour by layer kind (tokens.md, `colors.labels.*`).
pub fn label_color(kind: &str, labels: &pm_design::Labels) -> u32 {
    match kind {
        "text" | "shape" => labels.blue,
        "video" | "image" | "sequence" => labels.teal,
        "audio" => labels.green,
        "solid" => labels.orange,
        "adjustment" | "effect" => labels.violet,
        _ => labels.gray,
    }
}

/// Display name of a layer kind (`Video`, `Text`, …).
pub fn kind_name(kind: &str) -> String {
    match kind {
        "precomp" => "Composition".into(),
        "" => "Layer".into(),
        k => {
            let mut c = k.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect())
                .unwrap_or_default()
        }
    }
}

/// `a` moved towards `b` by `t` (sRGB, per channel).
pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |v: u32, s: u32| ((v >> s) & 255) as f32;
    [16, 8, 0].iter().fold(0, |acc, &s| {
        let v = ch(a, s) + (ch(b, s) - ch(a, s)) * t;
        acc | ((v.round().clamp(0., 255.) as u32) << s)
    })
}

/// The comp frame gets a separator outline when its background is
/// transparent or too close to the viewer surround to show its bounds.
pub fn needs_frame_outline(background: Option<u32>, surround: u32) -> bool {
    background.is_none_or(|bg| {
        [16, 8, 0]
            .iter()
            .map(|s| (((bg >> s) & 255) as i32 - ((surround >> s) & 255) as i32).abs())
            .max()
            .unwrap_or(0)
            <= 24
    })
}

/// `#rgb`, `#rrggbb` or `#rrggbbaa`; translucent colours count as transparent.
pub fn parse_background(text: &str) -> Option<u32> {
    let hex = text.strip_prefix('#')?;
    let expand: String = if hex.len() == 3 {
        hex.chars().flat_map(|c| [c, c]).collect()
    } else {
        hex.to_owned()
    };
    let v = u32::from_str_radix(expand.get(..6)?, 16).ok()?;
    match expand.len() {
        6 => Some(v),
        8 if expand.get(6..8)? == "ff" || expand.get(6..8)? == "FF" => Some(v),
        _ => None,
    }
}

/// Major ruler interval in frames: the smallest "nice" step at least
/// `min_px` wide.
pub fn ruler_step(fps: f64, px_per_second: f64, min_px: f64) -> f64 {
    let fps = fps.max(1.);
    let frames = [1., 2., 5., 10.];
    let seconds = [
        1., 2., 5., 10., 15., 30., 60., 120., 300., 600., 1800., 3600.,
    ];
    frames
        .into_iter()
        .chain(seconds.into_iter().map(|s| s * fps))
        .find(|f| f / fps * px_per_second >= min_px)
        .unwrap_or(3600. * fps)
}

/// Ruler label for a frame: `0:05` on whole seconds, else `0:05:12`.
pub fn ruler_label(frame: f64, fps: f64) -> String {
    let fps = fps.max(1.).round();
    let frame = frame.round();
    let total = (frame / fps).floor() as i64;
    let rest = (frame - total as f64 * fps) as i64;
    let (m, s) = (total / 60, total % 60);
    if rest == 0 {
        format!("{m}:{s:02}")
    } else {
        format!("{m}:{s:02}:{rest:02}")
    }
}

fn plural(n: usize, one: &str) -> String {
    format!("{n} {one}{}", if n == 1 { "" } else { "s" })
}

/// Status bar copy: document state, then counts. No developer words.
pub fn status_line(dirty: bool, saved_path: bool, layers: usize, keyframes: usize) -> String {
    let counts = format!(
        "{} · {}",
        plural(layers, "layer"),
        plural(keyframes, "keyframe")
    );
    match (dirty, saved_path) {
        (true, _) => format!("Edited · {counts}"),
        (false, true) => format!("Saved · {counts}"),
        (false, false) => counts,
    }
}

fn tabular() -> FontFeatures {
    FontFeatures(Arc::new(vec![("tnum".into(), 1)]))
}

/// Small floating tooltip using the floating material colours.
struct Tip {
    text: SharedString,
    fill: u32,
    ink: u32,
    edge: u32,
    radius: f32,
    pad: f32,
    size: f32,
}
impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .bg(rgb(self.fill))
            .text_color(rgb(self.ink))
            .border(px(1.))
            .border_color(rgb(self.edge))
            .rounded(px(self.radius))
            .px(px(self.pad))
            .py(px(self.pad / 2.))
            .text_size(px(self.size))
            .child(self.text.clone())
    }
}
fn tooltip(
    t: &pm_design::Tokens,
    text: &'static str,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let (fill, ink, edge) = (t.colors.floating, t.colors.text, t.colors.separator);
    let (radius, pad, size) = (
        t.metrics.control_radius,
        t.metrics.gap,
        t.typography.minimum,
    );
    move |_, cx| {
        cx.new(|_| Tip {
            text: text.into(),
            fill,
            ink,
            edge,
            radius,
            pad,
            size,
        })
        .into()
    }
}

pub fn render(
    v: &mut Controller,
    id: &str,
    bounds: Rect,
    window: &mut Window,
    cx: &mut Context<Controller>,
) -> Option<AnyElement> {
    let _span = crate::diag::span(match id {
        "project" | "assets" => "render.project",
        "timeline" => "render.timeline",
        "viewer" => "render.viewer",
        _ => "render.other",
    });
    let t = v.host.borrow().tokens.clone();
    let c = t.colors.clone();
    let m = t.metrics;
    let ty = t.typography;
    let body_h = (bounds.h - m.header).max(0.);
    let mut body = div()
        .id(format!("p1-{id}"))
        .h(px(body_h))
        .w_full()
        .p(px(m.inset))
        .flex()
        .flex_col()
        .gap(px(m.gap));
    match id {
        "project" | "assets" => {
            body = body.px(px(m.gap)).gap(px(0.));
            body = body.child(
                div()
                    .id("library-toggle")
                    .h(px(m.row))
                    .px(px(m.gap))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .text_color(rgb(c.accent_text))
                    .child(if v.show_library {
                        "‹ Composition Layers"
                    } else {
                        "Library & Recent Projects ›"
                    })
                    .on_click(cx.listener(|v, _, _, cx| {
                        v.show_library = !v.show_library;
                        v.load_recent(cx);
                        cx.notify();
                    })),
            );
            if v.show_library || v.session.is_none() {
                body = body.overflow_y_scroll();
                for (index, (id, name)) in v.recent.clone().into_iter().enumerate() {
                    body = body.child(
                        list_row(&t, ElementId::Integer(index as u64), false)
                            .child(div().truncate().child(name))
                            .on_click(
                                cx.listener(move |v, _, _, cx| v.open_library(id.clone(), cx)),
                            ),
                    );
                }
                if v.recent.is_empty() {
                    body = body.justify_center().child(pm_shell::view::empty_state(
                        &t,
                        "No Recent Projects",
                        "Choose File ▸ Open… to open a project.",
                    ));
                }
            } else {
                // Only visible rows are built: a 60,000-layer composition must
                // not lay out 60,000 rows on the UI thread per repaint.
                let rows = uniform_list(
                    "project-layers",
                    v.layers().len(),
                    cx.processor(move |v: &mut Controller, range: Range<usize>, _, cx| {
                        let rows = v.rows.rows.clone();
                        rows[range.start.min(rows.len())..range.end.min(rows.len())]
                            .iter()
                            .enumerate()
                            .map(|(offset, layer)| project_row(v, range.start + offset, layer, cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&v.project_scroll)
                .flex_1();
                body = body.child(rows);
            }
        }
        "properties" | "inspector" => {
            body = body.overflow_y_scroll();
            if let Some(layer) = v
                .layers()
                .iter()
                .find(|l| Some(&l.id) == v.selected.as_ref())
                .cloned()
            {
                body = body.child(
                    div()
                        .id("layer-name")
                        .h(px(m.row))
                        .flex()
                        .items_center()
                        .gap(px(m.gap))
                        .cursor_pointer()
                        .aria_label(format!("{} · {}", layer.name, kind_name(&layer.kind)))
                        .child(
                            div()
                                .truncate()
                                .text_size(px(ty.title))
                                .font_weight(FontWeight(ty.semibold_weight))
                                .text_color(rgb(c.text))
                                .child(layer.name.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(px(ty.title))
                                .text_color(rgb(c.muted))
                                .child(format!("· {}", kind_name(&layer.kind))),
                        )
                        .on_click(cx.listener(|v, _, _, cx| v.edit_field("name".into(), cx))),
                );
                for channel in CHANNELS {
                    let label = channel_label(channel);
                    let value = v.number(channel);
                    body = body.child(
                        div()
                            .flex()
                            .items_center()
                            .h(px(m.row))
                            .gap(px(m.gap))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .truncate()
                                    .text_size(px(ty.label))
                                    .text_color(rgb(c.secondary))
                                    .child(label),
                            )
                            .child(
                                div()
                                    .id(channel)
                                    .role(Role::TextInput)
                                    .aria_label(label)
                                    .flex_none()
                                    .w(px(m.value_column))
                                    .h(px(m.field))
                                    .px(px(m.gap))
                                    .flex()
                                    .items_center()
                                    .bg(rgb(c.field))
                                    .rounded(px(m.control_radius))
                                    .font_features(tabular())
                                    .text_color(rgb(c.text))
                                    .cursor(CursorStyle::ResizeLeftRight)
                                    .tooltip(tooltip(&t, VALUE_TIP))
                                    .child(display_value(channel, value))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |v, e: &MouseDownEvent, _, cx| {
                                            v.start_number(channel.into(), e.position, cx)
                                        }),
                                    ),
                            ),
                    );
                }
            } else {
                let (heading, line, _) = pm_shell::layout::empty_state("inspector");
                body = body
                    .justify_center()
                    .child(pm_shell::view::empty_state(&t, heading, line));
            }
            body = body
                .on_mouse_move_all({
                    let listener = cx.listener(|v, e: &MouseMoveEvent, _, cx| {
                        if e.dragging()
                            && let Some(d) = &mut v.drag
                        {
                            let delta = f32::from(e.position.x - d.start.x)
                                - f32::from(e.position.y - d.start.y);
                            if delta.abs() >= v.host.borrow().tokens.metrics.drag_threshold
                                || d.moved
                            {
                                d.moved = true;
                                d.latest = d.value
                                    + delta as f64 * if e.modifiers.shift { 10. } else { 1. };
                                if d.channel == "opacity" {
                                    d.latest = d.latest.clamp(0., 100.);
                                }
                                v.step_number(cx);
                            }
                        }
                    });
                    move |e, phase, _, w, cx| {
                        if phase == DispatchPhase::Bubble {
                            listener(e, w, cx)
                        }
                    }
                })
                .on_mouse_up_all({
                    let listener = cx.listener(|v, e: &MouseUpEvent, _, cx| {
                        if e.button == MouseButton::Left {
                            v.finish_number(cx)
                        }
                    });
                    move |e, phase, _, w, cx| {
                        if phase == DispatchPhase::Bubble {
                            listener(e, w, cx)
                        }
                    }
                });
        }
        "timeline" => {
            body = body.p(px(0.)).gap(px(0.)).overflow_hidden();
            let column = m.layer_column.min((bounds.w * 0.5).max(m.min_panel));
            let width = (bounds.w - column - m.inset).max(1.);
            let duration = v.duration.max(0.001);
            let fps = v.rate.to_f64();
            // Ruler: major ticks `secondary` with `minimum` labels, minor ticks `muted`.
            let pps = width as f64 / duration;
            let step = ruler_step(fps, pps, 72.);
            let mut ruler = div()
                .id("timeline-ruler")
                .relative()
                .w(px(width))
                .h(px(m.ruler))
                .overflow_hidden()
                .cursor_pointer();
            let total_frames = duration * fps;
            let mut frame = 0.;
            while frame <= total_frames + 0.5 {
                let x = (frame / fps * pps) as f32;
                ruler = ruler
                    .child(
                        div()
                            .absolute()
                            .left(px(x))
                            .bottom(px(0.))
                            .w(px(m.border))
                            .h(px(m.ruler / 3.))
                            .bg(rgb(c.secondary)),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(x + m.border * 3.))
                            .top(px(m.border * 2.))
                            .text_size(px(ty.minimum))
                            .font_features(tabular())
                            .text_color(rgb(c.muted))
                            .child(ruler_label(frame, fps)),
                    );
                let half = x + (step / 2. / fps * pps) as f32;
                if step >= 2. && half < width {
                    ruler = ruler.child(
                        div()
                            .absolute()
                            .left(px(half))
                            .bottom(px(0.))
                            .w(px(m.border))
                            .h(px(m.ruler / 6.))
                            .bg(rgb(c.muted)),
                    );
                }
                frame += step;
            }
            let scrub = move |v: &mut Controller, x: Pixels| {
                let x = (f32::from(x) - bounds.x - column).clamp(0., width);
                v.seek(x as f64 / width as f64 * v.duration);
            };
            ruler = ruler
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |v, e: &MouseDownEvent, _, _| scrub(v, e.position.x)),
                )
                .on_mouse_move(cx.listener(move |v, e: &MouseMoveEvent, _, _| {
                    if e.pressed_button == Some(MouseButton::Left) {
                        scrub(v, e.position.x)
                    }
                }));
            body = body.child(
                div()
                    .flex_none()
                    .flex()
                    .border_b(px(m.border))
                    .border_color(rgb(c.separator))
                    .child(
                        div()
                            .flex_none()
                            .w(px(column))
                            .h(px(m.ruler))
                            .bg(rgb(c.panel_secondary))
                            .border_r(px(m.border))
                            .border_color(rgb(c.separator)),
                    )
                    .child(ruler),
            );
            let rows = uniform_list(
                "timeline-layers",
                v.layers().len(),
                cx.processor(move |v: &mut Controller, range: Range<usize>, _, cx| {
                    let rows = v.rows.rows.clone();
                    rows[range.start.min(rows.len())..range.end.min(rows.len())]
                        .iter()
                        .enumerate()
                        .map(|(offset, layer)| {
                            timeline_row(v, range.start + offset, layer, column, width, scrub, cx)
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&v.timeline_scroll)
            .flex_1();
            // The layer column runs to the bottom of the panel, behind the rows.
            body = body.relative().child(
                div()
                    .absolute()
                    .left(px(0.))
                    .top(px(m.ruler))
                    .bottom(px(0.))
                    .w(px(column))
                    .bg(rgb(c.panel_secondary))
                    .border_r(px(m.border))
                    .border_color(rgb(c.separator)),
            );
            body = body.child(rows);
            // The native playhead runs from the ruler's top edge; its head sits in the ruler.
            crate::playback::set_timeline(cx.entity_id(), bounds, column, m.header);
        }
        "viewer" => {
            // The shell's header is the viewer bar; the image is centred in the
            // body below it. Native Metal sits beneath this panel.
            let fit = fit_scale(bounds.w, body_h, v.dimensions, m.inset);
            body = body.p(px(0.)).gap(px(0.));
            let zoom = Some(v.zoom.unwrap_or(fit));
            v.host.borrow_mut().viewer_transform(v.dimensions, zoom);
            crate::playback::frame_outline(
                &v.host.borrow(),
                needs_frame_outline(v.background, c.viewer),
                c.separator,
                m.border,
            );
            let _ = window;
        }
        _ => return None,
    }
    Some(body.into_any_element())
}

/// Header content for panels that own their 32 pt bar (language §10, §11):
/// the viewer's zoom pop-up (trailing, no title) and the timeline transport
/// (play, timecode, fps). The shell draws it in the panel header.
pub fn header(
    v: &mut Controller,
    id: &str,
    bounds: Rect,
    _window: &mut Window,
    cx: &mut Context<Controller>,
) -> Option<AnyElement> {
    let t = v.host.borrow().tokens.clone();
    let (c, m, ty) = (&t.colors, t.metrics, t.typography);
    let bar = div().size_full().flex().items_center().gap(px(m.gap));
    match id {
        "viewer" => {
            let body_h = (bounds.h - m.header).max(0.);
            let fit = fit_scale(bounds.w, body_h, v.dimensions, m.inset);
            let title = zoom_title(v.zoom, fit);
            Some(
                bar.justify_end()
                    .child(
                        div()
                            .id("viewer-zoom")
                            .tab_index(10)
                            .focus_style(|s| s.ring(px(m.border * 2.)).ring_color(rgb(c.focus)))
                            .role(Role::Button)
                            .aria_label(format!("Zoom: {title}"))
                            .h(px(m.field))
                            .px(px(m.gap * 1.5))
                            .flex()
                            .items_center()
                            .gap(px(m.gap))
                            .rounded(px(m.control_radius))
                            .text_size(px(ty.body))
                            .font_features(tabular())
                            .text_color(rgb(c.text))
                            .hover_style(|s| s.bg(rgb(c.hover)))
                            .cursor_pointer()
                            .child(title)
                            .child(div().text_color(rgb(c.secondary)).child("▾"))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |v, e: &ClickEvent, _, cx| {
                                let x = f32::from(e.position().x);
                                let y = bounds.y + m.header;
                                v.zoom_menu(x, y, fit, cx);
                            })),
                    )
                    .into_any_element(),
            )
        }
        "timeline" => {
            let playing = v.playing();
            let fps = v.rate.to_f64();
            // Play button, then the native timecode layer's slot, then fps.
            let slot = crate::playback::timecode_slot(&t);
            Some(
                bar.child(
                    div()
                        .id("timeline-play")
                        .tab_index(10)
                        .focus_style(|s| s.ring(px(m.border * 2.)).ring_color(rgb(c.focus)))
                        .role(Role::Button)
                        .aria_label(if playing { "Pause" } else { "Play" })
                        .flex_none()
                        .size(px(m.hit))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(m.control_radius))
                        .hover_style(|s| s.bg(rgb(c.hover)))
                        .cursor_pointer()
                        .child(SymbolImage(
                            if playing { "pause.fill" } else { "play.fill" },
                            m.symbol_panel,
                            c.text,
                        ))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|v, _, w, cx| {
                            let play = pm_shell::Command {
                                name: "play".into(),
                                args: vec![],
                                token: None,
                            };
                            v.command(&play, w, cx);
                            cx.notify();
                        })),
                )
                .child(div().flex_none().w(px(slot.w)).h(px(slot.h)))
                .child(
                    div()
                        .text_size(px(ty.minimum))
                        .font_features(tabular())
                        .text_color(rgb(c.muted))
                        .child(format!("{} fps", trim_rate(fps))),
                )
                .into_any_element(),
            )
        }
        _ => None,
    }
}

/// An SF Symbol (`pm_shell::symbol`) at `points` in `color`, rendered at the
/// window's scale when laid out.
#[derive(IntoElement)]
struct SymbolImage(&'static str, f32, u32);
impl RenderOnce for SymbolImage {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        pm_shell::symbol::symbol(
            self.0,
            self.1,
            pm_shell::symbol::Weight::Medium,
            self.2,
            window,
        )
    }
}

fn list_row(t: &pm_design::Tokens, id: ElementId, selected: bool) -> Stateful<Div> {
    let (c, m) = (&t.colors, t.metrics);
    div()
        .id(id)
        .w_full()
        .h(px(m.row))
        .flex_none()
        .px(px(m.gap))
        .flex()
        .items_center()
        .gap(px(m.gap))
        .rounded(px(m.control_radius))
        .when(selected, |d| d.bg(rgb(c.selected_fill)))
        .when(!selected, |d| d.hover_style(|s| s.bg(rgb(c.hover))))
        .cursor_pointer()
}

/// Kind symbol (`film`, `textformat`, …) in a label-colour badge (language §8, §11).
fn kind_badge(t: &pm_design::Tokens, kind: &str) -> Div {
    let fill = label_color(kind, &t.colors.labels);
    let side = t.metrics.symbol_inline + t.metrics.border * 6.;
    div()
        .flex_none()
        .size(px(side))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(t.metrics.inner_radius))
        .bg(rgb(fill))
        .child(SymbolImage(
            pm_shell::symbol::kind_symbol(kind),
            t.metrics.symbol_inline,
            t.colors.text,
        ))
}

fn project_row(
    v: &Controller,
    index: usize,
    layer: &LayerInfo,
    cx: &mut Context<Controller>,
) -> Stateful<Div> {
    let t = v.host.borrow().tokens.clone();
    let c = &t.colors;
    let selected = v.selected.as_ref() == Some(&layer.id);
    let id = layer.id.clone();
    list_row(&t, ElementId::Integer(index as u64), selected)
        .aria_label(format!("{} · {}", layer.name, kind_name(&layer.kind)))
        .child(kind_badge(&t, &layer.kind))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .when(layer.effectively_locked, |d| d.text_color(rgb(c.secondary)))
                .child(layer.name.clone()),
        )
        .when(layer.effectively_locked, |d| {
            d.child(
                div()
                    .text_size(px(t.typography.minimum))
                    .text_color(rgb(c.muted))
                    .child("Locked"),
            )
        })
        .on_click(cx.listener(move |v, _, _, cx| v.select(id.clone(), cx)))
}

#[allow(clippy::too_many_arguments)]
fn timeline_row(
    v: &Controller,
    index: usize,
    layer: &LayerInfo,
    column: f32,
    width: f32,
    scrub: impl Fn(&mut Controller, Pixels) + Copy + 'static,
    cx: &mut Context<Controller>,
) -> Stateful<Div> {
    let t = v.host.borrow().tokens.clone();
    let (c, m, ty) = (&t.colors, t.metrics, t.typography);
    let duration = v.duration.max(0.001);
    let from = (layer.in_time / duration).clamp(0., 1.) as f32;
    let span = ((layer.out_time - layer.in_time) / duration).clamp(0., 1. - from as f64) as f32;
    let active = v.selected.as_ref() == Some(&layer.id);
    let fill = label_color(&layer.kind, &c.labels);
    let clip_h = (m.track_row - 8.).min(20.);
    let (name_id, track_id) = (layer.id.clone(), layer.id.clone());
    div()
        .id(index)
        .h(px(m.track_row))
        .flex()
        .child(
            div()
                .flex_none()
                .w(px(column))
                .h_full()
                .px(px(m.inset))
                .flex()
                .items_center()
                .gap(px(m.gap))
                .bg(rgb(if active {
                    c.selected_fill
                } else {
                    c.panel_secondary
                }))
                .border_r(px(m.border))
                .border_color(rgb(c.separator))
                .cursor_pointer()
                .child(
                    div()
                        .flex_none()
                        .min_w(px(ty.minimum * 1.5))
                        .text_size(px(ty.minimum))
                        .font_features(tabular())
                        .text_color(rgb(c.muted))
                        .child(format!("{}", index + 1)),
                )
                .child(kind_badge(&t, &layer.kind))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(px(ty.body))
                        .text_color(rgb(c.text))
                        .child(layer.name.clone()),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |v, _, _, cx| v.select(name_id.clone(), cx)),
                ),
        )
        .child(
            div()
                .id(format!("track-{index}"))
                .relative()
                .w(px(width))
                .h_full()
                .child(
                    div()
                        .absolute()
                        .left(px(width * from))
                        .top(px((m.track_row - clip_h) / 2.))
                        .w(px((width * span).max(m.border * 2.)))
                        .h(px(clip_h))
                        .px(px(m.gap))
                        .flex()
                        .items_center()
                        .overflow_hidden()
                        .rounded(px(m.inner_radius))
                        .bg(rgb(if active {
                            mix(fill, c.text, 0.12)
                        } else {
                            fill
                        }))
                        .when(active, |d| {
                            d.border(px(m.playhead)).border_color(rgb(c.text))
                        })
                        .child(
                            div()
                                .truncate()
                                .text_size(px(ty.body))
                                .text_color(rgb(c.text))
                                .child(layer.name.clone()),
                        ),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |v, e: &MouseDownEvent, _, cx| {
                        v.select(track_id.clone(), cx);
                        scrub(v, e.position.x);
                    }),
                ),
        )
}

fn trim_rate(fps: f64) -> String {
    let text = format!("{fps:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

impl Controller {
    pub fn playing(&self) -> bool {
        self.viewer.as_ref().is_some_and(|v| {
            v.status()
                .clock
                .is_some_and(|c| c.state != pm_protocol::clock::TransportState::Stopped)
        })
    }
    /// Native zoom pop-up menu under the viewer bar, checked at the current zoom.
    fn zoom_menu(&mut self, x: f32, y: f32, fit: f64, cx: &mut Context<Self>) {
        let items: Vec<(String, bool)> = ZOOMS
            .iter()
            .map(|z| {
                let checked = match (z, self.zoom) {
                    (None, None) => true,
                    (Some(a), Some(b)) => (a - b).abs() < 1e-6,
                    _ => false,
                };
                (
                    z.map_or_else(|| format!("Fit ({})", percent(fit)), percent),
                    checked,
                )
            })
            .collect();
        let window = self.host.borrow().window.clone();
        cx.spawn(async move |this, cx| {
            // Deferred out of GPUI's event dispatch; the menu runs its own tracking loop.
            let chosen = platform::pop_up_menu(&window, &items, x as f64, y as f64);
            if let Some(index) = chosen {
                let _ = this.update(cx, |v, cx| {
                    v.zoom = ZOOMS[index];
                    v.persist_ui();
                    cx.notify();
                });
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: gpui's `test` attribute macro would shadow `#[test]`.
    use super::{
        CHANNELS, channel_label, display_value, fit_scale, kind_name, label_color, mix,
        needs_frame_outline, parse_background, ruler_label, ruler_step, status_line, zoom_title,
    };

    #[test]
    fn values_carry_units_at_display_precision() {
        assert_eq!(display_value("opacity", 100.), "100 %");
        assert_eq!(display_value("opacity", 50.26), "50.3 %");
        assert_eq!(display_value("position.x", 0.), "0 px");
        assert_eq!(display_value("position.x", -0.001), "0 px");
        assert_eq!(display_value("position.y", 12.5), "12.5 px");
        assert_eq!(display_value("position.y", -3.456), "-3.46 px");
        assert_eq!(channel_label("opacity"), "Opacity");
        assert!(CHANNELS.iter().all(|c| !channel_label(c).contains('%')));
    }

    #[test]
    fn zoom_titles_and_fit_keep_the_image_clear_of_the_bar() {
        assert_eq!(zoom_title(None, 0.5414), "Fit (54 %)");
        assert_eq!(zoom_title(Some(1.), 0.5), "100 %");
        // 1000×600 body below the bar, 12 pt margin, 1920×1080 comp.
        let fit = fit_scale(1000., 600., [1920., 1080.], 12.);
        let image_h = 1080. * fit;
        let top = (600. - image_h) / 2.;
        assert!(top >= 12. - 1e-9, "image top {top} touches the viewer bar");
        assert!(1920. * fit <= 1000. - 24. + 1e-9);
    }

    #[test]
    fn clips_use_label_colours_never_the_accent() {
        for (scheme, increase_contrast) in [
            (pm_design::Scheme::Light, false),
            (pm_design::Scheme::Dark, false),
            (pm_design::Scheme::Light, true),
            (pm_design::Scheme::Dark, true),
        ] {
            let accessibility = pm_design::Accessibility {
                reduce_transparency: false,
                increase_contrast,
                reduce_motion: false,
            };
            let c = pm_design::Tokens::new(scheme, accessibility).colors;
            for kind in [
                "text", "shape", "video", "image", "audio", "solid", "null", "group",
            ] {
                let fill = label_color(kind, &c.labels);
                assert_ne!(fill, c.accent, "{kind}");
                assert_ne!(mix(fill, c.text, 0.12), c.accent, "{kind}");
            }
            assert_eq!(label_color("text", &c.labels), c.labels.blue);
            assert_eq!(label_color("video", &c.labels), c.labels.teal);
        }
        assert_eq!(mix(0x000000, 0xffffff, 0.5), 0x808080);
        assert_eq!(kind_name("video"), "Video");
        assert_eq!(kind_name("precomp"), "Composition");
    }

    #[test]
    fn frame_outline_shows_when_the_comp_cannot_be_told_from_the_surround() {
        assert!(needs_frame_outline(None, 0x0e0e0e));
        assert!(needs_frame_outline(parse_background("#000000"), 0x0e0e0e));
        assert!(needs_frame_outline(parse_background("#1C1C21"), 0x0e0e0e));
        assert!(!needs_frame_outline(parse_background("#ffffff"), 0x0e0e0e));
        assert!(needs_frame_outline(parse_background("#ffffff80"), 0x0e0e0e));
        assert_eq!(parse_background("#abc"), Some(0xaabbcc));
        assert_eq!(parse_background("transparent"), None);
    }

    #[test]
    fn ruler_steps_are_readable_and_labels_are_short() {
        assert_eq!(ruler_step(30., 300., 72.), 10.);
        assert_eq!(ruler_step(30., 60., 72.), 60.);
        assert_eq!(ruler_label(0., 30.), "0:00");
        assert_eq!(ruler_label(150., 30.), "0:05");
        assert_eq!(ruler_label(160., 30.), "0:05:10");
        assert_eq!(ruler_label(30. * 75., 30.), "1:15");
    }

    #[test]
    fn status_line_has_no_developer_words() {
        assert_eq!(
            status_line(false, true, 2, 0),
            "Saved · 2 layers · 0 keyframes"
        );
        assert_eq!(
            status_line(true, true, 1, 1),
            "Edited · 1 layer · 1 keyframe"
        );
        assert_eq!(status_line(false, false, 3, 2), "3 layers · 2 keyframes");
        for s in [
            status_line(true, false, 0, 0),
            status_line(false, true, 5, 9),
        ] {
            assert!(!s.contains("Writable") && !s.contains("Opened"), "{s}");
        }
    }
}
