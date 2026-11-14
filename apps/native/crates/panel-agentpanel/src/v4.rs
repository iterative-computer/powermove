//! Thin worker adapter for panels/agent. No renderer, Store writer,
//! command registration or effect runner.
use crate::{
    Error, Result,
    activity::{Rows, TranscriptIndex},
    input::{self, Key},
    interaction::Submission,
    model::{Effect, Panel, ShellAction},
};
use pm_agents::{Access, ApprovalMode, Provider};
use serde_json::{Value, json};
use std::ops::Range;

/// Native coordinator supplies fresh ids and time, never extension-authored ids.
pub struct Context<'a> {
    pub run: &'a str,
    pub request: &'a str,
    pub new_thread: &'a str,
    pub now: u64,
    pub popover_open: bool,
    /// The choices of a provider the thread switches to.
    pub catalogs: &'a dyn Fn(Provider) -> crate::input::Catalog,
}
/// Keep a matching immutable archive/index together. Refresh after structural
/// archive changes, on a worker; streaming live rows need no archive rebuild.
pub struct Window {
    project: String,
    thread: String,
    archived: Rows,
    index: TranscriptIndex,
}
impl Window {
    pub fn new(panel: &Panel, thread: &str) -> Result<Self> {
        let archived = panel.session(thread)?.conversation.clone();
        let index = TranscriptIndex::build(&archived);
        Ok(Self {
            project: panel.project().into(),
            thread: thread.into(),
            archived,
            index,
        })
    }
    pub fn count(&self, panel: &Panel) -> Result<usize> {
        if panel.project() != self.project || panel.active_id() != self.thread {
            return Err(Error::Stale);
        }
        let session = panel.session(&self.thread)?;
        if session.conversation.len() != self.archived.len() {
            return Err(Error::Stale);
        }
        Ok(self.index.len() + session.trace.rows.len())
    }
    /// Bounded serialization, including when one archived trace has 60k steps.
    pub fn snapshot(&self, panel: &Panel, range: Range<usize>, now: u64) -> Result<Value> {
        let count = self.count(panel)?;
        let session = panel.session(&self.thread)?;
        if range.start > range.end || range.end > count || range.len() > 256 {
            return Err(Error::Invalid("v4 row window"));
        }
        let mut items = vec![];
        for i in range.clone() {
            // An archived step names the run it came from (its trace
            // message), so the view can fold a finished turn's work.
            let mut trace = Value::Null;
            let (id, value) = if i < self.index.len() {
                let row = &self.index.page(i..i + 1)[0];
                if row.step_index.is_some()
                    && let Some(message) = self.archived.get(&row.message)
                {
                    trace = json!({"id":row.message,"durationMs":message.value["durationMs"],
                        "steering":message.value["steering"] == true});
                }
                (
                    format!("history:{}", row.id),
                    self.index
                        .resolve(row, &self.archived)
                        .ok_or(Error::Stale)?,
                )
            } else {
                let row = session
                    .trace
                    .rows
                    .at(i - self.index.len())
                    .ok_or(Error::Stale)?;
                (format!("live:{}", row.id), row.value.as_ref())
            };
            let command_id = value["id"].as_str().unwrap_or_default();
            let chosen = session
                .questions
                .get(command_id)
                .map_or_else(Default::default, |draft| draft.chosen().clone());
            let mut item = json!({"id":id,"commandId":id,"value":value,"chosen":chosen,
                "open":session.disclosures.get(&id).copied().unwrap_or_else(||session.disclosure_open(command_id, false))});
            if !trace.is_null() {
                item["trace"] = trace;
            }
            items.push(item);
        }
        let mut value = panel.inspect(&self.thread)?;
        let text = session.draft.text();
        let selection = session.draft.text_selection();
        value["draft"] = json!({"text":text,"selection":[selection.start,selection.end],"armed":session.draft.armed()});
        value["rows"] = json!({"count":count,"range":[range.start,range.end],"items":items});
        let mut threads = panel.summaries();
        value["threadTitle"] = json!(
            threads
                .iter()
                .find(|s| s.id == self.thread)
                .map(|s| s.title.clone())
                .unwrap_or_default()
        );
        // The thread picker's list, the latest first (v1 ThreadPicker).
        threads.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        value["threads"] = json!(
            threads
                .iter()
                .map(|t| json!({"id":t.id,"title":t.title,"busy":t.busy,"updatedAt":t.updated_at}))
                .collect::<Vec<_>>()
        );
        value["elapsedMs"] = json!(session.elapsed_ms(now));
        // What the run is doing before its first step lands.
        value["activity"] = json!(session.activity);
        value["stopping"] = json!(session.run.as_ref().is_some_and(|r| r.stopping));
        value["showJump"] = json!(session.follow.show_jump);
        let access = match session.settings.access {
            Access::Editor => "editor",
            Access::Computer => "full",
            Access::Project if session.settings.approval == ApprovalMode::Supervised => {
                "supervised"
            }
            Access::Project => "auto",
        };
        value["settings"] = json!({"provider":session.settings.provider.name(),"model":session.settings.model,
            "effort":session.settings.effort,"access":access,"scope":session.settings.scope});
        let choices = input::slash_choices(
            &text,
            &session.catalog,
            session.run.is_some(),
            session.applying,
        );
        value["slash"] = json!({"visible":session.slash.visible(&text,session.applying,&choices),
            "index":session.slash.index,"choices":choices.iter().map(|c|json!({"id":c.id,"value":c.value})).collect::<Vec<_>>()});
        value["catalog"] = json!({"models":session.catalog.models,"providers":session.catalog.providers,"efforts":session.catalog.efforts_for(&session.settings.model)});
        Ok(value)
    }
}
/// The text of a plain key press. The v4 host reports keys, not composed text.
fn typed(key: &Key) -> Option<String> {
    let mut letters = key.name.chars();
    let letter = letters.next().filter(|_| letters.next().is_none())?;
    (!key.meta && !key.control && !key.composing && !letter.is_control()).then(|| {
        if key.shift {
            letter.to_uppercase().collect()
        } else {
            letter.to_string()
        }
    })
}
fn string<'a>(value: &'a Value, name: &str) -> Result<&'a str> {
    value[name]
        .as_str()
        .ok_or(Error::Invalid("v4 string argument"))
}
fn boolean(value: &Value, name: &str) -> Result<bool> {
    value[name]
        .as_bool()
        .ok_or(Error::Invalid("v4 boolean argument"))
}
fn index(value: &Value) -> Result<usize> {
    value
        .as_u64()
        .and_then(|v| usize::try_from(v).ok())
        .ok_or(Error::Invalid("v4 text range"))
}
/// Coordinator-owned publication admission. Call on its serial worker before
/// dispatch; advance the sequence only after a successful model action. HostClient
/// and HostRouter separately enforce realm/generation/document revision admission.
pub fn dispatch_at(
    panel: &mut Panel,
    sequence: u64,
    envelope: &Value,
    context: Context<'_>,
) -> Result<Option<Effect>> {
    let thread = panel.active_id().to_owned();
    let run = panel.session(&thread)?.run.as_ref().map(|r| &r.id);
    let target = envelope
        .get("target")
        .and_then(Value::as_object)
        .ok_or(Error::Stale)?;
    // A streaming run republishes constantly, so input can't wait for the latest
    // publication. Only a whole-text draft replaces what the user last saw.
    let kind = envelope["action"]["type"].as_str().unwrap_or_default();
    let ordered = kind == "draft";
    if sequence == 0
        || ordered && envelope["sequence"].as_u64() != Some(sequence)
        || target.get("project") != Some(&json!(panel.project()))
        || target.get("thread") != Some(&json!(thread))
        || kind != "key" && target.get("run") != Some(&json!(run))
    {
        return Err(Error::Stale);
    }
    dispatch(panel, &thread, &envelope["action"], context)
}
fn control_id(panel: &Panel, thread: &str, id: &str) -> Result<String> {
    let s = panel.session(thread)?;
    if let Some(raw) = id.strip_prefix("live:") {
        return s
            .trace
            .rows
            .get(raw)
            .map(|_| raw.into())
            .ok_or_else(|| Error::UnknownId(id.into()));
    }
    let index = TranscriptIndex::build(&s.conversation);
    let raw = if let Some(scoped) = id.strip_prefix("history:") {
        let row = index
            .get(scoped)
            .ok_or_else(|| Error::UnknownId(id.into()))?;
        index
            .resolve(row, &s.conversation)
            .and_then(|v| v["id"].as_str())
            .ok_or(Error::Stale)?
    } else if s.trace.rows.get(id).is_some() {
        return Ok(id.into());
    } else {
        id
    };
    let matches = index
        .page(0..index.len())
        .iter()
        .filter(|row| {
            index
                .resolve(row, &s.conversation)
                .is_some_and(|v| v["id"] == raw)
        })
        .count();
    if matches != 1 || s.trace.rows.get(raw).is_some() {
        return Err(Error::Invalid("ambiguous archived control"));
    }
    Ok(raw.into())
}
fn retry_turn(
    panel: &mut Panel,
    thread: &str,
    message: Option<&str>,
    run: &str,
    request: &str,
    now: u64,
) -> Result<Effect> {
    let s = panel.session(thread)?;
    if s.run.is_some() {
        return Err(Error::Busy);
    }
    let rows = s.conversation.page(0..s.conversation.len());
    let end = match message {
        Some(id) => rows
            .iter()
            .position(|r| r.id == id)
            .ok_or_else(|| Error::UnknownId(id.into()))?,
        None => rows.len(),
    };
    let user = rows[..end]
        .iter()
        .rev()
        .find(|r| r.value["role"] == "user")
        .ok_or(Error::Invalid("retry has no user turn"))?;
    let text = user.value["text"].as_str().unwrap_or_default().to_owned();
    let files = user.value["attachments"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let request_draft = crate::composer::Draft::from_legacy(&text, &files)
        .or_else(|_| crate::composer::Draft::from_legacy(&text, &[]))?;
    let saved = std::mem::replace(panel.draft_mut(thread)?, request_draft);
    let effect = panel.submit(thread, run, request, now);
    *panel.draft_mut(thread)? = saved;
    effect
}
fn optional_boolean(value: &Value, name: &str) -> Result<bool> {
    match value.get(name) {
        None => Ok(false),
        Some(value) => value.as_bool().ok_or(Error::Invalid("v4 key boolean")),
    }
}
/// Native worker must admit the project/thread/run against its latest publication
/// before calling. Effects are returned intact to the existing host coordinator;
/// no successful provider, consent, archive, shell or file result is fabricated.
pub fn dispatch(
    panel: &mut Panel,
    thread: &str,
    action: &Value,
    context: Context<'_>,
) -> Result<Option<Effect>> {
    if panel.active_id() != thread {
        return Err(Error::Stale);
    }
    panel.session(thread)?;
    let now = context.now;
    let effect = match string(action, "type")? {
        "draft" => {
            let text = string(action, "text")?;
            let selection = action["selection"]
                .as_array()
                .filter(|a| a.len() == 2)
                .ok_or(Error::Invalid("v4 text range"))?;
            let range = index(&selection[0])?..index(&selection[1])?;
            let mut draft = panel.session(thread)?.draft.clone();
            draft.set_text(text, now)?;
            draft.select_text(range)?;
            *panel.draft_mut(thread)? = draft;
            None
        }
        "focus" => {
            panel.focus_composer(thread, boolean(action, "focused")?)?;
            None
        }
        "key" => {
            let key = &action["key"];
            let key = Key {
                name: string(key, "key")?.into(),
                meta: optional_boolean(key, "metaKey")?,
                control: optional_boolean(key, "ctrlKey")?,
                shift: optional_boolean(key, "shiftKey")?,
                alt: optional_boolean(key, "altKey")?,
                repeat: optional_boolean(key, "repeat")?,
                composing: optional_boolean(key, "composing")?,
                code_229: false,
            };
            let outcome = panel.composer_key(
                thread,
                &key,
                context.popover_open,
                Submission {
                    run: context.run,
                    request: context.request,
                },
                now,
            )?;
            if outcome.route.intent == input::Intent::NativeText {
                let (backward, name) = (key.name == "Backspace", key.name.as_str());
                if backward || name == "Delete" {
                    panel.draft_mut(thread)?.delete(backward, key.repeat, now);
                } else if let Some(text) = typed(&key) {
                    panel.draft_mut(thread)?.insert_text(&text, now, true);
                }
            }
            outcome.effect
        }
        "openSettings" => Some(panel.host_settings(thread)?),
        "submit" => Some(panel.submit(thread, context.run, context.request, now)?),
        "stop" => Some(panel.stop(thread)?),
        "newThread" => {
            panel.new_thread(context.new_thread, now)?;
            None
        }
        "switchThread" => {
            panel.switch(string(action, "id")?)?;
            None
        }
        // Asks first: the host's confirmation, then the host archives it.
        "deleteThread" => panel.archive_thread(string(action, "id")?, "", false, now)?,
        "disclose" => {
            let id = string(action, "id")?;
            let open = boolean(action, "open")?;
            if let Some(scoped) = id.strip_prefix("history:") {
                let s = panel.session(thread)?;
                if TranscriptIndex::build(&s.conversation)
                    .get(scoped)
                    .is_none()
                {
                    return Err(Error::UnknownId(id.into()));
                }
                panel
                    .session_mut(thread)?
                    .disclosures
                    .insert(id.into(), open);
            } else {
                let raw = id.strip_prefix("live:").unwrap_or(id);
                panel.disclose(thread, raw, open)?;
                panel
                    .session_mut(thread)?
                    .disclosures
                    .insert(id.into(), open);
            }
            None
        }
        "chooseSlash" => panel.choose_slash(thread, string(action, "id")?, now)?,
        "removeAttachment" => {
            panel
                .draft_mut(thread)?
                .remove_file(string(action, "id")?, now)?;
            None
        }
        "openAttachment" => Some(panel.open_attachment(thread, string(action, "id")?)?),
        "setAccess" => panel.set_access(thread, string(action, "access")?)?,
        "setProvider" => {
            let provider = match string(action, "provider")? {
                "chatgpt" => Provider::Chatgpt,
                "claude" => Provider::Claude,
                "compatible" => Provider::Compatible,
                _ => return Err(Error::Invalid("provider")),
            };
            panel.set_provider(
                thread,
                provider,
                String::new(),
                (context.catalogs)(provider),
            )?;
            None
        }
        "setModel" => {
            panel.set_model(
                thread,
                string(action, "model")?.into(),
                string(action, "effort")?.into(),
            )?;
            None
        }
        "chooseAnswer" => {
            panel.choose_answer(
                thread,
                &control_id(panel, thread, string(action, "step")?)?,
                string(action, "question")?,
                string(action, "option")?,
            )?;
            None
        }
        "typeAnswer" => {
            panel.type_answer(
                thread,
                &control_id(panel, thread, string(action, "step")?)?,
                string(action, "question")?,
                string(action, "text")?.into(),
            )?;
            None
        }
        "answer" => Some(panel.answer_at(
            thread,
            &control_id(panel, thread, string(action, "step")?)?,
            boolean(action, "skip")?,
            now,
        )?),
        "stopTask" => {
            Some(panel.stop_task(thread, &control_id(panel, thread, string(action, "id")?)?)?)
        }
        "keepResult" => {
            panel.keep_result(thread)?;
            None
        }
        "undoResult" => Some(panel.undo_result(thread)?),
        // Try Again on a failed turn: the request before it, sent again
        // (with the files it carried); the composer's own draft stays.
        "retry" => {
            let message = action["id"].as_str().map(|id| {
                let id = id.strip_prefix("history:").unwrap_or(id);
                id.split('/').next().unwrap_or(id).to_owned()
            });
            Some(retry_turn(
                panel,
                thread,
                message.as_deref(),
                context.run,
                context.request,
                now,
            )?)
        }
        "artifact" => {
            Some(panel.artifact(thread, string(action, "path")?, boolean(action, "import")?)?)
        }
        "shell" => Some(panel.host_action(
            thread,
            match string(action, "action")? {
                "open" => ShellAction::Open,
                "close" => ShellAction::Close,
                "pin" => ShellAction::Pin,
                "beginDrag" => ShellAction::BeginDrag,
                "cancelDrag" => ShellAction::CancelDrag,
                "commitDrag" => ShellAction::CommitDrag {
                    dock: action["dock"].as_str().map(str::to_owned),
                },
                _ => return Err(Error::Invalid("v4 shell action")),
            },
        )?),
        _ => return Err(Error::Invalid("unsupported v4 agent action")),
    };
    Ok(effect)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        input::Catalog,
        model::{EffectKind, Update},
    };
    use pm_agents::persistence::Store;
    use std::{collections::BTreeMap, io};
    #[derive(Default)]
    struct Memory(BTreeMap<String, Value>);
    impl Store for Memory {
        fn get(&mut self, key: &str) -> io::Result<Option<Value>> {
            Ok(self.0.get(key).cloned())
        }
        fn set(&mut self, key: &str, value: &Value) -> io::Result<()> {
            self.0.insert(key.into(), value.clone());
            Ok(())
        }
    }
    fn panel() -> Panel {
        Panel::load(&mut Memory::default(), "synthetic-project", "thread", 0).unwrap()
    }
    fn act(p: &mut Panel, v: Value) -> Result<Option<Effect>> {
        dispatch(
            p,
            "thread",
            &v,
            Context {
                run: "run",
                request: "request",
                new_thread: "new",
                now: 10,
                popover_open: true,
                catalogs: &|_| Default::default(),
            },
        )
    }
    fn snapshot(p: &Panel) -> Value {
        let w = Window::new(p, "thread").unwrap();
        let count = w.index.len() + p.session("thread").unwrap().trace.rows.len();
        w.snapshot(p, 0..count.min(256), 1000).unwrap()
    }
    fn draft(p: &mut Panel) {
        act(
            p,
            json!({"type":"draft","text":"Authored 日本🙂\n","selection":[14,14]}),
        )
        .unwrap();
    }
    #[test]
    fn exact_draft_and_utf16_selection_are_published_without_attachment_payloads() {
        let mut p = panel();
        draft(&mut p);
        let v = snapshot(&p);
        assert_eq!(v["draft"]["text"], "Authored 日本🙂\n");
        assert_eq!(v["draft"]["selection"], json!([14, 14]));
        assert_eq!(v["sendable"], true);
        assert_eq!(v["rows"]["count"], 0);
    }
    #[test]
    fn malformed_selection_is_atomic_and_split_surrogate_is_refused() {
        let mut p = panel();
        draft(&mut p);
        let before = snapshot(&p);
        for selection in [
            json!([0, 99]),
            json!([12, 12]),
            json!([2, 1]),
            json!([0]),
            json!([0, -1]),
        ] {
            assert!(
                act(
                    &mut p,
                    json!({"type":"draft","text":"Authored 日本🙂\n","selection":selection})
                )
                .is_err()
            );
            assert_eq!(snapshot(&p), before);
        }
    }
    #[test]
    fn ime_and_repeat_never_submit_and_shift_return_inserts_a_newline() {
        let mut p = panel();
        draft(&mut p);
        for key in [
            json!({"key":"Enter","composing":true}),
            json!({"key":"Enter","repeat":true}),
        ] {
            assert!(
                act(&mut p, json!({"type":"key","key":key}))
                    .unwrap()
                    .is_none()
            );
            assert!(p.session("thread").unwrap().run.is_none());
        }
        act(
            &mut p,
            json!({"type":"key","key":{"key":"Enter","shiftKey":true}}),
        )
        .unwrap();
        assert_eq!(snapshot(&p)["draft"]["text"], "Authored 日本🙂\n\n");
    }
    #[test]
    fn submit_stop_and_rejected_stop_keep_model_ownership_and_allow_explicit_retry() {
        let mut p = panel();
        draft(&mut p);
        let e = act(&mut p, json!({"type":"submit"})).unwrap().unwrap();
        assert!(matches!(e.kind, EffectKind::Send { .. }));
        assert_eq!(e.target.thread, "thread");
        let stop = act(&mut p, json!({"type":"stop"})).unwrap().unwrap();
        assert!(snapshot(&p)["stopping"] == true);
        assert!(act(&mut p, json!({"type":"stop"})).is_err());
        p.acknowledge(stop.id, false).unwrap();
        assert!(snapshot(&p)["stopping"] == false);
        assert!(act(&mut p, json!({"type":"stop"})).unwrap().is_some());
    }
    #[test]
    fn full_access_returns_consent_instead_of_granting_authority() {
        let mut p = panel();
        let e = act(&mut p, json!({"type":"setAccess","access":"full"}))
            .unwrap()
            .unwrap();
        assert!(matches!(e.kind, EffectKind::ComputerConsent));
        assert_eq!(snapshot(&p)["settings"]["access"], "editor");
        p.acknowledge(e.id, false).unwrap();
        assert_eq!(snapshot(&p)["settings"]["access"], "editor");
    }
    #[test]
    fn access_modes_and_catalog_validation_use_real_model_methods() {
        let mut p = panel();
        for access in ["editor", "supervised", "auto"] {
            act(&mut p, json!({"type":"setAccess","access":access})).unwrap();
            assert_eq!(snapshot(&p)["settings"]["access"], access);
        }
        p.set_catalog(
            "thread",
            Catalog {
                models: vec![("model".into(), "GPT-6.1".into())],
                efforts: vec!["high".into()],
                ..Default::default()
            },
        )
        .unwrap();
        act(
            &mut p,
            json!({"type":"setModel","model":"model","effort":"high"}),
        )
        .unwrap();
        assert!(
            act(
                &mut p,
                json!({"type":"setModel","model":"unknown","effort":"high"})
            )
            .is_err()
        );
        assert_eq!(snapshot(&p)["settings"]["model"], "model");
    }
    #[test]
    fn pin_drag_cancel_and_close_are_shell_effects_without_a_local_layout_mirror() {
        let mut p = panel();
        for action in ["pin", "beginDrag", "cancelDrag", "close", "open"] {
            let e = act(&mut p, json!({"type":"shell","action":action}))
                .unwrap()
                .unwrap();
            assert!(matches!(e.kind, EffectKind::Shell(_)));
            assert_eq!(e.target.project, "synthetic-project");
        }
        assert!(act(&mut p, json!({"type":"shell","action":"invented"})).is_err());
    }
    #[test]
    fn escape_in_the_popover_composer_closes_it_unless_the_shortcuts_menu_takes_it() {
        // The reported bug: Escape in the composer never closed the popover.
        let mut p = panel();
        let e = act(&mut p, json!({"type":"key","key":{"key":"Escape"}}))
            .unwrap()
            .unwrap();
        assert!(matches!(e.kind, EffectKind::Shell(ShellAction::Close)));
        // An open shortcuts menu takes the first Escape and stays put.
        act(&mut p, json!({"type":"focus","focused":true})).unwrap();
        act(&mut p, json!({"type":"draft","text":"/","selection":[1,1]})).unwrap();
        assert_eq!(snapshot(&p)["slash"]["visible"], true);
        assert!(
            act(&mut p, json!({"type":"key","key":{"key":"Escape"}}))
                .unwrap()
                .is_none()
        );
        assert_eq!(snapshot(&p)["slash"]["visible"], false);
        let e = act(&mut p, json!({"type":"key","key":{"key":"Escape"}}))
            .unwrap()
            .unwrap();
        assert!(matches!(e.kind, EffectKind::Shell(ShellAction::Close)));
    }
    #[test]
    fn try_again_sends_the_failed_turns_request_and_keeps_the_composer_draft() {
        let mut p = panel();
        draft(&mut p);
        act(&mut p, json!({"type":"submit"})).unwrap();
        p.receive(
            "thread",
            "run",
            1,
            Update::Complete {
                text: r#"{"type":"error","error":{"message":"Overloaded"}}"#.into(),
                error: true,
                cancelled: false,
            },
            20,
        )
        .unwrap();
        let v = snapshot(&p);
        let failed = v["rows"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["value"]["error"] == true)
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        p.draft_mut("thread")
            .unwrap()
            .insert_text("next idea", 30, false);
        let ctx = Context {
            run: "run-2",
            request: "request-2",
            new_thread: "new",
            now: 40,
            popover_open: true,
            catalogs: &|_| Default::default(),
        };
        let e = dispatch(&mut p, "thread", &json!({"type":"retry","id":failed}), ctx)
            .unwrap()
            .unwrap();
        match e.kind {
            EffectKind::Send { text, .. } => assert_eq!(text, "Authored 日本🙂\n"),
            other => panic!("{other:?}"),
        }
        assert_eq!(snapshot(&p)["draft"]["text"], "next idea");
        // A running turn has nothing to try again.
        assert!(act(&mut p, json!({"type":"retry"})).is_err());
    }
    #[test]
    fn the_snapshot_names_threads_and_each_archived_steps_run() {
        let mut p = panel();
        draft(&mut p);
        act(&mut p, json!({"type":"submit"})).unwrap();
        p.receive(
            "thread",
            "run",
            1,
            Update::Trace(json!({"kind":"tool-start","itemId":"call","toolName":"get_project_state","label":"Get project state"})),
            12,
        )
        .unwrap();
        p.receive(
            "thread",
            "run",
            2,
            Update::Complete {
                text: "Done.".into(),
                error: false,
                cancelled: false,
            },
            5_012,
        )
        .unwrap();
        let v = snapshot(&p);
        assert_eq!(v["threads"][0]["id"], "thread");
        let step = v["rows"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["value"]["kind"] == "tool")
            .unwrap();
        assert!(step["trace"]["id"].is_string());
        assert_eq!(step["trace"]["steering"], false);
        assert!(step["trace"]["durationMs"].as_u64().is_some());
    }
    #[test]
    fn switched_thread_refuses_old_actions_and_old_windows() {
        let mut p = panel();
        let w = Window::new(&p, "thread").unwrap();
        act(&mut p, json!({"type":"newThread"})).unwrap();
        // A blank UI thread can be reused by the model. Make a second nonblank thread explicitly.
        let active = p.active_id().to_owned();
        p.draft_mut(&active).unwrap().insert_text("keep", 1, false);
        p.new_thread("other", 2).unwrap();
        assert!(matches!(
            act(&mut p, json!({"type":"stop"})),
            Err(Error::Stale)
        ));
        assert!(w.snapshot(&p, 0..0, 0).is_err());
    }
    #[test]
    fn archived_sixty_thousand_steps_page_in_the_middle_without_serializing_the_trace() {
        let mut store = Memory::default();
        let mut p = Panel::load(&mut store, "synthetic-project", "thread", 0).unwrap();
        p.save(&mut store, 0).unwrap();
        store.0.get_mut("agentThreads.synthetic-project").unwrap()["threads"][0]["conversation"] = json!([{"role":"trace","steps":(0..60_000).map(|i|json!({"id":format!("t-{i}"),"kind":"tool","label":"Make the headline","status":"done"})).collect::<Vec<_>>()}]);
        let p = Panel::load(&mut store, "synthetic-project", "thread", 0).unwrap();
        let w = Window::new(&p, "thread").unwrap();
        let v = w.snapshot(&p, 30_000..30_003, 0).unwrap();
        assert_eq!(v["rows"]["count"], 60_000);
        assert_eq!(v["rows"]["items"].as_array().unwrap().len(), 3);
        assert_eq!(v["rows"]["items"][0]["value"]["id"], "t-30000");
        assert!(v.to_string().len() < 10_000);
        assert!(w.snapshot(&p, 0..257, 0).is_err());
        assert!(w.snapshot(&p, 60_000..60_001, 0).is_err());
    }
    #[test]
    fn live_streaming_preserves_row_id_and_previous_archive_snapshot() {
        let mut p = panel();
        draft(&mut p);
        act(&mut p, json!({"type":"submit"})).unwrap();
        let w = Window::new(&p, "thread").unwrap();
        for (seq, text) in [(1, "Authored "), (2, "日本🙂\n")] {
            p.receive(
                "thread",
                "run",
                seq,
                Update::Trace(json!({"kind":"answer","text":text})),
                seq + 10,
            )
            .unwrap();
        }
        let v = w.snapshot(&p, 0..2, 100).unwrap();
        assert_eq!(v["rows"]["items"][1]["id"], "live:text-1");
        assert_eq!(v["rows"]["items"][1]["value"]["text"], "Authored 日本🙂\n");
    }
    #[test]
    fn explicit_question_choice_does_not_submit_and_secret_drafts_do_not_publish() {
        let mut p = panel();
        draft(&mut p);
        act(&mut p, json!({"type":"submit"})).unwrap();
        p.receive("thread","run",1,Update::Trace(json!({"kind":"question","itemId":"q","requestId":"request","transport":"reply","blocking":true,
            "questions":[{"id":"first","question":"Choose","secret":true,"allowOther":true,"options":[{"label":"A"}]}]})),11).unwrap();
        act(&mut p,json!({"type":"typeAnswer","step":"q","question":"first","text":"synthetic-private-answer"})).unwrap();
        assert!(
            !snapshot(&p)
                .to_string()
                .contains("synthetic-private-answer")
        );
        let e = act(&mut p, json!({"type":"answer","step":"q","skip":false}))
            .unwrap()
            .unwrap();
        assert!(matches!(e.kind, EffectKind::Answer { .. }));
    }
    #[test]
    fn unknown_action_is_refused_without_consuming_draft() {
        let mut p = panel();
        draft(&mut p);
        let before = snapshot(&p);
        assert!(act(&mut p, json!({"type":"invented"})).is_err());
        assert_eq!(snapshot(&p), before);
    }
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    use pm_agents::persistence::Store;
    use std::io;
    struct Empty;
    impl Store for Empty {
        fn get(&mut self, _: &str) -> io::Result<Option<Value>> {
            Ok(None)
        }
        fn set(&mut self, _: &str, _: &Value) -> io::Result<()> {
            Ok(())
        }
    }
    fn context() -> Context<'static> {
        Context {
            run: "run",
            request: "request",
            new_thread: "new",
            now: 0,
            popover_open: true,
            catalogs: &|_| Default::default(),
        }
    }
    #[test]
    fn admission_rejects_stale_project_thread_run_and_sequence_before_mutation() {
        let mut panel = Panel::load(&mut Empty, "p", "t", 0).unwrap();
        let good = json!({"sequence":1,"target":{"project":"p","thread":"t","run":null},"action":{"type":"draft","text":"Keep","selection":[4,4]}});
        for field in ["project", "thread", "run"] {
            let mut bad = good.clone();
            bad["target"][field] = json!("foreign");
            assert!(matches!(
                dispatch_at(&mut panel, 1, &bad, context()),
                Err(Error::Stale)
            ));
        }
        assert!(matches!(
            dispatch_at(&mut panel, 2, &good, context()),
            Err(Error::Stale)
        ));
        assert!(!panel.session("t").unwrap().draft.sendable());
        dispatch_at(&mut panel, 1, &good, context()).unwrap();
        assert!(panel.session("t").unwrap().draft.sendable());
    }
    #[test]
    fn typing_outlives_a_publication_and_a_run_boundary_but_not_a_thread() {
        let mut panel = Panel::load(&mut Empty, "p", "t", 0).unwrap();
        let key = |thread: &str| {
            json!({"sequence":1,"target":{"project":"p","thread":thread,"run":"finished"},
                "action":{"type":"key","key":{"key":"a"}}})
        };
        dispatch_at(&mut panel, 9, &key("t"), context()).unwrap();
        assert!(panel.session("t").unwrap().draft.sendable());
        assert!(matches!(
            dispatch_at(&mut panel, 9, &key("other"), context()),
            Err(Error::Stale)
        ));
    }
    #[test]
    fn malformed_ime_flags_cannot_turn_into_a_submit() {
        let mut panel = Panel::load(&mut Empty, "p", "t", 0).unwrap();
        panel.draft_mut("t").unwrap().insert_text("Keep", 0, false);
        assert!(
            dispatch(
                &mut panel,
                "t",
                &json!({"type":"key","key":{"key":"Enter","composing":"true"}}),
                context()
            )
            .is_err()
        );
        assert!(panel.session("t").unwrap().run.is_none());
    }
}

