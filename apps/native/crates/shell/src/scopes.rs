//! The Scopes panel (design/surfaces/colour.md, N004): waveform, RGB parade,
//! vectorscope and histogram of the viewer's frame. Counts come from the
//! engine's GPU analysis; [`raster`] runs on a background thread and the view
//! only places the finished picture.
use gpui::{prelude::*, *};
use pm_design::{Colors, Tokens};
use std::sync::Arc;

/// What the panel shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    Histogram,
    #[default]
    Waveform,
    Parade,
    Vectorscope,
}

impl Kind {
    pub const ALL: [Kind; 4] = [
        Kind::Histogram,
        Kind::Waveform,
        Kind::Parade,
        Kind::Vectorscope,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Waveform => "Waveform",
            Kind::Parade => "RGB Parade",
            Kind::Vectorscope => "Vectorscope",
            Kind::Histogram => "Histogram",
        }
    }
}

/// Graticule levels: (fraction of the signal range, label).
pub fn scale(extended: bool) -> &'static [(f32, &'static str)] {
    if extended {
        // ST 2084 signal of each light level, in cd/m².
        &[
            (0.0, "0"),
            (0.5081, "100"),
            (0.5807, "SDR White"),
            (0.7518, "1000"),
            (0.9026, "4000"),
            (1.0, "10000"),
        ]
    } else {
        &[
            (0.0, "0"),
            (0.25, "25"),
            (0.5, "50"),
            (0.75, "75"),
            (1.0, "100"),
        ]
    }
}

