#[path = "support/render_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use pm_design::{Accessibility, Scheme, Tokens, dots};
use pm_panel_timeline::{
    RowId,
    agent_ink::{AgentInk, Event, Kind},
    render::*,
    v4,
};
fn scene(t: &Tokens) -> Scene {
    let row = v4::row_id(&RowId::Layer("L".into()));
    Scene::new(
        vec![fixture::row(
            &row,
            RowContent::Layer {
                clip: Some(fixture::clip("L")),
                summary: vec![fixture::key("k", 1.5)].into(),
            },
        )],
        6.,
        30.,
        t,
    )
    .unwrap()
}
fn event(kind: Kind) -> Event {
    Event {
        layer: "L".into(),
        property: None,
        kind,
        times: vec![1.5],
        step: 1,
        committed: false,
    }
}
#[test]
fn groups_sample_real_strip_label_and_key_then_resolve_together() {
    let t = Tokens::new(Scheme::Dark, Default::default());
    let scene = scene(&t);
    let frame = scene.prepare(fixture::viewport(&t), &t).unwrap();
    let normal = frame.clone();
    let mut ink = AgentInk::default();
    ink.receive(event(Kind::Create), 0);
    ink.receive(event(Kind::Key), 0);
    let groups = ink.groups(&frame, &scene, &t, 600);
    assert_eq!(groups.len(), 1);
    let g = &groups[0];
    assert!(g.indices.iter().any(|i| frame.ops[*i].id == "clip:L"));
    assert!(
        g.indices
            .iter()
            .any(|i| frame.ops[*i].id.starts_with("key:"))
    );
    assert_eq!(g.resolve, 0.);
    assert_eq!(g.detail, dots::detail(0.6));
    assert_eq!(frame.ops, normal.ops);
    assert!(!ink.moving(1000));
    ink.commit(1, 1000);
    let groups = ink.groups(&frame, &scene, &t, 1350);
    assert_eq!(groups.len(), 1);
    assert!((groups[0].resolve - 0.5).abs() < 0.01);
    assert!(ink.groups(&frame, &scene, &t, 1700).is_empty());
    assert!(!ink.moving(1700));
}
#[test]
fn generations_never_move_or_shrink_and_coarse_stays_unreadable() {
    let sample = |_| [0.5_f32, 0.2, 0.1, 1.];
    let coarse = dots::matrix([320., 24.], dots::detail(0.4), sample);
    let fine = dots::matrix([320., 24.], dots::detail(0.82), sample);
    assert!(fine.len() > coarse.len() * 4);
    for d in coarse {
        assert!(
            fine.iter()
                .any(|f| f.center == d.center && f.radius == d.radius && f.alpha >= d.alpha)
        );
    }
}
#[test]
fn reduced_motion_crossfades_only_and_stops_at_rest() {
    let t = Tokens::new(
        Scheme::Light,
        Accessibility {
            reduce_motion: true,
            ..Default::default()
        },
    );
    let scene = scene(&t);
    let f = scene.prepare(fixture::viewport(&t), &t).unwrap();
    let mut ink = AgentInk::default();
    let mut e = event(Kind::Create);
    e.committed = true;
    ink.receive(e, 0);
    let g = ink.groups(&f, &scene, &t, 80);
    assert!(g.iter().all(|g| g.reduced));
    assert!((g[0].resolve - 0.5).abs() < 0.01);
    assert!(!ink.moving_with_motion(160, true));
    assert!(ink.groups(&f, &scene, &t, 160).is_empty());
}
#[test]
fn native_group_planning_p95_stays_under_four_tenths_of_a_millisecond() {
    let t = Tokens::new(Scheme::Dark, Default::default());
    let scene = scene(&t);
    let f = scene.prepare(fixture::viewport(&t), &t).unwrap();
    let mut ink = AgentInk::default();
    ink.receive(event(Kind::Create), 0);
    ink.receive(event(Kind::Key), 0);
    let mut times = Vec::new();
    for ms in (0..1200).step_by(20) {
        let start = std::time::Instant::now();
        let _ = ink.groups(&f, &scene, &t, ms);
        times.push(start.elapsed().as_secs_f64() * 1000.);
    }
    times.sort_by(f64::total_cmp);
    assert!(times[times.len() * 95 / 100] <= 0.4);
}
