//! Add and remove text animators through the Properties panel and the actor.
use super::*;

fn setup() -> KernelActor {
    let actor = KernelActor::spawn(KernelConfig::default());
    let project = json!({
        "id":"synthetic","name":"Animators","compId":"root","w":320,"h":180,"fps":30,"dur":2,
        "bg":"#000000","assets":{"a1":{"name":"Clip"}},
        "assetFolders":{"f1":{"id":"f1","name":"One","parent":null}},"comps":{},
        "layers":[{"id":"t","type":"text","name":"Title","from":0,"dur":2,"on":true,
            "parent":null,"group":null,
            "p":{"position.x":{"v":160,"kf":[]},"position.y":{"v":90,"kf":[]},
                "anchor.x":{"v":0,"kf":[]},"anchor.y":{"v":0,"kf":[]},
                "scale.x":{"v":100,"kf":[]},"scale.y":{"v":100,"kf":[]},
                "rotation":{"v":0,"kf":[]},"opacity":{"v":100,"kf":[]}},
            "d":{"text":"Hello","font":"Inter","size":40,"color":"#FFFFFF"},"fx":[]}]
    });
    actor
        .handle()
        .open(
            "synthetic",
            OpenSource::Untitled(data::model(&project).unwrap()),
        )
        .wait()
        .unwrap()
        .unwrap();
    select_layer(&actor.handle(), "t");
    actor
}
fn animators(kernel: &pm_kernel::KernelHandle) -> Vec<Value> {
    let p = kernel.published("synthetic").unwrap();
    let root = p.snapshot.root().unwrap();
    let layer = pm_kernel::model::layer_at(root, &pm_kernel::model::find_layer(root, "t").unwrap())
        .unwrap()
        .to_value();
    data::wire(&layer)["d"]["animators"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}
fn click(panels: &mut Builtins, label: &str) {
    let publication = panels
        .take_publications()
        .pop()
        .expect("a current publication");
    let node = publication
        .surface
        .nodes()
        .values()
        .find(|n| n.props.get("label").and_then(Value::as_str) == Some(label))
        .unwrap_or_else(|| panic!("no control labelled {label}"));
    finish(
        panels
            .event(
                "inspector",
                publication.generation,
                publication.surface.revision(),
                node.id,
                "click",
                EventDetail::default(),
            )
            .unwrap(),
    );
}
fn settle(panels: &mut Builtins, until: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !until() {
        assert!(Instant::now() < deadline, "{:?}", panels.take_errors());
        for (_, ticket) in panels.publish() {
            finish(ticket.unwrap());
        }
        thread::sleep(Duration::from_millis(1));
    }
    for (_, ticket) in panels.publish() {
        finish(ticket.unwrap());
    }
}

#[test]
fn properties_adds_a_preset_animator_with_font_sized_channels_and_removes_it() {
    let actor = setup();
    let kernel = actor.handle();
    let mut panels = Builtins::new(kernel.clone(), "synthetic", || {}).unwrap();
    finish(panels.mount("inspector").unwrap());
    click(&mut panels, "Add Text Animator");
    let deadline = Instant::now() + Duration::from_secs(20);
    let request = loop {
        if let Some(request) = panels
            .take_platform_requests()
            .into_iter()
            .find(|r| r.operation == "contextMenu")
        {
            break request;
        }
        assert!(Instant::now() < deadline, "the preset menu did not open");
        thread::sleep(Duration::from_millis(1));
    };
    let menu = request.input.as_array().unwrap();
    assert_eq!(menu[0]["label"], "Animate In");
    assert_eq!(menu[0]["disabled"], true);
    assert_eq!(menu[1]["id"], "fade");
    panels.platform_reply(request.id, Ok(json!("rise")));
    settle(&mut panels, || animators(&kernel).len() == 1);
    let added = animators(&kernel).remove(0);
    assert_eq!(added["name"], "Rise");
    assert_eq!(added["mode"], "stagger");
    assert_eq!(added["easing"], "expoOut");
    assert_eq!(added["p"]["amount"]["v"], 100.);
    assert_eq!(added["p"]["duration"]["v"], 0.8);
    // `round(size * 0.6)` at the text's size of 40.
    assert_eq!(added["p"]["y"]["v"], 24.);
    let mut model = pm_panel_inspector::Inspector::new(
        kernel.published("synthetic").unwrap(),
        Default::default(),
    );
    let mut presenter = properties::Properties::default();
    let snapshot = presenter.snapshot(&mut model, 0., false).unwrap();
    let section = snapshot["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "animators")
        .unwrap();
    let rows = section["animators"][0]["rows"].as_array().unwrap();
    for label in [
        "Based on",
        "Order",
        "Ease",
        "Amount",
        "Start",
        "Duration",
        "Stagger",
        "Position Y",
        "Opacity",
    ] {
        assert!(
            rows.iter().any(|r| r["label"] == label),
            "missing animator control {label}"
        );
    }
    let row = rows.iter().find(|r| r["label"] == "Position Y").unwrap();
    let request = presenter
        .write(
            &mut model,
            Caller::native(),
            row["control"]["axes"][0]["binding"]["handle"]
                .as_str()
                .unwrap(),
            42.0.into(),
            0.,
        )
        .unwrap();
    let ack = kernel.call("synthetic", request).wait().unwrap().unwrap();
    assert!(!matches!(ack.reply, pm_kernel::Reply::Throw(_)));
    assert_eq!(animators(&kernel)[0]["p"]["y"]["v"], 42.);
    for (_, ticket) in panels.publish() {
        finish(ticket.unwrap());
    }
    click(&mut panels, "Remove Rise");
    settle(&mut panels, || animators(&kernel).is_empty());
    assert!(panels.take_errors().is_empty());
}

#[test]
fn project_media_moves_are_a_plain_kernel_edit_with_undo() {
    let actor = setup();
    let kernel = actor.handle();
    let p = kernel.published("synthetic").unwrap();
    let catalog = pm_panel_media::Catalog::published(&p).unwrap();
    let edit = pm_panel_media::Edit::Move {
        assets: vec!["a1".into()],
        folders: vec![],
        target: Some("f1".into()),
    };
    let prepared = catalog.prepare(edit.clone()).unwrap();
    let request = models::project_edit_request(edit, &prepared, Caller::native());
    assert!(matches!(request.call, pm_kernel::Call::Transact { .. }));
    let ack = kernel.call("synthetic", request).wait().unwrap().unwrap();
    assert!(!matches!(ack.reply, pm_kernel::Reply::Throw(_)));
    let folder = |kernel: &pm_kernel::KernelHandle| {
        let p = kernel.published("synthetic").unwrap();
        data::wire(&p.snapshot.root().unwrap().to_value())["assets"]["a1"]["folder"].clone()
    };
    assert_eq!(folder(&kernel), json!("f1"));
    let ack = kernel
        .call(
            "synthetic",
            Request::new(Caller::native(), pm_kernel::HistoryCall::Undo),
        )
        .wait()
        .unwrap()
        .unwrap();
    assert!(!matches!(ack.reply, pm_kernel::Reply::Throw(_)));
    assert_eq!(folder(&kernel), Value::Null);
}

#[test]
fn text_size_control_resizes_each_selected_layer_about_its_anchor() {
    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    let project = json!({
        "id":"synthetic","name":"Text size","compId":"root","w":320,"h":180,"fps":30,"dur":2,
        "bg":"#000000","assets":{},"comps":{},
        "layers":[
            {"id":"point","type":"text","name":"Title","from":0,"dur":2,"on":true,
             "p":{"position.x":{"v":160,"kf":[]},"position.y":{"v":90,"kf":[]},
                  "anchor.x":{"v":30.1,"kf":[]},"anchor.y":{"v":60.3,"kf":[]}},
             "d":{"text":"Title","font":"Inter","size":80,"color":"#FFFFFF"},"fx":[]},
            {"id":"box","type":"text","name":"Paragraph","from":0,"dur":2,"on":true,
             "p":{"position.x":{"v":50,"kf":[]},"position.y":{"v":60,"kf":[]},
                  "anchor.x":{"v":30,"kf":[]},"anchor.y":{"v":20,"kf":[]}},
             "d":{"text":"Paragraph","font":"Inter","size":40,"color":"#FFFFFF",
                  "paragraph":true,"boxWidth":240},"fx":[]}
        ]
    });
    kernel
        .open(
            "synthetic",
            OpenSource::Untitled(data::model(&project).unwrap()),
        )
        .wait()
        .unwrap()
        .unwrap();
    let request = |call: pm_kernel::Call| {
        let ack = kernel
            .call("synthetic", Request::new(Caller::native(), call))
            .wait()
            .unwrap()
            .unwrap();
        assert!(!matches!(ack.reply, pm_kernel::Reply::Throw(_)), "{ack:?}");
    };
    request(
        SelectionCall::Layers {
            ids: vec!["point".into(), "box".into()],
            add: false,
        }
        .into(),
    );

    let mut panels = Builtins::new(kernel.clone(), "synthetic", || {}).unwrap();
    finish(panels.mount("inspector").unwrap());
    let publication = panels.take_publications().pop().unwrap();
    let field = publication
        .surface
        .nodes()
        .values()
        .find(|n| n.props.get("id").and_then(Value::as_str) == Some("value:content.size:0"))
        .expect("Size control");
    let handle = field.props["binding"].as_str().unwrap();
    let layers = || {
        let p = kernel.published("synthetic").unwrap();
        let root = p.snapshot.root().unwrap();
        ["point", "box"].map(|id| {
            let loc = pm_kernel::model::find_layer(root, id).unwrap();
            data::wire(&pm_kernel::model::layer_at(root, &loc).unwrap().to_value())
        })
    };
    let applied = || kernel.published("synthetic").unwrap().history.applied;
    let initial = layers();
    let before = applied();

    finish(panels.write("inspector", handle, json!(160.)).unwrap());
    let typed = layers();
    assert_eq!(typed[0]["d"]["size"], 160.);
    assert_eq!(typed[0]["p"]["anchor.x"]["v"], 60.2);
    assert_eq!(typed[0]["p"]["anchor.y"]["v"], 120.6);
    assert_eq!(typed[1]["d"]["size"], 160.);
    assert_eq!(typed[1]["p"]["anchor.x"]["v"], 30.);
    assert_eq!(typed[1]["p"]["anchor.y"]["v"], 80.);
    for i in 0..2 {
        for axis in ["position.x", "position.y"] {
            assert_eq!(typed[i]["p"][axis], initial[i]["p"][axis]);
        }
    }
    request(pm_kernel::HistoryCall::Undo.into());
    assert_eq!(layers(), initial);

    let ack = |pending: Result<
        pm_kernel::Pending<std::result::Result<pm_kernel::Ack, pm_kernel::KernelFailure>>,
    >| {
        let reply = pending.unwrap().wait().unwrap().unwrap().reply;
        assert!(!matches!(reply, pm_kernel::Reply::Throw(_)), "{reply:?}");
        reply
    };
    let begin = || match ack(panels.scrub_begin("inspector", Some("Size"))) {
        pm_kernel::Reply::Gesture(id) => id,
        other => panic!("no gesture: {other:?}"),
    };
    let step = |gesture, value| {
        ack(panels.scrub_step("inspector", handle, gesture, json!(value)));
    };
    let gesture = begin();
    step(gesture, 90.);
    step(gesture, 120.);
    assert_eq!(layers()[0]["p"]["anchor.y"]["v"], 60.3 * 1.5);
    step(gesture, 80.);
    // The primary returns exactly to its starting value and fractional pivot.
    // The second layer has its own baseline of 40 and receives the value 80.
    assert_eq!(layers()[0], initial[0]);
    assert_eq!(layers()[1]["p"]["anchor.y"]["v"], 40.);
    ack(panels.scrub_finish("inspector", gesture, false));
    assert_eq!(layers(), initial);
    assert_eq!(applied(), before);

    let gesture = begin();
    step(gesture, 90.);
    step(gesture, 120.);
    ack(panels.scrub_finish("inspector", gesture, true));
    assert_eq!(applied(), before + 1);
    request(pm_kernel::HistoryCall::Undo.into());
    assert_eq!(layers(), initial);
    panels.close();
}
