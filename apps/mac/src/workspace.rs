//! The editor window's GPUI root: sidebar, viewer, inspector, timeline.
//!
//! Spike: layout and the glass integration are real; panel contents are
//! placeholders until each lane ports its panel (docs/gpui/lanes.md).

use std::time::Duration;

use gpui::{
    Bounds, Context, FontWeight, InteractiveElement as _, IntoElement, MouseButton, ParentElement, Pixels, Render, SharedString,
    StatefulInteractiveElement as _, Styled, Window, canvas, div, linear_color_stop, linear_gradient, prelude::FluentBuilder as _, px, rgb,
};

use pm_glass::{self as glass, Action, Contents, Layer, Rect, Tool};
use pm_kit::theme;

const FPS: f64 = 30.0;
const DURATION: f64 = 12.0;

struct LayerRow {
    name: &'static str,
    kind: &'static str,
    start: f64,
    end: f64,
    color: u32,
}

const LAYERS: [LayerRow; 6] = [
    LayerRow { name: "Headline", kind: "textformat", start: 1.0, end: 7.5, color: 0xFF6019 },
    LayerRow { name: "iPad mini", kind: "cube", start: 0.0, end: 12.0, color: 0x7A8CFF },
    LayerRow { name: "Light sweep", kind: "light.max", start: 2.0, end: 6.0, color: 0xF5C542 },
    LayerRow { name: "Camera", kind: "video", start: 0.0, end: 12.0, color: 0x52C7A8 },
    LayerRow { name: "Backdrop", kind: "square.fill", start: 0.0, end: 12.0, color: 0x8E8A86 },
    LayerRow { name: "Score.wav", kind: "waveform", start: 0.0, end: 11.0, color: 0x5AA9FF },
];

const HUD: &[(&str, &str, Action)] = &[
    ("minus.magnifyingglass", "Zoom Out", Action::ZoomOut),
    ("arrow.up.left.and.down.right.magnifyingglass", "Fit", Action::ZoomFit),
    ("plus.magnifyingglass", "Zoom In", Action::ZoomIn),
];

pub struct Workspace {
    sidebar: bool,
    inspector: bool,
    playing: bool,
    time: f64,
    zoom: f32,
    tool: Tool,
    selected: usize,
}

fn rect(b: Bounds<Pixels>) -> Rect {
    Rect { x: f64::from(b.origin.x), y: f64::from(b.origin.y), w: f64::from(b.size.width), h: f64::from(b.size.height) }
}

/// An absolutely positioned glass region filling its parent.
fn glass_region(id: &'static str, radius: f32, layer: Layer, contents: Contents) -> impl IntoElement {
    canvas(move |b, _, _| glass::place(id, rect(b), radius as f64, layer, contents), |_, _, _, _| {}).absolute().size_full()
}

