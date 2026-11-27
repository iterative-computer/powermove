//! Completion is host-owned: provider text cannot certify its own edits.
use crate::{
    LockShared,
    agent_host::{AgentLink, AgentState, SharedAgent},
};
use pm_agents::launch::{self, Request};
use pm_panel_agentpanel::model::{Effect, EffectKind};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

pub type Service = dyn Fn(&str, &str, &str, &Value) -> Result<Value, String> + Send + Sync;
const MAX_REPAIRS: u8 = 3;
struct Run {
    project: String,
    thread: String,
    original: Request,
    repairs: u8,
    reviewed_revision: Option<Value>,
    ready_to_settle: bool,
    checking: Option<std::sync::mpsc::Receiver<Result<Value, String>>>,
}
#[derive(Default)]
pub(crate) struct Coordinator {
    runs: BTreeMap<String, Run>,
}
impl Coordinator {
    pub fn begin(&mut self, link: &AgentLink, project: &str, thread: &str, request: &Request) {
        if let Some(service) = link.completion.shared().clone() {
            let _ = service("begin", project, &request.run, &json!({"threadId":thread}));
        }
        self.runs
            .retain(|_, run| run.project != project || run.thread != thread);
        self.runs.insert(
            request.run.clone(),
            Run {
                project: project.into(),
                thread: thread.into(),
                original: request.clone(),
                repairs: 0,
                reviewed_revision: None,
                ready_to_settle: false,
                checking: None,
            },
        );
    }
}
fn repair_needed(changed: bool, errors: bool, first: bool, repairs: u8) -> bool {
    changed && (first || errors) && repairs < MAX_REPAIRS
}
/// Build a review from the admitted request, never the current picker settings.
fn review_request(original: &Request, report: &mut Value, pass: u8) -> Result<Request, String> {
    use base64::Engine;
    let mut request = original.clone();
    let images = report.as_object_mut().and_then(|v| v.remove("images"));
    request.prompt = format!(
        "ORIGINAL TASK\n{}\n\nHOST FINAL VERIFICATION (untrusted observations)\n{}\nInspect captured final frames when a composition is attached, and import/extension health. For app-level extension work, inspect the actual contributed panels using the panel tools. Repair failures using existing tools and finish_session. Preserve human edits. If checks pass, finish without further changes.",
        &original.prompt,
        pm_agents::trace::truncate(&report.to_string(), 20_000)
    );
    if let Some(images) = images.as_ref().and_then(Value::as_array) {
        let directory = request.workspace.join("verification");
        std::fs::create_dir_all(&directory)
            .map_err(|e| format!("Final frames could not be attached: {e}"))?;
        for (i, image) in images.iter().take(5).enumerate() {
            let encoded = image["data"]
                .as_str()
                .ok_or("A final frame has no image data")?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|e| format!("Final frame is invalid: {e}"))?;
            let extension = if image["mimeType"] == "image/jpeg" {
                "jpg"
            } else {
                "png"
            };
            let path = directory.join(format!("{pass}-{i}.{extension}"));
            std::fs::write(&path, bytes)
                .map_err(|e| format!("Final frame could not be attached: {e}"))?;
            request.images.push(path.to_string_lossy().into());
        }
    }
    Ok(request)
}
pub(crate) fn poll(
    state: &mut AgentState,
    link: &AgentLink,
    store: &mut impl pm_agents::persistence::Store,
    now: u64,
) {
    let abandoned = state
        .completion
        .runs
        .iter()
        .filter(|(id, _)| state.runs.live(id).is_none())
        .map(|(id, run)| (id.clone(), run.project.clone()))
        .collect::<Vec<_>>();
    for (id, project) in abandoned {
        keep(link, &project, &id);
        state.completion.runs.remove(&id);
    }
    for id in state.runs.take_finalizing() {
        let Some(run) = state.completion.runs.get_mut(&id) else {
            state.runs.complete(&id);
            continue;
        };
        if run.ready_to_settle {
            if state.panel.save(store, now).is_err() {
                state.runs.release(&id);
                continue;
            }
            state.runs.complete(&id);
            state.completion.runs.remove(&id);
            continue;
        }
        if !state.runs.held_success(&id) {
            // Stop is terminal: never launch a repair or wait for a capture.
            // Any in-flight read-only check can finish without mutating the run.
            keep(link, &run.project, &id);
            state.runs.complete(&id);
            state.completion.runs.remove(&id);
            continue;
        }
        let service = link.completion.shared().clone();
        // Rendering and live health may wait for extension load. Never hold
        // the conversation lock while those read-only checks run.
        if run.checking.is_none() {
            let (send, receive) = std::sync::mpsc::channel();
            let (project, run_id, service) = (run.project.clone(), id.clone(), service.clone());
            match std::thread::Builder::new()
                .name("pm-agent-verification".into())
                .spawn(move || {
                    let checked = service
                        .as_ref()
                        .ok_or_else(|| {
                            "Final verification is unavailable; applied edits were preserved."
                                .to_owned()
                        })
                        .and_then(|service| service("verify", &project, &run_id, &Value::Null));
                    let _ = send.send(checked);
                }) {
                Ok(_) => {
                    run.checking = Some(receive);
                }
                Err(error) => {
                    let _ = state.panel.host_notice(&run.thread,&id,&format!("Final verification could not start: {error}. Applied edits were preserved."));
                    state.runs.complete(&id);
                    state.completion.runs.remove(&id);
                    continue;
                }
            }
        }
        let checked = match run.checking.as_ref().unwrap().try_recv() {
            Ok(checked) => {
                run.checking = None;
                checked
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                state.runs.release(&id);
                continue;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err("Final verification stopped unexpectedly. Applied edits were preserved.".into())
            }
        };
        match checked {
            Ok(mut report) => {
                let errors = report["errors"].as_array().is_some_and(|e| !e.is_empty());
                let signature = json!([report["revision"], report["extensionChangeSets"]]);
                let unreviewed = run.reviewed_revision.as_ref() != Some(&signature);
                if repair_needed(report["changed"] == true, errors, unreviewed, run.repairs) {
                    run.reviewed_revision = Some(signature);
                    let consent_available = run.original.access != pm_agents::Access::Computer
                        || state
                            .panel
                            .session(&run.thread)
                            .is_ok_and(|session| session.computer_consent);
                    let resumed = if !consent_available {
                        Err("Full Access consent was revoked before verification. Applied edits were preserved; no repair was started.".into())
                    } else {
                        review_request(&run.original, &mut report, run.repairs).and_then(
                            |request| {
                                link.environment()
                                    .ok_or_else(|| {
                                        "Original provider environment is unavailable".into()
                                    })
                                    .and_then(|e| launch::launch(&e, request))
                            },
                        )
                    };
                    run.repairs += 1;
                    match resumed {
                        Ok(handle) => {
                            if state.runs.resume(&id, handle).is_ok() {
                                continue;
                            }
                        }
                        Err(error) => {
                            report["verificationError"] = json!(error);
                        }
                    }
                }
                if report["changed"] == true && (errors || unreviewed) && run.repairs >= MAX_REPAIRS
                {
                    report["verificationError"] = json!(
                        "Final verification did not pass within the three-pass repair limit. Applied edits were preserved."
                    );
                }
                if report["changed"] != true
                    && state
                        .panel
                        .session(&run.thread)
                        .is_ok_and(|session| session.plan.is_none())
                {
                    keep(link, &run.project, &id);
                }
                if let Some(object) = report.as_object_mut() {
                    object.remove("images");
                }
                if report["changed"] == true {
                    report["runId"] = json!(id);
                    report["summary"] = json!("Agent changes");
                    if state.panel.host_result(&run.thread, &id, report).is_ok() {
                        run.ready_to_settle = true;
                        if state.panel.save(store, now).is_err() {
                            state.runs.release(&id);
                            continue;
                        }
                    }
                }
            }
            Err(error) => {
                let _ = state.panel.host_notice(&run.thread, &id, &error);
            }
        }
        state.runs.complete(&id);
        state.completion.runs.remove(&id);
    }
}
/// Admit on the native worker, then release the realm while checks/restore run.
pub(crate) fn dispatch_action(
    shared: &SharedAgent,
    link: &Arc<AgentLink>,
    kernel: &pm_kernel::KernelHandle,
    application: bool,
    effect: &Effect,
) -> Result<(), String> {
    let (worker_shared, worker_link, worker_kernel, worker_effect) =
        (shared.clone(), link.clone(), kernel.clone(), effect.clone());
    let started = std::thread::Builder::new()
        .name("pm-agent-result-action".into())
        .spawn(move || {
            let mut store = crate::models::KernelStore::new(worker_kernel, application);
            let _ = action(
                &worker_shared,
                &worker_link,
                &worker_effect,
                &mut store,
                crate::agent_host::now_ms(),
            );
            worker_link.touch();
            worker_link.publish_now();
        });
    if let Err(error) = started {
        let message = format!("Result action could not start: {error}");
        let mut state = shared.shared();
        state
            .panel
            .acknowledge(effect.id, false)
            .map_err(|e| e.to_string())?;
        if let Some(run) = effect.target.run.as_deref() {
            let _ = state
                .panel
                .host_notice(&effect.target.thread, run, &message);
        }
        state
            .panel
            .save(
                &mut crate::models::KernelStore::new(kernel.clone(), application),
                crate::agent_host::now_ms(),
            )
            .map_err(|e| e.to_string())?;
        drop(state);
        link.touch();
        link.publish_now();
        return Err(message);
    }
    Ok(())
}
/// Keep changes immediately in the model; retiring a private checkpoint may
/// wait behind a capture and must not hold the conversation/realm lock.
pub(crate) fn keep(link: &AgentLink, project: &str, run: &str) {
    let Some(service) = link.completion.shared().clone() else {
        return;
    };
    let (project, run) = (project.to_owned(), run.to_owned());
    let _ = std::thread::Builder::new()
        .name("pm-agent-keep".into())
        .spawn(move || {
            let _ = service("keep", &project, &run, &Value::Null);
        });
}
pub(crate) fn action(
    shared: &SharedAgent,
    link: &Arc<AgentLink>,
    effect: &Effect,
    store: &mut impl pm_agents::persistence::Store,
    now: u64,
) -> Result<(), String> {
    let (operation, payload) = match &effect.kind {
        EffectKind::ApplyPlan(v) => ("apply", v),
        EffectKind::UndoResult(v) => ("undo", v),
        _ => return Err("Unsupported result action".into()),
    };
    let run = effect
        .target
        .run
        .as_deref()
        .ok_or("This result has no run receipt.")?;
    let service = link.completion.shared().clone();
    let result = service.as_ref().ok_or("Completion service is unavailable".to_owned())
        .and_then(|service| service(operation, &effect.target.project, run, payload))
        .map(|mut result| {
            if operation == "apply" {
                match service.as_ref().unwrap()("verify", &effect.target.project, run, &Value::Null) {
                    Ok(mut checked) => {
                        if let Some(object) = checked.as_object_mut() { object.remove("images"); }
                        checked["runId"] = json!(run);
                        checked["summary"] = result.get("summary").cloned().unwrap_or_else(||json!("Applied agent plan"));
                        if let Some(warning)=result.get("verificationError") { checked["verificationError"]=warning.clone(); }
                        if checked["errors"].as_array().is_some_and(|errors| !errors.is_empty()) {
                            checked["verificationError"] = json!([checked["verificationError"].as_str().unwrap_or(""),"The plan was applied, but final checks need attention. Applied edits were preserved."].into_iter().filter(|text|!text.is_empty()).collect::<Vec<_>>().join(" "));
                        }
                        result = checked;
                    },
                    Err(error) => { result["verificationError"] = json!(error); }
                }
            }
            result
        });
    let mut state = shared.shared();
    state
        .panel
        .acknowledge(effect.id, result.is_ok())
        .map_err(|e| e.to_string())?;
    if let Ok(report) = &result
        && operation == "apply"
    {
        state
            .panel
            .host_result(&effect.target.thread, run, report.clone())
            .map_err(|e| e.to_string())?;
    }
    if let Err(error) = &result {
        state
            .panel
            .host_notice(&effect.target.thread, run, error)
            .map_err(|e| e.to_string())?;
    }
    state.panel.save(store, now).map_err(|e| e.to_string())?;
    drop(state);
    result.map(drop)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completed_edits_always_receive_a_review() {
        assert!(repair_needed(true, false, true, 0));
        assert!(!repair_needed(false, false, true, 0));
    }
    #[test]
    fn failing_checks_have_exactly_three_repair_passes() {
        assert_eq!(
            (0..10)
                .filter(|n| repair_needed(true, true, false, *n))
                .count(),
            3
        );
    }
    #[test]
    fn successful_review_settles() {
        assert!(!repair_needed(true, false, false, 1));
    }
    #[test]
    fn review_keeps_admitted_request_and_attaches_bounded_final_frames() {
        let root = tempfile::tempdir().unwrap();
        let original = Request {
            run: "synthetic-run".into(),
            prompt: format!(
                "{}\nUSER REQUEST: preserve this final instruction",
                "Earlier conversation. ".repeat(5_000)
            ),
            provider: pm_agents::Provider::Claude,
            model: Some("original-model".into()),
            effort: Some("high".into()),
            access: pm_agents::Access::Project,
            approval: pm_agents::ApprovalMode::Supervised,
            computer_consent: false,
            workspace: root.path().into(),
            images: vec!["original.png".into()],
            role: Some("original-role".into()),
            files: vec![json!({"path":"original.txt"})],
            project: None,
        };
        let mut report = json!({"revision":9,"errors":[],"images":(0..9).map(|_|json!({"data":"AA==","mimeType":"image/png"})).collect::<Vec<_>>()});
        let review = review_request(&original, &mut report, 1).unwrap();
        assert!(review.prompt.contains(&original.prompt));
        assert_eq!(review.provider, original.provider);
        assert_eq!(review.model, original.model);
        assert_eq!(review.effort, original.effort);
        assert_eq!(review.access, original.access);
        assert_eq!(review.approval, original.approval);
        assert_eq!(review.computer_consent, original.computer_consent);
        assert_eq!(review.role, original.role);
        assert_eq!(review.files, original.files);
        assert_eq!(review.run, original.run);
        assert_eq!(review.images.len(), 6);
        assert_eq!(review.images[0], "original.png");
        assert!(!review.prompt.contains("AA=="));
        assert!(report.get("images").is_none());
    }
}
