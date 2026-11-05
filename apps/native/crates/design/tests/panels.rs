use pm_design::{Accessibility, Scheme, Tokens, panels};
#[test]
fn reviewed_panel_token_additions_are_patchable_without_changing_old_values() {
    for scheme in [Scheme::Light, Scheme::Dark] {
        for contrast in [false, true] {
            let mut t = Tokens::new(
                scheme,
                Accessibility {
                    increase_contrast: contrast,
                    ..Default::default()
                },
            );
            let before = t.clone();
            for (name, value) in [
                ("metrics.kind_badge", 18.),
                ("metrics.thumbnail_min", 120.),
                ("metrics.status_dot", 6.),
                ("colors.media_badge", 0.),
            ] {
                assert_eq!(t.field(name), Some(value));
                assert!(Tokens::PATCHABLE.contains(&name));
                t.apply_patch([(name, value)]).unwrap();
            }
            assert_eq!(t, before);
            assert_eq!(panels::MEDIA_BADGE_ALPHA, 0.55);
        }
    }
}
