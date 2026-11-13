use pm_design::*;
#[test]
fn snapshot() {
    let all: Vec<_> = [Scheme::Light, Scheme::Dark]
        .into_iter()
        .flat_map(|scheme| {
            [false, true]
                .into_iter()
                .flat_map(move |increase_contrast| {
                    [false, true].map(move |reduce_transparency| {
                        Tokens::new(
                            scheme,
                            Accessibility {
                                increase_contrast,
                                reduce_transparency,
                                reduce_motion: false,
                            },
                        )
                    })
                })
        })
        .collect();
    let json = serde_json::to_string_pretty(&all).unwrap() + "\n";
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/tokens.snapshot.json");
    if std::env::var_os("PM_UPDATE_DESIGN_SNAPSHOT").is_some() {
        std::fs::write(&p, &json).unwrap();
    }
    assert_eq!(json, std::fs::read_to_string(p).unwrap());
}
#[test]
fn placement_accessibility_and_motion_contract() {
    let mut t = Tokens::new(Scheme::Dark, Accessibility::default());
    for s in [
        Surface::Viewer,
        Surface::Timeline,
        Surface::InspectorRow,
        Surface::List,
        Surface::Panel,
    ] {
        assert_eq!(t.material(s, true), Material::Solid);
    }
    assert_eq!(t.material(Surface::Menu, true), Material::Glass);
    assert_eq!(t.material(Surface::Menu, false), Material::VisualEffect);
    t.accessibility.reduce_transparency = true;
    assert_eq!(t.material(Surface::Menu, true), Material::Solid);
    assert_eq!(
        (
            t.motion.enter,
            t.motion.bounce,
            t.motion.exit,
            t.motion.reduced,
            t.motion.start_scale
        ),
        (0.32, 0.15, 0.12, 0.15, 0.96)
    );
    assert!(t.typography.minimum >= 11.);
    assert_eq!(t.metrics.gap, 6.);
    assert_eq!(t.metrics.panel_radius, 12.);
    assert_eq!((t.metrics.header, t.metrics.status), (38., 24.));
    let mut themed = t.clone();
    themed.apply_patch([("metrics.gap", 10.)]).unwrap();
    assert_eq!(
        themed.metrics.dock_gap, 10.,
        "a theme's gap spaces the docks"
    );
    assert!(Tokens::new(Scheme::Dark, Accessibility::default()).translucent());
    let solid = Tokens::new(
        Scheme::Dark,
        Accessibility {
            reduce_transparency: true,
            ..Default::default()
        },
    );
    assert!(
        !solid.translucent(),
        "Reduce Transparency makes the window solid"
    );
    assert_eq!(solid.translucency.panel, 1.);
    assert_eq!(t.colors.accent, 0xfa6310);
}
fn contrast(a: u32, b: u32) -> f64 {
    let lum = |c: u32| {
        let f = |v: u32| {
            let v = v as f64 / 255.;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * f((c >> 16) & 255) + 0.7152 * f((c >> 8) & 255) + 0.0722 * f(c & 255)
    };
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}
const STATES: [StripState; 5] = [
    StripState::Rest,
    StripState::Hover,
    StripState::Selected,
    StripState::Dragging,
    StripState::Disabled,
];
/// A hidden or muted layer's name is quieter, and still reads.
fn floor(state: StripState) -> f64 {
    if state == StripState::Disabled {
        3.
    } else {
        4.5
    }
}
#[test]
fn strip_labels_read_on_every_layer_colour_and_state() {
    for scheme in [Scheme::Light, Scheme::Dark] {
        for increase_contrast in [false, true] {
            let t = Tokens::new(
                scheme,
                Accessibility {
                    increase_contrast,
                    ..Default::default()
                },
            );
            for fill in t.colors.labels.all() {
                for state in STATES {
                    let m = t.strip(fill, state);
                    // A translucent body is judged as it lands on the canvas,
                    // from its top to the foot of a name's descenders: the
                    // luminous foot below that carries no text.
                    for ground in [m.top, m.under_label].map(|c| mix(t.colors.panel, c, m.opacity))
                    {
                        assert!(
                            contrast(m.foreground, ground) >= floor(state),
                            "{scheme:?} {fill:06x} {state:?}: {:.2}",
                            contrast(m.foreground, ground)
                        );
                    }
                    // Outside the work area the body is veiled and the name
                    // is not: it reads there too.
                    let (veil, alpha) = t.work_area_veil();
                    for ground in [m.top, m.under_label].map(|c| mix(t.colors.panel, c, m.opacity))
                    {
                        let veiled = mix(ground, veil, alpha);
                        assert!(
                            contrast(m.foreground, veiled) >= floor(state),
                            "{scheme:?} {fill:06x} {state:?} veiled: {:.2}",
                            contrast(m.foreground, veiled)
                        );
                    }
                    if increase_contrast {
                        assert_eq!((m.top, m.opacity, m.shadow.1), (m.bottom, 1., 0.));
                    }
                }
            }
        }
    }
    assert_eq!(mix(0x000000, 0xffffff, 0.5), 0x808080);
}
#[test]
fn a_strip_keeps_its_layer_hue_and_its_states_differ_by_the_rim() {
    let dark = Tokens::new(Scheme::Dark, Accessibility::default());
    let green = dark.colors.labels.green;
    let m = dark.strip(green, StripState::Rest);
    for colour in [m.top, m.bottom, m.primary] {
        let [r, g, b] = [16, 8, 0].map(|s| (colour >> s) & 255);
        assert!(g > r && g > b, "green stays green: {colour:06x}");
    }
    let lightness = |c| pm_design::color::to_oklch(c).0;
    assert!(
        lightness(m.bottom) > lightness(m.top),
        "the lower half is the more luminous"
    );
    let hover = dark.strip(green, StripState::Hover);
    assert_eq!(
        (hover.top, hover.bottom),
        (m.top, m.bottom),
        "hover is the rim alone"
    );
    assert!(hover.rim.1 > m.rim.1 && hover.rim_top.1 > m.rim_top.1);
    assert!(dark.strip(green, StripState::Dragging).shadow.1 > 0.);
    // Selection is a 2 px ring at the edge: white on the dark canvas, the
    // hue's deepest shade on the light one.
    let light = Tokens::new(Scheme::Light, Accessibility::default());
    let selected = dark.strip(green, StripState::Selected);
    assert_eq!((selected.rim.0, selected.rim_width), (0xffffff, 2.));
    let pale = light.strip(light.colors.labels.green, StripState::Rest);
    let ringed = light.strip(light.colors.labels.green, StripState::Selected);
    assert_eq!((ringed.rim_width, ringed.rim_inset), (2., 0.));
    assert!(lightness(ringed.rim.0) < 0.5 && ringed.rim.1 == 1.);
    // The light canvas's strip is a pale tint of the hue under a dark name,
    // never the dark canvas's slab.
    assert!(lightness(pale.top) > 0.8 && lightness(pale.bottom) > lightness(pale.top));
    assert!(lightness(pale.foreground) < 0.45);
    assert!(lightness(m.top) < 0.45 && lightness(m.foreground) > 0.8);
    assert!(
        lightness(ringed.top) < lightness(pale.top),
        "selected is a step deeper"
    );
    let [r, g, b] = [16, 8, 0].map(|s| (pale.top >> s) & 255);
    assert!(g > r && g > b, "green stays green: {:06x}", pale.top);
    let none = light.strip(light.colors.labels.gray, StripState::Rest);
    assert!(
        lightness(none.top) > 0.8,
        "a neutral strip is light grey glass"
    );
    assert!(light.trim_grip().color != dark.trim_grip().color);
    assert_eq!(m.shadow.1, 0., "a resting strip has no shadow");
    let gray = dark.strip(dark.colors.labels.gray, StripState::Rest);
    assert!(
        pm_design::color::to_oklch(gray.bottom).1 < 0.01,
        "neutral stays neutral"
    );
    for t in [dark, Tokens::new(Scheme::Light, Accessibility::default())] {
        for fill in t.colors.labels.all() {
            let m = t.strip(fill, StripState::Rest);
            assert!(
                contrast(m.primary, m.bottom) >= 2.4,
                "{fill:06x} {:06x}",
                m.primary
            );
        }
        assert!(t.strip_radius(22.) >= t.radius.sm && t.strip_radius(200.) <= t.radius.lg);
        assert_eq!(t.playhead().line, t.metrics.border);
    }
}
