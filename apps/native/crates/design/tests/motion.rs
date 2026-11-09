use pm_design::motion::{Animated, Clock, Preset, Spring};
use pm_design::{Accessibility, Scheme, Tokens};

fn run(a: &mut Animated<1>, hz: f32, seconds: f32) -> (f32, usize) {
    // Peak value while moving, and frames until rest.
    let mut frames = 0;
    let mut peak = f32::MIN;
    while frames < (hz * seconds) as usize && a.step(1. / hz) {
        peak = peak.max(a.get());
        frames += 1;
    }
    (peak, frames)
}

#[test]
fn settles_to_rest_and_stops_ticking() {
    let t = Tokens::new(Scheme::Dark, Accessibility::default());
    for preset in [
        Preset::Layout,
        Preset::Show,
        Preset::HoverIn,
        Preset::HoverOut,
        Preset::HoverSwitch,
        Preset::Pop,
        Preset::Enter,
        Preset::Exit,
    ] {
        let mut a = Animated::unit(0.);
        a.to(1., t.spring(preset));
        let (_, frames) = run(&mut a, 120., 3.);
        assert!(frames < 360, "{preset:?} never rests");
        assert_eq!((a.get(), a.velocity[0]), (1., 0.), "{preset:?}");
        assert!(!a.step(1. / 120.), "a resting value asks for no frame");
    }
}

#[test]
fn same_curve_at_60_and_120_hz() {
    let spring = Preset::Layout.spring(&Tokens::new(Scheme::Light, Accessibility::default()).motion);
    let sample = |hz: f32| {
        let mut a = Animated::new([0.], spring, 0.05);
        a.animate_to([100.], spring);
        for _ in 0..(hz * 0.1) as usize {
            a.step(1. / hz);
        }
        a.value[0]
    };
    assert!((sample(60.) - sample(120.)).abs() < 1.5, "{} vs {}", sample(60.), sample(120.));
}

#[test]
fn retarget_keeps_value_and_velocity() {
    let spring = Spring::new(420., 31.);
    let mut a = Animated::new([0.], spring, 0.05);
    a.animate_to([100.], spring);
    for _ in 0..6 {
        a.step(1. / 120.);
    }
    let (value, velocity) = (a.value[0], a.velocity[0]);
    assert!(velocity > 0.);
    a.animate_to([-50.], spring);
    assert_eq!((a.value[0], a.velocity[0]), (value, velocity));
    a.step(1. / 120.);
    assert!(a.value[0] > value, "momentum carries through the interruption");
}

#[test]
fn pop_overshoots_once_and_enter_has_its_bounce() {
    let t = Tokens::new(Scheme::Dark, Accessibility::default());
    let mut pop = Animated::unit(0.);
    pop.to(1., t.spring(Preset::Pop));
    assert!(run(&mut pop, 120., 2.).0 > 1.05);
    // Fine precision so the half bounce's tiny overshoot is observed.
    let mut enter = Animated::new([t.motion.start_scale as f32], Spring::SNAP, 1e-7);
    enter.animate_to([1.], t.spring(Preset::Enter));
    let (peak, _) = run(&mut enter, 120., 2.);
    assert!(peak > 1. && peak < 1.01, "half-bounce enter, peak {peak}");
}

#[test]
fn reduce_motion_snaps_places_and_cross_fades_opacity() {
    let t = Tokens::new(
        Scheme::Dark,
        Accessibility {
            reduce_motion: true,
            ..Default::default()
        },
    );
    let mut place = Animated::new([0.], Spring::SNAP, 0.05);
    place.animate_to([40.], t.spring(Preset::Layout));
    assert_eq!(place.value[0], 40.);
    assert!(!place.moving());
    let mut fade = Animated::unit(0.);
    fade.to(1., t.spring(Preset::HoverIn));
    let mut arrived = None;
    for frame in 1..=120 {
        fade.step(1. / 120.);
        assert!(fade.get() <= 1., "a cross-fade never overshoots");
        if arrived.is_none() && fade.get() >= 0.997 {
            arrived = Some(frame as f64 / 120.);
        }
    }
    let arrived = arrived.unwrap();
    assert!((arrived - t.motion.reduced).abs() < 0.02, "cross-fade arrived after {arrived}s");
    assert!(!fade.moving());
}

#[test]
fn a_stall_never_makes_a_value_jump() {
    let spring = Spring::new(420., 31.);
    let mut a = Animated::new([0.], spring, 0.05);
    a.animate_to([100.], spring);
    a.step(2.);
    assert!(a.value[0] < 5., "a 2 s hitch steps as one short frame");
    let mut clock = Clock::default();
    let now = std::time::Instant::now();
    assert_eq!(clock.tick(now), 0.);
    assert!(clock.tick(now + std::time::Duration::from_millis(8)) > 0.);
    clock.rest();
    assert_eq!(clock.tick(now + std::time::Duration::from_secs(5)), 0.);
}
