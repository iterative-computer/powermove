//! Large-project refresh costs (diagnostic; prints timings).
use pm_kernel::{session::*, *};
use std::time::Instant;
fn answer(k: &KernelHandle, call: ShellCall) -> ShellReply {
    k.shell(call).wait().unwrap().unwrap()
}
#[test]
#[ignore]
fn timings_60k() {
    let n = 60_000;
    let mut layers = vec![];
    for i in 0..=n {
        layers.push(format!(r#"{{"id":"bulk-{i}","type":"shape","name":"Bulk {i}","from":0,"dur":2,"on":true,"p":{{"opacity":{{"v":100,"kf":[],"expr":null}}}}}}"#));
    }
    let json = format!(
        r##"{{"id":"big","name":"Big","compId":"main","w":320,"h":180,"fps":30,"dur":2,"bg":"#000000","assets":{{}},"layers":[{}],"comps":{{}}}}"##,
        layers.join(",")
    );
    let dir = tempfile::tempdir().unwrap();
    let actor = KernelActor::spawn(KernelConfig {
        journal_root: Some(dir.path().join("j")),
        ..Default::default()
    });
    let k = actor.handle();
    answer(
        &k,
        ShellCall::AttachLibrary {
            profile: dir.path().join("profile"),
            format: pm_kernel::persistence::DiskFormat::V1_2,
            electron_profile: None,
        },
    );
    let t = Instant::now();
    let ShellReply::Opened { .. } = answer(
        &k,
        ShellCall::Open {
            id: "big".into(),
            source: SessionSource::New(pm_model::json::parse(&json).unwrap()),
            electron_profile: None,
        },
    ) else {
        panic!()
    };
    eprintln!("open {:?}", t.elapsed());
    let p = k.published("big").unwrap();
    let t = Instant::now();
    let mut m = QueryMirror::new(&p);
    eprintln!("mirror new {:?}", t.elapsed());
    let t = Instant::now();
    let l = m.queries().layers(None);
    eprintln!("layers {} {:?}", l.len(), t.elapsed());
    k.call_blocking(
        "big",
        Request::new(
            Caller::native(),
            Call::Transact {
                label: None,
                expected_revision: None,
                body: WriteBody::Commands(vec![Command::SetStaticOpacity {
                    layer_id: "bulk-5".into(),
                    value: 50.,
                }]),
            },
        ),
    )
    .unwrap();
    let p = k.published("big").unwrap();
    let t = Instant::now();
    m.sync(&p);
    eprintln!("sync {:?}", t.elapsed());
    let t = Instant::now();
    let _ = m.queries().evaluate(None, "bulk-5", "opacity", 0.);
    eprintln!("evaluate {:?}", t.elapsed());
    actor.shutdown();
}
