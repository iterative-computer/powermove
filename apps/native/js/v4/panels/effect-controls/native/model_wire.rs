//! Test-only line protocol over the real pm-panel-inspector and kernel actor.
//! Blocking waits occur only in this Linux executable, never the application.
use pm_kernel::{Caller, KernelActor, KernelConfig, OpenSource, Reply, Request, SelectionCall};
use pm_model::json::{parse, to_canonical_string};
use pm_panel_inspector::{
    EffectClipboard, EffectDefinition, Inspector, ParamDefinition,
    v4::{EffectsPanel, ParameterPresentation, Presentation},
};
use std::io::{self, BufRead};
fn main() {
    let actor = KernelActor::spawn(KernelConfig::default());
    let handle = actor.handle();
    let project = parse(r##"{"id":"P","compId":"C","name":"Synthetic","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","assets":{},"comps":{},"layers":[{"id":"a","type":"shape","name":"Headline","from":0,"dur":10,"on":true,"lock":false,"p":{},"d":{"w":100,"h":100},"fx":[{"id":"f1","type":"glow","on":true,"open":true,"p":{"radius":{"v":24,"kf":[],"expr":null},"intensity":{"v":0.65,"kf":[],"expr":null},"color":{"v":"#FF7A3C","kf":[],"expr":null}}},{"id":"f2","type":"glow","on":false,"open":false,"p":{"radius":{"v":12,"kf":[],"expr":null},"intensity":{"v":0.5,"kf":[],"expr":null},"color":{"v":"#FFFFFF","kf":[],"expr":null}}},{"id":"f3","type":"missing","on":true,"open":true,"p":{"unknown":{"v":1,"opaque":"preserve"}}}]},{"id":"b","type":"text","name":"Destination","from":0,"dur":10,"on":true,"p":{},"d":{},"fx":[]},{"id":"locked","type":"shape","name":"Locked","from":0,"dur":10,"on":true,"lock":true,"p":{},"d":{},"fx":[]},{"id":"audio","type":"audio","name":"Audio","from":0,"dur":10,"on":true,"p":{},"d":{},"fx":[]}]}"##).unwrap();
    handle
        .open("P", OpenSource::Untitled(project))
        .wait()
        .unwrap()
        .unwrap();
    let mut model = Inspector::new(handle.published("P").unwrap(), EffectClipboard::default());
    model.register_effect(EffectDefinition {
        effect_type: "glow".into(),
        version: 1,
        label: "Glow".into(),
        source_processing: false,
        backdrop: false,
        params: vec![
            ParamDefinition {
                key: "radius".into(),
                kind: "number".into(),
                default: 0.into(),
                min: Some(0.0),
                max: None,
                reference: false,
            },
            ParamDefinition {
                key: "intensity".into(),
                kind: "number".into(),
                default: 1.into(),
                min: Some(0.0),
                max: None,
                reference: false,
            },
            ParamDefinition {
                key: "color".into(),
                kind: "color".into(),
                default: "#FFFFFF".into(),
                min: None,
                max: None,
                reference: false,
            },
        ],
    });
    let mut presentation = Presentation::default();
    for (kind, label) in [("shape", "Shape"), ("text", "Text"), ("audio", "Audio")] {
        presentation
            .layer_type_labels
            .insert(kind.into(), label.into());
    }
    for (key, label, control, step, unit) in [
        ("radius", "Radius", "number", Some(1.0), Some("px")),
        ("intensity", "Intensity", "number", Some(0.01), None),
        ("color", "Color", "color", None, None),
    ] {
        presentation.parameters.insert(
            ("glow".into(), key.into()),
            ParameterPresentation {
                label: label.into(),
                control: control.into(),
                step,
                unit: unit.map(str::to_owned),
                precision: None,
                options: vec![],
            },
        );
    }
    let panel = EffectsPanel::new(presentation, 0..32);
    let backend = std::rc::Rc::new(std::cell::RefCell::new(Backend {
        model,
        panel,
        handle,
        pending: vec![],
        bindings: std::collections::HashMap::new(),
        reject: false,
    }));
    let mut router = None;
    for line in io::stdin().lock().lines() {
        let result = (|| -> Result<serde_json::Value, String> {
            let input: serde_json::Value = serde_json::from_str(&line.map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            if let Some(request) = input.get("request") {
                let request: pm_surface::host_api::HostRequest =
                    serde_json::from_value(request.clone()).map_err(|e| e.to_string())?;
                let id = request.request_id;
                if router.is_none() {
                    let identity = pm_surface::host_api::Identity {
                        realm: request.realm.clone(),
                        generation: request.generation,
                        project: request.project.clone(),
                    };
                    router = Some(pm_surface::host_api::HostRouter::new(
                        identity.clone(),
                        pm_surface::host_data::HostDataService::new(
                            identity,
                            ModelKernel(backend.clone()),
                        )?,
                    )?);
                }
                let r = router.as_mut().unwrap();
                r.enqueue(request)?;
                // Test-process pump only. Production wakes/polls the worker queue.
                loop {
                    backend.borrow_mut().tick()?;
                    if let Some(ack) = r.poll(id)? {
                        return Ok(serde_json::json!({"ack":ack}));
                    }
                    std::thread::yield_now();
                }
            }
            if input.get("publish").is_some() {
                return Ok(
                    serde_json::json!({"publications": router.as_mut().ok_or("no router")?.publications()?}),
                );
            }
            let mut b = backend.borrow_mut();
            if let Some(id) = input.get("selectLayer").and_then(serde_json::Value::as_str) {
                let request = Request::new(
                    Caller::native(),
                    SelectionCall::Layers {
                        ids: if id.is_empty() {
                            vec![]
                        } else {
                            vec![id.into()]
                        },
                        add: false,
                    },
                );
                let ack = b.handle.call("P", request).wait().unwrap().unwrap();
                if matches!(ack.reply, Reply::Throw(_)) {
                    return Err(format!("{:?}", ack.reply));
                }
                b.sync();
            } else if let Some(history) = input.get("history") {
                let history = if history == "undo" {
                    pm_kernel::HistoryCall::Undo
                } else {
                    pm_kernel::HistoryCall::Redo
                };
                let ack = b
                    .handle
                    .call("P", Request::new(Caller::native(), history))
                    .wait()
                    .unwrap()
                    .unwrap();
                if matches!(ack.reply, Reply::Throw(_)) {
                    return Err(format!("{:?}", ack.reply));
                }
                b.sync();
            } else if input.get("reject").is_some() {
                b.reject = true;
            }
            let value = b.project()?;
            Ok(serde_json::json!({"value": value}))
        })();
        println!(
            "{}",
            match result {
                Ok(value) => value,
                Err(error) => serde_json::json!({"error": error}),
            }
        );
    }
    actor.shutdown();
}

use pm_surface::host_api::{HostAck, HostOperation, HostRequest, Identity, Query};
use pm_surface::host_data::{KernelData, QuerySnapshot};
use std::{cell::RefCell, rc::Rc, sync::mpsc::SyncSender};
struct Inflight {
    request: HostRequest,
    reply: SyncSender<HostAck>,
    plan: pm_panel_inspector::v4::Plan,
    pending: pm_kernel::Pending<Result<pm_kernel::Ack, pm_kernel::KernelFailure>>,
}
struct Backend {
    model: Inspector,
    panel: EffectsPanel,
    handle: pm_kernel::KernelHandle,
    pending: Vec<Inflight>,
    bindings: std::collections::HashMap<String, Vec<String>>,
    reject: bool,
}
impl Backend {
    fn sync(&mut self) {
        self.model.sync(self.handle.published("P").unwrap());
    }
    fn project(&mut self) -> Result<serde_json::Value, String> {
        let value = self
            .panel
            .project(&mut self.model, 0.0)
            .map_err(|e| e.to_string())?;
        serde_json::from_str(&to_canonical_string(&value)).map_err(|e| e.to_string())
    }
    fn reply(
        &mut self,
        request: HostRequest,
        reply: SyncSender<HostAck>,
        result: Result<serde_json::Value, String>,
    ) -> Result<(), String> {
        let (value, error) = match result {
            Ok(value) => (Some(value), None),
            Err(error) => (None, Some(error)),
        };
        reply
            .try_send(HostAck {
                version: 1,
                realm: request.realm,
                generation: request.generation,
                project: request.project,
                request_id: request.request_id,
                revision: self.model.revision(),
                state_revision: None,
                accepted: error.is_none(),
                value,
                error,
            })
            .map_err(|e| e.to_string())
    }
    fn tick(&mut self) -> Result<(), String> {
        let mut at = 0;
        while at < self.pending.len() {
            let Some(result) = self.pending[at].pending.try_take() else {
                at += 1;
                continue;
            };
            let item = self.pending.remove(at);
            self.sync();
            let result = result
                .map_err(|e| format!("{e:?}"))
                .and_then(|ack| {
                    self.panel
                        .acknowledge(&mut self.model, item.plan, &ack)
                        .map_err(|e| e.to_string())
                })
                .and_then(|_| self.project());
            self.reply(item.request, item.reply, result)?;
        }
        Ok(())
    }
}
struct ModelKernel(Rc<RefCell<Backend>>);
impl KernelData for ModelKernel {
    fn query(
        &mut self,
        identity: &Identity,
        query: &Query,
        channels: Option<&[String]>,
        _: Option<f64>,
    ) -> Result<QuerySnapshot, String> {
        if identity.project != "P" {
            return Err("foreign project".into());
        }
        if matches!(query, Query::Evaluated)
            && channels != Some(&["panel:layer-effects".to_owned()][..])
        {
            return Err("unknown model channel".into());
        }
        let mut b = self.0.borrow_mut();
        b.sync();
        Ok(QuerySnapshot {
            revision: b.model.revision(),
            value: b.project()?,
        })
    }
    fn enqueue(&mut self, request: HostRequest, reply: SyncSender<HostAck>) -> Result<(), String> {
        let mut b = self.0.borrow_mut();
        b.sync();
        if request.expected_revision != b.model.revision() {
            return b.reply(request, reply, Err("stale revision".into()));
        }
        match &request.operation {
            HostOperation::RegisterBinding { handle, channels } => {
                b.bindings.insert(handle.clone(), channels.clone());
            }
            HostOperation::DisposeBinding { handle } => {
                b.bindings.remove(handle);
            }
            HostOperation::Command { command, args } if command == "panel.effect-controls" => {
                if b.reject {
                    b.reject = false;
                    return b.reply(request, reply, Err("fixture admission rejected".into()));
                }
                let args = parse(&args.to_string()).map_err(|e| format!("{e:?}"))?;
                let Backend { panel, model, .. } = &mut *b;
                let plan = panel.plan(model, Caller::native(), &args, 0.0);
                match plan {
                    Err(error) => return b.reply(request, reply, Err(error.to_string())),
                    Ok(mut plan) => {
                        if let Some(mut kernel) = plan.request.take() {
                            kernel.txid = Some(request.request_id);
                            let pending = b.model.submit(&b.handle, kernel);
                            b.pending.push(Inflight {
                                request,
                                reply,
                                plan,
                                pending,
                            });
                            return Ok(());
                        }
                    }
                }
            }
            _ => return b.reply(request, reply, Err("unsupported test operation".into())),
        }
        let value = b.project();
        b.reply(request, reply, value)
    }
    fn retire(&mut self, _: &Identity) -> Result<(), String> {
        self.0.borrow_mut().bindings.clear();
        Ok(())
    }
}
