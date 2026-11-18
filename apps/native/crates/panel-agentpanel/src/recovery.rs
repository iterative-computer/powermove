//! Durable request intent; process handles and computer grants never survive restart.
use super::*;
use pm_agents::launch::Request;

fn summary(record: &Value) -> String {
    let archived = record["conversation"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| row["role"] == "trace" && row["runId"] == record["lastRunId"])
        .flat_map(|row| row["steps"].as_array().into_iter().flatten());
    let checkpoints = archived
        .chain(record["nativeLiveTrace"].as_array().into_iter().flatten())
        .collect::<Vec<_>>();
    let edits = checkpoints
        .into_iter()
        .filter(|s| s["kind"] == "tool" && s["status"] == "done")
        .filter(|s| {
            matches!(
                s["toolName"].as_str(),
                Some(
                    "apply_commands"
                        | "edit_layer_strips"
                        | "edit_video"
                        | "publish_extensions"
                        | "import_media_to_timeline"
                        | "panel_build"
                )
            )
        })
        .filter(|s| s["output"]["changed"] != false && s["output"]["isError"] != true)
        .filter_map(|s| {
            s["output"]["summary"]
                .as_str()
                .or(s["label"].as_str())
                .or(s["toolName"].as_str())
        })
        .rev()
        .take(5)
        .map(|s| pm_agents::trace::truncate(s, 180).to_owned())
        .collect::<Vec<_>>();
    if edits.is_empty() {
        record["nativeRunRequest"]["resumeSummary"].as_str().unwrap_or("Applied changes are not fully known from the saved checkpoint. Inspect the current project before continuing; existing edits are preserved.").to_owned()
    } else {
        format!(
            "Last saved completed edit operations: {}. Later or unfinished edits may also be present; inspect the current project before continuing.",
            edits.join("; ")
        )
    }
}
pub(super) fn notice(record: &Value) -> Value {
    let saved = &record["nativeRunRequest"];
    let original = match (
        serde_json::from_value::<Request>(saved["request"].clone()),
        serde_json::from_value::<Settings>(saved["settings"].clone()),
    ) {
        (Ok(r), Ok(s)) => (r.run == record["lastRunId"].as_str().unwrap_or("")
            && r.provider == s.provider
            && r.model.as_deref().unwrap_or("") == s.model
            && r.effort.as_deref().unwrap_or("") == s.effort
            && r.access == s.access
            && r.approval == s.approval)
            .then_some(s),
        _ => None,
    };
    let available = original.is_some();
    json!({"runId":record["lastRunId"].as_str().unwrap_or("unavailable"),"originalSettings":original,
        "appliedSummary":summary(record),"requestAvailable":available,
        "requiresComputerConsent":available && saved["request"]["access"] == "computer",
        "unavailableReason":if available { Value::Null } else { json!("The original request is unavailable. Dismiss this notice and start a new task.") }})
}
impl Panel {
    /// Called with the resolved launch request before ProviderRuns durably saves/launches it.
    pub fn host_admitted_request(
        &mut self,
        thread: &str,
        run: &str,
        request: &Request,
        settings: &Settings,
    ) -> Result<()> {
        let s = self.sessions.get_mut(thread).ok_or(Error::Stale)?;
        if s.run.as_ref().map(|r| r.id.as_str()) != Some(run) || request.run != run {
            return Err(Error::Stale);
        }
        if s.resume_request.is_none() {
            let mut original = request.clone();
            original.computer_consent = false;
            let mut resolved = settings.clone();
            resolved.provider = request.provider;
            resolved.access = request.access;
            resolved.approval = request.approval;
            resolved.model = request.model.clone().unwrap_or_default();
            resolved.effort = request.effort.clone().unwrap_or_default();
            s.admitted_request = Some(json!({"request":original,"settings":resolved}));
        }
        Ok(())
    }
    pub fn recovery_provider(&self, thread: &str) -> Result<Provider> {
        let s = self.session(thread)?;
        let saved = s.admitted_request.as_ref().ok_or(Error::Stale)?;
        let request: Request = serde_json::from_value(saved["request"].clone())
            .map_err(|e| Error::Host(e.to_string()))?;
        Ok(request.provider)
    }
    pub fn interrupted_snapshot(&self, thread: &str) -> Result<Value> {
        let s = self.session(thread)?;
        let Some(mut notice) = s.recovery.clone() else {
            return Ok(Value::Null);
        };
        let idle = s.run.is_none() && !s.applying && !s.archiving;
        notice["canDismiss"] = json!(idle);
        notice["canResume"] = json!(
            idle && notice["requestAvailable"] == true
                && (notice["requiresComputerConsent"] != true || s.computer_consent)
        );
        Ok(notice)
    }
    pub fn recovery_launch(&self, thread: &str, run: &str) -> Result<Option<Value>> {
        let s = self.session(thread)?;
        if s.run.as_ref().map(|r| r.id.as_str()) != Some(run) {
            return Err(Error::Stale);
        }
        Ok(s.resume_request.clone())
    }
    pub fn dismiss_interrupted(&mut self, thread: &str, interrupted: &str, now: u64) -> Result<()> {
        self.no_apply()?;
        let s = self.sessions.get_mut(thread).ok_or(Error::Stale)?;
        if s.run.is_some() || s.archiving {
            return Err(Error::Busy);
        }
        if s.recovery.as_ref().and_then(|v| v["runId"].as_str()) != Some(interrupted) {
            return Err(Error::Stale);
        }
        // Keep interrupted work in history, without carrying stale live tools forward.
        s.archive_trace(now, false)?;
        s.recovery = None;
        s.admitted_request = None;
        s.resume_request = None;
        self.archive.thread_mut(thread).ok_or(Error::Stale)?["lastRunStatus"] = json!("dismissed");
        Ok(())
    }
    /// The host has not launched a provider yet; failed durability restores admission.
    pub fn save_recovery_admission(
        &mut self,
        store: &mut impl Store,
        before: Panel,
        now: u64,
    ) -> Result<()> {
        if let Err(error) = self.save(store, now) {
            let save_error = self.save_error.clone();
            *self = before;
            self.save_error = save_error;
            return Err(error);
        }
        Ok(())
    }
    pub fn resume_interrupted(
        &mut self,
        thread: &str,
        interrupted: &str,
        run: &str,
        request: &str,
        now: u64,
    ) -> Result<Effect> {
        // Failed admission must preserve the notice, original request and composer draft.
        let mut next = self.clone();
        let effect = next.resume_interrupted_inner(thread, interrupted, run, request, now)?;
        *self = next;
        Ok(effect)
    }
    fn resume_interrupted_inner(
        &mut self,
        thread: &str,
        interrupted: &str,
        run: &str,
        request: &str,
        now: u64,
    ) -> Result<Effect> {
        self.no_apply()?;
        let s = self.sessions.get_mut(thread).ok_or(Error::Stale)?;
        if s.run.is_some() || s.archiving {
            return Err(Error::Busy);
        }
        let notice = s.recovery.as_ref().ok_or(Error::Stale)?;
        if notice["runId"] != interrupted || run == interrupted {
            return Err(Error::Stale);
        }
        if notice["requestAvailable"] != true {
            return Err(Error::Invalid("Original interrupted request unavailable"));
        }
        let mut saved = s.admitted_request.clone().ok_or(Error::Stale)?;
        let original: Request = serde_json::from_value(saved["request"].clone())
            .map_err(|e| Error::Host(e.to_string()))?;
        if original.access == Access::Computer && !s.computer_consent {
            return Err(Error::Invalid(
                "Approve Full Access again before resuming this run",
            ));
        }
        let settings: Settings = serde_json::from_value(saved["settings"].clone())
            .map_err(|e| Error::Host(e.to_string()))?;
        saved["request"]["run"] = json!(run);
        saved["request"]["computer_consent"] = json!(false);
        saved["resumeSummary"] = notice["appliedSummary"].clone();
        let draft = s.draft.clone();
        s.draft = Draft::from_legacy("Resume interrupted run", &[])?;
        s.settings = settings;
        s.settings_dirty = true;
        s.access_dirty = true;
        let effect = self.submit(thread, run, request, now)?;
        let s = self.sessions.get_mut(thread).expect("validated");
        s.draft = draft;
        s.admitted_request = Some(saved.clone());
        s.resume_request = Some(saved);
        Ok(effect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Memory(std::collections::BTreeMap<String, Value>);
    impl Store for Memory {
        fn get(&mut self, k: &str) -> io::Result<Option<Value>> {
            Ok(self.0.get(k).cloned())
        }
        fn set(&mut self, k: &str, v: &Value) -> io::Result<()> {
            self.0.insert(k.into(), v.clone());
            Ok(())
        }
    }
    fn interrupted(computer: bool) -> (Memory, Panel) {
        let mut store = Memory::default();
        let mut p = Panel::load(&mut store, "p", "t", 0).unwrap();
        if computer {
            let e = p.set_access("t", "full").unwrap().unwrap();
            p.acknowledge(e.id, true).unwrap();
        }
        p.draft_mut("t")
            .unwrap()
            .insert_text("original task", 1, false);
        p.submit("t", "old", "original-request", 1).unwrap();
        let settings = p.session("t").unwrap().settings.clone();
        let req = Request {
            run: "old".into(),
            prompt: "ORIGINAL including context".into(),
            provider: Provider::Claude,
            model: Some("original-model".into()),
            effort: Some("high".into()),
            access: settings.access,
            approval: ApprovalMode::Supervised,
            computer_consent: computer,
            workspace: "/synthetic".into(),
            images: vec!["original.png".into()],
            files: vec![json!({"name":"original.txt"})],
            role: Some("original-role".into()),
        };
        p.host_admitted_request("t", "old", &req, &settings)
            .unwrap();
        p.receive("t", "old", 1, Update::Trace(json!({"kind":"tool-start","itemId":"edit","toolName":"apply_commands","label":"Edit title"})), 1).unwrap();
        p.receive("t", "old", 2, Update::Trace(json!({"kind":"tool-end","itemId":"edit","output":{"summary":"Added title","changed":true}})), 1).unwrap();
        p.save(&mut store, 1).unwrap();
        let restored = Panel::load(&mut store, "p", "blank", 2).unwrap();
        (store, restored)
    }
    #[test]
    fn saved_work_summary_keeps_same_run_steering_checkpoints_and_excludes_failures() {
        let record = json!({"lastRunId":"old","conversation":[
            {"role":"trace","runId":"other","steps":[{"kind":"tool","status":"done","toolName":"apply_commands","label":"unrelated"}]},
            {"role":"trace","runId":"old","steps":[{"kind":"tool","status":"done","toolName":"apply_commands","output":{"summary":"Added title","changed":true}}]}
        ],"nativeLiveTrace":[
            {"kind":"tool","status":"error","toolName":"apply_commands","label":"failed"},
            {"kind":"tool","status":"done","toolName":"apply_commands","label":"no edits","output":{"changed":false}},
            {"kind":"tool","status":"done","toolName":"get_project_state","label":"read only"}
        ]});
        let text = summary(&record);
        assert!(text.contains("Added title"));
        for absent in ["unrelated", "failed", "no edits", "read only"] {
            assert!(!text.contains(absent));
        }
        let legacy = notice(&json!({"lastRunId":"legacy"}));
        assert_eq!(legacy["requestAvailable"], false);
        assert_eq!(legacy["originalSettings"], Value::Null);
        let (_, p) = interrupted(false);
        let mut record =
            json!({"lastRunId":"old","nativeRunRequest":p.sessions["t"].admitted_request});
        record["nativeRunRequest"]["settings"]["privateExtra"] = json!("unpublished metadata");
        let public = notice(&record);
        assert_eq!(public["requestAvailable"], true);
        assert!(public["originalSettings"].get("privateExtra").is_none());
    }
    #[test]
    fn restored_notice_requires_explicit_fresh_run_and_retains_original_request() {
        let (mut store, mut p) = interrupted(false);
        let snapshot = crate::v4::Window::new(&p, "t")
            .unwrap()
            .snapshot(&p, 0..0, 3)
            .unwrap();
        assert_eq!(snapshot["interruptedRun"]["runId"], "old");
        assert!(
            snapshot["interruptedRun"]["appliedSummary"]
                .as_str()
                .unwrap()
                .contains("Added title")
        );
        assert!(p.session("t").unwrap().run.is_none());
        assert!(!snapshot.to_string().contains("ORIGINAL including context"));
        p.sessions.get_mut("t").unwrap().settings.model = "changed picker".into();
        p.draft_mut("t").unwrap().insert_text("unsent", 3, false);
        assert!(p.resume_interrupted("t", "stale", "new", "r", 3).is_err());
        let e = crate::v4::dispatch_at(&mut p, 1, &json!({"target":{"project":"p","thread":"t","run":null},"sequence":1,"action":{"type":"resumeInterrupted","runId":"old"}}), crate::v4::Context {run:"new",request:"r",new_thread:"unused",now:4,popover_open:false,catalogs:&|_| Default::default()}).unwrap().unwrap();
        assert_eq!(e.target.run.as_deref(), Some("new"));
        assert!(p.session("t").unwrap().trace.rows.is_empty());
        assert!(
            p.session("t")
                .unwrap()
                .conversation
                .values()
                .any(|row| row["role"] == "trace" && row["runId"] == "old")
        );
        let saved = p.recovery_launch("t", "new").unwrap().unwrap();
        assert_eq!(saved["request"]["prompt"], "ORIGINAL including context");
        assert_eq!(saved["settings"]["provider"], "claude");
        assert_eq!(saved["settings"]["effort"], "high");
        assert_eq!(saved["request"]["model"], "original-model");
        assert_eq!(saved["request"]["images"][0], "original.png");
        assert_eq!(saved["request"]["role"], "original-role");
        assert_eq!(p.session("t").unwrap().draft.legacy().0, "unsent");
        p.receive(
            "t",
            "new",
            1,
            Update::Complete {
                text: "launch failed".into(),
                error: true,
                cancelled: false,
            },
            4,
        )
        .unwrap();
        assert_eq!(p.interrupted_snapshot("t").unwrap()["runId"], "new");
        p.save(&mut store, 4).unwrap();
        let again = Panel::load(&mut store, "p", "blank", 5).unwrap();
        assert_eq!(again.interrupted_snapshot("t").unwrap()["runId"], "new");
        assert!(again.session("t").unwrap().run.is_none());
    }
    #[test]
    fn recovery_store_failure_restores_notice_and_draft_without_a_fictitious_run() {
        struct Refuse;
        impl Store for Refuse {
            fn get(&mut self, _: &str) -> io::Result<Option<Value>> {
                Ok(None)
            }
            fn set(&mut self, _: &str, _: &Value) -> io::Result<()> {
                Err(io::Error::other("synthetic storage failure"))
            }
        }
        let (_, mut p) = interrupted(false);
        p.draft_mut("t").unwrap().insert_text("unsent", 3, false);
        let before = p.clone();
        p.resume_interrupted("t", "old", "fresh", "fresh-request", 4)
            .unwrap();
        assert!(p.save_recovery_admission(&mut Refuse, before, 4).is_err());
        assert!(p.session("t").unwrap().run.is_none());
        assert_eq!(p.interrupted_snapshot("t").unwrap()["runId"], "old");
        assert_eq!(p.session("t").unwrap().draft.legacy().0, "unsent");
        assert!(p.save_error.is_some());
        assert_eq!(
            p.resume_interrupted("t", "old", "retry", "retry-request", 5)
                .unwrap()
                .target
                .run
                .as_deref(),
            Some("retry")
        );
    }
    #[test]
    fn restored_computer_request_requires_fresh_consent_and_dismiss_is_durable() {
        let (mut store, mut p) = interrupted(true);
        assert_eq!(p.interrupted_snapshot("t").unwrap()["canResume"], false);
        assert_eq!(
            p.sessions["t"].admitted_request.as_ref().unwrap()["request"]["computer_consent"],
            false
        );
        assert!(p.resume_interrupted("t", "old", "new", "r", 3).is_err());
        assert!(p.recovery_launch("t", "old").is_err());
        p.sessions.get_mut("t").unwrap().settings.provider = Provider::Compatible;
        assert_eq!(p.recovery_provider("t").unwrap(), Provider::Claude);
        let ask = p.set_access("t", "full").unwrap().unwrap();
        p.acknowledge(ask.id, true).unwrap();
        assert_eq!(p.interrupted_snapshot("t").unwrap()["canResume"], true);
        p.dismiss_interrupted("t", "old", 4).unwrap();
        p.save(&mut store, 4).unwrap();
        let again = Panel::load(&mut store, "p", "blank", 5).unwrap();
        assert_eq!(again.interrupted_snapshot("t").unwrap(), Value::Null);
    }
}