/// One frame's counters, as the engine reports them.
#[derive(Clone, Copy, Debug)]
pub struct Counts<'a> {
    /// The counters hold BT.2100 PQ, not display code values.
    pub extended: bool,
    pub rec2020: bool,
    pub columns: usize,
    pub levels: usize,
    pub vector_bins: usize,
    pub pixels: u64,
    pub waveform: &'a [u32],
    pub parade: [&'a [u32]; 3],
    pub vectorscope: &'a [u32],
    pub luma_histogram: &'a [u32],
    pub histogram: [&'a [u32]; 3],
    pub under: u32,
    pub over: u32,
    pub outside: u32,
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

/// Horizontal lines at the signal levels, under the trace.
fn level_lines(out: &mut Raster, extended: bool, color: u32) {
    let (width, height) = (out.width as usize, out.height as usize);
    for (fraction, _) in scale(extended) {
        let y = ((1.0 - fraction) * (height - 1) as f32).round() as usize;
        for x in 0..width {
            out.add(x, y, color, 0.5);
        }
    }
}

/// The skin-tone line's angle on the component vectorscope, counter-clockwise
/// from +Cb: 123°, between the red (103°) and yellow (167°) targets, where
/// every complexion's hue falls (the "I" line Resolve, Final Cut and
/// Premiere draw).
pub const SKIN_TONE_DEGREES: f32 = 123.0;

/// The vectorscope's frame: the codable circle, centre cross, the skin-tone
/// line and the 75 % Rec.709 primary and secondary targets.
fn vector_lines(out: &mut Raster, c: &Counts, color: u32) {
    let n = c.vector_bins;
    let middle = (n - 1) as f32 / 2.0;
    for i in 0..n {
        out.add(i, n / 2, color, 0.25);
        out.add(n / 2, i, color, 0.25);
    }
    // Rows run from +Cr at the top.
    let (cos, sin) = (
        SKIN_TONE_DEGREES.to_radians().cos(),
        SKIN_TONE_DEGREES.to_radians().sin(),
    );
    for step in 0..n * 2 {
        let r = step as f32 / (n * 2) as f32 * middle;
        let (x, y) = (middle + r * cos, middle - r * sin);
        out.add(x.round() as usize, y.round() as usize, color, 0.4);
    }
    for step in 0..n * 8 {
        let a = step as f32 / (n * 8) as f32 * core::f32::consts::TAU;
        let (x, y) = (middle * (1.0 + a.cos()), middle * (1.0 + a.sin()));
        out.add(x as usize, y as usize, color, 0.12);
    }
    if !c.rec2020 {
        let (kr, kg, kb) = (0.2126f32, 0.7152f32, 0.0722f32);
        for rgb in [
            [0.75, 0.0, 0.0],
            [0.0, 0.75, 0.0],
            [0.0, 0.0, 0.75],
            [0.75, 0.75, 0.0],
            [0.0, 0.75, 0.75],
            [0.75, 0.0, 0.75],
        ] {
            let y = kr * rgb[0] + kg * rgb[1] + kb * rgb[2];
            let cb = (rgb[2] - y) / (2.0 * (1.0 - kb));
            let cr = (rgb[0] - y) / (2.0 * (1.0 - kr));
            let (x, row) = (
                ((cb + 0.5) * (n - 1) as f32) as usize,
                ((0.5 - cr) * (n - 1) as f32) as usize,
            );
            for d in 0..=3usize {
                for (dx, dy) in [(d, 0), (d, 3), (0, d), (3, d)] {
                    out.add(
                        (x + dx).saturating_sub(1).min(n - 1),
                        (row + dy).saturating_sub(1).min(n - 1),
                        color,
                        0.6,
                    );
                }
            }
        }
    }
}

/// Maps a count to trace brightness on a log scale: one sample shows, a dense
/// bin saturates.
fn glow(count: u32, reference: f32) -> f32 {
    if count == 0 {
        0.
    } else {
        (0.25 + (1. + count as f32).ln() / (1. + reference).ln()).min(1.)
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
            level_lines(&mut out, c.extended, colors.separator);
            plane(&mut out, 0, c.waveform, c, colors.text, bins_per_column);
            out
        }
        Kind::Parade => {
            let mut out = Raster::new(c.columns * 3, c.levels, colors.viewer);
            level_lines(&mut out, c.extended, colors.separator);
            let traces = [colors.scope_red, colors.scope_green, colors.scope_blue];
            for (i, (counts, color)) in c.parade.iter().zip(traces).enumerate() {
                plane(&mut out, i * c.columns, counts, c, color, bins_per_column);
            }
            out
        }
        Kind::Vectorscope => {
            let n = c.vector_bins;
            let mut out = Raster::new(n, n, colors.viewer);
            vector_lines(&mut out, c, colors.secondary);
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
            for (fraction, _) in scale(c.extended) {
                let x = (fraction * (c.levels - 1) as f32).round() as usize;
                for y in 0..height {
                    out.add(x, y, colors.separator, 0.5);
                }
            }
            let traces = [colors.scope_red, colors.scope_green, colors.scope_blue];
            let peak = c
                .histogram
                .iter()
                .flat_map(|h| h[1..c.levels - 1].iter())
                .copied()
                .max()
                .unwrap_or(1)
                .max(1) as f32;
            let bar = |count: u32| ((count as f32 / peak).sqrt() * height as f32) as usize;
            for (counts, color) in c.histogram.iter().zip(traces) {
                for (level, count) in counts.iter().enumerate() {
                    for y in 0..bar(*count).min(height) {
                        out.add(level, height - 1 - y, color, 0.6);
                    }
                }
            }
            for (level, count) in c.luma_histogram.iter().enumerate() {
                let top = bar(*count).min(height - 1);
                out.add(level, height - 1 - top, colors.text, 0.8);
            }
            out
        }
    }
}

/// What the frame says at a glance: the footer's counts and the plot's value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reading {
    /// `{stage} · {matrix}`.
    pub stage: String,
    /// `Under {n} %` and `Over {n} %`, or `Outside {n} %` on the vectorscope.
    pub limits: Vec<String>,
    pub peak: u32,
    pub minimum: u32,
}

fn percent(count: u32, pixels: u64) -> String {
    let share = f64::from(count) / pixels.max(1) as f64 * 100.;
    if share < 0.1 {
        "< 0.1".into()
    } else {
        format!("{share:.1}")
    }
}

