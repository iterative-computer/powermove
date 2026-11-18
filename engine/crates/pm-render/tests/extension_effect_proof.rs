//! Reference extension on real text, through the native compositor.
mod support;
#[cfg(target_os = "macos")]
fn main() {
    use pm_protocol::{color::Precision, common::Size};
    use pm_render::{MapResolver, RenderRequest, Renderer};
    use serde_json::json;
    use support::doc::{ch, layer, load, project, solid_fill};
    let server = std::sync::Arc::new(pm_text::served::DirectServer::new(
        pm_text::registry::CoreTextRegistry::new(),
        |r| r.generation(),
    ));
    let mut renderer = Renderer::headless().unwrap();
    renderer.set_text_fonts(Some(Box::new(pm_render::text::served::ServedFonts::new(
        server,
        std::sync::Arc::new(pm_render::text::served::NoSeeds),
    ))));
    let definitions: Vec<pm_protocol::definitions::DefinitionSnapshot> =
        serde_json::from_str(include_str!("fixtures/extension-effects.json")).unwrap();
    let document = |amount: f64| {
        let lines = [("break","BREAK",260.,1), ("free","FREE",425.,17)].map(|(id,text,y,seed)| {
            let mut text = layer(id,"text",json!({"text":text,"font":"Arial Black","weight":900,"size":110,
                "leading":1.,"tracking":-4,"color":"#FFFFFF","align":"center"}),320.,y,3.);
            text["fx"]=json!([{"id":format!("{id}-pin"),"type":"corner-pin-snap.fx","on":true,"p":{
                "amount":ch(json!(amount)),"perspective":ch(json!(100)),"seedOffset":ch(json!(seed)),"every":ch(json!(12)),"sharpness":ch(json!(100))
            }}]); text
        });
        load(&project(
            640,
            800,
            3.,
            solid_fill("#F02028"),
            lines.to_vec(),
        ))
    };
    let draw = |renderer: &mut Renderer, doc: &pm_model::Document, t: f64, export: bool| {
        assert!(
            renderer
                .set_document_effects(doc, &definitions)
                .unwrap()
                .is_empty()
        );
        let mut request = RenderRequest::sized(Size::new(640, 800), Precision::Float16);
        request.export = export;
        let frame = renderer
            .render(
                doc,
                "Ptest-root",
                support::ticks(t),
                &request,
                &MapResolver::default(),
            )
            .unwrap();
        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
        renderer.read_rgba(&frame).unwrap()
    };
    let plain = draw(&mut renderer, &document(0.), 0., false);
    let doc = document(100.);
    let mut previous: Option<Vec<f32>> = None;
    for (name, time) in [("pose-0", 0.), ("pose-12", 0.4), ("pose-24", 0.8)] {
        let preview = draw(&mut renderer, &doc, time, false);
        let export = draw(&mut renderer, &doc, time, true);
        let diff = |a: &[f32], b: &[f32]| {
            a.iter()
                .zip(b)
                .map(|(a, b)| (a - b).abs())
                .fold(0f32, f32::max)
        };
        assert!(diff(&preview, &export) < 0.01, "preview/export parity");
        assert!(diff(&preview, &plain) > 0.1, "warp changes text pixels");
        if let Some(previous) = previous {
            assert!(diff(&preview, &previous) > 0.1, "independent snap poses");
        }
        if let Some(folder) = std::env::var_os("PM_EXTENSION_EFFECT_SHOTS") {
            let folder = std::path::PathBuf::from(folder);
            std::fs::create_dir_all(&folder).unwrap();
            let write = |name: &str, pixels: &[f32]| {
                let mut encoder = png::Encoder::new(
                    std::fs::File::create(folder.join(format!("{name}.png"))).unwrap(),
                    640,
                    800,
                );
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                encoder
                    .write_header()
                    .unwrap()
                    .write_image_data(
                        &pixels
                            .iter()
                            .map(|v| (v.clamp(0., 1.) * 255.).round() as u8)
                            .collect::<Vec<_>>(),
                    )
                    .unwrap();
            };
            write(name, &preview);
            write("before", &plain);
        }
        previous = Some(preview);
    }
    println!("3 text poses, preview/export parity, deterministic frame animation: passed");
}
#[cfg(not(target_os = "macos"))]
fn main() {
    panic!("The text proof requires the macOS CoreText registry");
}
