//! Text commands against the real font registry, which AppKit confines to the
//! main thread: this binary keeps main and serves asks the way the app does.
#[path = "../src/text_host.rs"]
mod text_host;

use pm_kernel::commands::text::{SplitMode, TextCommand};
use pm_kernel::commands::transform::{AlignMode, TransformCommand};
use pm_kernel::{Call, Caller, Command, KernelActor, KernelConfig, OpenSource, Reply, Request};
use pm_kernel::{SelectionCall, WriteBody};
use pm_model::Value;
use pm_node::fonts::{
    FontService,
    render::{FontBroker, RenderFonts, serve},
};
use std::{sync::mpsc, time::Duration};

fn project(text: &str) -> Value {
    pm_model::json::parse(&format!(
        r##"{{"id":"synthetic","name":"Text","compId":"root","w":640,"h":360,"fps":30,"dur":3,
        "bg":"#000000","assets":{{}},"comps":{{}},"layers":[
        {{"id":"t","type":"text","name":"Title","from":0,"dur":3,"on":true,"lock":false,
        "parent":null,"group":null,"blend":"normal","mblur":false,"fx":[],"masks":[],
        "p":{{"position.x":{{"v":100,"kf":[]}},"position.y":{{"v":200,"kf":[]}},
        "anchor.x":{{"v":0,"kf":[]}},"anchor.y":{{"v":0,"kf":[]}},
        "scale.x":{{"v":100,"kf":[]}},"scale.y":{{"v":100,"kf":[]}},
        "rotation":{{"v":0,"kf":[]}},"opacity":{{"v":100,"kf":[]}}}},
        "d":{{"text":"{text}","font":"Helvetica","weight":400,"size":48,"tracking":0,
        "leading":1.2,"align":"left","color":"#FFFFFF"}}}}]}}"##
    ))
    .unwrap()
}
fn number(layer: &Value, channel: &str) -> f64 {
    layer
        .get("p")
        .unwrap()
        .get(channel)
        .unwrap()
        .get("v")
        .unwrap()
        .as_f64()
        .unwrap()
}
fn find<'a>(layers: &'a [Value], id: &str) -> &'a Value {
    layers
        .iter()
        .find(|l| l.get("id").and_then(Value::as_str) == Some(id))
        .unwrap()
}