/// Reads the footer and the plot's accessibility value from the counts.
pub fn reading(c: &Counts, kind: Kind) -> Reading {
    let mut limits = vec![];
    if kind == Kind::Vectorscope {
        if c.outside > 0 {
            limits.push(format!("Outside {} %", percent(c.outside, c.pixels)));
        }
    } else {
        if c.under > 0 {
            limits.push(format!("Under {} %", percent(c.under, c.pixels)));
        }
        if c.over > 0 {
            limits.push(format!("Over {} %", percent(c.over, c.pixels)));
        }
    }
    let level = |bin: usize| (bin * 100 / c.levels.saturating_sub(1).max(1)) as u32;
    let lit = |bin: &usize| c.luma_histogram[*bin] > 0;
    let bins = 0..c.luma_histogram.len();
    Reading {
        stage: format!(
            "Output · {}",
            if c.rec2020 { "Rec. 2020" } else { "Rec. 709" }
        ),
        limits,
        peak: bins.clone().rev().find(lit).map_or(0, level),
        minimum: bins.clone().find(lit).map_or(0, level),
    }
}

/// The header control that asks the host for a native menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Menu {
    Scope,
    More,
}

type MenuRequest = Box<dyn Fn(Menu, &mut Window, &mut App)>;

/// The plot's errors.
pub const READ_FAILED: &str = "Scopes can’t read this frame.";
pub const INVALID_FRAME: &str = "This frame has invalid values.";

/// The panel's view: the newest picture and its footer. Header controls ask the
/// host for AppKit menus; settings live here, per panel.
pub struct ScopesView {
    pub tokens: Tokens,
    pub kind: Kind,
    pub rec2020: bool,
    /// The extended range: PQ, with absolute light on the axis.
    pub extended: bool,
    image: Option<Arc<RenderImage>>,
    reading: Option<Reading>,
    error: Option<SharedString>,
    /// Replaced pictures, released at the next paint.
    retired: Vec<Arc<RenderImage>>,
    /// When the panel last painted: a hidden panel stops the analysis.
    painted: std::time::Instant,
    on_menu: Option<MenuRequest>,
}

impl ScopesView {
    pub fn new(tokens: Tokens) -> Self {
        Self {
            tokens,
            kind: Kind::default(),
            rec2020: false,
            extended: false,
            image: None,
            reading: None,
            error: None,
            retired: Vec::new(),
            painted: std::time::Instant::now(),
            on_menu: None,
        }
    }

    pub fn painted(&self) -> std::time::Instant {
        self.painted
    }

    pub fn on_menu(&mut self, request: impl Fn(Menu, &mut Window, &mut App) + 'static) {
        self.on_menu = Some(Box::new(request));
    }

    /// Shows a finished trace.
    pub fn show(&mut self, raster: Raster, reading: Reading, cx: &mut Context<Self>) {
        self.error = None;
        self.reading = Some(reading);
        self.retired.extend(self.image.take());
        self.image = image::RgbaImage::from_raw(raster.width, raster.height, raster.bgra)
            .map(|buffer| Arc::new(RenderImage::new([image::Frame::new(buffer)])));
        cx.notify();
    }

    /// Clears the plot: nothing stale stays up beside an error.
    pub fn fail(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.error = Some(message.into());
        self.reading = None;
        self.retired.extend(self.image.take());
        cx.notify();
    }

    pub fn has_picture(&self) -> bool {
        self.image.is_some()
    }

    /// The panel header's controls: Scope pop-up and More.
    pub fn header(&self, cx: &mut Context<Self>) -> Div {
        let t = &self.tokens;
        let (c, m, ty) = (&t.colors, &t.metrics, t.typography);
        let button = |id: &'static str, label: String, ax: &'static str, menu: Menu| {
            div()
                .id(id)
                .role(Role::Button)
                .aria_label(ax)
                .tab_index(0)
                .px(px(m.gap))
                .rounded(px(m.control_radius))
                .text_size(px(ty.reading))
                .text_color(rgb(c.secondary))
                .child(label)
                .on_click(cx.listener(move |v, _, window, cx| {
                    if let Some(request) = &v.on_menu {
                        request(menu, window, cx);
                    }
                }))
        };
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_end()
            .gap(px(m.gap))
            .h_full()
            .child(button(
                "scope-kind",
                format!("{} ⌄", self.kind.label()),
                "Scope",
                Menu::Scope,
            ))
            .child(button("scope-more", "⋯".into(), "More", Menu::More))
    }
}

