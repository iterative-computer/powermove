//! Linux test process only. Uses a real kernel actor with synthetic projects;
//! worker blocking waits here are never used by the production bridge.
use pm_kernel::{Caller, HistoryCall, KernelActor, KernelConfig, OpenSource};
use pm_panel_media::v4::{ProjectBridge, Request};
use pm_panel_media::{Availability, Catalog, MediaRuntime, RuntimeState, Stamp};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{self, BufRead, Write},
};
#[derive(Default)]
struct Runtime(HashMap<String, Availability>);
impl MediaRuntime for Runtime {
    fn state(&self, _: &Stamp, id: &str) -> RuntimeState {
        RuntimeState {
            availability: self.0.get(id).cloned().unwrap_or(Availability::Unknown),
            ..Default::default()
        }
    }
}
/// Kernel waits are illegal on the macOS main thread (actor.rs), so the host
/// loop runs on a worker thread.
fn main() {
    std::thread::spawn(run).join().expect("project model host");
}
fn run() {
    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    let mut bridge = ProjectBridge::default();
    let mut runtime = Runtime::default();
    let mut catalog: Option<Catalog> = None;
    for line in io::stdin().lock().lines() {
        let result = (|| -> Result<Value, String> {
            let value: Value = serde_json::from_str(&line.map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            if let Some(project) = value.get("initialize") {
                let data =
                    pm_model::json::parse(&project.to_string()).map_err(|e| format!("{e:?}"))?;
                kernel
                    .open("P", OpenSource::Untitled(data))
                    .wait()
                    .ok_or("kernel disconnected")?
                    .map_err(|e| format!("{e:?}"))?;
                runtime.0 =
                    serde_json::from_value(value.get("runtime").cloned().unwrap_or(json!({})))
                        .map_err(|e| e.to_string())?;
                let c = Catalog::published(kernel.published("P").ok_or("no publication")?.as_ref())
                    .map_err(|e| format!("{e:?}"))?;
                bridge.refresh(&c, &runtime);
                let view = bridge.view(&c, &runtime).map_err(|e| format!("{e:?}"))?;
                catalog = Some(c);
                return Ok(json!({"view": view}));
            }
            let c = catalog.as_ref().ok_or("initialize first")?;
            if let Some(history) = value.get("history").and_then(Value::as_str) {
                let call = if history == "Undo" {
                    HistoryCall::Undo
                } else {
                    HistoryCall::Redo
                };
                kernel
                    .call("P", pm_kernel::Request::new(Caller::native(), call))
                    .wait()
                    .ok_or("kernel disconnected")?
                    .map_err(|e| format!("{e:?}"))?;
            } else {
                let request: Request = serde_json::from_value(value).map_err(|e| e.to_string())?;
                let mut reply = bridge
                    .request(c, &runtime, request)
                    .map_err(|e| format!("{e:?}"))?;
                let mut applied = false;
                if let Some(edit) = reply.outcome.as_ref().and_then(|o| o.edit.clone()) {
                    let plan = c.prepare(edit).map_err(|e| format!("{e:?}"))?;
                    let ack = plan
                        .submit(&kernel, Caller::native())
                        .wait()
                        .ok_or("kernel disconnected")?
                        .map_err(|e| format!("{e:?}"))?;
                    let next =
                        Catalog::published(kernel.published("P").ok_or("no publication")?.as_ref())
                            .map_err(|e| format!("{e:?}"))?;
                    bridge
                        .panel
                        .accept_edit(&plan, &ack, &next, &runtime)
                        .map_err(|e| format!("{e:?}"))?;
                    applied = true;
                    catalog = Some(next);
                }
                reply.view = bridge
                    .view(catalog.as_ref().ok_or("no catalog")?, &runtime)
                    .map_err(|e| format!("{e:?}"))?;
                return Ok(json!({"reply": reply, "applied": applied}));
            }
            let next = Catalog::published(kernel.published("P").ok_or("no publication")?.as_ref())
                .map_err(|e| format!("{e:?}"))?;
            bridge.refresh(&next, &runtime);
            catalog = Some(next);
            Ok(
                json!({"view": bridge.view(catalog.as_ref().ok_or("no catalog")?, &runtime).map_err(|e| format!("{e:?}"))?}),
            )
        })();
        println!(
            "{}",
            match result {
                Ok(value) => value,
                Err(error) => json!({"error": error}),
            }
        );
        io::stdout().flush().expect("stdout");
    }
}
