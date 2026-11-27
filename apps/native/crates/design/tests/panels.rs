use pm_design::{Accessibility, Scheme, Tokens};
#[test]
fn reviewed_panel_token_additions_patch_and_read_back() {
    for scheme in [Scheme::Light, Scheme::Dark] {
        for contrast in [false, true] {
            let mut t = Tokens::new(
                scheme,
                Accessibility {
                    increase_contrast: contrast,
                    ..Default::default()
                },
            );
            for (name, value, patched) in [
                ("metrics.kind_badge", 18., 20.),
                ("metrics.thumbnail_min", 120., 140.),
                ("metrics.status_dot", 6., 8.),
                ("colors.media_badge", 0., 0x102030 as f64),
            ] {
                assert_eq!(t.field(name), Some(value));
                assert!(Tokens::PATCHABLE.contains(&name));
                t.apply_patch([(name, patched)]).unwrap();
                assert_eq!(t.field(name), Some(patched), "{name}");
            }
            assert_eq!(t.metrics.media_badge_opacity, 0.55);
        }
    }
}
