#[path = "support/render_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use pm_design::{Accessibility, Scheme, Tokens};
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
    }
}
#[test]
fn reduced_motion_has_no_light_core_and_idle_does_not_modify_a_frame() {
    let t = Tokens::new(
        Scheme::Light,
        Accessibility {
            reduce_motion: true,
            ..Default::default()
        },
    );
    let scene = scene(&t);
    let v = fixture::viewport(&t);
    let baseline = scene.prepare(v, &t).unwrap();
    let mut f = baseline.clone();
    let mut ink = AgentInk::default();
    ink.paint(&mut f, &scene, &t, 0);
    assert_eq!(f.ops, baseline.ops);
    ink.receive(event(Kind::Key), 0);
    ink.paint(&mut f, &scene, &t, 100);
    assert!(f.ops.iter().all(|o| !o.id.starts_with("agent-core")));
    assert!(!ink.moving(15_000));
    let mut idle = baseline.clone();
    ink.paint(&mut idle, &scene, &t, 15_000);
    assert_eq!(idle.ops, baseline.ops);
}
#[test]
fn wash_is_confined_to_the_header_column() {
    let t = Tokens::new(Scheme::Dark, Default::default());
    let scene = scene(&t);
    let mut f = scene.prepare(fixture::viewport(&t), &t).unwrap();
    let mut ink = AgentInk::default();
    ink.receive(event(Kind::Focus), 0);
    ink.paint(&mut f, &scene, &t, 200);
    let washes: Vec<_> = f
        .ops
        .iter()
        .filter(|o| o.id.starts_with("agent-wash"))
        .collect();
    assert_eq!(washes.len(), 12);
    assert!(
        washes
            .iter()
            .all(|o| o.bounds.x + o.bounds.width <= f.viewport.layer_column)
    );
}
#[test]
fn effect_cpu_p95_stays_under_four_tenths_of_a_millisecond() {
    let t = Tokens::new(Scheme::Dark, Default::default());
    let scene = scene(&t);
    let base = scene.prepare(fixture::viewport(&t), &t).unwrap();
    let mut ink = AgentInk::default();
    ink.receive(event(Kind::Create), 0);
    ink.receive(event(Kind::Key), 0);
    let mut times = Vec::new();
    for ms in (0..1200).step_by(20) {
        let mut f = base.clone();
        let start = std::time::Instant::now();
        ink.paint(&mut f, &scene, &t, ms);
        times.push(start.elapsed().as_secs_f64() * 1000.);
    }
    times.sort_by(f64::total_cmp);
    let p95 = times[times.len() * 95 / 100];
    assert!(p95 <= 0.4, "headless effect p95 {p95:.3} ms exceeds 0.4 ms");
}
