//! Synthetic CPU paint-plan captures using the panel-owned sampler, not app screenshots.
#[path = "../tests/support/render_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use pm_design::{Accessibility, Scheme, Tokens, dots};
use pm_panel_timeline::{
    RowId,
    agent_ink::{AgentInk, Event, Kind},
    render::*,
    v4,
};
use std::{collections::BTreeSet, fmt::Write, path::PathBuf};
fn save(directory: &std::path::Path, name: &str, frame: &Frame) {
    std::fs::write(
        directory.join(format!("{name}.json")),
        frame.wire().to_string(),
    )
    .unwrap();
    let mut svg = String::from(
        "<svg xmlns='http://www.w3.org/2000/svg' width='960' height='150' viewBox='0 0 960 150'>",
    );
    for op in &frame.ops {
        if op.opacity <= 0. || op.covered {
            continue;
        }
        let b = op.bounds;
        let fill = format!("#{:06x}", op.color);
        let _ = write!(svg, "<g opacity='{}'>", op.opacity);
        if op.shape == "circle" {
            let _ = write!(
                svg,
                "<circle cx='{}' cy='{}' r='{}' fill='{fill}'/>",
                b.x + b.width / 2.,
                b.y + b.height / 2.,
                b.width / 2.
            );
        } else if op.shape.starts_with("diamond") {
            let _ = write!(
                svg,
                "<path d='M {} {} L {} {} L {} {} L {} {} Z' fill='{fill}'/>",
                b.x + b.width / 2.,
                b.y,
                b.x + b.width,
                b.y + b.height / 2.,
                b.x + b.width / 2.,
                b.y + b.height,
                b.x,
                b.y + b.height / 2.
            );
        } else if let Some(text) = &op.text {
            let text = text
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            let _ = write!(
                svg,
                "<text x='{}' y='{}' fill='{fill}' font-family='Helvetica' font-size='{}'>{text}</text>",
                b.x,
                b.y + b.height * 0.7,
                op.font_size.max(11.)
            );
        } else {
            let _ = write!(
                svg,
                "<rect x='{}' y='{}' width='{}' height='{}' rx='{}' fill='{fill}'/>",
                b.x,
                b.y,
                b.width.max(0.),
                b.height.max(0.),
                op.radius
            );
        }
        svg.push_str("</g>");
    }
    svg.push_str("</svg>");
    std::fs::write(directory.join(format!("{name}.svg")), svg).unwrap();
}
fn main() {
    let directory = PathBuf::from(std::env::args().nth(1).expect("external capture directory"));
    let prepare = std::env::args().any(|a| a == "--prepare");
    std::fs::create_dir_all(&directory).unwrap();
    for scheme in [Scheme::Dark, Scheme::Light] {
        let t = Tokens::new(scheme, Accessibility::default());
        let theme = format!("{scheme:?}").to_lowercase();
        let row = v4::row_id(&RowId::Layer("L".into()));
        let mut bar = fixture::clip("L");
        bar.name = "Lettering".into();
        let scene = Scene::new(
            vec![fixture::row(
                &row,
                RowContent::Layer {
                    clip: Some(bar),
                    summary: vec![fixture::key("k", 1.5)].into(),
                },
            )],
            6.,
            30.,
            &t,
        )
        .unwrap();
        let mut viewport = fixture::viewport(&t);
        viewport.height = 150.;
        viewport.scroll_time = 0.;
        let base = scene.prepare(viewport, &t).unwrap();
        let mut effect = AgentInk::default();
        for kind in [Kind::Create, Kind::Key] {
            effect.receive(
                Event {
                    layer: "L".into(),
                    property: None,
                    kind,
                    times: vec![1.5],
                    step: 1,
                    committed: false,
                },
                0,
            );
        }
        if prepare {
            save(&directory, &format!("normal-{theme}"), &base);
            for (n, group) in effect.groups(&base, &scene, &t, 950).iter().enumerate() {
                let mut source = base.clone();
                source.ops = group.indices.iter().map(|i| base.ops[*i].clone()).collect();
                save(&directory, &format!("{theme}-source-{n}"), &source);
            }
            continue;
        }
        for (stage, ms) in [
            ("coarse", 250),
            ("refining", 650),
            ("dotted", 950),
            ("resolving", 1350),
            ("crisp", 1700),
        ] {
            if ms >= 1000 {
                effect.commit(1, 1000);
            }
            let groups = effect.groups(&base, &scene, &t, ms);
            let affected: BTreeSet<_> = groups
                .iter()
                .flat_map(|g| g.indices.iter().copied())
                .collect();
            let mut frame = base.clone();
            frame.ops = base
                .ops
                .iter()
                .enumerate()
                .filter(|(i, _)| !affected.contains(i))
                .map(|(_, op)| op.clone())
                .collect();
            for (n, group) in groups.iter().enumerate() {
                let raw = std::fs::read(directory.join(format!("{theme}-source-{n}.rgba")))
                    .expect("run external source rasterizer after --prepare");
                assert_eq!(raw.len(), 960 * 150 * 4);
                let b = group.bounds;
                let sample = |p: [f32; 2]| {
                    let x = (b.x + p[0] as f64).floor().clamp(0., 959.) as usize;
                    let y = (b.y + p[1] as f64).floor().clamp(0., 149.) as usize;
                    let i = (y * 960 + x) * 4;
                    let alpha = raw[i + 3] as f32 / 255.;
                    [
                        raw[i] as f32 / 255. * alpha,
                        raw[i + 1] as f32 / 255. * alpha,
                        raw[i + 2] as f32 / 255. * alpha,
                        alpha,
                    ]
                };
                for (i, dot) in
                    dots::matrix([b.width as f32, b.height as f32], group.detail, sample)
                        .iter()
                        .enumerate()
                {
                    let mut op = base.ops[0].clone();
                    op.id = format!("dot:{n}:{i}");
                    op.shape = "circle";
                    op.bounds = Rect::new(
                        b.x + dot.center[0] as f64 - dot.radius as f64,
                        b.y + dot.center[1] as f64 - dot.radius as f64,
                        dot.radius as f64 * 2.,
                        dot.radius as f64 * 2.,
                    );
                    op.color = pm_design::color::mix(dot.color, t.colors.accent, 0.4);
                    op.opacity = dot.alpha as f64 * (1. - group.resolve as f64);
                    op.text = None;
                    op.detail = serde_json::Value::Null;
                    op.clip = Some(b);
                    op.covered = false;
                    frame.ops.push(op);
                }
                if group.resolve > 0. {
                    for &i in &group.indices {
                        let mut op = base.ops[i].clone();
                        op.opacity *= group.resolve as f64;
                        frame.ops.push(op);
                    }
                }
            }
            save(&directory, &format!("{theme}-{stage}"), &frame);
        }
    }
}
