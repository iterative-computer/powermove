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
    let mut definitions: Vec<pm_protocol::definitions::DefinitionSnapshot> =
        serde_json::from_str(include_str!("fixtures/extension-effects.json")).unwrap();
    // Give each real monospace glyph a unique ink color before pinning it.
    // Pixel centroids then reveal any left/right letter-order reversal.
    let mut ink = definitions[0].clone();
    ink.id = "test.letter-colors".into();
    ink.params.clear();
    ink.flags.viewport_padding.clear();
    ink.flags.convex_quad = None;
    ink.glsl.as_mut().unwrap().source = r#"#version 300 es
precision highp float;
in vec2 v_st; out vec4 o;
uniform sampler2D u_tex;
uniform vec2 u_res;
uniform mat3 u_layerUV;
void main(){
 vec3 p=u_layerUV*vec3(vec2(v_st.x,1.0-v_st.y)*u_res,1);
 float glyph=floor(clamp(p.x/p.z,0.0,0.9999)*4.0);
 vec3 color=glyph<1.0?vec3(1,0,0):glyph<2.0?vec3(0,1,0):glyph<3.0?vec3(0,0,1):vec3(1,1,0);
 vec4 src=texture(u_tex,v_st);o=vec4(src.rgb*color,src.a);
}"#
    .into();
    definitions.push(ink);
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
    let mut captures = Vec::new();
    for (name, time) in [
        ("pose-0", 0.),
        ("pose-12", 0.4),
        ("pose-24", 0.8),
        ("pose-36", 1.2),
    ] {
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
        captures.push(preview.clone());
        previous = Some(preview);
    }
    if let Some(folder) = std::env::var_os("PM_EXTENSION_EFFECT_SHOTS") {
        // Contact sheet is assembled directly from the verified render buffers.
        let mut sheet = vec![0u8; 1280 * 1680 * 4];
        for pixel in sheet.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[22, 22, 22, 255]);
        }
        let digits = [
            [31, 17, 17, 17, 31],
            [4, 12, 4, 4, 14],
            [31, 1, 31, 16, 31],
            [31, 1, 15, 1, 31],
            [17, 17, 31, 1, 1],
            [31, 16, 31, 17, 31],
        ];
        for (index, pixels) in captures.iter().enumerate() {
            let ox = (index % 2) * 640;
            let oy = (index / 2) * 840;
            for y in 0..800 {
                for x in 0..640 {
                    for c in 0..4 {
                        sheet[((oy + 40 + y) * 1280 + ox + x) * 4 + c] =
                            (pixels[(y * 640 + x) * 4 + c].clamp(0., 1.) * 255.).round() as u8;
                    }
                }
            }
            let label: &[usize] = match index {
                0 => &[0],
                1 => &[1, 2],
                2 => &[2, 4],
                _ => &[3, 5],
            };
            for (n, digit) in label.iter().enumerate() {
                for y in 0..5 {
                    for x in 0..5 {
                        if digits[*digit][y] & (1 << (4 - x)) != 0 {
                            for dy in 0..4 {
                                for dx in 0..4 {
                                    let at = ((oy + 8 + y * 4 + dy) * 1280
                                        + ox
                                        + 16
                                        + n * 28
                                        + x * 4
                                        + dx)
                                        * 4;
                                    sheet[at..at + 4].copy_from_slice(&[255, 255, 255, 255]);
                                }
                            }
                        }
                    }
                }
            }
        }
        let mut encoder = png::Encoder::new(
            std::fs::File::create(std::path::PathBuf::from(folder).join("readable-sheet.png"))
                .unwrap(),
            1280,
            1680,
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&sheet)
            .unwrap();
    }
    for seed in [1, 17, 999] {
        let mut letters = layer(
            "letters",
            "text",
            json!({"text":"ABCD","font":"Menlo","weight":700,"size":100,"color":"#FFFFFF","align":"left"}),
            150.,
            300.,
            3.,
        );
        letters["fx"] = json!([
          {"id":"ink","type":"test.letter-colors","on":true,"p":{}},
          {"id":"pin","type":"corner-pin-snap.fx","on":true,"p":{"amount":ch(json!(100)),"perspective":ch(json!(100)),"seed":ch(json!(seed))}}
        ]);
        let doc = load(&project(640, 800, 3., solid_fill("#000000"), vec![letters]));
        for frame in [0, 12, 24, 36] {
            let pixels = draw(&mut renderer, &doc, frame as f64 / 30., false);
            let mut sum = [0.; 4];
            let mut weight = [0.; 4];
            for (i, p) in pixels.chunks_exact(4).enumerate() {
                let letter = if p[0] > 0.1 && p[1] < 0.05 && p[2] < 0.05 {
                    Some(0)
                } else if p[1] > 0.1 && p[0] < 0.05 && p[2] < 0.05 {
                    Some(1)
                } else if p[2] > 0.1 && p[0] < 0.05 && p[1] < 0.05 {
                    Some(2)
                } else if p[0] > 0.1 && p[1] > 0.1 && p[2] < 0.05 {
                    Some(3)
                } else {
                    None
                };
                if let Some(letter) = letter {
                    let w = p[0].max(p[1]).max(p[2]) as f64;
                    sum[letter] += (i % 640) as f64 * w;
                    weight[letter] += w;
                }
            }
            assert!(
                weight.iter().all(|w| *w > 50.),
                "every letter remains visible: seed {seed}, frame {frame}, {weight:?}"
            );
            let x: Vec<_> = sum.iter().zip(weight).map(|(s, w)| s / w).collect();
            assert!(
                x.windows(2).all(|p| p[0] + 5. < p[1]),
                "letter baseline order: seed {seed}, frame {frame}, {x:?}"
            );
        }
    }
    println!("4 text poses, preview/export parity, 12 real-text letter-order pixel cases: passed");
}
#[cfg(not(target_os = "macos"))]
fn main() {
    panic!("The text proof requires the macOS CoreText registry");
}
