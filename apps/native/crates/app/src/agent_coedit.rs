//! Explicit turn undo, with the user's changes retained after confirmation.
use crate::{
    alert::{Alert, Button},
    session::Controller,
};
use gpui::Context;
use pm_kernel::{Caller, HistoryCall, KernelError, KernelHandle, Reply, Request, Writer};
use pm_shell::AgentStatus;

#[derive(Debug, PartialEq)]
enum UndoResult {
    Applied {
        changed: bool,
    },
    Confirm,
    Failed {
        title: &'static str,
        message: String,
    },
}

fn undo_request(document: u64, writers: &[Writer], label: &str, keep_mine: bool) -> Request {
    let label = format!("Undo Agent · {label}");
    let call = match writers {
        [writer] => HistoryCall::UndoWriter {
            writer: *writer,
            keep_mine,
            label,
        },
        _ => HistoryCall::UndoWriters {
            writers: writers.to_vec(),
            keep_mine,
            label,
        },
    };
    Request::new(Caller::native(), call).with_document(document)
}

fn undo_result(reply: Option<Reply>) -> UndoResult {
    match reply {
        Some(Reply::Applied { changed, .. }) => UndoResult::Applied { changed },
        Some(Reply::Workspace(_)) => UndoResult::Confirm,
        Some(Reply::Throw(error)) => {
            let title = match &error {
                KernelError::Guard(message) if message.starts_with("Finish the active edit") => {
                    "Finish the active edit first"
                }
                _ => "Couldn't undo",
            };
            UndoResult::Failed {
                title,
                message: error.to_string(),
            }
        }
        _ => UndoResult::Failed {
            title: "Couldn't undo",
            message: "Couldn't undo the agent's project edits.".into(),
        },
    }
}

async fn undo_turn(
    kernel: &KernelHandle,
    project: &str,
    document: u64,
    writers: &[Writer],
    label: &str,
    keep_mine: bool,
) -> UndoResult {
    let ack = kernel
        .call(project, undo_request(document, writers, label, keep_mine))
        .await;
    undo_result(ack.and_then(Result::ok).map(|ack| ack.reply))
}

impl Controller {
    pub(crate) fn undo_agent_turn(&mut self, cx: &mut Context<Self>) {
        let project = self.session.as_ref().map(|s| s.project.clone());
        let document = project
            .as_deref()
            .and_then(|id| self.kernel.published(id))
            .map(|view| view.snapshot.doc_id);
        let Some(version) = self.host.borrow_mut().begin_agent_undo(document) else {
            return;
        };
        let Some((project, document, turn)) = project
            .zip(document)
            .and_then(|(id, doc)| self.mcp.latest_turn(&id, doc).map(|turn| (id, doc, turn)))
        else {
            self.finish_agent_undo(
                document,
                version,
                "Nothing to undo",
                "This agent turn has no project edits to undo.".into(),
                false,
                cx,
            );
            return;
        };
        let (kernel, host) = (self.kernel.clone(), self.mcp.clone());
        let writers: Vec<_> = turn.writers.iter().map(|(writer, _)| *writer).collect();
        let label = turn
            .writers
            .last()
            .map_or("Project edits", |(_, label)| label.as_str())
            .to_owned();
        cx.spawn(async move |this, cx| {
            let is_pending = |s: &Controller| {
                s.session
                    .as_ref()
                    .is_some_and(|session| session.project == project)
                    && s.kernel
                        .published(&project)
                        .is_some_and(|view| view.snapshot.doc_id == document)
                    && s.host.borrow().agent_undo_pending(Some(document), version)
            };
            let mut keep_mine = false;
            let mut result = undo_turn(&kernel, &project, document, &writers, &label, false).await;
            if result == UndoResult::Confirm {
                let answer = this
                    .update(cx, |s, _| {
                        if !is_pending(s) {
                            return None;
                        }
                        Some(
                            Alert::new(
                                "You've changed things the agent edited.",
                                "Undo the rest of this agent turn and keep your changes?",
                                vec![
                                    Button::new("Keep Mine and Undo Rest", true).default(),
                                    Button::new("Cancel", false).cancel(),
                                ],
                            )
                            .sheet(&s.host.borrow().window),
                        )
                    })
                    .ok()
                    .flatten();
                let Some(answer) = answer else {
                    return;
                };
                keep_mine = answer.recv().await.ok().flatten() == Some(true);
                if !keep_mine {
                    let _ = this.update(cx, |s, cx| {
                        if is_pending(s) {
                            s.finish_agent_undo(
                                Some(document),
                                version,
                                "Undo cancelled",
                                "Undo cancelled.".into(),
                                true,
                                cx,
                            );
                        }
                    });
                    return;
                }
                if this.update(cx, |s, _| is_pending(s)).ok() != Some(true) {
                    return;
                }
                result = undo_turn(&kernel, &project, document, &writers, &label, true).await;
            }
            let (title, message, retry) = match result {
                UndoResult::Applied { changed } => {
                    for writer in writers {
                        host.forget_turn_writer(&project, document, writer);
                    }
                    if changed && keep_mine {
                        (
                            AgentStatus::UNDONE,
                            "Undid the agent's project edits and kept your changes.".into(),
                            false,
                        )
                    } else if changed {
                        (
                            AgentStatus::UNDONE,
                            "Undid the agent's project edits.".into(),
                            false,
                        )
                    } else if keep_mine {
                        (
                            "Kept your changes",
                            "No other project edits could be undone.".into(),
                            false,
                        )
                    } else {
                        (
                            "Nothing to undo",
                            "This agent turn has no project edits to undo.".into(),
                            false,
                        )
                    }
                }
                UndoResult::Failed { title, message } => (title, message, true),
                UndoResult::Confirm => (
                    "Couldn't undo",
                    "The agent's project edits could not be undone.".into(),
                    true,
                ),
            };
            let _ = this.update(cx, |s, cx| {
                if is_pending(s) {
                    s.finish_agent_undo(Some(document), version, title, message, retry, cx);
                }
            });
        })
        .detach();
    }

