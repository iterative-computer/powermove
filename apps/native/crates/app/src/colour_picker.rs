//! Anchored v2 colour editor. Pointer work stays native; no JS round trip.
use crate::colour_model::Color;
thread_local! {static PREFERENCES: std::cell::Cell<(bool, &'static str, u8)> = const { std::cell::Cell::new((false,"Hex",0)) };}
use gpui::{prelude::*, *};
use pm_shell::{
    kit::{Button, Kit, Recipes},
    text::{TextEvent, TextField, TextKind},
};

pub type SampleSource = std::rc::Rc<
    dyn Fn(
        Point<Pixels>,
        &App,
    ) -> Result<async_channel::Receiver<Result<[f32; 4], String>>, String>,
>;

#[derive(Clone, Debug)]
pub enum Edit {
    Begin,
    Value(String),
    Finish(bool),
    Sample,
}
#[derive(Clone, Copy)]
enum Track {
    Plane,
    Alpha,
    Hue,
}
pub struct Picker {
    /// Full precision picker state. Read this on `Edit::Value` when a caller
    /// stores normalized RGBA rather than the document hex representation.
    pub color: Color,
    previous: Color,
    hue: f32,
    pub sampling: bool,
    sample_source: Option<SampleSource>,
    generation: u64,
    status: String,
    stops: Vec<FocusHandle>,
    focus_index: usize,
    window: AnyWindowHandle,
    menu: Option<bool>,
    menu_index: usize,
    active: bool,
    p3: bool,
    notation: &'static str,
    hue_mode: u8,
    drag: Option<(Track, Bounds<Pixels>)>,
    focus: FocusHandle,
    fields: Vec<Entity<TextField>>,
    subscriptions: Vec<Subscription>,
    tokens: pm_design::Tokens,
    motion: pm_shell::motion::Motion<u64>,
}
impl EventEmitter<Edit> for Picker {}
impl Picker {
    pub fn new(
        color: Color,
        tokens: pm_design::Tokens,
        window: &mut Window,
        sample_source: Option<SampleSource>,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        let (p3, notation, hue_mode) = PREFERENCES.get();
        let mut p = Self {
            color,
            previous: color,
            hue: color.hsv()[0],
            sampling: false,
            sample_source,
            generation: 0,
            status: String::new(),
            stops: (0..23).map(|_| cx.focus_handle().tab_stop(true)).collect(),
            focus_index: 21,
            window: window.window_handle(),
            menu: None,
            menu_index: 0,
            active: false,
            p3,
            notation,
            hue_mode,
            drag: None,
            focus,
            fields: vec![],
            subscriptions: vec![],
            tokens,
            motion: Default::default(),
        };
        if pm_shell::text::available(window) {
            for i in 0..10 {
                let value = p.channel_text(i);
                let t = p.tokens.clone();
                let field = cx.new(|cx| {
                    TextField::new(
                        if i == 9 {
                            TextKind::DenseText
                        } else {
                            TextKind::DenseNumber
                        },
                        &p.channel_label(i),
                        &value,
                        t,
                        window,
                        cx,
                    )
                });
                field.update(cx, |f, _| f.set_host_navigation(true));
                p.subscriptions
                    .push(cx.subscribe(&field, move |p, _, event, cx| match event {
                        TextEvent::Key("focus") | TextEvent::Changed(_) => {
                            p.focus_index = Self::field_stop(i)
                        }
                        TextEvent::Key(key @ ("up" | "down" | "shiftUp" | "shiftDown"))
                            if i < 9 =>
                        {
                            let n = p.fields[i]
                                .read(cx)
                                .editing_text()
                                .parse::<f32>()
                                .unwrap_or(p.row_values(i / 3)[i % 3]);
                            let step = if i == 1 {
                                0.001
                            } else if p.p3 && i >= 3 {
                                0.01
                            } else {
                                1.
                            };
                            let d = if matches!(*key, "up" | "shiftUp") {
                                1.
                            } else {
                                -1.
                            } * if key.starts_with("shift") { 10. } else { 1. };
                            p.input(i, &format!("{}", n + d * step), cx);
                        }
                        TextEvent::Key("tab") => {
                            let text = p.fields[i].read(cx).editing_text();
                            p.input(i, &text, cx);
                            p.navigate(1, cx);
                        }
                        TextEvent::Key("backtab") => {
                            let text = p.fields[i].read(cx).editing_text();
                            p.input(i, &text, cx);
                            p.navigate(-1, cx);
                        }
                        TextEvent::Key("escape") | TextEvent::Cancelled => {
                            if p.sampling {
                                p.sampling = false;
                                p.generation += 1;
                                cx.notify();
                            } else {
                                p.finish(false, cx);
                            }
                        }
                        TextEvent::Submitted(s) => {
                            p.input(i, s, cx);
                            if p.status.is_empty() {
                                if i == 9 {
                                    p.finish(true, cx);
                                } else {
                                    p.navigate(1, cx);
                                }
                            }
                        }
                        TextEvent::Committed(s) => {
                            p.input(i, s, cx);
                            if !p.status.is_empty() {
                                p.sync_fields(cx);
                            }
                        }
                        _ => {}
                    }));
                p.fields.push(field);
            }
        }
        for index in 0..p.stops.len() {
            let stop = p.stops[index].clone();
            p.subscriptions
                .push(cx.on_focus(&stop, window, move |p, _, cx| {
                    p.focus_index = index;
                    if let Some(i) = (0..10).find(|i| Self::field_stop(*i) == index) {
                        if let Some(f) = p.fields.get(i) {
                            f.update(cx, |f, cx| f.begin_editing_selecting_all(cx));
                        }
                    }
                }));
        }
        p.focus.focus(window, cx);
        if let Some(f) = p.fields.get(9) {
            f.update(cx, |f, cx| f.begin_editing_selecting_all(cx));
        }
        p
    }
    fn resolve_sample(
        &mut self,
        generation: u64,
        result: Result<Result<[f32; 4], String>, async_channel::RecvError>,
        cx: &mut Context<Self>,
    ) {
        if !self.sampling || self.generation != generation {
            return;
        }
        self.sampling = false;
        match result {
            Ok(Ok(c)) => self.set(Color(c), cx),
            Ok(Err(e)) => self.status = e,
            Err(_) => self.status = "Could not sample that color".into(),
        }
        if let Some(f) = self.fields.get(9) {
            f.update(cx, |f, cx| f.begin_editing_selecting_all(cx));
        }
        cx.notify();
    }
    fn apply_active(&mut self, cx: &mut Context<Self>) {
        if let Some(i) = (0..10).find(|i| Self::field_stop(*i) == self.focus_index) {
            if let Some(f) = self.fields.get(i) {
                let text = f.read(cx).editing_text();
                if text != self.channel_text(i) {
                    self.input(i, &text, cx);
                }
            }
        }
    }
    fn modes(&self) -> &'static [&'static str] {
        if self.menu == Some(true) {
            &[
                "Find closest color",
                "Maintain chroma",
                "Maintain lightness",
            ]
        } else if self.p3 {
            &["OKLCH", "Display P3"]
        } else {
            &["OKLCH", "HSL", "RGB", "Hex"]
        }
    }
    fn choose_menu(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.menu == Some(true) {
            self.hue_mode = index as u8;
        } else {
            self.notation = self.modes()[index];
            self.sync_fields(cx);
        }
        self.menu = None;
        PREFERENCES.set((self.p3, self.notation, self.hue_mode));
        cx.notify();
    }
    fn field_stop(i: usize) -> usize {
        if i == 9 { 21 } else { 9 + (i / 3) * 4 + i % 3 }
    }
    fn navigate(&mut self, d: isize, cx: &mut Context<Self>) {
        let index = (self.focus_index as isize + d).rem_euclid(23) as usize;
        self.focus_index = index;
        for f in &self.fields {
            f.update(cx, |f, cx| f.blur(cx));
        }
        let handle = self.stops[index].clone();
        let window = self.window;
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, w, cx| handle.focus(w, cx));
        });
    }
    fn channel_label(&self, i: usize) -> String {
        if i == 9 {
            return format!("{} value", self.notation);
        }
        let (mode, labels) = match i / 3 {
            0 => ("OKLCH", ["L", "C", "H"]),
            1 => {
                if self.p3 {
                    ("OKLab", ["L", "A", "B"])
                } else {
                    ("HSL", ["H", "S", "L"])
                }
            }
            _ => {
                if self.p3 {
                    ("P3", ["R", "G", "B"])
                } else {
                    ("RGB", ["R", "G", "B"])
                }
            }
        };
        format!("{mode} {}", labels[i % 3])
    }
    fn channel_text(&self, i: usize) -> String {
        if i == 9 {
            return self.color.draft(self.notation);
        }
        let row = self.row_values(i / 3);
        if i == 1 {
            format!("{:.3}", row[i % 3])
        } else if self.p3 && i >= 3 {
            format!("{:.3}", row[i % 3])
        } else if i >= 6 {
            format!("{:.0}", row[i % 3])
        } else {
            format!("{:.1}", row[i % 3])
        }
    }
    fn row_values(&self, row: usize) -> [f32; 3] {
        match row {
            0 => {
                let [l, c, h] = self.color.oklch();
                [l * 100., c, h]
            }
            1 => {
                if self.p3 {
                    let [l, a, b] = self.color.oklab();
                    [l * 100., a, b]
                } else {
                    let [h, s, l] = self.color.hsl();
                    [h, s * 100., l * 100.]
                }
            }
            _ => {
                let c = if self.p3 {
                    self.color.p3()
                } else {
                    self.color.clipped()
                };
                let k = if self.p3 { 1. } else { 255. };
                [c.0[0] * k, c.0[1] * k, c.0[2] * k]
            }
        }
    }
    fn sync_fields(&self, cx: &mut Context<Self>) {
        PREFERENCES.set((self.p3, self.notation, self.hue_mode));
        for (i, f) in self.fields.iter().enumerate() {
            let text = self.channel_text(i);
            f.update(cx, |f, cx| {
                f.set_label(&self.channel_label(i));
                f.set_value(&text, cx);
            });
        }
    }
    fn input(&mut self, i: usize, value: &str, cx: &mut Context<Self>) {
        let next = if i == 9 {
            Color::parse_draft(value, self.notation, self.color.0[3])
        } else if let Ok(n) = value.parse::<f32>() {
            if !n.is_finite() {
                return;
            }
            let mut row = self.row_values(i / 3);
            row[i % 3] = n;
            let [a, b, c] = row;
            let alpha = self.color.0[3];
            Some(match i / 3 {
                0 => Color::from_oklch((a / 100.).clamp(0., 1.), b.clamp(0., 0.4), c, alpha),
                1 => {
                    if self.p3 {
                        Color::from_oklab((a / 100.).clamp(0., 1.), b, c, alpha)
                    } else {
                        Color::from_hsl(
                            a,
                            (b / 100.).clamp(0., 1.),
                            (c / 100.).clamp(0., 1.),
                            alpha,
                        )
                    }
                }
                _ => {
                    if self.p3 {
                        Color::from_p3(Color([
                            a.clamp(0., 1.),
                            b.clamp(0., 1.),
                            c.clamp(0., 1.),
                            alpha,
                        ]))
                    } else {
                        Color([
                            (a / 255.).clamp(0., 1.),
                            (b / 255.).clamp(0., 1.),
                            (c / 255.).clamp(0., 1.),
                            alpha,
                        ])
                    }
                }
            })
        } else {
            None
        };
        if let Some(c) = next {
            self.status.clear();
            self.set(c, cx);
        } else {
            self.status = "Enter a valid color".into();
            cx.notify();
        }
    }
    fn hsv(&self) -> [f32; 3] {
        if self.p3 {
            self.color.p3().hsv()
        } else {
            self.color.clipped().hsv()
        }
    }
    fn plane(&self, h: f32, s: f32, v: f32, a: f32) -> Color {
        let c = Color::from_hsv(h, s, v, a);
        if self.p3 { Color::from_p3(c) } else { c }
    }
    fn change_hue(&mut self, h: f32, cx: &mut Context<Self>) {
        let [_, s, v] = self.hsv();
        let a = self.color.0[3];
        let next = if self.hue_mode == 0 || s == 0. {
            self.plane(h, s, v, a)
        } else {
            let [l, c, old] = self.color.oklch();
            let h = (old + h - self.hue).rem_euclid(360.);
            let mut next = Color::from_oklch(l, c, h, a);
            if self.hue_mode == 1 && next.outside() {
                for step in 0..=200 {
                    let distance = step as f32 * 0.005;
                    for l in [l - distance, l + distance] {
                        if !(0. ..=1.).contains(&l) {
                            continue;
                        }
                        let trial = Color::from_oklch(l, c, h, a);
                        if !trial.outside() {
                            next = trial;
                            break;
                        }
                    }
                    if !next.outside() {
                        break;
                    }
                }
            }
            if next.outside() {
                let (mut low, mut high) = (0., c);
                for _ in 0..24 {
                    let mid = (low + high) / 2.;
                    if Color::from_oklch(l, mid, h, a).outside() {
                        high = mid;
                    } else {
                        low = mid;
                    }
                }
                next = Color::from_oklch(l, low, h, a);
            }
            next
        };
        self.hue = h;
        self.set(next, cx);
        self.hue = h;
    }
    /// Precise normalized RGBA entry point for shared hosts and headless tests.
    /// Emits the same Begin/Value events as numeric entry; no byte quantization.
    pub fn edit_rgba(&mut self, rgba: [f32; 4], cx: &mut Context<Self>) {
        if rgba.iter().all(|v| v.is_finite()) {
            self.set(Color(rgba.map(|v| v.clamp(0., 1.))), cx);
        }
    }
    /// Numeric field pathway. sRGB fields: OKLCH 0..2, HSL 3..5,
    /// RGB 6..8 (0..255, fractional values accepted), value/opacity text 9.
    pub fn input_numeric(&mut self, index: usize, text: &str, cx: &mut Context<Self>) {
        if index <= 9 {
            self.input(index, text, cx);
        }
    }
    pub fn set(&mut self, c: Color, cx: &mut Context<Self>) {
        if c == self.color {
            return;
        }
        if !self.active {
            self.active = true;
            cx.emit(Edit::Begin);
        }
        self.color = c;
        if self.hsv()[1] > 0.00001 {
            self.hue = self.hsv()[0];
        }
        for (i, field) in self.fields.iter().enumerate() {
            let text = self.channel_text(i);
            field.update(cx, |f, cx| f.set_value(&text, cx));
        }
        cx.emit(Edit::Value(c.hex()));
        cx.notify();
    }
    /// Close the transaction. True commits; false cancels all live preview.
    pub fn finish(&mut self, commit: bool, cx: &mut Context<Self>) {
        if commit {
            self.apply_active(cx);
        }
        self.drag = None;
        self.generation += 1;
        self.sampling = false;
        cx.emit(Edit::Finish(commit));
    }
    fn pick(&mut self, at: Point<Pixels>, kind: Track, b: Bounds<Pixels>, cx: &mut Context<Self>) {
        let x = (f32::from(at.x - b.origin.x) / f32::from(b.size.width)).clamp(0., 1.);
        let y = (f32::from(at.y - b.origin.y) / f32::from(b.size.height)).clamp(0., 1.);
        let a = self.color.0[3];
        let next = match kind {
            Track::Plane => self.plane(self.hue, x, 1. - y, a),
            Track::Hue => {
                self.change_hue(y * 360., cx);
                return;
            }
            Track::Alpha => Color([self.color.0[0], self.color.0[1], self.color.0[2], 1. - y]),
        };
        self.set(next, cx);
    }
    fn nudge(&mut self, kind: Track, key: &str, shift: bool, cx: &mut Context<Self>) {
        let d = if matches!(key, "up" | "right") {
            1.
        } else {
            -1.
        } * if shift { 10. } else { 1. };
        let [_, mut s, mut v] = self.hsv();
        let mut a = self.color.0[3];
        match kind {
            Track::Hue => {
                self.change_hue((self.hue + d).rem_euclid(360.), cx);
                return;
            }
            Track::Alpha => a = (a + d / 100.).clamp(0., 1.),
            Track::Plane => {
                if matches!(key, "left" | "right") {
                    s = (s + d / 100.).clamp(0., 1.)
                } else {
                    v = (v + d / 100.).clamp(0., 1.)
                }
            }
        }
        self.set(self.plane(self.hue, s, v, a), cx);
    }
    fn track(&self, kind: Track, cx: &mut Context<Self>) -> AnyElement {
        let (width, label) = match kind {
            Track::Plane => (
                self.tokens.metrics.color_picker_plane_width,
                "Saturation and brightness",
            ),
            Track::Alpha => (self.tokens.metrics.color_picker_track, "Opacity"),
            Track::Hue => (self.tokens.metrics.color_picker_track, "Hue"),
        };
        let color = if self.p3 {
            self.color.p3()
        } else {
            self.color.clipped()
        };
        let hue = self.hue;
        let t = self.tokens.clone();
        let focus_color = t.colors.focus;
        let weak = cx.entity().downgrade();
        div()
            .id(label)
            .relative()
            .w(px(width))
            .h(px(self.tokens.metrics.color_picker_plane_height))
            .track_focus(
                &self.stops[match kind {
                    Track::Plane => 5,
                    Track::Alpha => 6,
                    Track::Hue => 7,
                }],
            )
            .focus_visible_style(move |s| s.border_1().border_color(rgb(focus_color)))
            .tab_index(0)
            .aria_label(label)
            .on_key_down(cx.listener(move |p, e: &KeyDownEvent, _, cx| {
                let key = e.keystroke.key.as_str();
                if matches!(key, "up" | "down" | "left" | "right") {
                    cx.stop_propagation();
                    p.nudge(kind, key, e.keystroke.modifiers.shift, cx);
                }
            }))
            .child(
                canvas(
                    move |bounds, window, _| {
                        let weak = weak.clone();
                        window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                            if phase == DispatchPhase::Bubble
                                && e.button == MouseButton::Left
                                && bounds.contains(&e.position)
                            {
                                let _ = weak.update(cx, |p, cx| {
                                    let i = match kind {
                                        Track::Plane => 5,
                                        Track::Alpha => 6,
                                        Track::Hue => 7,
                                    };
                                    p.stops[i].focus(window, cx);
                                    p.focus_index = i;
                                    p.drag = Some((kind, bounds));
                                    p.pick(e.position, kind, bounds, cx);
                                });
                            }
                        });
                    },
                    move |bounds, _, window, _| {
                        let opaque = |c: Color| {
                            rgb(c.0[..3]
                                .iter()
                                .fold(0u32, |out, c| (out << 8) | (*c * 255.).round() as u32))
                        };
                        let gradient = |angle, from: Rgba, to: Rgba| {
                            let mut bg = linear_gradient(
                                angle,
                                linear_color_stop(from, 0.),
                                linear_color_stop(to, 1.),
                            );
                            bg.color_space = ColorSpace::Srgb;
                            bg
                        };
                        match kind {
                            Track::Plane => {
                                window.paint_quad(fill(
                                    bounds,
                                    gradient(
                                        90.,
                                        rgb(0xffffff),
                                        opaque(Color::from_hsv(hue, 1., 1., 1.)),
                                    ),
                                ));
                                window.paint_quad(fill(bounds, gradient(180., rgba(0), rgb(0))));
                            }
                            Track::Hue => {
                                for i in 0..6 {
                                    let b = Bounds::new(
                                        bounds.origin
                                            + point(px(0.), bounds.size.height * i as f32 / 6.),
                                        size(bounds.size.width, bounds.size.height / 6. + px(0.5)),
                                    );
                                    window.paint_quad(fill(
                                        b,
                                        gradient(
                                            180.,
                                            opaque(Color::from_hsv(i as f32 * 60., 1., 1., 1.)),
                                            opaque(Color::from_hsv(
                                                (i + 1) as f32 * 60.,
                                                1.,
                                                1.,
                                                1.,
                                            )),
                                        ),
                                    ));
                                }
                            }
                            Track::Alpha => {
                                let cell = t.space.xs.max(1.);
                                for y in 0..(f32::from(bounds.size.height) / cell).ceil() as u32 {
                                    for x in 0..(f32::from(bounds.size.width) / cell).ceil() as u32
                                    {
                                        let b = Bounds::new(
                                            bounds.origin
                                                + point(px(x as f32 * cell), px(y as f32 * cell)),
                                            size(
                                                px(cell.min(
                                                    f32::from(bounds.size.width) - x as f32 * cell,
                                                )),
                                                px(cell.min(
                                                    f32::from(bounds.size.height) - y as f32 * cell,
                                                )),
                                            ),
                                        );
                                        window.paint_quad(fill(
                                            b,
                                            rgb(if (x + y) % 2 == 0 {
                                                t.colors.field
                                            } else {
                                                t.colors.muted
                                            }),
                                        ));
                                    }
                                }
                                let c = opaque(color);
                                let mut transparent = c;
                                transparent.a = 0.;
                                window.paint_quad(fill(bounds, gradient(180., c, transparent)));
                            }
                        }
                        let [h, s, v] = color.hsv();
                        let (x, y, w, h) = match kind {
                            Track::Plane => (s, 1. - v, 8., 8.),
                            Track::Hue => (0., h / 360., 14., 4.),
                            Track::Alpha => (0., 1. - color.0[3], 14., 4.),
                        };
                        let b = Bounds::new(
                            bounds.origin
                                + point(
                                    bounds.size.width * x
                                        - px(if matches!(kind, Track::Plane) { 4. } else { 0. }),
                                    bounds.size.height * y - px(h / 2.),
                                ),
                            size(px(w), px(h)),
                        );
                        window.paint_quad(outline(b, rgb(t.colors.text), BorderStyle::Solid));
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }
}
impl Render for Picker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.motion.frame(window);
        let t = self.tokens.clone();
        let focus_color = t.colors.focus;
        let kit = Kit::new(&t, &self.motion, |p| &mut p.motion, window, cx);
        let weak = cx.entity().downgrade();
        let tracking = canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let sample = weak.clone();
                window.on_mouse_event(move |e: &MouseDownEvent, phase, _, cx| {
                    if phase != DispatchPhase::Capture || e.button != MouseButton::Left {
                        return;
                    }
                    let _ = sample.update(cx, |p, cx| {
                        if !p.sampling {
                            return;
                        }
                        cx.stop_propagation();
                        let Some(source) = &p.sample_source else {
                            p.status = "Composition viewer unavailable".into();
                            cx.notify();
                            return;
                        };
                        match source(e.position, cx) {
                            Ok(rx) => {
                                let generation = p.generation;
                                cx.spawn(async move |this, cx| {
                                    let result = rx.recv().await;
                                    let _ = this.update(cx, |p, cx| {
                                        p.resolve_sample(generation, result, cx);
                                    });
                                })
                                .detach();
                            }
                            Err(e) => {
                                p.status = e;
                                cx.notify();
                            }
                        }
                    });
                });
                let moved = weak.clone();
                window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture {
                        let _ = moved.update(cx, |p, cx| {
                            if let Some((k, b)) = p.drag {
                                p.pick(e.position, k, b, cx);
                            }
                        });
                    }
                });
                let up = weak.clone();
                window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture {
                        let _ = up.update(cx, |p, _| p.drag = None);
                    }
                });
            },
        )
        .absolute()
        .inset_0();
        let mut details = div().flex().flex_col().gap(px(t.space.sm)).flex_1();
        let swatch = |c: Color| {
            let ink = Rgba {
                r: c.0[0],
                g: c.0[1],
                b: c.0[2],
                a: c.0[3],
            };
            let t = t.clone();
            div().relative().flex_1().h(px(t.metrics.control)).child(
                canvas(
                    |_, _, _| {},
                    move |b, _, w, _| {
                        let cell = t.space.xs.max(1.);
                        for y in 0..(f32::from(b.size.height) / cell).ceil() as u32 {
                            for x in 0..(f32::from(b.size.width) / cell).ceil() as u32 {
                                let b2 = Bounds::new(
                                    b.origin + point(px(x as f32 * cell), px(y as f32 * cell)),
                                    size(
                                        px(cell.min(f32::from(b.size.width) - x as f32 * cell)),
                                        px(cell.min(f32::from(b.size.height) - y as f32 * cell)),
                                    ),
                                );
                                w.paint_quad(fill(
                                    b2,
                                    rgb(if (x + y) % 2 == 0 {
                                        t.colors.field
                                    } else {
                                        t.colors.muted
                                    }),
                                ));
                            }
                        }
                        w.paint_quad(fill(b, ink));
                    },
                )
                .size_full(),
            )
        };
        details = details
            .child(
                div()
                    .flex()
                    .child(
                        swatch(self.previous)
                            .track_focus(&self.stops[8])
                            .focus_visible_style(move |s| {
                                s.border_1().border_color(rgb(focus_color))
                            })
                            .id("previous")
                            .on_click(cx.listener(|p, _, _, cx| p.set(p.previous, cx))),
                    )
                    .child(swatch(self.color)),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .child("Previous")
                    .child("New"),
            );
        for row in 0..3 {
            let mut channels = div().flex().gap(px(t.space.xs));
            for i in row * 3..row * 3 + 3 {
                channels = channels.child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .track_focus(&self.stops[Self::field_stop(i)])
                        .focus_visible_style(move |s| s.border_1().border_color(rgb(focus_color)))
                        .child(if let Some(f) = self.fields.get(i) {
                            f.clone().into_any_element()
                        } else {
                            div()
                                .h(px(t.metrics.field))
                                .px(px(t.space.sm))
                                .rounded(px(t.metrics.control_radius))
                                .bg(rgb(t.colors.field))
                                .child(self.channel_text(i))
                                .into_any_element()
                        }),
                );
            }
            let copy_mode = if self.p3 {
                ["OKLCH", "OKLab", "P3"][row]
            } else {
                ["OKLCH", "HSL", "RGB"][row]
            };
            channels = channels.child(
                kit.icon_button(
                    format!("color-copy-{row}"),
                    "doc.on.doc",
                    format!("Copy {copy_mode}"),
                    false,
                )
                .track_focus(&self.stops[12 + row * 4])
                .focus_visible_style(move |s| s.border_1().border_color(rgb(focus_color)))
                .on_click(cx.listener(move |p, _, _, cx| {
                    let text = match copy_mode {
                        "P3" => format!("color(display-p3 {})", p.color.format("Display P3")),
                        "OKLab" => {
                            let [l, a, b] = p.color.oklab();
                            format!("oklab({:.1}% {a:.3} {b:.3})", l * 100.)
                        }
                        _ => format!(
                            "{}({})",
                            copy_mode.to_lowercase(),
                            p.color.format(copy_mode)
                        ),
                    };
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                })),
            );
            details = details.child(
                div()
                    .child(if self.p3 {
                        ["OKLCH", "OKLab", "P3"][row]
                    } else {
                        ["OKLCH", "HSL", "RGB"][row]
                    })
                    .child(channels),
            );
        }
        if let Some(f) = self.fields.get(9) {
            details = details.child(
                div()
                    .track_focus(&self.stops[21])
                    .focus_visible_style(move |s| s.border_1().border_color(rgb(focus_color)))
                    .child(f.clone()),
            );
        } else {
            details = details.child(self.color.draft(self.notation));
        }
        details = details.child(
            kit.button("color-notation", self.notation, Button::Quiet, None, None)
                .track_focus(&self.stops[22])
                .focus_visible_style(move |s| s.border_1().border_color(rgb(focus_color)))
                .on_click(cx.listener(|p, _, _, cx| {
                    p.menu = if p.menu == Some(false) {
                        None
                    } else {
                        Some(false)
                    };
                    cx.notify();
                })),
        );
        if self.color.outside() {
            details = details.child("Clipped to sRGB when saved");
        }
        div()
            .id("colour-picker")
            .relative()
            .occlude()
            .track_focus(&self.focus)
            .key_context("ColorPicker")
            .w(px(t.metrics.color_picker_width))
            .p(px(t.space.md))
            .bg(rgb(t.colors.floating))
            .text_color(rgb(t.colors.text))
            .text_size(px(t.typography.body))
            .corner(&t, t.metrics.popover_radius)
            .elevated(&t, pm_design::Elevation::Menu)
            .on_key_down(cx.listener(|p, e: &KeyDownEvent, _, cx| {
                cx.stop_propagation();
                match e.keystroke.key.as_str() {
                    "tab" => {
                        p.navigate(if e.keystroke.modifiers.shift { -1 } else { 1 }, cx);
                    }
                    "escape" => {
                        if p.menu.take().is_some() {
                            cx.notify();
                        } else if p.sampling {
                            p.sampling = false;
                            p.generation += 1;
                            cx.notify();
                        } else {
                            p.finish(false, cx);
                        }
                    }
                    "up" | "down" if p.menu.is_some() => {
                        let n = p.modes().len();
                        p.menu_index =
                            (p.menu_index + n + if e.keystroke.key == "up" { n - 1 } else { 1 })
                                % n;
                        cx.notify();
                    }
                    "enter" | "space" if p.menu.is_some() => p.choose_menu(p.menu_index, cx),
                    "enter" if p.focus_index == 22 => {
                        p.menu = Some(false);
                        p.menu_index = 0;
                        cx.notify();
                    }
                    "enter" if p.focus_index == 3 => {
                        p.menu = Some(true);
                        p.menu_index = 0;
                        cx.notify();
                    }
                    "enter" if p.focus_index == 8 => p.set(p.previous, cx),
                    "enter" if p.focus_index == 0 || p.focus_index == 1 => {
                        p.p3 = p.focus_index == 1;
                        p.notation = "OKLCH";
                        p.hue = p.hsv()[0];
                        p.sync_fields(cx);
                        cx.notify();
                    }
                    "enter" if p.focus_index == 2 => {
                        p.sampling = true;
                        p.generation += 1;
                        p.status.clear();
                        cx.notify();
                    }
                    "enter" => {
                        p.apply_active(cx);
                        if p.status.is_empty() {
                            p.finish(true, cx);
                        }
                    }
                    _ => {}
                }
            }))
            .on_mouse_down_out(cx.listener(|p, _, _, cx| {
                if !p.sampling {
                    p.finish(true, cx);
                }
            }))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .mb(px(t.space.sm))
                    .child(
                        div()
                            .flex()
                            .child(
                                kit.button("color-srgb", "sRGB", Button::Quiet, None, None)
                                    .track_focus(&self.stops[0])
                                    .focus_visible_style(move |s| {
                                        s.border_1().border_color(rgb(focus_color))
                                    })
                                    .when(!self.p3, |d| d.bg(rgb(t.colors.selected_fill)))
                                    .on_click(cx.listener(|p, _, _, cx| {
                                        p.p3 = false;
                                        p.notation = "OKLCH";
                                        p.hue = p.hsv()[0];
                                        p.sync_fields(cx);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                kit.button("color-p3", "Display P3", Button::Quiet, None, None)
                                    .track_focus(&self.stops[1])
                                    .focus_visible_style(move |s| {
                                        s.border_1().border_color(rgb(focus_color))
                                    })
                                    .when(self.p3, |d| d.bg(rgb(t.colors.selected_fill)))
                                    .on_click(cx.listener(|p, _, _, cx| {
                                        p.p3 = true;
                                        p.notation = "OKLCH";
                                        p.hue = p.hsv()[0];
                                        p.sync_fields(cx);
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        kit.icon_button(
                            "color-hue-settings",
                            "line.3.horizontal",
                            "Picker settings",
                            self.menu == Some(true),
                        )
                        .track_focus(&self.stops[3])
                        .focus_visible_style(move |s| s.border_1().border_color(rgb(focus_color)))
                        .on_click(cx.listener(|p, _, _, cx| {
                            p.menu = if p.menu == Some(true) {
                                None
                            } else {
                                Some(true)
                            };
                            cx.notify();
                        })),
                    )
                    .child(
                        kit.icon_button(
                            "sample-comp",
                            "eyedropper",
                            "Sample composition color",
                            self.sampling,
                        )
                        .track_focus(&self.stops[2])
                        .focus_visible_style(move |s| s.border_1().border_color(rgb(focus_color)))
                        .on_click(cx.listener(|p, _, _, cx| {
                            p.sampling = true;
                            p.generation += 1;
                            p.status.clear();
                            cx.emit(Edit::Sample);
                            cx.notify();
                        })),
                    )
                    .child(
                        kit.icon_button("close-color", "xmark", "Close", false)
                            .track_focus(&self.stops[4])
                            .focus_visible_style(move |s| {
                                s.border_1().border_color(rgb(focus_color))
                            })
                            .on_click(cx.listener(|p, _, _, cx| p.finish(true, cx))),
                    ),
            )
            .when(self.sampling, |d| {
                d.child("Click a composition pixel · Esc to cancel")
            })
            .child(
                div()
                    .flex()
                    .gap(px(t.space.md))
                    .child(self.track(Track::Plane, cx))
                    .child(self.track(Track::Alpha, cx))
                    .child(self.track(Track::Hue, cx))
                    .child(details),
            )
            .when(!self.status.is_empty(), |d| d.child(self.status.clone()))
            .when_some(self.menu, |d, settings| {
                let modes: &[&str] = if settings {
                    &[
                        "Find closest color",
                        "Maintain chroma",
                        "Maintain lightness",
                    ]
                } else if self.p3 {
                    &["OKLCH", "Display P3"]
                } else {
                    &["OKLCH", "HSL", "RGB", "Hex"]
                };
                let mut menu = kit.menu().absolute().right(px(t.space.md));
                menu = if settings {
                    menu.top(px(t.metrics.control + t.space.md))
                } else {
                    menu.bottom(px(t.metrics.control + t.space.md))
                };
                for (i, &name) in modes.iter().enumerate() {
                    menu = menu.child(
                        kit.menu_row(
                            format!("picker-menu-{i}"),
                            name,
                            Some(if settings {
                                self.hue_mode == i as u8
                            } else {
                                self.notation == name
                            }),
                            None,
                        )
                        .on_click(cx.listener(move |p, _, _, cx| p.choose_menu(i, cx))),
                    );
                }
                d.child(menu)
            })
            .child(tracking)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn colour_drag_alpha_and_keyboard_are_one_gesture(cx: &mut TestAppContext) {
        let (picker, mut vcx) = cx.add_window_view(|w, cx| {
            Picker::new(
                Color::parse("#FF0000").unwrap(),
                pm_design::Tokens::new(pm_design::Scheme::Dark, Default::default()),
                w,
                None,
                cx,
            )
        });
        let events = std::rc::Rc::new(std::cell::RefCell::new(vec![]));
        let saved = events.clone();
        let _sub = vcx.update(|_, cx| {
            cx.subscribe(&picker, move |_, e, _| saved.borrow_mut().push(e.clone()))
        });
        vcx.update(|_, cx| {
            picker.update(cx, |p, cx| {
                let b = Bounds::new(point(px(0.), px(0.)), size(px(100.), px(100.)));
                p.pick(point(px(50.), px(25.)), Track::Plane, b, cx);
                assert_eq!(p.color.hex(), "#BF6060");
                p.pick(point(px(75.), px(50.)), Track::Plane, b, cx);
                assert_eq!(p.color.hex(), "#802020");
                p.nudge(Track::Alpha, "down", true, cx);
                assert!((p.color.0[3] - 0.9).abs() < 0.0001);
                p.finish(true, cx);
            })
        });
        vcx.run_until_parked();
        assert_eq!(
            events
                .borrow()
                .iter()
                .filter(|e| matches!(e, Edit::Begin))
                .count(),
            1
        );
        assert_eq!(
            events
                .borrow()
                .iter()
                .filter(|e| matches!(e, Edit::Finish(true)))
                .count(),
            1
        );
    }
    #[gpui::test]
    fn colour_focus_and_pending_sample_are_scoped_to_picker(cx: &mut TestAppContext) {
        let (picker, mut vcx) = cx.add_window_view(|w, cx| {
            Picker::new(
                Color::parse("#FF6B1A").unwrap(),
                pm_design::Tokens::new(pm_design::Scheme::Dark, Default::default()),
                w,
                None,
                cx,
            )
        });
        for result in [
            Ok([0.04, 0.52, 1., 1.]),
            Err("Native sampling failed".into()),
        ] {
            vcx.update(|w, cx| {
                picker.update(cx, |p, cx| {
                    p.sampling = true;
                    p.generation += 1;
                    let generation = p.generation;
                    p.resolve_sample(generation, Ok(result), cx);
                    assert!(!p.sampling);
                    assert!(p.focus.contains_focused(w, cx));
                })
            });
            vcx.run_until_parked();
        }
        vcx.update(|_, cx| {
            picker.update(cx, |p, cx| {
                p.sampling = true;
                p.generation += 1;
                let old = p.generation;
                p.finish(false, cx);
                let kept = p.color;
                p.resolve_sample(old, Ok(Ok([0., 1., 0., 1.])), cx);
                assert_eq!(p.color, kept);
                p.sampling = true;
                p.generation += 1;
                p.resolve_sample(old, Ok(Ok([0., 1., 0., 1.])), cx);
                assert_eq!(p.color, kept);
                assert!(p.sampling);
                p.focus_index = 22;
                p.navigate(1, cx);
                assert_eq!(p.focus_index, 0);
                p.navigate(-1, cx);
                assert_eq!(p.focus_index, 22);
            })
        });
    }
    #[test]
    fn colour_picker_headless_captures() {
        let Ok(dir) = std::env::var("PM_COLOR_PICKER_CAPTURE") else {
            return;
        };
        std::fs::create_dir_all(&dir).unwrap();
        for scheme in [pm_design::Scheme::Dark, pm_design::Scheme::Light] {
            for sampling in [false, true] {
                let mut cx = HeadlessAppContext::with_platform(
                    std::sync::Arc::new(
                        gpui_parley::ParleyTextSystem::new(gpui_parley::SystemFonts::Load)
                            .with_automatic_optical_sizing(),
                    ),
                    std::sync::Arc::new(()),
                    || {
                        Some(Box::new(
                            gpui_apple::metal_renderer::MetalHeadlessRenderer::new(),
                        ))
                    },
                );
                let window = cx
                    .open_window(size(px(565.), px(350.)), |w, cx| {
                        cx.new(|cx| {
                            let mut p = Picker::new(
                                Color::parse("#FF6B1ACC").unwrap(),
                                pm_design::Tokens::new(scheme, Default::default()),
                                w,
                                None,
                                cx,
                            );
                            p.sampling = sampling;
                            p
                        })
                    })
                    .unwrap();
                cx.run_until_parked();
                cx.capture_screenshot(window.into())
                    .unwrap()
                    .save(format!(
                        "{dir}/picker-{}-{}.png",
                        if scheme == pm_design::Scheme::Dark {
                            "dark"
                        } else {
                            "light"
                        },
                        if sampling { "eyedropper" } else { "open" }
                    ))
                    .unwrap();
            }
        }
    }
}
