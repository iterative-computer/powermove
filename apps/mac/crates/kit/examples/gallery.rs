//! Run the built binary at ~/Developer/pm-target/debug/examples/gallery.
use gpui::{prelude::*, *};
use pm_kit::{
    color::Color,
    theme::{Appearance, ControlSize, Theme},
    *,
};
struct Entry {
    name: &'static str,
    views: Vec<AnyView>,
}
struct Gallery {
    rows: Vec<Entry>,
    appearance: Appearance,
    updates: Vec<Box<dyn Fn(Appearance, &mut App)>>,
    status: Entity<String>,
}
impl Gallery {
    fn new(w: &mut Window, cx: &mut Context<Self>) -> Self {
        let status =
            cx.new(|_| "Drag previews live; release commits once. Escape restores.".to_string());
        cx.observe(&status, |_, _, cx| cx.notify()).detach();
        let mut this = Self {
            rows: vec![],
            appearance: Appearance::Dark,
            updates: vec![],
            status,
        };
        macro_rules! row {
            ($name:expr,$ty:ty,$ctor:expr) => {{
                let mut views = vec![];
                for state in 0..3 {
                    let log = this.status.clone();
                    let entity = cx.new(|cx| {
                        let field: $ty = $ctor(w, cx);
                        field
                            .label($name)
                            .mixed(state == 1)
                            .disabled(state == 2)
                            .appearance(Appearance::Dark)
                            .on_edit(move |e, cx| {
                                log.update(cx, |log, cx| {
                                    *log = format!("{} · {:?}", $name, e);
                                    cx.notify();
                                })
                            })
                    });
                    let copy = entity.clone();
                    this.updates.push(Box::new(move |a, cx| {
                        copy.update(cx, |field, cx| {
                            field.state.appearance = a;
                            cx.notify();
                        })
                    }));
                    views.push(entity.into());
                }
                this.rows.push(Entry { name: $name, views });
            }};
        }
        row!("Number", NumField, |w, cx| NumField::new(42., w, cx)
            .range(0., 100.)
            .unit(" px"));
        row!("Slider", SliderField, |w, cx| SliderField::slider(
            68., 0., 100., w, cx
        )
        .unit("%"));
        row!("Angle", SliderField, |w, cx| SliderField::slider(
            270., 0., 360., w, cx
        )
        .angle(true)
        .unit("°"));
        row!("Text", TextField, |w, cx| TextField::new(
            "Powermove",
            w,
            cx
        )
        .mono(false));
        row!("Toggle", ToggleField, |w, cx| ToggleField::new(true, w, cx));
        row!("Checkbox", ToggleField, |w, cx| ToggleField::new(
            true, w, cx
        )
        .variant(ToggleVariant::Checkbox));
        row!("Select", SelectField<i32>, |w, cx| SelectField::new(
            24,
            vec![
                SelectOption::new(24, "24 fps"),
                SelectOption::new(30, "30 fps"),
                SelectOption::new(60, "60 fps"),
                SelectOption::new(120, "120 fps").disabled(true)
            ],
            w,
            cx
        ));
        row!("Search picker", SelectField<String>, |w, cx| {
            SelectField::new(
                "Screen".into(),
                [
                    "Normal",
                    "Multiply",
                    "Screen",
                    "Overlay",
                    "Soft Light",
                    "Difference",
                ]
                .into_iter()
                .map(|s| SelectOption::new(s.to_string(), s))
                .collect(),
                w,
                cx,
            )
            .searchable(true)
        });
        row!("Segments", Segmented<String>, |w, cx| Segmented::new(
            "solid".into(),
            vec![
                SelectOption::new("solid".into(), "Solid"),
                SelectOption::new("line".into(), "Line"),
                SelectOption::new("none".into(), "None")
            ],
            w,
            cx
        )
        .segmented(true));
        row!("Popover menu", PopoverMenu, |w, cx| PopoverMenu::menu(
            "Actions",
            vec![
                pm_kit::MenuItem::new("Duplicate", |_| {})
                    .icon("⧉")
                    .detail("Copy selected layers"),
                pm_kit::MenuItem::separator(),
                pm_kit::MenuItem::new("Delete", |_| {}).warning(true),
                pm_kit::MenuItem::new("Checked choice", |_| {}).checked(true)
            ],
            w,
            cx
        )
        .header("Layer actions"));
        let fonts = cx.text_system().all_font_names();
        row!("Font", FontField, |w, cx| FontField::font(
            "Helvetica Neue",
            fonts.clone(),
            w,
            cx
        ));
        row!("Color", ColorField, |w, cx| ColorField::new(
            Color::new(1., 0.375, 0.1, 1.),
            w,
            cx
        ));
        row!("Fill", FillField, |w, cx| FillField::new(
            FillValue {
                mode: FillMode::Linear,
                ..Default::default()
            },
            w,
            cx
        ));
        row!("Gradient ramp", RampField, |w, cx| RampField::new(
            RampValue::default(),
            w,
            cx
        ));
        row!("Point", PointField, |w, cx| PointField::new(
            [0.35, 0.65],
            w,
            cx
        )
        .range([0.; 2], [1.; 2]));
        let mut views = vec![];
        for size in [ControlSize::Regular, ControlSize::Small, ControlSize::Mini] {
            let field = cx.new(|cx| {
                NumField::new(12.5, w, cx)
                    .label("Size metric")
                    .size(size)
                    .appearance(Appearance::Dark)
            });
            let copy = field.clone();
            this.updates.push(Box::new(move |a, cx| {
                copy.update(cx, |field, cx| {
                    field.state.appearance = a;
                    cx.notify();
                })
            }));
            views.push(field.into());
        }
        this.rows.push(Entry {
            name: "Regular / Small / Mini",
            views,
        });
        if std::env::args().any(|a| a == "--light") {
            this.appearance = Appearance::Light;
            for f in &this.updates {
                f(Appearance::Light, cx);
            }
        }
        this
    }
    fn theme(&mut self, cx: &mut Context<Self>) {
        self.appearance = if self.appearance == Appearance::Dark {
            Appearance::Light
        } else {
            Appearance::Dark
        };
        for f in &self.updates {
            f(self.appearance, cx);
        }
        cx.notify();
    }
}
impl Render for Gallery {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let a = self.appearance;
        let t = Theme::for_window(a, w);
        let weak = cx.weak_entity();
        let rows = self.rows.iter().map(|row| {
            div()
                .flex()
                .items_center()
                .gap_4()
                .min_h(px(if row.name == "Point" {
                    64.
                } else if row.name == "Gradient ramp" {
                    50.
                } else {
                    32.
                }))
                .child(
                    div()
                        .w(px(136.))
                        .text_size(px(12.))
                        .text_color(t.label)
                        .child(row.name),
                )
                .children(
                    row.views
                        .iter()
                        .cloned()
                        .map(|v| div().flex_1().min_w_0().child(v)),
                )
        });
        let keyframes = [
            KeyframeState::Unanimated,
            KeyframeState::Animated,
            KeyframeState::AtPlayhead,
            KeyframeState::Expression,
        ]
        .into_iter()
        .enumerate()
        .map(|(i, s)| {
            KeyframeButton::new(i, format!("Keyframe {s:?}"), s, |_, _| {}).appearance(a)
        });
        let layout = Section::new("Shared layout and actions")
            .appearance(a)
            .child(
                Disclosure::new("gallery-more", "Disclosure")
                    .count(4)
                    .changed(true)
                    .remember(true)
                    .appearance(a)
                    .child(
                        Row::new("Paired row")
                            .pair(true)
                            .appearance(a)
                            .child("X / Y values")
                            .action(
                                KeyframeButton::new(
                                    "key",
                                    "Animate position",
                                    KeyframeState::AtPlayhead,
                                    |_, _| {},
                                )
                                .appearance(a),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(BackTitle::new("BackTitle").on_back(|_, _| {}).appearance(a))
                    .children(keyframes),
            )
            .child(
                div()
                    .id("tip-target")
                    .tooltip(move |_, cx| {
                        cx.new(|_| {
                            let mut tip =
                                Tooltip::new("Component help — Keyboard and pointer support (⌘K)");
                            tip.appearance = a;
                            tip
                        })
                        .into()
                    })
                    .tooltip_show_delay(pm_kit::layout::TOOLTIP_DELAY)
                    .child(ActionButton::new("tip", "Hover for tooltip", |_, _| {}).appearance(a)),
            );
        div()
            .size_full()
            .bg(t.window)
            .text_color(t.text)
            .font_family(theme::SYSTEM_FONT)
            .p_4()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(BackTitle::new("Powermove · Component Kit").appearance(a))
                    .child(
                        ActionButton::new(
                            "theme",
                            if a == Appearance::Dark {
                                "Switch to Light"
                            } else {
                                "Switch to Dark"
                            },
                            move |_, cx| {
                                let _ = weak.update(cx, |this, cx| this.theme(cx));
                            },
                        )
                        .appearance(a),
                    ),
            )
            .child(div().flex().pl(px(152.)).gap_4().children(
                ["Live · focus, type, drag", "Mixed selection", "Disabled"].map(|s| {
                    div()
                        .flex_1()
                        .text_size(px(12.))
                        .text_color(t.text_2)
                        .child(s)
                }),
            ))
            .child(
                div()
                    .id("gallery-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(
                        Section::new("Inspector controls")
                            .appearance(a)
                            .children(rows),
                    ),
            )
            .child(layout)
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(t.text_2)
                    .child(self.status.read(cx).clone()),
            )
    }
}
fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        pm_kit::init(cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1120.), px(940.)),
                    cx,
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Powermove Kit Gallery".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |w, cx| cx.new(|cx| Gallery::new(w, cx)),
        )
        .expect("gallery window");
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        cx.activate(true);
    });
}