fn run(fonts: RenderFonts) {
    let actor = KernelActor::spawn(KernelConfig {
        text: Some(text_host::text_host(&fonts)),
        ..Default::default()
    });
    let kernel = actor.handle();
    kernel
        .open("synthetic", OpenSource::Untitled(project("One Two")))
        .wait()
        .unwrap()
        .unwrap();
    let edit = |command: Command| {
        let ack = kernel
            .call(
                "synthetic",
                Request::new(
                    Caller::native(),
                    Call::Transact {
                        label: None,
                        expected_revision: None,
                        body: WriteBody::Commands(vec![command]),
                    },
                ),
            )
            .wait()
            .unwrap()
            .unwrap();
        assert!(!matches!(ack.reply, Reply::Throw(_)), "{:?}", ack.reply);
    };
    let layers = || {
        let p = kernel.published("synthetic").unwrap();
        let root = p.snapshot.root().unwrap().to_value();
        let comp = root
            .get("comps")
            .and_then(|c| c.get("root"))
            .unwrap_or(&root);
        comp.get("layers").unwrap().as_array().unwrap().clone()
    };
    let _ = kernel
        .call(
            "synthetic",
            Request::new(
                Caller::native(),
                SelectionCall::Layers {
                    ids: vec!["t".into()],
                    add: false,
                },
            ),
        )
        .wait();
    // The viewer's text questions (caret geometry, typography offset,
    // picking) go through the same font-fed query path as the app's.
    let published = kernel.published("synthetic").unwrap();
    let layout = text_host::query(&published, |q| q.text_layout(None, "t", 0.)).unwrap();
    let lines = layout.get("lines").and_then(Value::as_array).unwrap();
    assert_eq!(lines.len(), 1);
    let stops = lines[0]
        .get("boundaries")
        .and_then(Value::as_array)
        .unwrap();
    let last = stops
        .last()
        .unwrap()
        .get("x")
        .and_then(Value::as_f64)
        .unwrap();
    assert!(last > 48., "measured with real fonts: {last}");
    let offset = text_host::query(&published, |q| q.text_font_offset(None, "t", 0.)).unwrap();
    assert_eq!(offset, [0., 0.], "no typography anchoring recorded");
    let hit = text_host::query(&published, |q| {
        q.pick(None, 110., 220., 0., &Default::default())
    })
    .unwrap();
    assert_eq!(hit.layer_id.as_deref(), Some("t"));
    edit(
        TransformCommand::CenterAnchor {
            layer_ids: vec!["t".into()],
            time: 0.,
        }
        .into(),
    );
    let text = layers().remove(0);
    let (ax, ay) = (number(&text, "anchor.x"), number(&text, "anchor.y"));
    assert!(ax > 40., "the anchor moves to the middle of the type: {ax}");
    assert!(ay.abs() < 48., "and to its height: {ay}");
    assert!(
        (number(&text, "position.x") - (100. + ax)).abs() < 1e-6,
        "the position compensates exactly"
    );
    edit(
        TransformCommand::Align {
            layer_ids: vec!["t".into()],
            mode: AlignMode::HorizontalCenter,
            time: 0.,
        }
        .into(),
    );
    let aligned = number(&layers()[0], "position.x");
    assert!(
        (aligned - 320.).abs() < 1.,
        "centred on the frame: {aligned}"
    );
    edit(
        TextCommand::Split {
            layer_ids: vec!["t".into()],
            group_ids: vec!["G".into()],
            mode: SplitMode::Words,
            time: 0.,
        }
        .into(),
    );
    let all = layers();
    let group = find(&all, "G");
    assert_eq!(group.get("type").and_then(Value::as_str), Some("group"));
    let pieces: Vec<&Value> = all
        .iter()
        .filter(|l| l.get("group").and_then(Value::as_str) == Some("G"))
        .collect();
    assert_eq!(pieces.len(), 3, "two words and the hidden source");
    let word = |l: &Value| {
        l.get("d")
            .unwrap()
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    assert_eq!(word(pieces[0]).as_deref(), Some("One"));
    assert_eq!(word(pieces[1]).as_deref(), Some("Two"));
    let advance = number(pieces[0], "anchor.x") - number(pieces[1], "anchor.x");
    assert!(
        advance > 48. && advance < 200.,
        "the second word sits one word and a space along: {advance}"
    );
    // The compat page measures with the engine's own faces: a family this Mac
    // lacks is the face a text layer falls back to (`system-ui`).
    let faces = text_host::page_faces(&[
        ("Helvetica".into(), 400, false),
        ("No Such Family Q".into(), 400, false),
        ("system-ui".into(), 400, false),
    ]);
    assert_eq!(faces.len(), 3);
    assert!(faces.iter().all(|f| f.bytes.len() > 1000));
    assert_eq!(faces[1].family, "No Such Family Q");
    assert_eq!(faces[1].bytes, faces[2].bytes);
    assert_ne!(faces[0].bytes, faces[2].bytes);
}

fn main() {
    let broker = FontBroker::new();
    broker.set_wake(std::sync::Arc::new(|| {}));
    let fonts = RenderFonts::new(broker.clone());
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        run(fonts);
        let _ = tx.send(());
    });
    let mut source = FontService::new().expect("the font registry needs the main thread");
    loop {
        serve(&mut source, broker.take_asks());
        match rx.try_recv() {
            Ok(()) => break,
            Err(mpsc::TryRecvError::Disconnected) => break,
            Err(_) => std::thread::sleep(Duration::from_millis(2)),
        }
    }
    broker.close();
    worker.join().expect("text commands");
    println!("test result: ok. text commands against the font registry");
}
