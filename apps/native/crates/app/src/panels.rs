//! Temporary P1 controls. P3 replaces these through the same shell slots.
use crate::{platform, session::Controller};
use gpui::{prelude::*, *};
use pm_kernel::{
    commands::{layers::LayerPatch, properties::SetProperty},
    *,
};
use pm_shell::layout::Rect;
use std::ops::Range;

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
        let rx = platform::field(
            &self.host.borrow().window,
            &format!("Edit {channel}"),
            &value,
        );
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

pub fn render(
    v: &mut Controller,
    id: &str,
    bounds: Rect,
    _: &mut Window,
    cx: &mut Context<Controller>,
) -> Option<AnyElement> {
    let _span = crate::diag::span(match id {
        "project" | "assets" => "render.project",
        "timeline" => "render.timeline",
        "viewer" => "render.viewer",
        _ => "render.other",
    });
    let t = v.host.borrow().tokens.clone();
    let c = t.colors;
    let m = t.metrics;
    let mut body = div()
        .id(format!("p1-{id}"))
        .h(px((bounds.h - m.header).max(0.)))
        .w_full()
        .p(px(m.inset))
        .flex()
        .flex_col()
        .gap(px(m.gap));
    match id {
        "project" | "assets" => {
            body = body.child(
                div()
                    .text_size(px(t.typography.minimum))
                    .text_color(rgb(c.muted))
                    .child("TEMPORARY · PHASE 1"),
            );
            body = body.child(
                div()
                    .id("library-toggle")
                    .cursor_pointer()
                    .text_color(rgb(c.accent))
                    .child(if v.show_library {
                        "‹ Composition layers"
                    } else {
                        "Library & recent projects ›"
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
                        div()
                            .id(index)
                            .p(px(m.gap))
                            .rounded(px(m.control_radius))
                            .hover_style(|s| s.bg(rgb(c.hover)))
                            .cursor_pointer()
                            .child(name)
                            .on_click(
                                cx.listener(move |v, _, _, cx| v.open_library(id.clone(), cx)),
                            ),
                    );
                }
                if v.recent.is_empty() {
                    body = body.child("No projects in this isolated library. Use File → Open…");
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
            body = body.child(
                div()
                    .text_size(px(t.typography.minimum))
                    .text_color(rgb(c.muted))
                    .child("TEMPORARY · PHASE 1"),
            );
            if let Some(layer) = v
                .layers()
                .iter()
                .find(|l| Some(&l.id) == v.selected.as_ref())
                .cloned()
            {
                body = body.child(
                    div()
                        .id("layer-name")
                        .rounded(px(m.control_radius))
                        .p(px(m.gap))
                        .bg(rgb(c.field))
                        .child(layer.name)
                        .cursor_pointer()
                        .on_click(cx.listener(|v, _, _, cx| v.edit_field("name".into(), cx))),
                );
                for (channel, label) in [
                    ("opacity", "Opacity %"),
                    ("position.x", "Position X"),
                    ("position.y", "Position Y"),
                ] {
                    let value = v.number(channel);
                    body = body.child(
                        div()
                            .flex()
                            .items_center()
                            .h(px(m.row))
                            .gap(px(m.gap))
                            .child(div().flex_1().text_color(rgb(c.secondary)).child(label))
                            .child(
                                div()
                                    .id(channel)
                                    .role(Role::TextInput)
                                    .aria_label(label)
                                    .w(px(m.min_panel / 2.))
                                    .p(px(m.gap))
                                    .bg(rgb(c.field))
                                    .rounded(px(m.control_radius))
                                    .cursor(CursorStyle::ResizeLeftRight)
                                    .child(format!("{value:.2}"))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |v, e: &MouseDownEvent, _, cx| {
                                            v.start_number(channel.into(), e.position, cx)
                                        }),
                                    ),
                            ),
                    );
                }
                body = body.child(
                    div()
                        .text_color(rgb(c.muted))
                        .child("Drag a value to adjust. Click to type."),
                );
            } else {
                body = body.child("Select a layer in Project or Timeline.");
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
            body = body.child(
                div()
                    .text_size(px(t.typography.minimum))
                    .text_color(rgb(c.muted))
                    .child("TEMPORARY · PHASE 1    ·    Click a track to seek"),
            );
            let width = (bounds.w - m.inset * 2. - m.min_panel).max(1.);
            let rows = uniform_list(
                "timeline-layers",
                v.layers().len(),
                cx.processor(move |v: &mut Controller, range: Range<usize>, _, cx| {
                    let rows = v.rows.rows.clone();
                    rows[range.start.min(rows.len())..range.end.min(rows.len())]
                        .iter()
                        .enumerate()
                        .map(|(offset, layer)| {
                            timeline_row(v, range.start + offset, layer, bounds, width, cx)
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&v.timeline_scroll)
            .flex_1();
            body = body.child(rows);
            // Native playhead coordinates are published with this panel's geometry.
            crate::playback::set_timeline(
                cx.entity_id(),
                bounds,
                m.min_panel + m.inset,
                m.header + m.inset + t.typography.minimum + m.gap * 2.,
            );
        }
        "viewer" => {
            // The surface stays clear; native Metal and transport layers are below/above it.
            body = body.p(px(m.gap)).flex_row().items_start().justify_end();
            for (label, zoom) in [("Fit", None), ("50%", Some(0.5)), ("100%", Some(1.))] {
                body = body.child(
                    div()
                        .id(label)
                        .bg(rgb(c.viewer))
                        .rounded(px(m.control_radius))
                        .px(px(m.gap))
                        .cursor_pointer()
                        .child(label)
                        .on_click(cx.listener(move |v, _, _, cx| {
                            v.zoom = zoom;
                            v.persist_ui();
                            cx.notify();
                        })),
                );
            }
            v.host.borrow_mut().viewer_transform(v.dimensions, v.zoom);
        }
        _ => return None,
    }
    Some(body.into_any_element())
}

fn project_row(
    v: &Controller,
    index: usize,
    layer: &LayerInfo,
    cx: &mut Context<Controller>,
) -> Stateful<Div> {
    let t = v.host.borrow().tokens.clone();
    let (c, m) = (t.colors, t.metrics);
    let selected = v.selected.as_ref() == Some(&layer.id);
    let id = layer.id.clone();
    div()
        .id(index)
        .h(px(m.row))
        .px(px(m.gap))
        .flex()
        .items_center()
        .rounded(px(m.control_radius))
        .bg(rgb(if selected { c.selected_fill } else { c.panel }))
        .hover_style(|s| s.bg(rgb(c.hover)))
        .cursor_pointer()
        .child(format!(
            "{}   {}",
            if layer.effectively_locked {
                "◇"
            } else {
                "▱"
            },
            layer.name
        ))
        .on_click(cx.listener(move |v, _, _, cx| v.select(id.clone(), cx)))
}

fn timeline_row(
    v: &Controller,
    index: usize,
    layer: &LayerInfo,
    bounds: Rect,
    width: f32,
    cx: &mut Context<Controller>,
) -> Stateful<Div> {
    let t = v.host.borrow().tokens.clone();
    let (c, m) = (t.colors, t.metrics);
    let from = (layer.in_time / v.duration).clamp(0., 1.) as f32;
    let span = ((layer.out_time - layer.in_time) / v.duration).clamp(0., 1. - from as f64) as f32;
    let active = v.selected.as_ref() == Some(&layer.id);
    let (name_id, track_id) = (layer.id.clone(), layer.id.clone());
    div()
        .id(index)
        .h(px(m.row))
        .flex()
        .items_center()
        .child(
            div()
                .w(px(m.min_panel))
                .truncate()
                .child(layer.name.clone())
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
                .h(px(m.row - m.gap))
                .bg(rgb(c.panel_secondary))
                .rounded(px(m.control_radius))
                .child(
                    div()
                        .absolute()
                        .left(px(width * from))
                        .w(px(width * span))
                        .h_full()
                        .bg(rgb(if active { c.accent } else { c.selected_fill }))
                        .rounded(px(m.control_radius)),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |v, e: &MouseDownEvent, _, cx| {
                        let x = f32::from(e.position.x) - bounds.x - m.inset - m.min_panel;
                        v.select(track_id.clone(), cx);
                        v.seek(x as f64 / width as f64 * v.duration);
                    }),
                ),
        )
}
