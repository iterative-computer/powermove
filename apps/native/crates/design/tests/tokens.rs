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
    assert_eq!(t.metrics.panel_radius, 8.);
    assert_eq!((t.metrics.header, t.metrics.status), (38., 24.));
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
    assert_eq!(t.colors.accent, 0xff6019);
}
