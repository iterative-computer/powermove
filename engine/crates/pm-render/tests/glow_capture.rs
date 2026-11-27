//! Optional external, headless comparison captures; never writes into the repository.
mod support;
use pm_protocol::{color::Precision, common::Size};
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::Value;
fn main() {
    let Ok(root) = std::env::var("PM_GLOW_AUDIT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let out = root.join("native-after");
    std::fs::create_dir_all(&out).unwrap();
    let value: Value =
        serde_json::from_slice(&std::fs::read(root.join("audit-project.json")).unwrap()).unwrap();
    let mut doc = support::doc::load(&value);
    doc.set_saving_app_version(Some("1.3.0".into()));
    let mut r = Renderer::headless().unwrap();
    r.set_text_fonts(Some(Box::new(support::text::MainThreadFonts(
        pm_eval::text_measure::CompatMeasure::new(pm_text::registry::CoreTextRegistry::new()),
        None,
    ))));
    r.set_main_effects_for_document(&doc).unwrap();
    let mut resolver = MapResolver::default();
    resolver.paths.insert("pic".into(), root.join("input.png"));
    let cases: Vec<Value> =
        serde_json::from_slice(&std::fs::read(root.join("cases.json")).unwrap()).unwrap();
    let mut rows = vec![];
    for c in cases {
        let mut req = RenderRequest::sized(Size::new(384, 216), Precision::Float16);
        req.output_kind = pm_render::OutputKind::Final;
        let f = r
            .render(
                &doc,
                "root",
                support::ticks(c["time"].as_f64().unwrap()),
                &req,
                &resolver,
            )
            .unwrap();
        let px = r.read_rgba(&f).unwrap();
        let file = std::fs::File::create(out.join(format!(
            "{}-{}.png",
            c["id"].as_str().unwrap(),
            c["variant"]
        )))
        .unwrap();
        let mut e = png::Encoder::new(std::io::BufWriter::new(file), 384, 216);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        let bytes: Vec<u8> = px
            .iter()
            .map(|v| (v.clamp(0., 1.) * 255.).round() as u8)
            .collect();
        e.write_header().unwrap().write_image_data(&bytes).unwrap();
        req.output_kind = pm_render::OutputKind::Preview;
        req.viewer = true;
        let viewer = r
            .render(
                &doc,
                "root",
                support::ticks(c["time"].as_f64().unwrap()),
                &req,
                &resolver,
            )
            .unwrap();
        let viewer_px = r.read_rgba(&viewer).unwrap();
        let max_viewer_export_cv = px
            .iter()
            .zip(viewer_px)
            .map(|(a, b)| (a - b).abs() * 255.)
            .fold(0_f32, f32::max);
        rows.push(serde_json::json!({"case":c,"failures":format!("{:?}",f.failures), "viewerFailures":format!("{:?}",viewer.failures), "maxViewerExportCV":max_viewer_export_cv}));
    }
    std::fs::write(
        root.join("native-results.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}