impl Workspace {
    pub fn new(actions: async_channel::Receiver<Action>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.spawn(async move |this, cx| {
            while let Ok(a) = actions.recv().await {
                if this.update(cx, |w, cx| w.handle(a, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        Self { sidebar: true, inspector: true, playing: false, time: 2.4, zoom: 0.62, tool: Tool::Select, selected: 0 }
    }

    fn handle(&mut self, a: Action, cx: &mut Context<Self>) {
        match a {
            Action::Tool(t) => self.tool = t,
            Action::ToggleSidebar => self.sidebar = !self.sidebar,
            Action::ToggleInspector => self.inspector = !self.inspector,
            Action::ZoomIn => self.zoom = (self.zoom * 1.25).min(4.0),
            Action::ZoomOut => self.zoom = (self.zoom / 1.25).max(0.1),
            Action::ZoomFit => self.zoom = 0.62,
            Action::PlayPause => {
                self.playing = !self.playing;
                if self.playing {
                    self.play(cx);
                }
            }
            Action::ToggleAgent | Action::Export => {}
        }
        cx.notify();
    }

    fn play(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs_f64(1.0 / FPS)).await;
                let go = this
                    .update(cx, |w, cx| {
                        if w.playing {
                            w.time = (w.time + 1.0 / FPS) % DURATION;
                            cx.notify();
                        }
                        w.playing
                    })
                    .unwrap_or(false);
                if !go {
                    break;
                }
            }
        })
        .detach();
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = LAYERS.iter().enumerate().map(|(i, l)| {
            let selected = i == self.selected;
            div()
                .id(("layer", i))
                .h(px(theme::ROW_H))
                .mx_2()
                .px_2()
                .rounded(px(8.0))
                .flex()
                .items_center()
                .gap_2()
                .when(selected, |d| d.bg(theme::accent_dim()))
                .hover(|d| d.bg(gpui::white().opacity(0.06)))
                .on_click(cx.listener(move |w, _, _, cx| {
                    w.selected = i;
                    cx.notify();
                }))
                .child(div().size(px(8.0)).rounded_full().bg(rgb(l.color)))
                .child(div().text_sm().text_color(if selected { theme::text() } else { theme::text_2() }).child(l.name))
        });
        div()
            .relative()
            .w(px(theme::SIDEBAR_W))
            .h_full()
            .flex_none()
            .child(glass_region("sidebar", theme::PANEL_RADIUS, Layer::Behind, Contents::None))
            .child(
                div()
                    .relative()
                    .size_full()
                    .flex()
                    .flex_col()
                    .pt_3()
                    .gap_0p5()
                    .child(section_title("Layers"))
                    .children(rows)
                    .child(div().h_4())
                    .child(section_title("Library"))
                    .child(div().px_4().text_xs().text_color(theme::text_3()).child("Drop media here or press ⌘I")),
            )
    }

    fn inspector(&self) -> impl IntoElement {
        let l = &LAYERS[self.selected];
        div()
            .relative()
            .w(px(theme::INSPECTOR_W))
            .h_full()
            .flex_none()
            .child(glass_region("inspector", theme::PANEL_RADIUS, Layer::Behind, Contents::None))
            .child(
                div()
                    .relative()
                    .size_full()
                    .flex()
                    .flex_col()
                    .p_3()
                    .gap_3()
                    .child(div().text_base().font_weight(FontWeight::SEMIBOLD).text_color(theme::text()).child(l.name))
                    .child(group("Transform", &[("Position", "960, 540"), ("Scale", "100%"), ("Rotation", "0°"), ("Opacity", "100%")]))
                    .child(group("Timing", &[("In", &timecode(l.start)), ("Out", &timecode(l.end))])),
            )
    }

    fn viewer(&self) -> impl IntoElement {
        let w = 1920.0 * self.zoom * 0.5;
        let h = 1080.0 * self.zoom * 0.5;
        let t = (self.time / DURATION) as f32;
        div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::sunken())
            .rounded(px(theme::PANEL_RADIUS))
            .overflow_hidden()
            .child(
                // Placeholder for the engine frame (IOSurface → GPUI surface).
                div()
                    .w(px(w))
                    .h(px(h))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(linear_gradient(135.0 + 90.0 * t, linear_color_stop(rgb(0x1B1F3A), 0.0), linear_color_stop(rgb(0x3A1B12), 1.0)))
                    .child(div().text_size(px(48.0 * self.zoom)).font_weight(FontWeight::BOLD).text_color(gpui::white()).child("The new iPad mini")),
            )
            .child(
                div()
                    .absolute()
                    .bottom_3()
                    .w(px(116.0))
                    .h(px(36.0))
                    .child(glass_region("viewer-hud", 18.0, Layer::Floating, Contents::Buttons(HUD))),
            )
            .child(
                div()
                    .absolute()
                    .top_3()
                    .left_3()
                    .text_xs()
                    .text_color(theme::text_3())
                    .child(SharedString::from(format!("{:?} · {}%", self.tool, (self.zoom * 100.0 / 0.62 * 0.5).round() as i32 * 2))),
            )
    }