#[cfg(test)]
mod scoped_tests {
    use super::*;
    use crate::composer::Attachment;
    use pm_agents::persistence::Store;
    use std::{collections::BTreeMap, io};
    #[derive(Default)]
    struct Memory(BTreeMap<String, Value>);
    impl Store for Memory {
        fn get(&mut self, key: &str) -> io::Result<Option<Value>> {
            Ok(self.0.get(key).cloned())
        }
        fn set(&mut self, key: &str, value: &Value) -> io::Result<()> {
            self.0.insert(key.into(), value.clone());
            Ok(())
        }
    }
    #[test]
    fn scoped_archive_disclosures_are_independent_and_ambiguous_effect_targets_are_refused() {
        let mut store = Memory::default();
        let mut panel = Panel::load(&mut store, "p", "t", 0).unwrap();
        panel.save(&mut store, 0).unwrap();
        store.0.get_mut("agentThreads.p").unwrap()["threads"][0]["conversation"] = json!([{"role":"trace","steps":[{"id":"duplicate","kind":"tool"},{"id":"duplicate","kind":"tool"}]}]);
        let mut panel = Panel::load(&mut store, "p", "t", 0).unwrap();
        let window = Window::new(&panel, "t").unwrap();
        let before = window.snapshot(&panel, 0..2, 0).unwrap();
        let id = before["rows"]["items"][0]["commandId"].as_str().unwrap();
        dispatch(
            &mut panel,
            "t",
            &json!({"type":"disclose","id":id,"open":true}),
            Context {
                run: "r",
                request: "q",
                new_thread: "n",
                now: 0,
                popover_open: true,
                catalogs: &|_| Default::default(),
            },
        )
        .unwrap();
        let after = window.snapshot(&panel, 0..2, 0).unwrap();
        assert_eq!(after["rows"]["items"][0]["open"], true);
        assert_eq!(after["rows"]["items"][1]["open"], false);
        assert!(matches!(
            control_id(&panel, "t", id),
            Err(Error::Invalid("ambiguous archived control"))
        ));
    }
    #[test]
    fn text_only_draft_projection_keeps_authored_text_and_omits_attachment_bodies() {
        let mut panel = Panel::load(&mut Memory::default(), "p", "t", 0).unwrap();
        panel
            .draft_mut("t")
            .unwrap()
            .insert_text("Authored 日本🙂\n", 0, false);
        panel.add_attachments("t",vec![Attachment::from_record(json!({"id":"file","name":"synthetic.txt","type":"text","text":"synthetic-attachment-body"})).unwrap()],0).unwrap();
        assert_eq!(
            panel.session("t").unwrap().draft.text(),
            "Authored 日本🙂\n"
        );
        let value = Window::new(&panel, "t")
            .unwrap()
            .snapshot(&panel, 0..0, 0)
            .unwrap();
        assert!(!value.to_string().contains("synthetic-attachment-body"));
        assert_eq!(
            panel
                .session("t")
                .unwrap()
                .draft
                .attachment("file")
                .unwrap()
                .record()["text"],
            "synthetic-attachment-body"
        );
    }
}
