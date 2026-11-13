use pm_scene3d::{light_tool::*, navigation::*, presentation::*};
fn pose() -> Pose {
    Pose {
        position: [3., 2., 6.],
        target: [0.; 3],
        zoom: 1.,
    }
}
fn lens() -> Lens {
    Lens::Perspective {
        fov: 45.,
        aspect: 16. / 9.,
        near: 0.01,
        far: 1000.,
    }
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-10, "{a} != {b}");
}
#[test]
fn orbit_preserves_target_and_distance() {
    let p = pose();
    let end = orbit(p, 100., -30.);
    assert_eq!(end.target, p.target);
    near(length(sub(end.position, end.target)), 7.);
}
#[test]
fn pan_moves_eye_and_target_equally() {
    let p = pose();
    let end = pan(p, 30., -10., lens(), 1080., [1., 0., 0.], [0., 1., 0.]);
    assert_eq!(end.position, add(p.position, end.target));
    assert!(end.position[0] < p.position[0]);
}
#[test]
fn dolly_handles_zero_distance_and_clamps_orthographic_zoom() {
    let p = Pose {
        position: [0.; 3],
        ..pose()
    };
    let end = dolly(p, 100., lens());
    assert!(length(sub(end.position, end.target)) >= 0.05);
    let end = dolly(
        p,
        -1e20,
        Lens::Orthographic {
            width: 6.,
            height: 6.,
        },
    );
    assert!((0.01..=1000.).contains(&end.zoom));
}
#[test]
fn frame_fits_portrait_and_landscape() {
    for aspect in [0.5, 2.] {
        let b = Bounds {
            min: [-1., -1., -1.],
            max: [1., 1., 1.],
        };
        let l = Lens::Perspective {
            fov: 45.,
            aspect,
            near: 0.01,
            far: 1000.,
        };
        let end = frame(pose(), b, l);
        assert_eq!(end.target, [0.; 3]);
        let vertical = (22.5f64.to_radians().tan()).atan();
        let horizontal = (vertical.tan() * aspect).atan();
        assert!(length(end.position) * vertical.min(horizontal).sin() >= b.radius() * 1.25 - 1e-12);
    }
}
#[test]
fn ownership_cancel_and_fine_navigation_are_transient() {
    let mut n = Navigation::default();
    assert!(
        !n.begin(
            pose(),
            lens(),
            0,
            false,
            false,
            true,
            0.,
            0.,
            500.,
            [1., 0., 0.],
            [0., 1., 0.]
        )
        .unwrap()
    );
    assert!(
        n.begin(
            pose(),
            lens(),
            0,
            true,
            false,
            true,
            0.,
            0.,
            500.,
            [1., 0., 0.],
            [0., 1., 0.]
        )
        .unwrap()
    );
    assert_eq!(n.view, View::Editor);
    assert_eq!(
        n.moved(100., 20., true).unwrap().unwrap(),
        orbit(pose(), 10., 2.)
    );
    assert_eq!(n.cancel(), Some(pose()));
    assert_eq!(n.view, View::Camera);
    assert!(!n.active());
}
#[test]
fn wheel_is_claimed_only_by_navigation_or_option() {
    let mut n = Navigation::default();
    assert!(
        n.wheel(pose(), lens(), 10., 0, 500., false, true)
            .unwrap()
            .is_none()
    );
    let end = n
        .wheel(pose(), lens(), 1., 1, 500., true, true)
        .unwrap()
        .unwrap();
    assert_eq!(end, dolly(pose(), 16., lens()));
    assert!(
        n.wheel(pose(), lens(), f64::NAN, 0, 500., true, true)
            .is_err()
    );
}
#[test]
fn invalid_navigation_inputs_are_rejected() {
    let mut n = Navigation::default();
    n.mode = Mode::Orbit;
    assert!(
        n.begin(
            pose(),
            Lens::Perspective {
                fov: 0.,
                aspect: 1.,
                near: 1.,
                far: 2.
            },
            0,
            false,
            false,
            true,
            0.,
            0.,
            1.,
            [1., 0., 0.],
            [0., 1., 0.]
        )
        .is_err()
    );
    assert!(n.moved(f64::INFINITY, 0., false).is_err());
}
fn light() -> Snapshot {
    Snapshot {
        position: [0., 3., 4.],
        target: [0.; 3],
        intensity: 6.,
        width: 2.,
        height: 1.2,
        angle: 45.,
    }
}
#[test]
fn reflect_obeys_reflection_law() {
    let p = [0.; 3];
    let n = [0., 1., 0.];
    let eye = [0., 3., 4.];
    let at = highlight_placement(p, n, eye, 5.).unwrap();
    near(length(at), 5.);
    near(at[1], 3.);
    near(at[2], -4.);
    assert!(highlight_placement(p, [0.; 3], eye, 5.).is_err());
}
#[test]
fn upright_glass_highlight_stays_above_the_floor_on_the_same_reflection_ray() {
    let point = [0., 1., 0.];
    let normal = [0., 0., 1.];
    let eye = [0., 4., 6.];
    let requested = 6.;
    let original = highlight_placement(point, normal, eye, requested).unwrap();
    assert!(original[1] < 0., "reproduce the reported below-floor placement");
    for size in [None, Some(0.2)] {
        let (at, width) = highlight_above_floor(point, normal, eye, requested, size, 0.).unwrap();
        assert!(at[1] - width * 0.31 > 0., "whole panel clears the floor");
        assert!(length(sub(at, point)) < requested);
        let direction = normalize(sub(at, point));
        let reflected = normalize(sub(original, point));
        for i in 0..3 { near(direction[i], reflected[i]); }
        if let Some(size) = size { near(width, size); }
    }
    assert!(highlight_above_floor([0.;3], normal, eye, requested, None, 0.).is_err());
    let (at, width) = highlight_above_floor([0.;3], [0.,1.,0.], eye, requested, None, 0.).unwrap();
    assert!(at[1] - width * 0.31 > 0.);
}
#[test]
fn direct_aim_faces_surface_normal() {
    let h = Hit {
        point: [1., 0., 2.],
        normal: [0., 1., 0.],
        eye: [0., 3., 4.],
    };
    let end = aim(light(), h, Aim::Direct).unwrap();
    assert_eq!(end.target, h.point);
    assert_eq!(end.position, [1., 5., 2.]);
}
#[test]
fn distance_adjustment_preserves_inverse_square_brightness() {
    let end = adjust(light(), Kind::Area, Adjust::Distance, 10.).unwrap();
    near(length(sub(end.position, end.target)), 10.);
    near(end.intensity, 24.);
}
#[test]
fn size_preserves_softbox_aspect_ratio() {
    let end = adjust(light(), Kind::Area, Adjust::Size, 10.).unwrap();
    near(end.width / end.height, 2. / 1.2);
    near(end.width, 10.);
    assert!(adjust(light(), Kind::Area, Adjust::Size, f64::NAN).is_err());
}
struct Picker;
impl SurfacePicker for Picker {
    fn pick(&self, _: f64, _: f64) -> pm_scene3d::Result<Option<Hit>> {
        Ok(Some(Hit {
            point: [1., 0., 0.],
            normal: [0., 1., 0.],
            eye: [0., 3., 4.],
        }))
    }
}
#[test]
fn light_tool_cancel_restores_and_release_is_one_change() {
    let mut tool = LightTool::default();
    tool.begin("key".into(), light(), Aim::Direct, 0.5, 0.5)
        .unwrap();
    let preview = tool.moved(0.6, 0.5, &Picker).unwrap().unwrap();
    assert_ne!(preview, light());
    assert_eq!(tool.cancel(), Some(("key".into(), light())));
    tool.begin("key".into(), light(), Aim::Reflect, 0.5, 0.5)
        .unwrap();
    tool.moved(0.6, 0.5, &Picker).unwrap();
    let c = tool.end().unwrap();
    assert_eq!(c.before, light());
    assert_eq!(c.id, "key");
    assert!(tool.end().is_none());
}
#[test]
fn light_orbit_preserves_distance() {
    let mut tool = LightTool::default();
    tool.begin("key".into(), light(), Aim::Orbit, 0., 0.)
        .unwrap();
    let end = tool.moved(0.25, 0.1, &Picker).unwrap().unwrap();
    near(length(sub(end.position, end.target)), 5.);
}
#[test]
fn design_names_are_canonical_with_stored_types_unchanged() {
    assert_eq!(
        [Kind::Area, Kind::Spot, Kind::Point, Kind::Sun].map(light_name),
        ["Softbox", "Spotlight", "Bulb", "Sun"]
    );
    assert_eq!(
        Failure::CadConversion.message(),
        "This CAD model couldn’t be converted."
    );
}
#[test]
fn progress_never_fabricates_samples_or_shows_live() {
    assert!(
        Progress::Live
            .status(Preview::Quality, true, true)
            .is_none()
    );
    assert!(
        Progress::Refining {
            completed: 8,
            total: 0
        }
        .status(Preview::Quality, true, true)
        .is_none()
    );
    assert!(
        Progress::Refining {
            completed: 8,
            total: 16
        }
        .status(Preview::Quality, false, true)
        .is_none()
    );
    assert!(
        Progress::Refining {
            completed: 8,
            total: 16
        }
        .status(Preview::Fast, true, true)
        .is_none()
    );
    let s = Progress::Refining {
        completed: 8,
        total: 16,
    }
    .status(Preview::Quality, true, true)
    .unwrap();
    assert_eq!(s.text, "Refining · 50 %");
    assert_eq!(s.fraction, Some(0.5));
    let s = Progress::Preparing {
        triangles: 1_000_000,
        completed: 2,
        total: 4,
    }
    .status(Preview::Fast, false, false)
    .unwrap();
    assert_eq!(s.text, "Preparing 1.0M triangles");
    assert_eq!(s.fraction, Some(0.5));
}
#[test]
fn newly_pointed_light_uses_source_power_and_aspect_ratio() {
    let h = Hit {
        point: [0.; 3],
        normal: [0., 1., 0.],
        eye: [0., 3., 4.],
    };
    let s = new_light(Kind::Area, Aim::Reflect, h).unwrap();
    near(length(sub(s.position, s.target)), 4.5);
    near(s.height / s.width, 0.62);
    assert_eq!(s.intensity, 6.);
    assert!(new_light(Kind::Sun, Aim::Orbit, h).is_err());
}
#[test]
fn tuning_coalesces_and_cancel_retains_the_original_snapshot() {
    let mut t = Tuning::default();
    t.begin("key".into(), light()).unwrap();
    t.set(Kind::Area, Adjust::Power, 10.).unwrap();
    t.set(Kind::Area, Adjust::Power, 20.).unwrap();
    let c = t.commit().unwrap();
    assert_eq!(c.before, light());
    assert_eq!(c.after.intensity, 20.);
    assert!(t.commit().is_none());
    t.begin("key".into(), light()).unwrap();
    t.set(Kind::Area, Adjust::Size, 10.).unwrap();
    assert_eq!(t.cancel(), Some(("key".into(), light())));
}
