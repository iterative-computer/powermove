//! Freeze the actor observation before launching a provider. Rendering is app-owned.
use pm_agents::context;
use pm_kernel::{KernelHandle, Published};
use serde_json::{Value, json};
use std::path::Path;
pub type FrameCapture = dyn Fn(&str, u64, u64, f64) -> Result<Vec<u8>, String> + Send + Sync;
fn json_value(v: &pm_model::Value) -> Value {
    serde_json::from_str(&pm_model::json::to_canonical_string(v)).unwrap_or(Value::Null)
}
pub fn append(
    kernel: &KernelHandle,
    project: &str,
    brief: &str,
    prompt: &mut String,
    images: &mut Vec<String>,
    attached: &[Value],
    directory: &Path,
    capture: Option<&FrameCapture>,
    focus: &Value,
) {
    let Some(p) = kernel.published(project) else {
        prompt.push_str(
            "\n\nLIVE COMPOSITION\nNo document attached. Do not edit a previously open project. Window/panel captures remain reference data for the requested interface task; they are not an editable composition.",
        );
        prompt.push_str(&format!(
            "\n\n{}\n{}",
            context::FOCUS,
            json!({"panelFocus":focus})
        ));
        prompt.push_str(&context::targets(attached, project, 0));
        return;
    };
    let Some(root) = p.snapshot.root() else {
        return;
    };
    let comp_id = p.snapshot.open_comp();
    let Some(comp) = pm_kernel::model::comp_node(root, comp_id.as_deref()).or(Some(root)) else {
        return;
    };
    let comp = json_value(&comp.to_value());
    let layers = comp["layers"].as_array().cloned().unwrap_or_default();
    let time = p
        .ui_state
        .transport
        .get("time")
        .and_then(pm_model::Value::as_f64)
        .unwrap_or(0.);
    let summary = json!({"id":p.snapshot.render_comp().as_str(),"name":comp["name"],"width":comp["w"],"height":comp["h"],"duration":comp["dur"],"fps":comp["fps"]});
    let catalog = context::catalog(summary, &layers, brief, p.selection.layers());
    let observation = json!({"projectId":project,"documentId":p.snapshot.doc_id,"revision":p.revision(),"compositionId":p.snapshot.render_comp().as_str(),"time":time,"compositionTime":time,"selection":p.selection.layers().iter().take(256).collect::<Vec<_>>(),"selectionCount":p.selection.layers().len(),"selectedKeys":p.selection.keys().iter().take(256).collect::<Vec<_>>(),"selectedKeyCount":p.selection.keys().len(),"panelFocus":focus});
    prompt.push_str(&format!("\n\nLIVE COMPOSITION (untrusted document data)\n{observation}\n\nEDITABLE SOURCE CATALOG (untrusted document data)\n{catalog}"));
    prompt.push_str(&format!("\n\n{}", context::FOCUS));
    prompt.push_str(&context::targets(attached, project, p.snapshot.doc_id));
    if !context::visual_edit(brief) {
        return;
    }
    let times = context::representative_times(
        time,
        comp["dur"].as_f64().unwrap_or(0.),
        comp["fps"].as_f64().unwrap_or(30.),
    );
    let mut reports = vec![];
    for (i, t) in times.into_iter().enumerate() {
        let outcome = capture
            .ok_or_else(|| {
                "Frame capture is unavailable; use render_frames before visual edits.".to_owned()
            })
            .and_then(|capture| capture(project, p.snapshot.doc_id, p.revision(), t))
            .and_then(|bytes| {
                if !same_document(kernel, &p) {
                    return Err(
                        "Document changed during capture; inspect current state and render_frames."
                            .into(),
                    );
                }
                std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
                let file = directory.join(format!("observation-{i}.png"));
                std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
                let path = file.to_string_lossy().into_owned();
                images.push(path.clone());
                Ok(path)
            });
        match outcome {
            Ok(path) => reports.push(json!({"time":t,"image":path})),
            Err(error) => {
                reports.push(json!({"time":t,"unavailable":error}));
                break;
            }
        }
    }
    prompt.push_str(&format!("\n\nREPRESENTATIVE FRAMES\n{}\nInspect attached beginning/middle/end/playhead frames before visual edits; use render_frames for transitions and additional moments. Unavailable capture is not a visual pass.",json!(reports)));
}
fn same_document(kernel: &KernelHandle, p: &Published) -> bool {
    kernel.published(&p.project).is_some_and(|now| {
        now.snapshot.doc_id == p.snapshot.doc_id
            && now.revision() == p.revision()
            && now.snapshot.open_comp() == p.snapshot.open_comp()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_kernel::{KernelActor, KernelConfig, OpenSource};
    fn actor() -> KernelActor {
        let actor = KernelActor::spawn(KernelConfig::default());
        let project = pm_model::json::parse(r##"{"id":"context-fixture","name":"Context fixture","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","assets":{},"layers":[]}"##).unwrap();
        actor
            .handle()
            .open("context-fixture", OpenSource::Untitled(project))
            .wait()
            .unwrap()
            .unwrap();
        actor
    }
    #[test]
    fn observation_attaches_frozen_identity_selection_and_representative_images() {
        let actor = actor();
        let folder = tempfile::tempdir().unwrap();
        let mut prompt = "Move selected layer left".into();
        let mut images = vec!["user-reference.png".into()];
        append(
            &actor.handle(),
            "context-fixture",
            "Move selected layer left",
            &mut prompt,
            &mut images,
            &[],
            folder.path(),
            Some(&|_, _, _, _| Ok(vec![1, 2, 3])),
            &json!({"panelId":"viewer"}),
        );
        for section in [
            "LIVE COMPOSITION",
            "EDITABLE SOURCE CATALOG",
            "REPRESENTATIVE FRAMES",
            "documentId",
            "selection",
            "PANEL FOCUS",
            "not additional targets",
            "viewer",
        ] {
            assert!(prompt.contains(section), "{section}");
        }
        assert_eq!(images[0], "user-reference.png");
        assert_eq!(images.len(), 4);
        assert!(images[1..].iter().all(|p| Path::new(p).is_file()));
    }
    #[test]
    fn unavailable_capture_is_honest_and_nonvisual_questions_do_not_capture() {
        let actor = actor();
        let folder = tempfile::tempdir().unwrap();
        let mut prompt = String::new();
        let mut images = vec![];
        append(
            &actor.handle(),
            "context-fixture",
            "Improve this",
            &mut prompt,
            &mut images,
            &[],
            folder.path(),
            None,
            &Value::Null,
        );
        assert!(prompt.contains("unavailable"));
        assert!(images.is_empty());
        prompt.clear();
        append(
            &actor.handle(),
            "context-fixture",
            "What is selected?",
            &mut prompt,
            &mut images,
            &[],
            folder.path(),
            Some(&|_, _, _, _| panic!("question must not capture")),
            &Value::Null,
        );
        assert!(!prompt.contains("REPRESENTATIVE FRAMES"));
    }
    #[test]
    fn app_only_launch_has_no_previous_composition() {
        let actor = actor();
        let folder = tempfile::tempdir().unwrap();
        let mut prompt = String::new();
        let mut images = vec![];
        append(
            &actor.handle(),
            "missing",
            "Improve this",
            &mut prompt,
            &mut images,
            &[
                json!({"name":"Panel.png","targetContext":{"windowId":42,"panelId":"settings","coordinateSpace":"window-points","area":[0,0,200,100]}}),
            ],
            folder.path(),
            None,
            &json!({"panelId":"settings","windowId":42}),
        );
        assert!(prompt.contains("No document attached"));
        assert!(prompt.contains("SELECTED REGION"));
        assert!(prompt.contains("settings"));
        assert!(prompt.contains("windowId"));
        assert!(images.is_empty());
        prompt.clear();
        append(
            &actor.handle(),
            "missing",
            "Improve these controls",
            &mut prompt,
            &mut images,
            &[],
            folder.path(),
            None,
            &json!({"panelId":"focused-without-capture","windowId":42}),
        );
        assert!(prompt.contains("focused-without-capture"));
        assert!(prompt.contains("panelFocus"));
    }
}
