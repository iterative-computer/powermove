//! Blender's default 3D view navigation steps (zoom notch, numpad orbit and
//! the numpad views), as the viewer's wheel and keys use them.
use pm_scene3d::navigation::*;
fn pose() -> Pose {
    Pose {
        position: [3., 2., 6.],
        target: [0.5, 0., -1.],
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
fn dist(p: Pose) -> f64 {
    length(sub(p.position, p.target))
}
fn close(a: Vec3, b: Vec3) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() < 1e-9)
}
#[test]
fn a_wheel_notch_is_blenders_1_2_step_toward_the_view_centre() {
    let p = pose();
    let z = zoom_step(p, 1., lens());
    assert!((dist(p) / dist(z) - ZOOM_STEP).abs() < 1e-9);
    // Straight in: same direction, same centre.
    assert!(close(
        normalize(sub(z.position, z.target)),
        normalize(sub(p.position, p.target))
    ));
    assert_eq!(z.target, p.target);
    // A notch out undoes a notch in.
    assert!(close(zoom_step(z, -1., lens()).position, p.position));
    // Orthographic: the zoom factor steps instead.
    let o = Lens::Orthographic {
        width: 16.,
        height: 9.,
    };
    assert!((zoom_step(p, 2., o).zoom - 1.44).abs() < 1e-9);
}
#[test]
fn numpad_orbit_steps_are_15_degrees_and_undo_each_other() {
    let p = pose();
    let right = orbit_by(p, ORBIT_STEP, 0.);
    let back = orbit_by(right, -ORBIT_STEP, 0.);
    assert!(close(back.position, p.position));
    assert!(
        (dist(right) - dist(p)).abs() < 1e-9,
        "orbits keep the distance"
    );
    // The turn is 15° about the vertical through the view centre.
    let flat = |v: Vec3| normalize([v[0], 0., v[2]]);
    let (a, b) = (
        flat(sub(p.position, p.target)),
        flat(sub(right.position, right.target)),
    );
    assert!((dot(a, b).acos().to_degrees() - 15.).abs() < 1e-6);
    // Up looks from higher; the opposite view (numpad 9) is half a turn.
    assert!(orbit_by(p, 0., ORBIT_STEP).position[1] > p.position[1]);
    let opposite = orbit_by(p, 180., 0.);
    assert!(close(orbit_by(opposite, 180., 0.).position, p.position));
}
#[test]
fn numpad_views_look_at_the_centre_from_their_side_at_the_same_distance() {
    let p = pose();
    let d = dist(p);
    for (side, dir) in [
        (Side::Front, [0., 0., 1.]),
        (Side::Back, [0., 0., -1.]),
        (Side::Right, [1., 0., 0.]),
        (Side::Left, [-1., 0., 0.]),
    ] {
        let v = side_view(p, side);
        assert_eq!(v.target, p.target);
        assert!(close(normalize(sub(v.position, v.target)), dir), "{side:?}");
        assert!((dist(v) - d).abs() < 1e-9);
    }
    // Top and Bottom look almost straight down and up (a heading remains).
    let top = normalize(sub(side_view(p, Side::Top).position, p.target));
    let bottom = normalize(sub(side_view(p, Side::Bottom).position, p.target));
    assert!(top[1] > 0.999 && bottom[1] < -0.999);
    assert!(top[2] > 0. && bottom[2] > 0.);
}
