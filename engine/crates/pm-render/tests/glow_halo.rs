//! Visible premultiplied halos, animated controls and preview/export parity.
mod support;
use pm_protocol::{color::Precision, common::Size};
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::{Value, json};
use support::doc::*;
fn scene(intensity: Value) -> Value {
    let mut chip = layer(
        "chip",
        "solid",
        json!({"w":20,"h":20,"color":"#FFFFFF"}),
        54.,
        54.,
        2.,
    );
    chip["fx"] = json!([{"id":"g","type":"glow","version":1,"on":true,"p":{"radius":ch(json!(20)),"intensity":intensity}}]);
    project(128, 128, 2., no_fill(), vec![chip])
}
fn draw(r: &mut Renderer, v: &Value, t: f64, viewer: bool) -> Vec<f32> {
    let mut doc = load(v);
    doc.set_saving_app_version(Some("1.3.0".into()));
    r.set_main_effects_for_document(&doc).unwrap();
    let mut req = RenderRequest::sized(Size::new(128, 128), Precision::Float32);
    req.viewer = viewer;
    req.output_kind = if viewer {
        pm_render::OutputKind::Preview
    } else {
        pm_render::OutputKind::Final
    };
    req.background = false;
    let f = r
        .render(
            &doc,
            "Ptest-root",
            support::ticks(t),
            &req,
            &MapResolver::default(),
        )
        .unwrap();
    assert!(f.failures.is_empty(), "{:?}", f.failures);
    r.read_rgba(&f).unwrap()
}
fn halo(p: &[f32]) -> f64 {
    p.chunks_exact(4)
        .enumerate()
        .filter(|(i, _)| {
            let x = i % 128;
            let y = i / 128;
            x < 54 || x >= 74 || y < 54 || y >= 74
        })
        .map(|(_, p)| p[3] as f64)
        .sum()
}
#[test]
fn glow_halo_visible_outside_transparent_layer_and_intensity_and_keys_invalidate() {
    let mut r = Renderer::headless().unwrap();
    let mut default = scene(ch(json!(90)));
    default["layers"][0]["fx"][0]["p"] = json!({});
    assert!(halo(&draw(&mut r, &default, 0., true)) > 1.);
    let mut previous = -1.;
    for intensity in [0., 40., 90., 180.] {
        let v = scene(ch(json!(intensity)));
        let a = draw(&mut r, &v, 0., true);
        let h = halo(&a);
        assert!(
            h > previous,
            "intensity {intensity}: halo {h} <= {previous}"
        );
        previous = h;
        if intensity == 90. {
            assert!(h > 1., "default intensity must produce a visible halo");
        }
        let b = draw(&mut r, &v, 0., false);
        assert!(
            a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-5),
            "viewer/export differ"
        );
        assert!(
            a.chunks_exact(4)
                .all(|p| p.iter().all(|v| v.is_finite()) && p[0] <= p[3] + 1e-4),
            "premultiplication"
        );
    }
    // Radius grows the halo footprint; individual pixels need not brighten.
    let mut narrow = scene(ch(json!(90)));
    narrow["layers"][0]["fx"][0]["p"]["radius"] = ch(json!(4));
    let mut wide = narrow.clone();
    wide["layers"][0]["fx"][0]["p"]["radius"] = ch(json!(40));
    let far = |p: &[f32]| -> f64 {
        p.chunks_exact(4)
            .enumerate()
            .filter(|(i, _)| {
                let x = i % 128;
                let y = i / 128;
                x < 44 || x >= 84 || y < 44 || y >= 84
            })
            .map(|(_, p)| f64::from(p[3]))
            .sum()
    };
    assert!(far(&draw(&mut r, &wide, 0., true)) > far(&draw(&mut r, &narrow, 0., true)) + 1.);
    // A higher threshold must select less light, including partial alpha.
    let mut dim = scene(ch(json!(90)));
    dim["layers"][0]["d"]["color"] = json!("#999999");
    dim["layers"][0]["p"]["opacity"] = ch(json!(50));
    dim["layers"][0]["fx"][0]["p"]["threshold"] = ch(json!(10));
    let low = halo(&draw(&mut r, &dim, 0., true));
    dim["layers"][0]["fx"][0]["p"]["threshold"] = ch(json!(90));
    assert!(low > halo(&draw(&mut r, &dim, 0., true)) + 1.);
    let v = scene(keys(json!(0), &[(0., json!(0)), (1., json!(180))]));
    assert!(halo(&draw(&mut r, &v, 1., true)) > halo(&draw(&mut r, &v, 0., true)) + 1.);
}
