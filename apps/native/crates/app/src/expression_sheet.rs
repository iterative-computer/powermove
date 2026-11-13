//! Add Expression… / Edit Expression… from a Properties row: the editor the
//! shipped app opened, on the app's sheet recipe. Apply writes the source to
//! every channel of the row as one Undo step; a cleared field removes it.
use crate::dialogs::{DialogId, Field, NativePiece, Surface, WindowId};
use crate::session::Controller;
use crate::sheets::{OpenSheet, from_surface};
use gpui::Context;
use pm_kernel::commands::properties::{PropertyCommand, expression_diagnostic};
use pm_kernel::{Call, Caller, Reply, Request, WriteBody};
use std::rc::Rc;

/// What a new expression starts from (`ChannelRow.svelte`).
pub use pm_builtins::STARTING_EXPRESSION as STARTING;
/// The names an expression can use, under the field (the shipped hint).
const HINT: &str = "t: layer seconds · T: composition seconds · value: the keyframed value · wiggle(f,a) random(s) linear(x,x0,x1,y0,y1) ease(…) loop(d,x) param(\"name\")";
const VALID: &str = "Expression is valid";

/// What the sheet edits, as the Properties panel names it.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub document: u64,
    /// `Layer · Property`, or the count of layers for a multiple selection.
    pub subject: String,
    pub source: Option<String>,
    /// (layer, channel) of every property the row reaches.
    pub targets: Vec<(String, String)>,
}
impl Target {
    pub fn read(input: &serde_json::Value) -> Option<Self> {
        let targets: Vec<(String, String)> = input["targets"]
            .as_array()?
            .iter()
            .filter_map(|t| Some((t[0].as_str()?.to_owned(), t[1].as_str()?.to_owned())))
            .collect();
        let property = input["property"].as_str()?;
        let subject = match input["layers"].as_u64() {
            Some(n) if n > 1 => format!("{n} layers · {property}"),
            _ => format!("{} · {property}", input["layer"].as_str()?),
        };
        (!targets.is_empty()).then(|| Self {
            document: input["document"].as_u64().unwrap_or(0),
            subject,
            source: input["source"].as_str().map(str::to_owned),
            targets,
        })
    }
    pub fn surface(&self, id: DialogId, owner: WindowId) -> Surface {
        let text = self.source.clone().unwrap_or_else(|| STARTING.into());
        let mut field = Field::new(
            "expression",
            "Expression",
            NativePiece::TextField,
            vec![text],
        );
        field.description = Some(HINT.into());
        let mut sheet = Surface::sheet(id, owner, "Expression", vec![field], "Apply");
        sheet.line = Some(self.subject.clone());
        sheet.expression_editor = true;
        sheet
    }
    /// Apply with `typed` in the field: the edit, nothing when the field
    /// says what the property already has, or why the source can't be used.
    pub fn apply(&self, typed: &str, revision: Option<u64>) -> Result<Option<Call>, String> {
        let source = typed.trim();
        if source == self.source.as_deref().unwrap_or("") {
            return Ok(None);
        }
        if !source.is_empty()
            && let Some(problem) = expression_diagnostic(source)
        {
            return Err(problem);
        }
        let expression = (!source.is_empty()).then(|| source.to_owned());
        let label = if expression.is_some() {
            "Set Expression"
        } else {
            "Remove Expression"
        };
        Ok(Some(Call::Transact {
            label: Some(label.into()),
            expected_revision: revision,
            body: WriteBody::Commands(
                self.targets
                    .iter()
                    .map(|(layer, channel)| {
                        PropertyCommand::SetExpression {
                            layer_id: layer.clone(),
                            channel: channel.clone(),
                            expression: expression.clone(),
                        }
                        .into()
                    })
                    .collect(),
            ),
        }))
    }
}
/// The line under the field while typing: the problem, or that it is valid.
pub fn status(typed: &str) -> (String, bool) {
    let source = typed.trim();
    // An empty editor removes the expression: nothing to say about it.
    if source.is_empty() {
        return (String::new(), false);
    }
    match expression_diagnostic(source) {
        Some(problem) => (problem, true),
        None => (VALID.into(), false),
    }
}
impl Controller {
    pub fn expression_sheet(&mut self, input: &serde_json::Value, cx: &mut Context<Self>) {
        let (Some(target), Some(session)) = (Target::read(input), self.session.clone()) else {
            return;
        };
        if self
            .published
            .as_ref()
            .is_none_or(|p| p.snapshot.doc_id != target.document)
        {
            return;
        }
        let surface = target.surface(
            DialogId(format!("expression:{}", session.project)),
            WindowId(session.project.clone()),
        );
        let tokens = self.host.borrow().tokens.clone();
        let open = Rc::new(OpenSheet::present(
            from_surface(&surface, &tokens),
            &self.host.borrow().window,
            surface.first_selected.as_deref(),
        ));
        let (actions, kernel) = (open.actions.clone(), self.kernel.clone());
        let (muted, danger) = (tokens.colors.muted, tokens.colors.danger);
        cx.spawn(async move |this, cx| {
            while let Ok(action) = actions.recv().await {
                match action.as_str() {
                    "cancel" => break,
                    "changed:expression" => {
                        let (text, bad) = status(&open.text("expression"));
                        open.describe("expression", &text, if bad { danger } else { muted });
                        open.enable("apply", !bad);
                    }
                    "apply" => {
                        let revision = this
                            .read_with(cx, |v, _| v.published.as_ref().map(|p| p.revision()))
                            .ok()
                            .flatten();
                        let call = match target.apply(&open.text("expression"), revision) {
                            Ok(Some(call)) => call,
                            Ok(None) => break,
                            Err(problem) => {
                                open.describe("expression", &problem, danger);
                                open.enable("apply", false);
                                continue;
                            }
                        };
                        let pending =
                            kernel.call(&session.project, Request::new(Caller::native(), call));
                        let result = pending.await;
                        let _ = this.update(cx, |v, cx| {
                            match result {
                                Some(Ok(ack)) => {
                                    if let Reply::Throw(error) = ack.reply {
                                        v.message = error.to_string();
                                    }
                                }
                                other => v.edit_failed(other.and_then(Result::err)),
                            }
                            cx.notify();
                        });
                        break;
                    }
                    _ => {}
                }
            }
            open.close();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn target(source: Option<&str>) -> Target {
        Target::read(&json!({
            "document": 1, "layer": "Title", "layers": 1, "property": "Position",
            "source": source, "targets": [["a", "position.x"], ["a", "position.y"]],
        }))
        .unwrap()
    }
    #[test]
    fn a_new_expression_starts_from_the_shipped_text_and_names_its_property() {
        let sheet = target(None).surface(DialogId("d".into()), WindowId("w".into()));
        assert_eq!(sheet.title, "Expression");
        assert_eq!(sheet.line.as_deref(), Some("Title · Position"));
        assert_eq!(sheet.fields[0].values, ["value + wiggle(2, 20)"]);
        let edit = target(Some("t * 10")).surface(DialogId("d".into()), WindowId("w".into()));
        assert_eq!(edit.fields[0].values, ["t * 10"]);
    }
    #[test]
    fn apply_refuses_what_the_parser_refuses_and_skips_no_change() {
        assert!(target(None).apply("value +", None).is_err());
        assert!(status("value +").1);
        assert_eq!(status(STARTING), (VALID.to_owned(), false));
        assert!(target(Some("t")).apply(" t ", None).unwrap().is_none());
        assert!(target(None).apply("", None).unwrap().is_none());
        let Some(Call::Transact { label, body, .. }) = target(Some("t")).apply("", None).unwrap()
        else {
            panic!("clearing removes the expression")
        };
        assert_eq!(label.as_deref(), Some("Remove Expression"));
        let WriteBody::Commands(commands) = body else {
            panic!()
        };
        assert_eq!(commands.len(), 2, "both axes of the row");
    }
}