    fn timeline(&self) -> impl IntoElement {
        let label_w = 168.0;
        let px_per_s = 80.0;
        let playhead = label_w + self.time as f32 * px_per_s;
        let ruler = (0..=DURATION as i32).map(|s| {
            div().absolute().left(px(label_w + s as f32 * px_per_s)).top_1().text_xs().text_color(theme::text_3()).child(format!("{s}s"))
        });
        let rows = LAYERS.iter().enumerate().map(|(i, l)| {
            div()
                .relative()
                .h(px(26.0))
                .flex()
                .items_center()
                .when(i == self.selected, |d| d.bg(gpui::white().opacity(0.03)))
                .child(div().w(px(label_w)).px_3().text_xs().text_color(theme::text_2()).child(l.name))
                .child(
                    div()
                        .absolute()
                        .left(px(label_w + l.start as f32 * px_per_s))
                        .w(px((l.end - l.start) as f32 * px_per_s))
                        .h(px(20.0))
                        .rounded(px(5.0))
                        .bg(rgb(l.color).into_hsla_opacity(if i == self.selected { 0.85 } else { 0.45 })),
                )
        });
        div()
            .relative()
            .h(px(theme::TIMELINE_H))
            .flex_none()
            .bg(theme::panel())
            .rounded(px(theme::PANEL_RADIUS))
            .overflow_hidden()
            .child(
                div()
                    .h(px(36.0))
                    .flex()
                    .items_center()
                    .px_3()
                    .gap_3()
                    .border_b_1()
                    .border_color(theme::hairline())
                    .child(div().text_sm().font_weight(FontWeight::MEDIUM).text_color(theme::text()).child(timecode(self.time)))
                    .child(div().text_xs().text_color(theme::text_3()).child(if self.playing { "Playing" } else { "Paused · Space to play" })),
            )
            .child(div().relative().h(px(22.0)).children(ruler))
            .child(div().flex().flex_col().gap_0p5().children(rows))
            .child(div().absolute().top(px(36.0)).bottom_0().left(px(playhead)).w(px(1.5)).bg(theme::accent()))
    }
}

trait Opacity {
    fn into_hsla_opacity(self, o: f32) -> gpui::Hsla;
}
impl Opacity for gpui::Rgba {
    fn into_hsla_opacity(self, o: f32) -> gpui::Hsla {
        gpui::Hsla::from(self).opacity(o)
    }
}

fn timecode(t: f64) -> String {
    let frames = (t * FPS).round() as i64;
    format!("00:{:02}:{:02}:{:02}", frames / (FPS as i64 * 60), (frames / FPS as i64) % 60, frames % FPS as i64)
}

fn section_title(t: &'static str) -> impl IntoElement {
    div().px_4().pb_1().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(theme::text_3()).child(t)
}

fn group(title: &'static str, rows: &[(&str, &str)]) -> impl IntoElement {
    div().flex().flex_col().gap_1().child(div().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(theme::text_3()).child(title)).children(rows.iter().map(
        |(k, v)| {
            div()
                .h(px(26.0))
                .flex()
                .items_center()
                .justify_between()
                .child(div().text_sm().text_color(theme::text_2()).child(SharedString::from(k.to_string())))
                .child(div().w(px(120.0)).h(px(22.0)).px_2().rounded(px(6.0)).bg(theme::field()).text_sm().text_color(theme::text()).child(SharedString::from(v.to_string())))
        },
    ))
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let top = glass::toolbar_inset() as f32;
        let inset = theme::INSET;
        div()
            .size_full()
            .flex()
            .gap(px(inset))
            .pt(px(top))
            .px(px(inset))
            .pb(px(inset))
            .when(self.sidebar, |d| d.child(self.sidebar(cx)))
            .child(div().flex_1().min_w_0().h_full().flex().flex_col().gap(px(inset)).child(self.viewer()).child(self.timeline()))
            .when(self.inspector, |d| d.child(self.inspector()))
            .on_mouse_down(MouseButton::Left, |_, _, _| {})
            // Last: hide glass regions this frame did not place.
            .child(canvas(|_, _, _| glass::finish_frame(), |_, _, _, _| {}).absolute().size_0())
    }
}
