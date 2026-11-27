//! Test-only JSON-lines fixture. Synthetic project, real model/kernel actor.
//! Blocking stdin/future driving belongs to this headless test executable only.
//! HostEffects are reported separately and never claimed to have executed.
use pm_kernel::{Caller, KernelActor, KernelConfig, OpenSource};
use pm_model::json::parse;
use pm_panel_timeline::{PropertyFacts, TimelineClient, v4};
use serde_json::{Value, json};
use std::{
    future::Future,
    io::{self, BufRead},
    pin::pin,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
    time::Duration,
};

fn block_on<F: Future>(future: F) -> F::Output {
    struct Unpark(std::thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return value;
        }
        std::thread::park_timeout(Duration::from_secs(1));
    }
}
// Keep the kernel fixture and blocking executor off the macOS main thread.
fn main() {
    std::thread::spawn(run).join().expect("timeline model wire");
}
fn run() {
    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    let project = json!({"id":"P","compId":"C","w":100,"h":100,"fps":30,"dur":10,"work":[0,10],"comps":{},"assets":{},"layers":[
        {"id":"headline","name":"Headline","type":"shape","from":-0.125,"dur":4,"on":true,"p":{}},
        {"id":"kicker","name":"Kicker","type":"shape","from":5,"dur":2,"on":true,"p":{}}
    ]});
    block_on(kernel.open(
        "P",
        OpenSource::Untitled(parse(&project.to_string()).unwrap()),
    ))
    .unwrap()
    .unwrap();
    let mut client = TimelineClient::new(kernel, "P", Caller::native()).unwrap();
    for line in io::stdin().lock().lines() {
        let response = (|| -> Result<Value, String> {
            let request: Value = serde_json::from_str(&line.map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            client.sync().map_err(|e| e.to_string())?;
            match request["op"].as_str() {
                Some("window") => {
                    let first = request["first"]
                        .as_u64()
                        .and_then(|n| usize::try_from(n).ok())
                        .ok_or("invalid first")?;
                    let visible = request["visible"]
                        .as_u64()
                        .and_then(|n| usize::try_from(n).ok())
                        .ok_or("invalid visible")?;
                    Ok(
                        json!({"modelAccepted":true,"window":v4::window(&mut client, first, visible).map_err(|e| e.to_string())?}),
                    )
                }
                Some("act") => {
                    let facts = PropertyFacts::default();
                    let context = v4::Context {
                        time: 0.,
                        expected_document: client.model.index.doc_id,
                        expected_revision: client.model.index.revision,
                        millis: 100,
                        in_field: false,
                        keys_admitted: true,
                        property_facts: &facts,
                    };
                    let effects = block_on(v4::dispatch(&mut client, &request["intent"], &context))
                        .map_err(|e| e.to_string())?;
                    Ok(
                        json!({"modelAccepted":true,"hostCompleted":effects.is_empty(),"effects":effects.iter().map(|e| format!("{e:?}")).collect::<Vec<_>>()}),
                    )
                }
                _ => Err("unknown fixture operation".into()),
            }
        })();
        println!(
            "{}",
            response.unwrap_or_else(|error| json!({"modelAccepted":false,"error":error}))
        );
    }
    block_on(client.dispose()).unwrap();
}