impl Render for ScopesView {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.painted = std::time::Instant::now();
        for image in self.retired.drain(..) {
            window.drop_image(image).ok();
        }
        let t = &self.tokens;
        let (c, m, ty) = (&t.colors, &t.metrics, t.typography);
        let label = |text: SharedString| {
            div()
                .text_size(px(ty.minimum))
                .text_color(rgb(c.muted))
                .child(text)
        };
        let value = self.reading.as_ref().map_or(String::new(), |r| {
            format!(", Peak {} percent, minimum {} percent", r.peak, r.minimum)
        });
        let mut plot = div()
            .id("scope-plot")
            .role(Role::Image)
            .aria_label(format!("{} of the current frame{value}", self.kind.label()))
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .rounded(px(m.control_radius))
            .overflow_hidden()
            .bg(rgb(c.viewer))
            .flex()
            .items_center()
            .justify_center();
        plot = match (&self.image, &self.error) {
            (_, Some(error)) => plot.child(
                div()
                    .text_size(px(ty.reading))
                    .text_color(rgb(c.danger))
                    .child(error.clone()),
            ),
            (Some(image), _) => {
                plot.child(img(image.clone()).size_full().object_fit(ObjectFit::Fill))
            }
            _ => plot.child(crate::view::empty_state(
                t,
                crate::layout::empty_state("scopes").0,
                crate::layout::empty_state("scopes").1,
            )),
        };
        let traces = self.image.is_some() && self.error.is_none();
        if traces && matches!(self.kind, Kind::Waveform | Kind::Parade) {
            plot = plot.children(scale(self.extended).iter().map(|(fraction, text)| {
                label((*text).into())
                    .absolute()
                    .left(px(m.gap))
                    .top(relative(1.0 - fraction))
                    .mt(px(if *fraction > 0.5 {
                        0.
                    } else {
                        -ty.minimum * 1.3
                    }))
            }));
        }
        if traces && self.kind == Kind::Parade {
            plot = plot.children(["R", "G", "B"].into_iter().enumerate().map(|(i, name)| {
                label(name.into())
                    .absolute()
                    .top(px(m.gap))
                    .left(relative(i as f32 / 3. + 0.15))
            }));
        }
        let mut footer = div()
            .h(px(m.row))
            .px(px(m.gap))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .text_size(px(ty.minimum))
            .text_color(rgb(c.secondary));
        if let Some(reading) = &self.reading {
            footer = footer
                .child(reading.stage.clone())
                .child(reading.limits.join("  "));
        }
        div()
            .id("scopes")
            .size_full()
            .p(px(m.gap))
            .flex()
            .flex_col()
            .child(plot)
            .child(footer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, point};

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
            extended: false,
            rec2020: false,
            columns: 4,
            levels: 8,
            vector_bins: 4,
            pixels: 32,
            waveform,
            parade,
            vectorscope: vector,
            luma_histogram: luma,
            histogram,
            under: 0,
            over: 0,
            outside: 0,
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
        let trace = |p: &[u8]| p[1];
        let lit = (0..4)
            .max_by_key(|x| trace(&waveform.bgra[x * 4..x * 4 + 4]))
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
    fn the_graticule_follows_the_signal() {
        let t = tokens();
        let empty = [0u32; 32];
        let vector = [0u32; 16];
        let hist = [0u32; 8];
        let mut view = counts(&empty, [&empty; 3], &vector, [&hist; 3], &hist);
        let lines = |view: &Counts| {
            let image = raster(view, Kind::Waveform, &t.colors);
            let background = Raster::new(1, 1, t.colors.viewer).bgra;
            (0..8)
                .filter(|y| image.bgra[y * 16..y * 16 + 4] != background[..])
                .count()
        };
        assert_eq!(scale(false).len(), 5);
        assert!(lines(&view) >= 4);
        view.extended = true;
        assert!(scale(true).iter().any(|(_, label)| *label == "SDR White"));
        assert!(lines(&view) >= 4);
    }

