//! JSON-lines integration fixture: real HostRouter/HostDataService, synthetic
//! KernelData double. Fixture controls are not production extension operations.
use pm_surface::host_api::{HostAck, HostOperation, HostRequest, HostRouter, Identity, Query};
use pm_surface::host_data::{HostDataService, KernelData, QuerySnapshot};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{self, BufRead};
use std::sync::{Arc, Mutex, mpsc::SyncSender};

#[derive(Default)]
struct FixtureState {
    revision: u64,
    state_revision: u64,
    values: BTreeMap<String, Value>,
    settings: BTreeMap<String, Value>,
    edits: Vec<Value>,
    reject: bool,
}
struct FixtureKernel(Arc<Mutex<FixtureState>>);
impl KernelData for FixtureKernel {
    fn query(
        &mut self,
        _: &Identity,
        query: &Query,
        _: Option<&[String]>,
        _: Option<f64>,
    ) -> Result<QuerySnapshot, String> {
        let state = self.0.lock().unwrap();
        let key = serde_json::to_value(query).unwrap();
        Ok(QuerySnapshot {
            revision: state.revision,
            value: state
                .values
                .get(key.as_str().unwrap())
                .cloned()
                .unwrap_or(Value::Null),
        })
    }
    fn enqueue(&mut self, request: HostRequest, reply: SyncSender<HostAck>) -> Result<(), String> {
        let mut state = self.0.lock().unwrap();
        let settings = matches!(
            request.operation,
            HostOperation::GetSetting { .. } | HostOperation::SetSetting { .. }
        );
        let mut accepted = if settings {
            request.expected_state_revision == Some(state.state_revision)
        } else {
            request.expected_revision == state.revision
        };
        let mut value = Value::Null;
        match &request.operation {
            HostOperation::Edit { .. } | HostOperation::Command { .. } => {
                accepted &= !state.reject;
                state.reject = false;
                if accepted {
                    state
                        .edits
                        .push(serde_json::to_value(&request.operation).unwrap());
                    state.revision += 1;
                    value = json!({"txid": request.request_id});
                }
            }
            HostOperation::GetSetting { key } => {
                value = state.settings.get(key).cloned().unwrap_or(Value::Null)
            }
            HostOperation::SetSetting { key, value } if accepted => {
                state.settings.insert(key.clone(), value.clone());
                state.state_revision += 1;
            }
            HostOperation::RegisterBinding { .. }
            | HostOperation::DisposeBinding { .. }
            | HostOperation::SetSetting { .. } => {}
            _ => return Err("query must route through HostDataService".into()),
        }
        reply
            .try_send(HostAck {
                version: request.version,
                realm: request.realm,
                project: request.project,
                generation: request.generation,
                request_id: request.request_id,
                revision: state.revision,
                accepted,
                state_revision: settings.then_some(state.state_revision),
                value: Some(value),
                error: (!accepted).then(|| "synthetic kernel admission declined".into()),
            })
            .map_err(|e| e.to_string())
    }
    fn retire(&mut self, _: &Identity) -> Result<(), String> {
        Ok(())
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Fixture {
    query: String,
    value: Value,
    revision: u64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
enum Input {
    Request(HostRequest),
    Fixture(Fixture),
    RejectNextEdit(bool),
    Publish(bool),
    Retire(bool),
}
fn main() {
    let state = Arc::new(Mutex::new(FixtureState::default()));
    let mut router = None;
    for line in io::stdin().lock().lines() {
        let result = (|| -> Result<Value, String> {
            let input: Input = serde_json::from_str(&line.map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            match input {
                Input::Request(request) => {
                    let id = request.request_id;
                    if router.is_none() {
                        let identity = Identity {
                            realm: request.realm.clone(),
                            generation: request.generation,
                            project: request.project.clone(),
                        };
                        router = Some(HostRouter::new(
                            identity.clone(),
                            HostDataService::new(identity, FixtureKernel(state.clone()))?,
                        )?);
                    }
                    let router = router.as_mut().unwrap();
                    router.enqueue(request)?;
                    Ok(json!({"ack": router.poll(id)?}))
                }
                Input::Fixture(fixture) => {
                    let mut state = state.lock().unwrap();
                    state.revision = fixture.revision;
                    state.values.insert(fixture.query, fixture.value);
                    Ok(json!({"fixture": true}))
                }
                Input::RejectNextEdit(value) => {
                    state.lock().unwrap().reject = value;
                    Ok(json!({"fixture": true}))
                }
                Input::Publish(true) => {
                    Ok(json!({"publications": router.as_mut().ok_or("no host")?.publications()?}))
                }
                Input::Retire(true) => {
                    router.as_mut().ok_or("no host")?.retire()?;
                    Ok(json!({"retired": true}))
                }
                _ => Err("fixture control must be true".into()),
            }
        })();
        println!(
            "{}",
            match result {
                Ok(value) => value,
                Err(error) => json!({"error": error}),
            }
        );
    }
}
