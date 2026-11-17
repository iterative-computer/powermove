//! Trim Paths on a pen path's stroke in Properties (PM-502): v1's Structured
//! section listed Start, End and Offset for each path; V2 showed no path
//! rows at all.
use super::*;

#[test]
fn a_pen_path_lists_its_trim_paths_in_properties() {
    let actor = KernelActor::spawn(KernelConfig::default());
    let channel = |v: f64| format!(r#"{{"v":{v},"kf":[]}}"#);
    let project = format!(
        r##"{{"id":"synthetic","name":"Paths","compId":"c","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","assets":{{}},"comps":{{}},"layers":[{{"id":"a","type":"shape","name":"Path","from":0,"dur":10,"on":true,"lock":false,"p":{{"opacity":{o},"position.x":{x},"position.y":{y}}},"d":{{"paths":[{{"id":"p1","name":"Path 1","parent":null,"vertices":[],"p":{{"trimStart":{s},"trimEnd":{e},"trimOffset":{f},"strokeWidth":{w}}}}}]}},"fx":[]}}]}}"##,
        o = channel(100.),
        x = channel(320.),
        y = channel(180.),
        s = channel(10.),
        e = channel(90.),
        f = channel(0.),
        w = channel(2.),
    );
    actor
        .handle()
        .open(
            "synthetic",
            OpenSource::Untitled(pm_model::json::parse(&project).unwrap()),
        )
        .wait()
        .unwrap()
        .unwrap();
    select_layer(&actor.handle(), "a");
    let identity = pm_surface::host_api::Identity {
        realm: "inspector".into(),
        generation: 1,
        project: "synthetic".into(),
    };
    let mut backend = data::Backend::new(actor.handle(), identity, "inspector", false).unwrap();
    let snapshot = backend.inspector_snapshot().unwrap();
    let section = snapshot["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "trim")
        .unwrap_or_else(|| panic!("no Trim Paths: {}", snapshot["sections"]))
        .clone();
    assert_eq!(section["title"], "Trim Paths");
    let rows: Vec<(String, f64, String)> = section["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let axis = &r["control"]["axes"][0];
            (
                r["label"].as_str().unwrap().to_owned(),
                axis["value"].as_f64().unwrap(),
                axis["unit"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("Start".into(), 10., "%".into()),
            ("End".into(), 90., "%".into()),
            ("Offset".into(), 0., "°".into()),
        ]
    );
    assert!(
        section["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["animation"] == "static"),
        "keyable"
    );
    actor.shutdown();
}
