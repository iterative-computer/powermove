//! Headless test-only worker: real model/actor acknowledgements, synthetic project.
//! Not an application KernelData adapter; no media/renderer is fabricated.
use pm_kernel::{Caller, KernelActor, KernelConfig, OpenSource};
use pm_model::{
    Value,
    json::{parse, to_canonical_string},
};
use pm_panel_viewerchrome::{v4::Session, *};
use std::io::{self, BufRead, Write};
fn main() {
    // Blocking kernel waits are forbidden on the macOS process main thread.
    // Keep the complete request/publication loop on the same worker thread.
    std::thread::spawn(run).join().expect("model wire worker");
}

fn run() {
    let actor = KernelActor::spawn(KernelConfig::default());
    let fixture = r##"{"id":"p","name":"Launch Film","compId":"c","compName":"Launch Film","w":1920,"h":1080,"fps":24,"dur":10,"work":[1,9],"bg":"#000000","layers":[],"assets":{},"comps":{}}"##;
    actor
        .handle()
        .open("p", OpenSource::Untitled(parse(fixture).unwrap()))
        .wait()
        .unwrap()
        .unwrap();
    let controller = Controller::new(actor.handle(), "p");
    let viewport = controller
        .read()
        .unwrap()
        .viewport(
            Size {
                width: 1052.8,
                height: 599.2,
            },
            8.0,
        )
        .unwrap();
    let mut session = Session::default();
    let publish = |session: &mut Session| {
        session
            .publish(&controller.read().unwrap(), viewport)
            .unwrap()
    };
    println!("{}", to_canonical_string(&publish(&mut session)));
    io::stdout().flush().unwrap();
    for line in io::stdin().lock().lines() {
        let result = parse(&line.unwrap())
            .map_err(|_| Error::Invalid("JSON"))
            .and_then(|request| {
                let model = controller.read()?;
                let plan = session.drive(&model, &request, viewport, Caller::native())?;
                // Refuse host-only requests: InspectHost enqueue isn't execution proof.
                if matches!(plan.effect, Effect::Host(_)) {
                    return Err(Error::Unsupported("host execution".into()));
                }
                match controller.submit(plan, &mut InspectHost::default())? {
                    Submission::State(p) => {
                        p.wait().ok_or(Error::Closed)?.map_err(|_| Error::Stale)?;
                    }
                    Submission::Document(_) | Submission::Enqueued => {
                        return Err(Error::Unsupported("test transport".into()));
                    }
                }
                Ok(publish(&mut session))
            });
        let mut response = pm_model::Map::new();
        match result {
            Ok(model) => {
                response.insert("accepted", Value::Bool(true));
                response.insert("model", model);
            }
            Err(error) => {
                response.insert("accepted", Value::Bool(false));
                response.insert("error", Value::from(error.to_string()));
            }
        }
        println!("{}", to_canonical_string(&Value::Object(response)));
        io::stdout().flush().unwrap();
    }
}
