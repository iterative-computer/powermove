use pm_protocol::color::Precision;
use pm_protocol::common::{Seq, Size, Ticks};
use pm_protocol::definitions::{DefinitionSetSnapshot, DefinitionSnapshot};
use pm_render::{MapResolver, RenderError, RenderRequest, Renderer};
use serde_json::{Value, json};

fn ch(v: Value) -> Value {
    json!({ "v": v, "kf": [], "expr": null })
}

fn layer(id: &str, ty: &str, d: Value, from: f64, dur: f64) -> Value {
    json!({
        "id": id, "type": ty, "name": id, "from": from, "dur": dur, "on": true, "lock": false,
        "threeD": false, "solo": false, "blend": "normal", "mblur": false, "parent": null,
        "group": null, "matteSource": null,
        "p": {
            "anchor.x": ch(json!(0)), "anchor.y": ch(json!(0)), "position.x": ch(json!(0)),
            "position.y": ch(json!(0)), "scale.x": ch(json!(100)), "scale.y": ch(json!(100)),
            "rotation": ch(json!(0)), "opacity": ch(json!(100)), "skew": ch(json!(0))
        },
        "fx": [], "transitionIn": null, "transitionOut": null, "masks": [], "d": d
    })
}

fn document(w: u32, h: u32, fps: f64, layers: Vec<Value>) -> pm_model::Document {
    let doc = json!({
        "id": "P", "name": "p", "w": w, "h": h, "fps": fps, "dur": 10, "bg": "#000000",
        "backgroundFill": { "type": "none", "angle": 0, "stops": [{ "id": "s1", "color": "#000000", "position": 0 }] },
        "compId": "P-root", "compName": "p", "markers": [], "work": [0, 10], "params": {},
        "revision": 0, "edits": [], "created": 0, "shutter": 0.5, "layers": layers, "assets": {}, "comps": {}
    });
    pm_model::Document::new(Some(pm_model::json::parse(&doc.to_string()).unwrap()), 1)
}

fn ticks(seconds: f64) -> Ticks {
    Ticks::new((seconds * pm_time::TICKS_PER_SECOND as f64).round() as i64).unwrap()
}

/// Read the whole headless frame over transparency as premultiplied RGBA.
fn render(
    r: &mut Renderer,
    doc: &pm_model::Document,
    size: (u32, u32),
    t: f64,
) -> Result<(Vec<f32>, Vec<pm_render::LayerFailure>), RenderError> {
    let mut request = RenderRequest::sized(Size::new(size.0, size.1), Precision::Float16);
    request.background = false;
    let frame = r.render(doc, "P-root", ticks(t), &request, &MapResolver::default())?;
    let px = r.read_rgba(&frame).unwrap();
    Ok((px, frame.failures))
}

fn definition(broken: bool) -> DefinitionSnapshot {
    let fragment = include_str!("fixtures/macos-cursor.glsl");
    let fragment = if broken {
        fragment.replace("nativeInvert != u_swap", "nativeInvert != (u_swap > 0.5)")
    } else {
        fragment.into()
    };
    let uniforms = "uniform float u_cursor; uniform float u_size; uniform float u_press; uniform float u_outline; uniform vec3 u_fill; uniform vec3 u_edge; uniform bool u_swap; uniform float u_shadow; uniform float u_opacity;";
    let prefix = format!("{}\n{uniforms}\n", pm_render::shader_layers::header());
    let mut params = vec![];
    for (key, value) in [
        ("cursor", 5.0),
        ("size", 1.0),
        ("press", 0.0),
        ("outline", 1.0),
        ("shadow", 0.6),
        ("opacity", 1.0),
    ] {
        params
            .push(json!({"type":"number","key":key,"default":value,"min":0,"max":10,"step":null}));
    }
    params.extend([
        json!({"type":"color","key":"fill","default":"#ffffff"}),
        json!({"type":"color","key":"edge","default":"#000000"}),
        json!({"type":"toggle","key":"swap","default":false}),
    ]);
    serde_json::from_value(json!({
        "id":"macos.cursor","kind":"layer","version":1,"origin":"local",
        "renderer":{"kind":"fragment","fragment":fragment},
        "glsl":{"source":format!("{prefix}{fragment}"),"bodyLineOffset":prefix.matches('\n').count(),"bodyUtf16Len":fragment.encode_utf16().count(),"bodyUtf8Len":fragment.len()},
        "params":params,
        "flags":{"passes":1,"keepOrig":false,"backdrop":false,"rawShader":false,"viewportSafe":false,"viewportPadding":[],"clamp":false}
    })).unwrap()
}
fn cursor_render(broken: bool) -> (Vec<f32>, Vec<pm_render::LayerFailure>) {
    let definitions = DefinitionSetSnapshot {
        generation: Seq::new(1).unwrap(),
        definitions: vec![definition(broken)],
        missing: vec![],
    };
    let mut renderer = Renderer::headless().unwrap();
    renderer.set_definitions(&definitions).unwrap();
    let doc = document(
        128,
        128,
        30.,
        vec![layer(
            "cursor",
            "extension",
            json!({"definition":"macos.cursor","version":1,"params":{},"data":{}}),
            0.,
            4.,
        )],
    );
    render(&mut renderer, &doc, (128, 128), 1.).unwrap()
}
#[test]
fn patched_cursor_renders_headlessly_without_a_layer_failure() {
    let (pixels, failures) = cursor_render(false);
    assert!(failures.is_empty(), "{failures:?}");
    assert!(
        pixels.chunks_exact(4).any(|p| p[3] > 0.01),
        "cursor drew no pixels"
    );
}
#[test]
fn broken_cursor_compilation_leaves_exactly_one_error_log_row() {
    let (pixels, failures) = cursor_render(true);
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(pixels.chunks_exact(4).all(|p| p[3] == 0.0));
    let mut log = crate::error_log::Log::default();
    for _ in 0..3 {
        super::record_layer_failures(&mut log, &failures, |f| {
            super::definition_failure_source(&f.layer, "macos.cursor", None)
        });
    }
    assert_eq!(log.entries.len(), 1);
    assert_eq!(log.entries[0].message, failures[0].message);
    assert!(log.entries[0].source.contains("macos.cursor"));
}
