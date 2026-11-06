//! Test-only JSON-lines bridge: public HostClient -> HostRouter/HostDataService
//! -> real Agent Panel -> six-op view. Synthetic Store/native facts, no runtime IO.
use pm_agents::persistence::Store;
use pm_panel_agentpanel::{
    model::{Effect, Panel},
    v4::{self, Context, Window},
};
use pm_surface::{
    host_api::{HostAck, HostOperation, HostRequest, HostRouter, Identity, Query},
    host_data::{HostDataService, KernelData, QuerySnapshot},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{self, BufRead},
    sync::{Arc, Mutex, mpsc::SyncSender},
};
#[derive(Default)]
struct Memory(BTreeMap<String, Value>);
impl Store for Memory {
    fn get(&mut self, k: &str) -> io::Result<Option<Value>> {
        Ok(self.0.get(k).cloned())
    }
    fn set(&mut self, k: &str, v: &Value) -> io::Result<()> {
        self.0.insert(k.into(), v.clone());
        Ok(())
    }
}
struct State {
    panel: Panel,
    native: Value,
    sequence: u64,
    effects: Vec<Effect>,
}
struct Backend(Arc<Mutex<State>>);
impl KernelData for Backend {
    fn query(
        &mut self,
        _: &Identity,
        _: &Query,
        _: Option<&[String]>,
        _: Option<f64>,
    ) -> Result<QuerySnapshot, String> {
        Ok(QuerySnapshot {
            revision: 0,
            value: json!({"synthetic":true}),
        })
    }
    fn enqueue(&mut self, request: HostRequest, reply: SyncSender<HostAck>) -> Result<(), String> {
        let result = (|| -> Result<Value, String> {
            let mut state = self.0.lock().unwrap();
            let HostOperation::Command { command, args } = &request.operation else {
                return Err("unregistered synthetic operation".into());
            };
            if request.expected_revision != 0 {
                return Err("stale document revision".into());
            }
            if command == "fixture.dispatch" {
                let sequence = state.sequence;
                let next = sequence + 1;
                let effect = v4::dispatch_at(
                    &mut state.panel,
                    sequence,
                    args,
                    Context {
                        run: &format!("run-{next}"),
                        request: &format!("request-{next}"),
                        new_thread: &format!("thread-{next}"),
                        now: next * 1000,
                        popover_open: true,
                        catalogs: &|_| Default::default(),
                    },
                )
                .map_err(|e| format!("{e:?}"))?;
                if let Some(effect) = effect {
                    state.effects.push(effect);
                }
                state.sequence = next;
            } else if command != "fixture.read" {
                return Err("unregistered synthetic command".into());
            }
            let window =
                Window::new(&state.panel, state.panel.active_id()).map_err(|e| format!("{e:?}"))?;
            let end = window.count(&state.panel).map_err(|e| format!("{e:?}"))?;
            let snapshot = window
                .snapshot(&state.panel, 0..end.min(256), state.sequence * 1000)
                .map_err(|e| format!("{e:?}"))?;
            Ok(
                json!({"generation":request.generation,"sequence":state.sequence,"snapshot":snapshot,"native":state.native}),
            )
        })();
        let (accepted, value, error) = match result {
            Ok(value) => (true, Some(value), None),
            Err(error) => (false, None, Some(error)),
        };
        reply
            .try_send(HostAck {
                version: request.version,
                realm: request.realm,
                project: request.project,
                generation: request.generation,
                request_id: request.request_id,
                revision: 0,
                state_revision: None,
                accepted,
                value,
                error,
            })
            .map_err(|e| e.to_string())
    }
    fn retire(&mut self, _: &Identity) -> Result<(), String> {
        Ok(())
    }
}
fn main() {
    let state = Arc::new(Mutex::new(State {
        panel: Panel::load(&mut Memory::default(), "synthetic-project", "thread", 0).unwrap(),
        native: Value::Null,
        sequence: 1,
        effects: vec![],
    }));
    let mut router = None;
    let mut surface = pm_surface::Surface::default();
    for line in io::stdin().lock().lines() {
        let result = (|| -> Result<Value, String> {
            let input: Value = serde_json::from_str(&line.map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            if let Some(commit) = input.get("commit") {
                let diagnostics = surface
                    .apply_json(&commit.to_string())
                    .map_err(|e| format!("{e:?}"))?;
                let ax = pm_surface::kit::accessibility_tree(
                    &surface,
                    &pm_surface::kit::Layout::new(),
                    None,
                );
                let nodes: Vec<_> = ax.nodes.iter().map(|(id,node)|json!({"id":id.0,"role":format!("{:?}",node.role()),"label":node.label(),"value":node.value(),"rowCount":node.row_count(),"disabled":node.is_disabled()})).collect();
                return Ok(json!({"diagnostics":diagnostics,"accessibility":nodes}));
            }
            if let Some(native) = input.get("native") {
                state.lock().unwrap().native = native.clone();
                return Ok(json!({"fixture":true}));
            }
            let request: HostRequest =
                serde_json::from_value(input["request"].clone()).map_err(|e| e.to_string())?;
            let id = request.request_id;
            if router.is_none() {
                let identity = Identity {
                    realm: request.realm.clone(),
                    project: request.project.clone(),
                    generation: request.generation,
                };
                router = Some(HostRouter::new(
                    identity.clone(),
                    HostDataService::new(identity, Backend(state.clone()))?,
                )?);
            }
            let router = router.as_mut().unwrap();
            router.enqueue(request)?;
            let ack = router.poll(id)?;
            let effects = state.lock().unwrap().effects.len();
            Ok(json!({"ack":ack,"pendingEffects":effects}))
        })();
        println!(
            "{}",
            match result {
                Ok(v) => v,
                Err(error) => json!({"error":error}),
            }
        );
    }
}
