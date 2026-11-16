//! Disposable headless timeline paint captures, never application screenshots.
#[path = "../tests/support/render_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use pm_design::{Accessibility, Scheme, Tokens};
use pm_panel_timeline::{
    RowId,
    agent_ink::{AgentInk, Event, Kind},
    render::*,
    v4,
};
use std::{fmt::Write, path::PathBuf};
fn main() {
    let directory = PathBuf::from(std::env::args().nth(1).expect("external capture directory"));
    std::fs::create_dir_all(&directory).unwrap();
    for scheme in [Scheme::Dark, Scheme::Light] {
        let tokens = Tokens::new(scheme, Accessibility::default());
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
            &tokens,
        )
        .unwrap();
        let mut viewport = fixture::viewport(&tokens);
        viewport.height = 150.;
        viewport.scroll_time = 0.;
        let mut ink = AgentInk::default();
        ink.receive(
            Event {
                layer: "L".into(),
                property: None,
                kind: Kind::Create,
                times: vec![],
            },
            0,
        );
        ink.receive(
            Event {
                layer: "L".into(),
                property: None,
                kind: Kind::Key,
                times: vec![1.5],
            },
            0,
        );
        for (stage, ms) in [
            ("sketch", 160),
            ("ink", 530),
            ("dry", 820),
            ("settled", 1700),
        ] {
            let mut frame = scene.prepare(viewport, &tokens).unwrap();
            ink.paint(&mut frame, &scene, &tokens, ms);
            let mut svg = format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='960' height='150' viewBox='0 0 960 150'><rect width='960' height='150' fill='#{:06x}'/>",
                tokens.colors.panel
            );
            for op in &frame.ops {
                if op.opacity <= 0. || op.covered {
                    continue;
                }
                let r = op.bounds;
                let fill = format!("#{:06x}", op.color);
                let _ = write!(svg, "<g opacity='{}'>", op.opacity);
                if op.shape == "circle" {
                    let _ = write!(
                        svg,
                        "<ellipse cx='{}' cy='{}' rx='{}' ry='{}' fill='{fill}'/>",
                        r.x + r.width / 2.,
                        r.y + r.height / 2.,
                        r.width / 2.,
                        r.height / 2.
                    );
                } else if op.shape.starts_with("diamond") || op.shape == "keyframe.hold" {
                    let _ = write!(
                        svg,
                        "<path d='M {} {} L {} {} L {} {} L {} {} Z' fill='{fill}'/>",
                        r.x + r.width / 2.,
                        r.y,
                        r.x + r.width,
                        r.y + r.height / 2.,
                        r.x + r.width / 2.,
                        r.y + r.height,
                        r.x,
                        r.y + r.height / 2.
                    );
                } else if let Some(text) = &op.text {
                    let text = text
                        .replace('&', "&amp;")
                        .replace('<', "&lt;")
                        .replace('>', "&gt;");
                    let _ = write!(
                        svg,
                        "<text x='{}' y='{}' fill='{fill}' font-family='Helvetica' font-size='{}'>{text}</text>",
                        r.x,
                        r.y + r.height * 0.7,
                        op.font_size.max(11.)
                    );
                } else {
                    let _ = write!(
                        svg,
                        "<rect x='{}' y='{}' width='{}' height='{}' rx='{}' fill='{fill}'/>",
                        r.x,
                        r.y,
                        r.width.max(0.),
                        r.height.max(0.),
                        op.radius
                    );
                }
                svg.push_str("</g>");
            }
            svg.push_str("</svg>");
            let name = format!("{scheme:?}-{stage}").to_lowercase();
            std::fs::write(directory.join(format!("{name}.svg")), svg).unwrap();
            std::fs::write(
                directory.join(format!("{name}.json")),
                frame.wire().to_string(),
            )
            .unwrap();
        }
    }
}
