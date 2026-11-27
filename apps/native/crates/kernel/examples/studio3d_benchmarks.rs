//! Generates portable, synthetic benchmark projects through ordinary kernel ops.
//! Inputs are emitted by the test-only studio-fixtures binary. Output stays out of Git.
use pm_kernel::{
    DocumentStore, Origin, Scene3dCommand,
    project_io::{SaveOptions, SaveTarget},
};
use pm_scene3d::{
    import::{self, ImportFile, archive::Archive},
    to_model,
};
use serde_json::{Value, json};
use std::{path::Path, sync::Arc};
fn apply(s: &mut DocumentStore, args: Value) {
    s.execute(
        Scene3dCommand::Apply {
            comp: None,
            args: to_model(&args),
            selection: vec![],
            time: 0.,
            autokey: false,
        }
        .into(),
        Origin::Interface,
    )
    .unwrap();
}
fn hex(v: &Value) -> String {
    let mut out = String::from("#");
    for key in ["x", "y", "z"] {
        let c = v[key].as_f64().unwrap_or(0.).clamp(0., 1.);
        let srgb = if c <= 0.0031308 {
            c * 12.92
        } else {
            1.055 * c.powf(1. / 2.4) - 0.055
        };
        out.push_str(&format!("{:02X}", (srgb * 255.).round() as u8));
    }
    out
}
fn new(name: &str) -> DocumentStore {
    DocumentStore::new_untitled(to_model(
        &json!({"id":format!("bench-{name}"),"name":name,"compId":"C","w":1920,"h":1080,"fps":30,"dur":1,"layers":[],"assets":{}}),
    ))
}
fn camera(s: &mut DocumentStore, c: &Value) {
    let p = &c["position"];
    let t = &c["target"];
    apply(
        s,
        json!({"operation":"add_camera","id":"camera","camera":{"p":{"x":p["x"],"y":p["y"],"z":p["z"],"targetX":t["x"],"targetY":t["y"],"targetZ":t["z"],"fov":c["fov"],"far":10000}}}),
    );
}
fn save(mut s: DocumentStore, path: &Path) {
    let receipt = s
        .prepare_save(
            SaveTarget::SaveAs {
                path: path.into(),
                replace: false,
            },
            SaveOptions::default(),
        )
        .unwrap()
        .run()
        .unwrap();
    s.finish_save(receipt).unwrap();
    let opened = pm_kernel::project_io::open_project(path, Default::default()).unwrap();
    let reopened = DocumentStore::open(opened);
    assert_eq!(s.media().len(), reopened.media().len());
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let input = std::path::PathBuf::from(args.next().ok_or("scene input directory required")?);
    let out = std::path::PathBuf::from(args.next().ok_or("output directory required")?);
    std::fs::create_dir_all(&out)?;
    let archive = Archive::open(&out.join("archive"))?;
    for name in ["product-shot", "hero", "chrome"] {
        let dir = input.join(name);
        let scene: Value = serde_json::from_slice(&std::fs::read(dir.join("scene.json"))?)?;
        let mut s = new(name);
        for (i, obj) in scene["objects"].as_array().unwrap().iter().enumerate() {
            let file = format!("part-{i}.obj");
            let receipt = Arc::new(archive.admit(import::prepare(
                vec![ImportFile {
                    name: file.clone(),
                    mime: "model/obj".into(),
                    bytes: std::fs::read(dir.join(file))?,
                }],
                None,
            )?)?);
            let model = receipt.model();
            let import::Model::Obj(mesh) = model.as_ref() else {
                panic!("OBJ fixture")
            };
            let center = mesh.center;
            let radius = mesh.radius;
            let id = format!("model-{i}");
            s.execute(
                Scene3dCommand::Import {
                    comp: None,
                    asset_id: format!("asset-{i}"),
                    layer_id: id.clone(),
                    receipt,
                    time: 0.,
                    setup_studio: false,
                }
                .into(),
                Origin::Interface,
            )
            .unwrap();
            let m = &scene["materials"][obj["material"].as_u64().unwrap() as usize];
            let emission = m["emission"]
                .as_object()
                .unwrap()
                .values()
                .filter_map(Value::as_f64)
                .fold(0., f64::max);
            apply(
                &mut s,
                json!({"operation":"update_object","target":id,"patch":{"p":{"x":center[0],"y":center[1],"z":center[2],"sx":radius,"sy":radius,"sz":radius},"material":{"p":{"color":hex(&m["base"]),"roughness":m["roughness"].as_f64().unwrap().max(0.02),"metalness":m["metallic"],"clearcoat":m["coat"],"clearcoatRoughness":m["coat_roughness"],"transmission":m["transmission"],"ior":m["ior"],"specular":m["specular"],"emissive":if emission>0. {"#FFFFFF".into()}else{hex(&m["emission"])},"emissiveIntensity":emission}}}}),
            );
        }
        camera(&mut s, &scene["camera"]);
        for (i, light) in scene["lights"].as_array().unwrap().iter().enumerate() {
            let c = &light["center"];
            let t = &scene["camera"]["target"];
            let length = |v: &Value| {
                ["x", "y", "z"]
                    .iter()
                    .map(|k| v[*k].as_f64().unwrap().powi(2))
                    .sum::<f64>()
                    .sqrt()
            };
            apply(
                &mut s,
                json!({"operation":"add_light","id":format!("light-{i}"),"light":{"type":"area","p":{"x":c["x"],"y":c["y"],"z":c["z"],"targetX":t["x"],"targetY":t["y"],"targetZ":t["z"],"width":2.*length(&light["u"]),"height":2.*length(&light["v"]),"intensity":light["radiance"]["x"]}}}),
            );
        }
        apply(
            &mut s,
            json!({"operation":"set_environment","patch":{"background":"#F6F5F8","shadowCatcher":true,"p":{"ambient":0.03}}}),
        );
        apply(
            &mut s,
            json!({"operation":"shot","shot":"push_in","duration":1}),
        );
        save(s, &out.join(format!("{name}.pmv")));
        println!("saved {name}");
    }
    let mut s = new("environment");
    apply(
        &mut s,
        json!({"operation":"set_world","id":"world","world":{"mode":"sky","preset":"clear-noon","p":{"metersPerUnit":1,"elevation":38,"haze":3,"intensity":1,"sunIntensity":0.9,"fogDistance":400,"fogHeightFalloff":0.06}}}),
    );
    apply(
        &mut s,
        json!({"operation":"add_terrain","id":"terrain","terrain":{"p":{"width":100,"depth":100,"amplitude":8,"resolution":32,"seed":7}}}),
    );
    apply(
        &mut s,
        json!({"operation":"add_scatter","id":"forest","scatter":{"model":"nature:conifer","surface":"terrain","p":{"count":320,"seed":71,"scaleMin":0.8,"scaleMax":1.2}}}),
    );
    camera(
        &mut s,
        &json!({"position":{"x":45,"y":25,"z":65},"target":{"x":0,"y":2,"z":0},"fov":45}),
    );
    apply(
        &mut s,
        json!({"operation":"shot","shot":"push_in","duration":1}),
    );
    save(s, &out.join("environment.pmv"));
    Ok(())
}
