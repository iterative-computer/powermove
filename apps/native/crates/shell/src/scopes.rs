//! The Scopes panel: waveform, RGB parade, vectorscope and histogram of the
//! viewer's frame. Counts come from the engine's GPU analysis; [`raster`] runs
//! on a background thread and the view only places the finished picture.
use gpui::{prelude::*, *};
use pm_design::{Colors, Tokens};
use std::sync::Arc;

/// What the panel shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Waveform,
    Parade,
    Vectorscope,
    Histogram,
}

impl Kind {
    pub const ALL: [Kind; 4] = [
        Kind::Waveform,
        Kind::Parade,
        Kind::Vectorscope,
        Kind::Histogram,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Waveform => "Waveform",
            Kind::Parade => "Parade",
            Kind::Vectorscope => "Vectorscope",
            Kind::Histogram => "Histogram",
        }
    }
}

/// One frame's counters, as the engine reports them.
#[derive(Clone, Copy, Debug)]
pub struct Counts<'a> {
    pub columns: usize,
    pub levels: usize,
    pub vector_bins: usize,
    pub pixels: u64,
    pub waveform: &'a [u32],
    pub parade: [&'a [u32]; 3],
    pub vectorscope: &'a [u32],
    pub luma_histogram: &'a [u32],
    pub histogram: [&'a [u32]; 3],
}

/// A finished trace, BGRA rows, top row first.
#[derive(Clone, Debug, PartialEq)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

impl Raster {
    fn new(width: usize, height: usize, background: u32) -> Self {
        let [a, r, g, b] = (background | 0xFF00_0000).to_be_bytes();
        Raster {
            width: width as u32,
            height: height as u32,
            bgra: [b, g, r, a].repeat(width * height),
        }
    }

    fn add(&mut self, x: usize, y: usize, color: u32, amount: f32) {
        let o = (y * self.width as usize + x) * 4;
        let rgb = [color >> 16, color >> 8, color].map(|c| (c & 0xFF) as f32);
        for (c, value) in [2, 1, 0].into_iter().zip(rgb) {
            let v = f32::from(self.bgra[o + c]) + value * amount;
            self.bgra[o + c] = v.min(255.) as u8;
        }
    }
}

/// Maps a count to trace brightness: one sample shows, a dense bin saturates.
fn glow(count: u32, reference: f32) -> f32 {
    if count == 0 {
        0.
    } else {
        (0.25 + (count as f32 / reference).sqrt()).min(1.)
    }
}

fn plane(out: &mut Raster, left: usize, counts: &[u32], c: &Counts, color: u32, reference: f32) {
    let height = out.height as usize;
    for level in 0..c.levels {
        for column in 0..c.columns {
            let amount = glow(counts[level * c.columns + column], reference);
            if amount > 0. {
                out.add(left + column, height - 1 - level, color, amount);
            }
        }
    }
}

/// Draws one view of the counts with the design's trace colours.
pub fn raster(c: &Counts, kind: Kind, colors: &Colors) -> Raster {
    let bins_per_column = c.pixels as f32 / c.columns.max(1) as f32 / 16.;
    match kind {
        Kind::Waveform => {
            let mut out = Raster::new(c.columns, c.levels, colors.viewer);
            plane(&mut out, 0, c.waveform, c, colors.text, bins_per_column);
            out
        }
        Kind::Parade => {
            let mut out = Raster::new(c.columns * 3, c.levels, colors.viewer);
            let traces = [colors.scope_red, colors.scope_green, colors.scope_blue];
            for (i, (counts, color)) in c.parade.iter().zip(traces).enumerate() {
                plane(&mut out, i * c.columns, counts, c, color, bins_per_column);
            }
            out
        }
        Kind::Vectorscope => {
            let n = c.vector_bins;
            let mut out = Raster::new(n, n, colors.viewer);
            let reference = c.pixels as f32 / (n * n) as f32 * 4.;
            for row in 0..n {
                for column in 0..n {
                    let amount = glow(c.vectorscope[row * n + column], reference);
                    if amount > 0. {
                        out.add(column, n - 1 - row, colors.text, amount);
                    }
                }
            }
            out
        }
        Kind::Histogram => {
            let height = c.levels / 2;
            let mut out = Raster::new(c.levels, height, colors.viewer);
            let traces = [colors.scope_red, colors.scope_green, colors.scope_blue];
            let peak = c
                .histogram
                .iter()
                .flat_map(|h| h[1..c.levels - 1].iter())
                .copied()
                .max()
                .unwrap_or(1)
                .max(1) as f32;
            for (counts, color) in c.histogram.iter().zip(traces) {
                for (level, count) in counts.iter().enumerate() {
                    let bar = ((*count as f32 / peak).sqrt() * height as f32) as usize;
                    for y in 0..bar.min(height) {
                        out.add(level, height - 1 - y, color, 0.6);
                    }
                }
            }
            out
        }
    }
}

/// The panel's view: a picker and the newest picture.
pub struct ScopesView {
    pub tokens: Tokens,
    pub kind: Kind,
    /// Read PQ, in BT.2020 with absolute light, rather than display codes.
    pub hdr: bool,
    image: Option<Arc<RenderImage>>,
    /// Replaced pictures, released at the next paint.
    retired: Vec<Arc<RenderImage>>,
    /// When the panel last painted: a hidden panel stops the analysis.
    painted: std::time::Instant,
    /// The picker's latest choice, for the host's next raster.
    on_kind: Option<Box<dyn Fn(Kind, &mut App)>>,
}

impl ScopesView {
    pub fn new(tokens: Tokens) -> Self {
        Self {
            tokens,
            kind: Kind::default(),
            hdr: false,
            image: None,
            retired: Vec::new(),
            painted: std::time::Instant::now(),
            on_kind: None,
        }
    }

    pub fn painted(&self) -> std::time::Instant {
        self.painted
    }

    pub fn on_kind(&mut self, callback: impl Fn(Kind, &mut App) + 'static) {
        self.on_kind = Some(Box::new(callback));
    }

    /// Shows a finished trace.
    pub fn show(&mut self, raster: Raster, hdr: bool, cx: &mut Context<Self>) {
        self.hdr = hdr;
        self.retired.extend(self.image.take());
        self.image = image::RgbaImage::from_raw(raster.width, raster.height, raster.bgra)
            .map(|buffer| Arc::new(RenderImage::new([image::Frame::new(buffer)])));
        cx.notify();
    }

    pub fn has_picture(&self) -> bool {
        self.image.is_some()
    }

    fn choose(&mut self, kind: Kind, cx: &mut Context<Self>) {
        self.kind = kind;
        if let Some(callback) = &self.on_kind {
            callback(kind, cx);
        }
        cx.notify();
    }
}

impl Render for ScopesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.painted = std::time::Instant::now();
        for image in self.retired.drain(..) {
            window.drop_image(image).ok();
        }
        let t = &self.tokens;
        let (c, m, ty) = (&t.colors, &t.metrics, t.typography);
        let mut picker = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(m.gap))
            .px(px(m.inset))
            .h(px(m.row));
        for kind in Kind::ALL {
            let selected = kind == self.kind;
            picker = picker.child(
                div()
                    .id(kind.label())
                    .role(Role::Button)
                    .aria_label(kind.label())
                    .tab_index(0)
                    .px(px(m.gap))
                    .rounded(px(m.control_radius))
                    .text_size(px(ty.reading))
                    .text_color(rgb(if selected { c.text } else { c.secondary }))
                    .when(selected, |d| d.bg(rgb(c.selected_fill)))
                    .child(kind.label())
                    .on_click(cx.listener(move |v, _, _, cx| v.choose(kind, cx))),
            );
        }
        let picture = div()
            .flex_1()
            .min_h_0()
            .w_full()
            .bg(rgb(c.viewer))
            .flex()
            .items_center()
            .justify_center();
        let picture = match &self.image {
            Some(image) => {
                picture.child(img(image.clone()).size_full().object_fit(ObjectFit::Fill))
            }
            None => picture.child(crate::view::empty_state(
                t,
                crate::layout::empty_state("scopes").0,
                crate::layout::empty_state("scopes").1,
            )),
        };
        let scale = div()
            .h(px(m.row))
            .px(px(m.inset))
            .flex()
            .items_center()
            .text_size(px(ty.reading))
            .text_color(rgb(c.muted))
            .child(if self.hdr {
                "BT.2100 PQ, BT.2020"
            } else {
                "Display code values, Rec.709"
            });
        div()
            .id("scopes")
            .size_full()
            .flex()
            .flex_col()
            .child(picker)
            .child(picture)
            .child(scale)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, point};

    fn tokens() -> Tokens {
        Tokens::new(pm_design::Scheme::Dark, Default::default())
    }

    fn app() -> HeadlessAppContext {
        HeadlessAppContext::with_platform(
            Arc::new(gpui_parley::ParleyTextSystem::new(
                gpui_parley::SystemFonts::Load,
            )),
            Arc::new(()),
            || None,
        )
    }

    fn counts<'a>(
        waveform: &'a [u32],
        parade: [&'a [u32]; 3],
        vector: &'a [u32],
        histogram: [&'a [u32]; 3],
        luma: &'a [u32],
    ) -> Counts<'a> {
        Counts {
            columns: 4,
            levels: 8,
            vector_bins: 4,
            pixels: 32,
            waveform,
            parade,
            vectorscope: vector,
            luma_histogram: luma,
            histogram,
        }
    }

    #[::core::prelude::v1::test]
    fn rasters_place_levels_low_to_high_and_colour_each_channel() {
        let t = tokens();
        let mut bright = [0u32; 32];
        bright[7 * 4 + 1] = 64; // Top level, second column.
        let empty = [0u32; 32];
        let vector = [0u32; 16];
        let hist = [0u32; 8];
        let view = counts(
            &bright,
            [&empty, &bright, &empty],
            &vector,
            [&hist; 3],
            &hist,
        );
        let waveform = raster(&view, Kind::Waveform, &t.colors);
        assert_eq!((waveform.width, waveform.height), (4, 8));
        let background = Raster::new(1, 1, t.colors.viewer).bgra;
        let lit = (0..waveform.bgra.len() / 4)
            .find(|i| waveform.bgra[i * 4..i * 4 + 4] != background[..])
            .unwrap();
        assert_eq!(lit, 1, "the top row is the highest level");
        let parade = raster(&view, Kind::Parade, &t.colors);
        assert_eq!(parade.width, 12);
        let green = |x: usize| parade.bgra[x * 4 + 1];
        assert!(
            green(5) > green(1),
            "the green trace sits in the middle third"
        );
    }

    #[::core::prelude::v1::test]
    fn a_real_click_and_the_keyboard_choose_a_scope() {
        let mut cx = app();
        let window = cx
            .open_window(size(px(640.), px(360.)), |_, cx| {
                cx.new(|_| ScopesView::new(tokens()))
            })
            .unwrap();
        let chosen = Arc::new(std::sync::Mutex::new(vec![]));
        let sink = chosen.clone();
        window
            .update(&mut cx, |v, _, _| {
                v.on_kind(move |kind, _| sink.lock().unwrap().push(kind));
            })
            .unwrap();
        cx.run_until_parked();
        // Sweep the picker row with a pointer until a segment answers.
        let y = px(tokens().metrics.row / 2.);
        for x in (0..640).step_by(8) {
            let position = point(px(x as f32), y);
            cx.update_window(window.into(), |_, w, cx| {
                for event in [
                    MouseDownEvent {
                        button: MouseButton::Left,
                        position,
                        modifiers: Modifiers::default(),
                        click_count: 1,
                        first_mouse: false,
                    }
                    .to_platform_input(),
                    MouseUpEvent {
                        button: MouseButton::Left,
                        position,
                        modifiers: Modifiers::default(),
                        click_count: 1,
                    }
                    .to_platform_input(),
                ] {
                    w.dispatch_event(event, cx);
                }
            })
            .unwrap();
            cx.run_until_parked();
        }
        let clicked = chosen.lock().unwrap().clone();
        for kind in Kind::ALL {
            assert!(
                clicked.contains(&kind),
                "{kind:?} takes a click: {clicked:?}"
            );
        }
        window
            .update(&mut cx, |_, w, cx| {
                w.focus_next(cx);
                w.focus_next(cx);
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(window.into(), |_, w, cx| {
            let keystroke = Keystroke::parse("enter").unwrap();
            w.dispatch_event(
                KeyDownEvent {
                    keystroke: keystroke.clone(),
                    is_held: false,
                    prefer_character_input: false,
                }
                .to_platform_input(),
                cx,
            );
            w.dispatch_event(KeyUpEvent { keystroke }.to_platform_input(), cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(chosen.lock().unwrap().last(), Some(&Kind::Parade));
    }

    #[::core::prelude::v1::test]
    fn a_picture_replaces_the_empty_state_and_states_its_signal() {
        let mut cx = app();
        let window = cx
            .open_window(size(px(640.), px(360.)), |_, cx| {
                cx.new(|_| ScopesView::new(tokens()))
            })
            .unwrap();
        window
            .update(&mut cx, |v, _, cx| {
                assert!(!v.has_picture());
                v.show(
                    Raster {
                        width: 2,
                        height: 2,
                        bgra: vec![255; 16],
                    },
                    true,
                    cx,
                );
                assert!(v.has_picture() && v.hdr);
            })
            .unwrap();
        cx.run_until_parked();
    }
}
