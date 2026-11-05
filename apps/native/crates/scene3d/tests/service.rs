use pm_scene3d::{
    schema,
    service::{self, Evaluator, WorldEvaluator},
};
use serde_json::{Value, json};
struct Host;
impl Evaluator for Host {
    fn value(&mut self, _: &Value, _: &str, c: &Value, _: f64) -> pm_scene3d::Result<Value> {
        Ok(c["v"].clone())
    }
    fn active(&mut self, l: &Value, _: f64) -> pm_scene3d::Result<bool> {
        Ok(l["on"] != false)
    }
}
impl WorldEvaluator for Host {
    fn world(&mut self, _: &Value, _: f64) -> pm_scene3d::Result<[f64; 16]> {
        Ok(identity())
    }
    fn parent(&mut self, _: &Value, _: f64) -> pm_scene3d::Result<[f64; 16]> {
        Ok(identity())
    }
    fn opacity(&mut self, l: &Value, _: f64) -> pm_scene3d::Result<f64> {
        Ok(l["opacity"].as_f64().unwrap_or(1.))
    }
}
fn identity() -> [f64; 16] {
    let mut m = [0.; 16];
    for i in [0, 5, 10, 15] {
        m[i] = 1.;
    }
    m
}
fn model(id: &str) -> Value {
    json!({"id":id,"d":{"definition":schema::OBJECT},"on":true})
}
#[test]
fn depth_sets_respect_active_2d_barriers_and_nonzero_authored_opacity() {
    let mut faint = model("faint");
    faint["opacity"] = json!(0.0000001);
    let layers = vec![
        model("a"),
        faint,
        json!({"id":"barrier","type":"shape","on":true}),
        model("b"),
    ];
    let comp = json!({"layers":layers});
    let ids = service::depth_ids(
        &comp,
        comp["layers"].as_array().unwrap(),
        "a",
        0.,
        &mut Host,
    )
    .unwrap();
    assert_eq!(ids.into_iter().collect::<Vec<_>>(), vec!["a", "faint"]);
    let mut comp = comp;
    comp["layers"][2]["on"] = json!(false);
    let ids = service::depth_ids(
        &comp,
        comp["layers"].as_array().unwrap(),
        "a",
        0.,
        &mut Host,
    )
    .unwrap();
    assert!(ids.contains("b"));
}
#[test]
fn depth_sets_honor_solo_model_groups() {
    let mut a = model("a");
    a["group"] = json!("group");
    let layers = vec![a, model("b")];
    let comp = json!({"layers":[{"id":"group","type":"group","solo":true},layers[0],layers[1]]});
    let ids = service::depth_ids(&comp, &layers, "a", 0., &mut Host).unwrap();
    assert_eq!(ids.into_iter().collect::<Vec<_>>(), vec!["a"]);
}
#[test]
fn transparent_ordering_retains_barriers_and_stable_ties() {
    let layers = vec![
        model("far"),
        model("near"),
        json!({"id":"2d","type":"shape"}),
        model("after"),
    ];
    let mut worlds = std::collections::BTreeMap::new();
    for (id, z) in [("far", -5.), ("near", -1.), ("after", 0.)] {
        let mut m = identity();
        m[14] = z;
        worlds.insert(id.into(), m);
    }
    let ordered = service::ordered_layers(&layers, &worlds, identity()).unwrap();
    assert_eq!(
        ordered
            .iter()
            .map(|l| l["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["near", "far", "2d", "after"]
    );
    worlds.get_mut("far").unwrap()[14] = -1.;
    let ordered = service::ordered_layers(&layers, &worlds, identity()).unwrap();
    assert_eq!(ordered[0]["id"], "far");
}
