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
/// Strip names are semibold (large-bold text): 3:1 in every state, a
/// hidden or muted layer's included.
fn floor(_state: StripState) -> f64 {
    3.
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
        lightness(m.top) < lightness(m.bottom),
        "as the reference: deep at the top, more luminous toward the foot"
    );
    assert!(m.opacity < 1., "glass: the grid and the row behind show through");
    let hover = dark.strip(green, StripState::Hover);
    assert_eq!(
        (hover.top, hover.bottom),
        (m.top, m.bottom),
        "hover is the rim alone"
    );
    assert!(hover.ring.1 > m.ring.1 && hover.light.1 > m.light.1);
    assert!(dark.strip(green, StripState::Dragging).shadow.1 > 0.);
    // Selection is a 2 px ring at the edge: white on the dark canvas, the
    // hue's deepest shade on the light one.
    let light = Tokens::new(Scheme::Light, Accessibility::default());
    let selected = dark.strip(green, StripState::Selected);
    assert_eq!((selected.ring.0, selected.ring_width), (0xffffff, 2.));
    let pale = light.strip(light.colors.labels.green, StripState::Rest);
    let ringed = light.strip(light.colors.labels.green, StripState::Selected);
    assert_eq!(ringed.ring_width, 2.);
    assert!(lightness(ringed.ring.0) < 0.5 && ringed.ring.1 == 1.);
    // The light canvas's strip is a pale tint of the hue under a dark name,
    // never the dark canvas's slab.
    assert!(lightness(pale.top) > 0.8 && lightness(pale.bottom) > lightness(pale.top));
    assert!(lightness(pale.foreground) < 0.45);
    // The dark canvas's slab: vivid glass, never the light canvas's pale
    // tint. Green is a light hue: as bright as the reference's amber, under a
    // dark name and over a waveform in a deeper shade of the hue; violet
    // stays the deeper slab under a pale name.
    assert!(lightness(m.top) < 0.8 && lightness(m.bottom) > 0.7);
    assert!(lightness(m.foreground) < 0.3 && lightness(m.primary) < lightness(m.bottom));
    let violet = dark.strip(dark.colors.labels.violet, StripState::Rest);
    assert!(lightness(violet.top) < 0.6 && lightness(violet.foreground) > 0.8);
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
        // v1's playhead: the accent, under an 11 pt pin resting on the ruler.
        assert_eq!(t.playhead().color, t.colors.accent);
        assert_eq!((t.playhead().head, t.playhead().head_foot), (11., 1.));
    }
}
/// The dark canvas's slab glows at its foot where its name is dark: its own
/// hue a step more luminous than the body, only in the band under a name,
/// fainter on a disabled strip, and absent under a pale name and from the
/// light canvas's pale tint.
#[test]
fn a_dark_strip_glows_at_its_foot_under_the_name() {
    let oklch = pm_design::color::to_oklch;
    let dark = Tokens::new(Scheme::Dark, Accessibility::default());
    let light = Tokens::new(Scheme::Light, Accessibility::default());
    let l = &dark.colors.labels;
    for hue in [l.red, l.orange, l.yellow, l.green, l.teal, l.blue, l.violet] {
        let m = dark.strip(hue, StripState::Rest);
        let (body, glow) = (oklch(m.bottom), oklch(m.glow.0));
        let step = glow.0 - body.0;
        assert!((0.05..0.16).contains(&step), "{hue:06x}: {step}");
        let turn = (glow.2 - body.2).abs();
        assert!(turn.min(360. - turn) < 12., "{hue:06x}: its own hue");
        // A collapsed layer's summary marks stand in the band in the name's
        // ink: the glow never costs them contrast, so it is a dark name's.
        let dark_name = oklch(m.foreground).0 < 0.5;
        if dark_name {
            assert!(m.glow.1 > 0.4 && m.glow.1 < 0.8, "{hue:06x}");
        } else {
            assert_eq!(m.glow.1, 0., "{hue:06x}");
        }
        let lit = pm_design::mix(m.bottom, m.glow.0, m.glow.1);
        let (was, is) = (
            contrast(m.foreground, m.bottom),
            contrast(m.foreground, lit),
        );
        assert!(is >= was, "{hue:06x}: marks {was:.2} -> {is:.2}");
        let off = dark.strip(hue, StripState::Disabled);
        assert!(
            off.glow.1 <= m.glow.1 / 3.,
            "{hue:06x}: a disabled strip is dim"
        );
        let chosen = dark.strip(hue, StripState::Selected);
        assert!(oklch(chosen.glow.0).0 >= glow.0, "{hue:06x}");
    }
    for hue in [light.colors.labels.green, light.colors.labels.violet] {
        assert_eq!(light.strip(hue, StripState::Rest).glow.1, 0., "{hue:06x}");
    }
    // The band ends where a name's foot is measured: no name stands on it.
    assert!(pm_design::GLOW_REACH > 0.15 && pm_design::GLOW_REACH < 0.3);
    // Green is the brightest of the slabs, never muted.
    let (green, blue) = (
        dark.strip(l.green, StripState::Rest),
        dark.strip(l.blue, StripState::Rest),
    );
    assert!(oklch(green.bottom).0 > oklch(blue.bottom).0);
    assert!(oklch(green.bottom).1 > 0.16, "vivid: {:06x}", green.bottom);
}

/// A waveform is light caught inside the glass (the owner's reference: pale
/// peach on amber, about 0.09 lighter than the slab once laid on it): on
/// the dark canvas a pale tint lighter than the slab's foot for every hue;
/// on the light canvas a soft mid-tone about 0.12 under it, never dark. The
/// level line is the hue's light tone, never dark either.
#[test]
fn a_waveform_is_light_inside_the_glass() {
    let lightness = |c| pm_design::color::to_oklch(c).0;
    let laid = |m: &Strip, (c, a): (u32, f32)| mix(m.bottom, c, a);
    for scheme in [Scheme::Dark, Scheme::Light] {
        let t = Tokens::new(scheme, Accessibility::default());
        let l = &t.colors.labels;
        for hue in [l.red, l.orange, l.yellow, l.green, l.teal, l.blue, l.violet] {
            let m = t.strip(hue, StripState::Rest);
            let step = lightness(laid(&m, m.wave)) - lightness(m.bottom);
            if scheme == Scheme::Dark {
                assert!((0.06..0.16).contains(&step), "{hue:06x}: {step}");
                assert!(lightness(m.level.0) > 0.85, "{hue:06x}: a pale line");
            } else {
                assert!((-0.16..-0.09).contains(&step), "{hue:06x}: {step}");
                assert!(lightness(m.wave.0) > 0.65 && lightness(m.level.0) > 0.55, "{hue:06x}");
            }
        }
    }
}
/// Two surface tones per appearance (owner, 2026-11-26): the chrome is the
/// window colour as the ground (title bar and sidebar draw that same ground),
/// and light content is solid paper, so Home's sheet, the editor's panels and
/// the viewer's surround are one tone.
#[test]
fn two_surface_tones() {
    for scheme in [Scheme::Light, Scheme::Dark] {
        let t = Tokens::new(scheme, Accessibility::default());
        assert_eq!(
            t.ground(),
            (t.colors.window, t.translucency.window),
            "{scheme:?}"
        );
    }
    let light = Tokens::new(Scheme::Light, Accessibility::default());
    assert_eq!(light.translucency.panel, 1., "light content is solid");
    assert_eq!(
        light.viewer_surround(),
        light.colors.panel,
        "the viewer's surround is the content tone"
    );
}
