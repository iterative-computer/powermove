use pm_design::{Accessibility, Scheme, Tokens};
use pm_panel_media::render::*;
struct Mono;
impl TextMetrics for Mono {
    fn width(&self, s: &str) -> f64 {
        s.chars().count() as f64
    } // Synthetic units, not production font metrics.
    fn boundaries(&self, s: &str) -> Vec<usize> {
        s.char_indices().map(|(i, _)| i).chain([s.len()]).collect()
    }
}
#[test]
fn cards_snapshot_every_scheme_contrast_selection_and_focus_state() {
    let mut snapshots = vec![];
    for scheme in [Scheme::Light, Scheme::Dark] {
        for contrast in [false, true] {
            for selected in [false, true] {
                for focused in [false, true] {
                    let t = Tokens::new(
                        scheme,
                        Accessibility {
                            increase_contrast: contrast,
                            ..Default::default()
                        },
                    );
                    let mut c = CardRecipe::new(
                        (12., 6.),
                        132.,
                        selected,
                        focused,
                        &[(Badge::Duration, 35.), (Badge::Status, 90.)],
                        3,
                        &t,
                    )
                    .unwrap();
                    c.measure_badges(&[18., 18.]).unwrap();
                    assert_eq!(
                        c.thumbnail.bounds.height,
                        c.thumbnail.bounds.width * 9. / 16.
                    );
                    assert_eq!(c.name_padding, t.metrics.gap as f64 - 2.);
                    assert_eq!(c.badge_inset, 5.);
                    assert_eq!(
                        c.badges[1].bounds.y + c.badges[1].bounds.height,
                        c.thumbnail.bounds.y + c.thumbnail.bounds.height - 5.
                    );
                    if selected {
                        assert_eq!(
                            c.selection.as_ref().unwrap().stroke,
                            if contrast { 3. } else { 2. }
                        );
                    }
                    if let (Some(selection), Some(focus)) = (&c.selection, &c.focus) {
                        assert!(focus.bounds.x < selection.bounds.x);
                    }
                    snapshots.push(serde_json::json!({"scheme":format!("{scheme:?}"),"contrast":contrast,"selected":selected,"focused":focused,"recipe":c}));
                }
            }
        }
    }
    let wire = serde_json::to_string_pretty(&snapshots).unwrap() + "\n";
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/cards.json");
    if std::env::var_os("PM_UPDATE_MEDIA_RENDER").is_some() {
        std::fs::write(&path, &wire).unwrap();
    }
    assert_eq!(wire, std::fs::read_to_string(path).unwrap());
}
#[test]
fn middle_truncation_retains_extension_and_balances_the_stem() {
    assert_eq!(
        middle_truncate("Atmosphere_loop.mov", 14., &Mono),
        "Atmos…loop.mov"
    );
    assert_eq!(middle_truncate("short.mov", 20., &Mono), "short.mov");
    assert_eq!(middle_truncate("Atmosphere_loop.mov", 4., &Mono), "…");
    assert_eq!(middle_truncate("名前動画.mov", 7., &Mono), "名…画.mov");
    assert_eq!(middle_truncate("file.mov", 0., &Mono), "");
}
#[test]
fn grid_columns_follow_native_width_and_invalid_sizes_fail_closed() {
    let t = Tokens::new(Scheme::Dark, Accessibility::default());
    assert_eq!(columns(300., None, &t), Ok(2));
    assert_eq!(columns(120., None, &t), Ok(1));
    assert_eq!(columns(600., None, &t), Ok(4));
    assert_eq!(columns(f64::NAN, None, &t), Err(Error::InvalidGeometry));
    assert_eq!(
        CardRecipe::new((0., 0.), f64::NAN, false, false, &[], 0, &t).unwrap_err(),
        Error::InvalidGeometry
    );
}
#[test]
fn status_only_badge_is_bottom_left_and_native_measurement_is_required() {
    let t = Tokens::new(Scheme::Light, Accessibility::default());
    let mut c =
        CardRecipe::new((0., 0.), 120., false, false, &[(Badge::Status, 60.)], 1, &t).unwrap();
    assert_eq!(c.badges[0].role, "status");
    assert_eq!(c.measure_badges(&[]), Err(Error::InvalidGeometry));
    c.measure_badges(&[18.]).unwrap();
    assert_eq!(c.badges[0].bounds.y, 120. * 9. / 16. - 5. - 18.);
    assert_eq!(c.actions[0].color, 0);
    assert_eq!(c.actions[0].opacity, 0.55);
}
