//! Development-only Linux/macOS conformance probe; no Node/app dependency.
use pm_design::{Accessibility, Scheme, Tokens};
use pm_surface::{
    Surface,
    binding::{BindingSink, Edit},
    controls::Controls,
    kit::{Layout, NumericScrub, Panel},
};
use serde_json::{Value, json};
use std::{
    io::{self, BufRead},
    sync::Arc,
};

#[derive(Default)]
struct Sink(Vec<Value>);
impl BindingSink for Sink {
    fn enqueue(&mut self, edit: Edit) -> Result<(), String> {
        self.0.push(match edit {
            Edit::Begin { binding } => json!({"type":"begin", "binding": binding}),
            Edit::Write { binding, value } => {
                json!({"type":"write", "binding":binding, "value":value})
            }
            Edit::Commit { binding } => json!({"type":"commit", "binding":binding}),
            Edit::Cancel { binding } => json!({"type":"cancel", "binding":binding}),
        });
        Ok(())
    }
}
fn id(surface: &Surface, request: &Value) -> Result<u64, String> {
    surface
        .nodes()
        .values()
        .find(|node| node.props.get("id") == request.get("id"))
        .map(|node| node.id)
        .ok_or_else(|| "Missing control".into())
}
fn flag(request: &Value, key: &str) -> bool {
    request.get(key).and_then(Value::as_bool).unwrap_or(false)
}
fn text<'a>(request: &'a Value, key: &str) -> &'a str {
    request.get(key).and_then(Value::as_str).unwrap_or("")
}
fn main() {
    let mut surface = Surface::default();
    let mut controls = Controls::default();
    let mut sink = Sink::default();
    let mut scrub: Option<NumericScrub> = None;
    for line in io::stdin().lock().lines() {
        let result = (|| -> Result<Value, String> {
            let line = line.map_err(|e| e.to_string())?;
            let input: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
            let options = &input["appearance"];
            let tokens = Tokens::new(
                if flag(options, "light") {
                    Scheme::Light
                } else {
                    Scheme::Dark
                },
                Accessibility {
                    increase_contrast: flag(options, "contrast"),
                    reduce_motion: flag(options, "motion"),
                    reduce_transparency: flag(options, "transparency"),
                },
            );
            let wire = if input.get("baseRevision").is_some() {
                let diagnostics = surface.apply_json(&line).map_err(|e| format!("{e:?}"))?;
                controls.reconcile(&surface, &mut sink)?;
                diagnostics
            } else {
                vec![]
            };
            if let Some(request) = input.get("control") {
                let id = id(&surface, request)?;
                match text(request, "type") {
                    "focus" => {
                        controls.focus(&surface, id, &mut sink)?;
                    }
                    "input" => {
                        controls.input(id, text(request, "text").into(), &mut sink)?;
                    }
                    "key" => {
                        controls.numeric_key(
                            &surface,
                            id,
                            text(request, "key"),
                            flag(request, "shift"),
                            &mut sink,
                        )?;
                    }
                    "finish" => {
                        controls.finish(&surface, id, flag(request, "cancel"), &mut sink)?;
                    }
                    "toggle" => controls.toggle(&surface, id, text(request, "key"), &mut sink)?,
                    "select" => controls.select(
                        &surface,
                        id,
                        request["index"].as_u64().ok_or("Missing option")? as usize,
                        false,
                        &mut sink,
                    )?,
                    _ => return Err("Unknown control operation".into()),
                }
            }
            if let Some(request) = input.get("scrub") {
                let position = (
                    request["x"].as_f64().unwrap_or(0.),
                    request["y"].as_f64().unwrap_or(0.),
                );
                match text(request, "type") {
                    "begin" => {
                        scrub = Some(NumericScrub::new(
                            &surface,
                            id(&surface, request)?,
                            position,
                            &tokens,
                        )?)
                    }
                    "sample" => {
                        scrub.as_mut().ok_or("Missing scrub")?.sample(
                            &surface,
                            position,
                            flag(request, "shift"),
                            flag(request, "alt"),
                            &mut controls.bindings,
                            &mut sink,
                        )?;
                    }
                    "finish" => {
                        scrub.as_mut().ok_or("Missing scrub")?.finish(
                            flag(request, "cancel"),
                            &mut controls.bindings,
                            &mut sink,
                        )?;
                    }
                    _ => return Err("Unknown scrub operation".into()),
                }
            }
            let panel = Panel::prepare(
                Arc::new(surface.clone()),
                &tokens,
                &Layout::new(),
                None,
                false,
            );
            let accessibility = panel.accessibility.nodes.iter().map(|(id, node)| json!({
                "id": id.0, "role": format!("{:?}",node.role()), "label":node.label(), "value":node.value(),
                "toggled":node.toggled().map(|v| format!("{v:?}")), "expanded":node.is_expanded(),
                "disabled":node.is_disabled(), "increment":node.supports_action(accesskit::Action::Increment),
                "decrement":node.supports_action(accesskit::Action::Decrement), "step":node.numeric_value_step(),
            })).collect::<Vec<_>>();
            Ok(
                json!({"accepted":true, "revision":surface.revision(), "nodes":surface.nodes(), "diagnostics":panel.diagnostics,
                "wireDiagnostics":wire, "accessibility":accessibility, "focusOrder":pm_surface::kit::focus_order(&surface), "edits":std::mem::take(&mut sink.0)}),
            )
        })();
        println!(
            "{}",
            pm_surface::inspection_json(match result {
                Ok(value) => value,
                Err(error) => json!({"accepted":false,"error":error}),
            })
        );
    }
}
