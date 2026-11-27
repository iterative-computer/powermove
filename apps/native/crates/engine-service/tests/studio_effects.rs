//! Effects that read their layer's bounds (`u_layerUV`) on 3D layers: a
//! scene made with Layer ▸ Add 3D Layer ▸ Cube (Floor, Key, World, Cube),
//! plus a camera and a point light, exports what the viewer shows. A 3D
//! object's bounds come from the composition's 3D scene in both.
#![cfg(feature = "kernel")]
use pm_engine_service::studio;
use pm_kernel::{
    Call, Caller, Command, KernelActor, KernelConfig, OpenSource, Reply, Request, WriteBody,
};
use pm_protocol::{
    color::Precision,
    common::{Size, Ticks},
};
use pm_render::{OutputKind, RenderRequest, Renderer};
use pm_scene3d::{from_model, to_model};
use serde_json::{Value, json};

const SIZE: u32 = 64;

fn transact(actor: &KernelActor, commands: Vec<Command>) {
    let ack = actor
        .handle()
        .call(
            "p",
            Request::new(
                Caller::native(),
                Call::Transact {
                    label: None,
                    body: WriteBody::Commands(commands),
                    expected_revision: None,
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    assert!(!matches!(ack.reply, Reply::Throw(_)), "{:?}", ack.reply);
}

/// The composition's layers, wherever the root keeps them.
fn layers(root: &mut Value) -> &mut Vec<Value> {
    if root["comps"]["c"]["layers"].is_array() {
        root["comps"]["c"]["layers"].as_array_mut().unwrap()
    } else {
        root["layers"].as_array_mut().unwrap()
    }
}

/// An empty composition with the menu's cube (Floor, Key, World, Cube), a
/// camera and a point light, at a few samples.
fn scene() -> Value {
    let actor = KernelActor::spawn(KernelConfig::default());
    actor
        .handle()
        .open(
            "p",
            OpenSource::Untitled(to_model(&json!({"id":"p","compId":"c","w":SIZE,"h":SIZE,
                "fps":30,"dur":1,"layers":[],"assets":{},"comps":{}}))),
        )
        .wait()
        .unwrap()
        .unwrap();
    for (kind, id) in [("box", "cube"), ("camera", "cam"), ("point", "lamp")] {
        let published = actor.handle().published("p").unwrap();
        let (_, mut commands, _) =
            pm_panel_studio3d::properties::add_layer(&published, 0., kind, id).unwrap();
        // The menu's cube comes with its key light only; the studio's sky and
        // floor, which the cube used to bring, are added as before.
        if kind == "box" {
            let root = from_model(&published.snapshot.root().unwrap().to_value()).unwrap();
            let before = root["comps"]["c"]["layers"]
                .as_array()
                .or_else(|| root["layers"].as_array())
                .cloned()
                .unwrap_or_default();
            for args in pm_panel_studio3d::view::starter(&before, id) {
                if args["operation"] != "add_light" {
                    commands.push(
                        pm_kernel::Scene3dCommand::Apply {
                            comp: published.snapshot.open_comp(),
                            args: to_model(&args),
                            selection: vec![],
                            time: 0.,
                            autokey: false,
                        }
                        .into(),
                    );
                }
            }
        }
        transact(&actor, commands);
    }
    let published = actor.handle().published("p").unwrap();
    let mut root = from_model(&published.snapshot.root().unwrap().to_value()).unwrap();
    let quality = pm_scene3d::quality::resolve(
        &pm_scene3d::quality::preset("production").unwrap(),
        &json!({"minSamples":4,"maxSamples":4,"noiseThreshold":0,"caustics":false,"denoise":false}),
    )
    .unwrap();
    if root["comps"]["c"].is_object() {
        root["comps"]["c"]["studioQuality"] = quality;
    } else {
        root["studioQuality"] = quality;
    }
    root
}

/// The frame as an export (`Final`) or the viewer (`Preview`) draws it.
fn render(root: &Value, export: bool) -> Result<Vec<f32>, String> {
    render_with(root, export, true)
}

/// The frame with the studio drawing the 3D layers, or without it (the
/// rasterized 3D pass, whose effects read the projected object's bounds).
fn render_with(root: &Value, export: bool, with_studio: bool) -> Result<Vec<f32>, String> {
    let doc = pm_kernel::DocumentStore::new_untitled(to_model(root))
        .snapshot()
        .document;
    let mut renderer = Renderer::headless().unwrap();
    // The document's built-in effects, as the app registers them.
    let refused = renderer
        .set_document_effects(&doc, &[])
        .expect("built-in effects");
    assert!(refused.is_empty(), "{refused:?}");
    if with_studio {
        studio::attach(&mut renderer, doc.clone(), &Default::default());
    }
    let mut request = RenderRequest::sized(Size::new(SIZE, SIZE), Precision::Float16);
    request.export = export;
    request.output_kind = if export {
        OutputKind::Final
    } else {
        OutputKind::Preview
    };
    let frame = renderer
        .render(
            &doc,
            "c",
            Ticks::ZERO,
            &request,
            &pm_render::MapResolver::default(),
        )
        .map_err(|e| e.to_string())?;
    if !frame.failures.is_empty() {
        return Err(format!("{:?}", frame.failures));
    }
    Ok(renderer.read_rgba(&frame).unwrap())
}

fn names(root: &mut Value) -> Vec<String> {
    layers(root)
        .iter()
        .map(|l| l["name"].as_str().unwrap_or_default().to_owned())
        .collect()
}

#[test]
fn gaussian_blur_on_the_world_layer_exports_what_the_viewer_shows() {
    let mut root = scene();
    let plain_export = render(&root, true).expect("the plain export draws");
    let plain_viewer = render(&root, false).expect("the plain viewer draws");
    let all = names(&mut root);
    let world = layers(&mut root)
        .iter_mut()
        .find(|l| l["name"] == "World")
        .unwrap_or_else(|| panic!("the menu's cube adds a World layer: {all:?}"));
    // As the Effects panel drops it: Gaussian Blur at its default 0 px.
    world["fx"] = json!([{"id":"f1","type":"blur","version":2,"on":true,"open":true,
        "p":{"amount":{"v":0,"kf":[],"expr":null}}}]);
    let exported = render(&root, true).expect("the export draws the frame");
    let viewed = render(&root, false).expect("the viewer draws the frame");
    let worst = |a: &[f32], b: &[f32]| {
        a.iter()
            .zip(b)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max)
    };
    // A 0 px blur leaves the picture as it was, in the export and the viewer.
    assert!(
        worst(&exported, &plain_export) < 0.02,
        "export: {}",
        worst(&exported, &plain_export)
    );
    assert!(
        worst(&viewed, &plain_viewer) < 0.02,
        "viewer: {}",
        worst(&viewed, &plain_viewer)
    );
    assert!(exported.chunks(4).any(|p| p[3] > 0.99), "something draws");
}

#[test]
fn every_effect_that_reads_layer_bounds_exports_on_every_3d_layer_kind() {
    // Each built-in effect the app offers whose shader reads `u_layerUV`.
    let offered: Vec<String> = pm_render::all_builtin_definitions()
        .definitions
        .iter()
        .map(|d| d.id.as_str().to_owned())
        .collect();
    let main = pm_fx_defs::catalog::main_effects();
    let mut bounded: Vec<(String, u64)> = Vec::new();
    for shader in &main.shaders {
        if !offered.contains(&shader.id)
            || (!shader.source.contains("u_layerUV")
                && !shader.source.contains("u_temporalOwnerUV"))
        {
            continue;
        }
        match bounded.iter_mut().find(|(id, _)| *id == shader.id) {
            Some(entry) => entry.1 = entry.1.max(shader.version),
            None => bounded.push((shader.id.clone(), shader.version)),
        }
    }
    assert!(
        bounded.iter().any(|(id, _)| id == "blur"),
        "Gaussian Blur reads its layer's bounds: {bounded:?}"
    );
    let mut root = scene();
    let kinds = names(&mut root);
    for name in ["Floor", "Key", "World", "Cube", "Camera", "Bulb"] {
        assert!(
            kinds.iter().any(|k| k == name),
            "{name} among the 3D layers: {kinds:?}"
        );
    }
    eprintln!("{} effects read their layer's bounds", bounded.len());
    for (id, version) in &bounded {
        for layer in layers(&mut root).iter_mut() {
            layer["fx"] = json!([{"id":"f1","type":id,"version":version,"on":true,"p":{}}]);
        }
        render(&root, true).unwrap_or_else(|e| panic!("{id} on every 3D layer exports: {e}"));
    }
}

#[test]
fn without_the_studio_an_object_layer_effect_reads_the_projected_object() {
    let mut root = scene();
    // The rasterized pass draws objects, lights and the camera, not the
    // studio's world.
    layers(&mut root).retain(|l| l["name"] != "World");
    let plain = render_with(&root, true, false).expect("the plain frame draws");
    let cube = layers(&mut root)
        .iter_mut()
        .find(|l| l["name"] == "Cube")
        .unwrap();
    cube["fx"] = json!([{"id":"f1","type":"blur","version":2,"on":true,
        "p":{"amount":{"v":0,"kf":[],"expr":null}}}]);
    let blurred = render_with(&root, true, false).expect("the object's effect draws");
    let worst = plain
        .iter()
        .zip(&blurred)
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    assert!(worst < 0.02, "a 0 px blur changed the cube by {worst}");
    assert!(plain.chunks(4).any(|p| p[3] > 0.5), "the cube draws");
}