    fn finish_agent_undo(
        &mut self,
        document: Option<u64>,
        version: u64,
        title: &'static str,
        message: String,
        retry: bool,
        cx: &mut Context<Self>,
    ) {
        if self
            .host
            .borrow_mut()
            .finish_agent_undo(document, version, title, retry)
        {
            self.message = message;
            cx.notify();
        }
    }
}

use gpui::{WindowKind, prelude::*, *};
use pm_shell::text::{TextEvent, TextField, TextKind};
struct Steer {
    field: Entity<TextField>,
    _events: Subscription,
    sending: bool,
    tokens: pm_design::Tokens,
}
impl Render for Steer {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .p(px(6.))
            .bg(rgb(self.tokens.colors.floating))
            .child(self.field.clone())
    }
}
impl Controller {
    pub(crate) fn open_agent_steer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(runtime) = self.builtins.runtime.clone() else {
            return;
        };
        let Some(thread) = runtime.agent_current_thread() else {
            return;
        };
        if !runtime.agent_status().0 {
            return;
        }
        let tokens = self.host.borrow().tokens.clone();
        let bounds = window.bounds();
        let parent = window.window_handle();
        let native = self.host.borrow().window.clone();
        let responder = native.firstResponder();
        let origin = bounds.origin
            + point(
                px((f32::from(bounds.size.width) - 330.).max(0.)),
                px(tokens.metrics.toolbar + tokens.metrics.gap),
            );
        let owner = cx.entity().downgrade();
        cx.defer(move |cx| {
            let mut options = WindowOptions::new()
                .kind(WindowKind::PopUp)
                .window_bounds(Some(WindowBounds::Windowed(Bounds::new(
                    origin,
                    size(px(320.), px(44.)),
                ))))
                .titlebar(None)
                .macos_window_background(gpui::MacosWindowBackground::Transparent);
            options.parent = Some(parent);
            let _ = cx.open_window(options, move |window, cx| {
                pm_shell::popover_frame(window, &tokens);
                cx.new(|cx| {
                    let field = cx.new(|cx| {
                        let mut field = TextField::new(
                            TextKind::Composer,
                            "Tell the agent",
                            "",
                            tokens.clone(),
                            window,
                            cx,
                        );
                        field.set_bare(true);
                        field.set_placeholder("Tell the agent…");
                        field.begin_editing(cx);
                        field
                    });
                    let events = cx.subscribe_in(
                        &field,
                        window,
                        move |steer: &mut Steer, _, event, window, cx| match event {
                            TextEvent::Submitted(text)
                                if !text.trim().is_empty() && !steer.sending =>
                            {
                                steer.sending = true;
                                let (runtime, thread, message, owner) = (
                                    runtime.clone(),
                                    thread.clone(),
                                    text.trim().to_owned(),
                                    owner.clone(),
                                );
                                cx.spawn_in(window, async move |this, cx| {
                                    let result = cx
                                        .background_executor()
                                        .spawn(async move {
                                            let result =
                                                runtime.agent_quick_prompt(&thread, &message);
                                            if result.is_ok() {
                                                pm_mcp::coedit::record_steer(runtime.project_id());
                                            }
                                            result
                                        })
                                        .await;
                                    let succeeded = result.is_ok();
                                    let _ = owner.update(cx, |s, cx| {
                                        s.message = match result {
                                            Ok(true) => "Queued for next step".into(),
                                            Ok(false) => "Direction submitted".into(),
                                            Err(error) => error,
                                        };
                                        cx.notify();
                                    });
                                    let _ = this.update_in(cx, |steer, window, _| {
                                        steer.sending = false;
                                        if succeeded {
                                            window.remove_window();
                                        }
                                    });
                                })
                                .detach();
                            }
                            TextEvent::Key("escape") => window.remove_window(),
                            _ => {}
                        },
                    );
                    cx.on_release(move |_, _| {
                        native.makeFirstResponder(responder.as_deref());
                    })
                    .detach();
                    Steer {
                        field,
                        _events: events,
                        sending: false,
                        tokens: tokens.clone(),
                    }
                })
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{Caller, KernelError, Reply, Request, UndoResult, Writer, undo_request, undo_result};
    use crate::{
        mcp::{Host, Tools},
        parity_tests::{Desk, project},
    };
    use pm_mcp::transport::ToolConnection;
    use serde_json::json;
    use std::sync::Arc;

    #[test]
    fn recorded_turns_undo_agent_and_mcp_writers_independently() {
        let profile = tempfile::tempdir().unwrap();
        // Keep a shared layer in the stack so inserts have stable identities
        // across writers, including the first write of the test.
        let desk = Desk::new(project(vec![json!({
            "id":"base", "name":"Base", "type":"shape", "from":0, "dur":2, "on":true,
            "d":{"shape":"rect", "w":10, "h":10}
        })]));
        let kernel = desk.kernel();
        let project = &desk.session.project;
        let document = desk.published().snapshot.doc_id;
        let host = Host::new(kernel.clone(), profile.path().to_owned());
        let _changes = host.attach_window(1);
        host.bind_window(1, Some(desk.session.clone()));
        let tools = Tools::new(host.clone(), profile.path().join("exports"));
        tools.automation.lock().unwrap().agent_grants =
            Some(Arc::new(|_| Some(("builtin-run".into(), "project".into()))));
        let start = |agent| {
            let mut client = tools.connect();
            if agent {
                client.bind_agent_run("builtin-test");
            }
            client
                .dispatch("start_session", &json!({"prompt":"Test turn"}))
                .unwrap();
            client
        };
        let add = |client: &mut crate::mcp::Connection, id: &str| {
            client
                .dispatch(
                    "apply_commands",
                    &json!({
                        "commands": [{"type": "add_layer", "id": id, "layerType": "solid"}]
                    }),
                )
                .unwrap();
        };
        let finish = |client: &mut crate::mcp::Connection| {
            client
                .dispatch("finish_session", &json!({"commit":true}))
                .unwrap();
        };
        let undo = |writers: &[Writer], label: &str| {
            let reply = kernel
                .call_blocking(project, undo_request(document, writers, label, false))
                .unwrap()
                .reply;
            undo_result(Some(reply))
        };

        host.begin_builtin_turn(project, "builtin-run");
        let mut builtin = start(true);
        assert!(host.latest_turn(project, document).is_none());
        add(&mut builtin, "first");
        add(&mut builtin, "second");
        finish(&mut builtin);
        let mut verification = start(true);
        finish(&mut verification);
        let mut external = start(false);
        add(&mut external, "external");
        finish(&mut external);
        let mut repair = start(true);
        add(&mut repair, "repaired");
        finish(&mut repair);
        host.finish_builtin_turn(project, document);

        let builtin = host.latest_turn(project, document).unwrap();
        assert_eq!(builtin.writers.len(), 2);
        let writers: Vec<_> = builtin.writers.iter().map(|(writer, _)| *writer).collect();
        assert_eq!(
            undo(&writers, "Built-in edits"),
            UndoResult::Applied { changed: true }
        );
        for writer in &writers {
            host.forget_turn_writer(project, document, *writer);
        }
        assert_eq!(
            undo(&writers, "Built-in edits"),
            UndoResult::Applied { changed: false }
        );
        assert!(host.latest_turn(project, document).is_none());
        let layers: Vec<_> = desk.layers().into_iter().map(|layer| layer.id).collect();
        assert_eq!(layers, ["external", "base"]);

        let mut external = start(false);
        add(&mut external, "external-again");
        finish(&mut external);
        let external_turn = host.latest_turn(project, document).unwrap();
        let [(writer, label)] = external_turn.writers.as_slice() else {
            panic!("expected one external writer");
        };
        assert_eq!(
            undo(&[*writer], label),
            UndoResult::Applied { changed: true }
        );
        host.forget_turn_writer(project, document, *writer);
        assert!(host.latest_turn(project, document).is_none());
        assert!(host.latest_turn(project, document + 1).is_none());
        let layers: Vec<_> = desk.layers().into_iter().map(|layer| layer.id).collect();
        assert_eq!(layers, ["external", "base"]);
    }

    #[test]
    fn undoing_multiple_writers_checks_conflicts_before_changing_anything() {
        use pm_kernel::{Call, Command, Origin, WriteBody, commands::comps::CompPatch};

        let desk = Desk::new(project(vec![json!({
            "id":"a", "name":"Shape", "type":"shape", "from":0, "dur":2, "on":true,
            "p":{"opacity":{"v":100, "kf":[]}}, "d":{"shape":"rect", "w":10, "h":10}
        })]));
        let kernel = desk.kernel();
        let project = &desk.session.project;
        let document = desk.published().snapshot.doc_id;
        let first = Caller::realm(1_200, 1, Origin::Agent);
        let repair = Caller::realm(1_201, 1, Origin::Mcp);
        let rename = |caller, name: &str| {
            let request = Request::new(
                caller,
                Call::Transact {
                    label: Some("Rename layer".into()),
                    body: WriteBody::Commands(vec![Command::SetLayerName {
                        layer_id: "a".into(),
                        name: name.into(),
                    }]),
                    expected_revision: None,
                },
            );
            assert!(matches!(
                kernel.call_blocking(project, request).unwrap().reply,
                Reply::Applied { changed: true, .. }
            ));
        };
        rename(first, "Agent rename");
        rename(Caller::native(), "Keep my name");
        let set_background = Request::new(
            repair,
            Call::Transact {
                label: Some("Change background".into()),
                body: WriteBody::Commands(vec![
                    pm_kernel::CompCommand::Set {
                        patch: Box::new(CompPatch {
                            background: Some("#123456".into()),
                            ..Default::default()
                        }),
                    }
                    .into(),
                ]),
                expected_revision: None,
            },
        );
        assert!(matches!(
            kernel.call_blocking(project, set_background).unwrap().reply,
            Reply::Applied { changed: true, .. }
        ));
        let revision = desk.published().revision();
        let before = desk.document();
        let undo = |keep_mine| {
            let writers = [first.writer, repair.writer];
            let request = undo_request(document, &writers, "Agent edits", keep_mine);
            undo_result(Some(kernel.call_blocking(project, request).unwrap().reply))
        };

        assert_eq!(undo(false), UndoResult::Confirm);
        assert_eq!(desk.published().revision(), revision);
        assert_eq!(desk.document(), before);
        assert_eq!(undo(true), UndoResult::Applied { changed: true });
        let model = pm_model::canonical::realize_project(desk.document());
        let after: serde_json::Value =
            serde_json::from_str(&pm_model::json::to_canonical_string(&model)).unwrap();
        assert_eq!(after["layers"][0]["name"], "Keep my name");
        assert_eq!(after["bg"], "#000000");
    }

    #[test]
    fn active_edit_refusal_has_actionable_feedback() {
        assert_eq!(
            undo_result(Some(Reply::Throw(KernelError::Guard(
                "Finish the active edit before Undo Agent Turn".into()
            )))),
            UndoResult::Failed {
                title: "Finish the active edit first",
                message: "Finish the active edit before Undo Agent Turn".into(),
            }
        );
    }
}