    /// The skin-tone line runs from the centre into the upper left, between
    /// the red and yellow targets: a typical complexion lands on it.
    #[::core::prelude::v1::test]
    fn the_vectorscope_draws_the_skin_tone_line_where_skin_falls() {
        let t = tokens();
        let empty = [0u32; 32];
        let vector = [0u32; 64 * 64];
        let hist = [0u32; 8];
        let mut view = counts(&empty, [&empty; 3], &vector, [&hist; 3], &hist);
        view.vector_bins = 64;
        let image = raster(&view, Kind::Vectorscope, &t.colors);
        let background = Raster::new(1, 1, t.colors.viewer).bgra;
        let lit = |x: usize, y: usize| {
            image.bgra[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4] != background[..]
        };
        // A complexion (sRGB 0.80, 0.60, 0.50) in Rec.709 Cb, Cr.
        let (r, g, b) = (0.80f32, 0.60f32, 0.50f32);
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let (cb, cr) = ((b - y) / 1.8556, (r - y) / 1.5748);
        let angle = cr.atan2(cb).to_degrees();
        assert!((angle - SKIN_TONE_DEGREES).abs() < 10.0, "skin at {angle}°");
        // The line is drawn along that direction, and not opposite it.
        let at = |s: f32| {
            let m = 63.0 / 2.0;
            let a = SKIN_TONE_DEGREES.to_radians();
            (
                (m + s * m * a.cos()).round() as usize,
                (m - s * m * a.sin()).round() as usize,
            )
        };
        for s in [0.4, 0.7, 0.9] {
            let (x, y) = at(s);
            assert!(lit(x, y), "the line at {s} of the radius");
        }
        let (x, y) = at(-0.7);
        assert!(!lit(x, y), "nothing opposite");
    }

    #[::core::prelude::v1::test]
    fn the_footer_states_the_stage_and_only_nonzero_limits() {
        let empty = [0u32; 32];
        let vector = [0u32; 16];
        let mut luma = [0u32; 8];
        luma[1] = 16;
        luma[6] = 16;
        let hist = [0u32; 8];
        let mut view = counts(&empty, [&empty; 3], &vector, [&hist; 3], &luma);
        let quiet = reading(&view, Kind::Waveform);
        assert_eq!(quiet.stage, "Output · Rec. 709");
        assert!(quiet.limits.is_empty());
        assert_eq!((quiet.peak, quiet.minimum), (85, 14));
        view.over = 8;
        view.under = 1;
        view.outside = 2;
        view.rec2020 = true;
        let loud = reading(&view, Kind::Waveform);
        assert_eq!(loud.stage, "Output · Rec. 2020");
        assert_eq!(loud.limits, ["Under 3.1 %", "Over 25.0 %"]);
        assert_eq!(reading(&view, Kind::Vectorscope).limits, ["Outside 6.2 %"]);
    }

    #[::core::prelude::v1::test]
    fn real_clicks_on_the_header_ask_for_its_menus() {
        struct Header(Entity<ScopesView>);
        impl Render for Header {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                self.0.update(cx, |view, cx| view.header(cx))
            }
        }
        let mut cx = app();
        let chosen = Arc::new(std::sync::Mutex::new(vec![]));
        let sink = chosen.clone();
        let window = cx
            .open_window(size(px(320.), px(32.)), move |_, cx| {
                let view = cx.new(|_| {
                    let mut view = ScopesView::new(tokens());
                    view.on_menu(move |menu, _, _| sink.lock().unwrap().push(menu));
                    view
                });
                cx.new(|_| Header(view))
            })
            .unwrap();
        cx.run_until_parked();
        for x in (0..320).step_by(6) {
            let position = point(px(x as f32), px(16.));
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
        let asked = chosen.lock().unwrap().clone();
        assert!(asked.contains(&Menu::Scope), "{asked:?}");
        assert!(asked.contains(&Menu::More), "{asked:?}");
    }

    #[::core::prelude::v1::test]
    fn a_picture_replaces_the_empty_state_and_an_error_clears_it() {
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
                    Reading {
                        stage: "Output · Rec. 709".into(),
                        limits: vec![],
                        peak: 100,
                        minimum: 0,
                    },
                    cx,
                );
                assert!(v.has_picture());
                v.fail(INVALID_FRAME, cx);
                assert!(!v.has_picture());
            })
            .unwrap();
        cx.run_until_parked();
    }
}
