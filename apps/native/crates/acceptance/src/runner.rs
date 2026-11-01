use crate::{
    catalog::{Catalog, LegacySpec},
    scenarios,
};
use serde::Serialize;
use std::{collections::BTreeSet, path::PathBuf};

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Pass,
    Fail,
    BlockedBy,
}
#[derive(Clone, Debug, Serialize)]
pub struct CaseResult {
    pub id: String,
    pub workflow: String,
    pub kind: String,
    pub status: Status,
    pub reason: String,
    pub blocked_by: Option<String>,
    pub needs_macos: bool,
}
#[derive(Debug, Serialize)]
pub struct WorkflowResult {
    pub id: String,
    pub title: String,
    pub status: Status,
    pub reason: String,
    pub passed: usize,
    pub failed: usize,
    pub blocked: usize,
}
#[derive(Debug, Serialize)]
pub struct SpecMapping {
    #[serde(flatten)]
    pub legacy: LegacySpec,
    pub acceptance_ids: Vec<String>,
    pub supporting_probe_ids: Vec<String>,
    /// No spec is retired by this harness. A group-level mapping alone cannot
    /// prove every assertion in a mixed/product e2e was actually replaced.
    pub replacement_status: Status,
    pub reason: String,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub format: String,
    pub version: u32,
    pub platform: String,
    pub scope: String,
    pub cases: Vec<CaseResult>,
    pub workflows: Vec<WorkflowResult>,
    pub specs: Vec<SpecMapping>,
}
#[derive(Clone, Debug, Default)]
pub struct Filter {
    pub workflow: Option<String>,
    pub case: Option<String>,
}
impl Filter {
    fn accepts(&self, id: &str, workflow: &str) -> bool {
        self.workflow.as_deref().is_none_or(|w| w == workflow)
            && self.case.as_deref().is_none_or(|c| c == id)
    }
}
fn blocker(workflow: &str) -> (&'static str, &'static str, bool) {
    match workflow {
        "W004" | "W007" | "W008" => (
            "project-lifecycle-integration",
            "Kernel slices run; complete corpus/media closure, recovery and shell lifecycle acceptance remains",
            true,
        ),
        "W019" | "W020" | "W021" | "W022" | "W023" | "W031" => (
            "media-service",
            "Durable import/relink/source-preview MCP adapter and decoded-media oracles are pending",
            true,
        ),
        "W024" | "W025" | "W026" | "W027" | "W028" | "W029" => (
            "agent-runtime",
            "Provider/thread/approval/composer integration is pending; use synthetic providers only",
            true,
        ),
        "W030" | "W033" => (
            "mcp-integration",
            "Implemented MCP cases run; remaining tool bodies, complete external entry/agent/render integration are pending",
            false,
        ),
        "W034" => (
            "owner-scope",
            "Existing browser/serve workflows have an explicit out-of-scope native boundary in W1; retain evidence",
            false,
        ),
        "W035" | "W036" => (
            "store-service",
            "Store/account lifecycle adapters and explicit-action synthetic service fixtures are pending",
            false,
        ),
        "W037" | "W038" | "W039" | "W040" => (
            "compat-runtime",
            "Production V8 v1–3 logic/PM/services/recorder canvas and WebKit remote DOM are pending; no extension is excluded",
            true,
        ),
        "W041" => (
            "transcription-service",
            "Speech-model download/resume/transcription adapters are pending",
            false,
        ),
        "W057" | "W058" => (
            "video-domain-adapter",
            "Typed timing probe is supporting evidence; edit_video and decoded start/end frame oracles remain pending",
            true,
        ),
        "W059" | "W068" => (
            "audio-service",
            "Real audio routing, levels and speaker synchronization need the macOS engine",
            true,
        ),
        "W069" | "W070" | "W071" => (
            "export-service",
            "MCP export jobs, codec/media adapters, cancellation and frame-rate output verification are pending",
            true,
        ),
        "W072" => (
            "owner-scope",
            "Owner removed standalone web-animation export from cutover requirements (2026-10-29); retain traceability, never claim pass",
            false,
        ),
        "W076" | "W077" | "W078" => (
            "engine-macos",
            "needs macOS: real Metal pixels, shader/text/colour standards oracles and physical performance evidence",
            true,
        ),
        "W079" => (
            "transport-integration",
            "needs macOS: J/K/L engine transport plus shipped command fallthrough/decline integration",
            true,
        ),
        "W080" | "W081" | "W082" | "W083" => (
            "pro-edit-domain",
            "Required cutover addition: ripple/roll/slide/three-point/persistent A/V domain and UI integration is pending",
            false,
        ),
        _ => (
            "shell-integration",
            "needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration",
            true,
        ),
    }
}
fn blocked(id: &str, workflow: &str, kind: &str) -> CaseResult {
    let (backing, reason, mac) = blocker(workflow);
    CaseResult {
        id: id.into(),
        workflow: workflow.into(),
        kind: kind.into(),
        status: Status::BlockedBy,
        reason: reason.into(),
        blocked_by: Some(backing.into()),
        needs_macos: mac,
    }
}
pub fn execute(
    id: &str,
    workflow: &str,
    kind: &str,
    title: &str,
    check: impl FnOnce() -> Result<(), String>,
) -> CaseResult {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(check));
    let (status, reason) = match result {
        Ok(Ok(())) => (Status::Pass, title.into()),
        Ok(Err(e)) => (Status::Fail, format!("{title}: {e}")),
        Err(payload) => {
            let message = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("assertion panicked");
            (Status::Fail, format!("{title}: {message}"))
        }
    };
    CaseResult {
        id: id.into(),
        workflow: workflow.into(),
        kind: kind.into(),
        status,
        reason,
        blocked_by: None,
        needs_macos: false,
    }
}
const KEY_CONTEXTS: &[&str] = &[
    "normal",
    "held",
    "extra-modifier",
    "input",
    "contenteditable",
    "read-only-selection",
    "button",
    "prior-decline",
    "menu-once",
];
pub fn run(catalog: &Catalog, filter: &Filter, corpus: &[PathBuf]) -> Result<Report, String> {
    catalog.validate()?;
    let scenarios = scenarios::all();
    validate_scenarios(catalog)?;
    let mut known = BTreeSet::new();
    let mut cases = vec![];
    for w in &catalog.workflows {
        let id = format!("{}.contract", w.id);
        known.insert(id.clone());
        if filter.accepts(&id, &w.id) {
            cases.push(blocked(&id, &w.id, "workflow-contract"));
        }
        for c in &w.cases {
            known.insert(c.id.clone());
            if !filter.accepts(&c.id, &w.id) {
                continue;
            }
            if let Some(s) = scenarios.iter().find(|s| s.id == c.id) {
                cases.push(execute(s.id, s.workflow, "original", s.title, s.run));
            } else {
                cases.push(blocked(&c.id, &w.id, "original"));
            }
        }
    }
    for k in &catalog.bindings {
        for context in KEY_CONTEXTS {
            let id = format!("{}.{context}", k.id);
            known.insert(id.clone());
            if filter.accepts(&id, &k.workflow) {
                let mut result = blocked(&id, &k.workflow, "binding");
                result.blocked_by = Some("real-keyboard-dispatch".into());
                result.needs_macos = true;
                result.reason = format!(
                    "needs macOS: dispatch {} → {} {} (priority {}, fields={}, repeat={}, loose={}) under {context}; exact args and resulting state, focused input ownership and menu dedup need the real shell",
                    k.chord, k.command, k.args, k.priority, k.in_fields, k.repeat, k.loose
                );
                cases.push(result);
            }
        }
    }
    for binding in &catalog.bindings {
        for context in crate::keyboard::CONTEXTS {
            let id = format!("{}.route-{context}", binding.id);
            known.insert(id.clone());
            if filter.accepts(&id, &binding.workflow) {
                cases.push(execute(
                    &id,
                    &binding.workflow,
                    "routing-probe",
                    "Real shell resolver admission, exact arguments and decline ordering",
                    || crate::keyboard::check(binding, context),
                ));
            }
        }
    }
    for s in &scenarios {
        known.insert(s.id.into());
        if !s.id.starts_with('A') && !s.id.starts_with('B') && filter.accepts(s.id, s.workflow) {
            let mut result = execute(s.id, s.workflow, "supporting-probe", s.title, s.run);
            if s.id.ends_with("-hook") {
                result.kind = "test-double-probe".into();
                result.needs_macos = true;
                result.reason.push_str("; needs macOS: replace double with real engine/surface adapter and verify pixels/input");
            }
            cases.push(result);
        }
    }
    for (name, _) in crate::fixture::CORPUS {
        let id = format!("W007.public-{name}");
        known.insert(id.clone());
        if filter.accepts(&id, "W007") {
            cases.push(execute(
                &id,
                "W007",
                "corpus-copy",
                "Public synthetic container copy roundtrips without changing its source",
                || crate::fixture::corpus_fixture(name),
            ));
        }
    }
    for (i, path) in corpus.iter().enumerate() {
        let id = format!("W007.corpus-{i:04}");
        known.insert(id.clone());
        if filter.accepts(&id, "W007") {
            cases.push(execute(
                &id,
                "W007",
                "corpus-copy",
                "Read-only corpus copy opens and saves/reopens exactly",
                || scenarios::corpus_copy(path, i),
            ));
        }
    }
    if filter
        .workflow
        .as_ref()
        .is_some_and(|id| !catalog.workflows.iter().any(|w| &w.id == id))
    {
        return Err("unknown workflow filter".into());
    }
    if filter.case.as_ref().is_some_and(|id| !known.contains(id)) {
        return Err("unknown acceptance case filter".into());
    }
    if cases.is_empty() {
        return Err("filters selected no acceptance cases".into());
    }
    let mut workflows = vec![];
    for w in &catalog.workflows {
        let results = cases
            .iter()
            .filter(|c| c.workflow == w.id)
            .collect::<Vec<_>>();
        if results.is_empty() {
            continue;
        }
        let passed = results.iter().filter(|c| c.status == Status::Pass).count();
        let failed = results.iter().filter(|c| c.status == Status::Fail).count();
        let blocked = results.len() - passed - failed;
        let status = if failed > 0 {
            Status::Fail
        } else if blocked > 0 {
            Status::BlockedBy
        } else {
            Status::Pass
        };
        let reason = if filter.case.is_some() {
            "Filtered case result only; not a full workflow acceptance verdict".into()
        } else {
            format!(
                "{passed} passed, {failed} failed, {blocked} blocked; {}",
                blocker(&w.id).1
            )
        };
        workflows.push(WorkflowResult {
            id: w.id.clone(),
            title: w.title.clone(),
            status,
            reason,
            passed,
            failed,
            blocked,
        });
    }
    let specs = catalog.specs.iter().map(|spec| {
        let mut acceptance_ids=vec![];
        let mut supporting_probe_ids=vec![];
        for w in catalog.workflows.iter().filter(|w|spec.workflows.contains(&w.id)) {
            acceptance_ids.push(format!("{}.contract",w.id));
            let basename=spec.file.rsplit('/').next().unwrap();
            for c in w.cases.iter().filter(|c|c.source.contains(basename)) {acceptance_ids.push(c.id.clone());}
            for s in scenarios.iter().filter(|s|s.workflow==w.id) {supporting_probe_ids.push(s.id.into());}
            if w.id=="W007" {supporting_probe_ids.extend(crate::fixture::CORPUS.iter().map(|(id,_)|format!("W007.public-{id}")));}
        }
        acceptance_ids.sort();acceptance_ids.dedup();
        supporting_probe_ids.sort();supporting_probe_ids.dedup();
        SpecMapping{legacy:spec.clone(),acceptance_ids,supporting_probe_ids,replacement_status:Status::BlockedBy,reason:if spec.classification=="plumbing-only" {"Retirement candidate only; keep frozen evidence until replacement safety/resource gates pass".into()}else{"Planned requirement links; supporting probes do not replace full assertions. Complete integration validation remains open".into()}}
    }).collect();
    Ok(Report {
        format: "powermove.acceptance".into(),
        version: 1,
        platform: std::env::consts::OS.into(),
        scope: if filter.case.is_some() || filter.workflow.is_some() {
            "filtered".into()
        } else {
            "full-inventory".into()
        },
        cases,
        workflows,
        specs,
    })
}
impl Report {
    pub fn counts(&self) -> (usize, usize, usize) {
        let pass = self
            .cases
            .iter()
            .filter(|c| c.status == Status::Pass)
            .count();
        let fail = self
            .cases
            .iter()
            .filter(|c| c.status == Status::Fail)
            .count();
        (pass, fail, self.cases.len() - pass - fail)
    }
    pub fn exit_code(&self, strict: bool) -> u8 {
        let (_, fail, blocked) = self.counts();
        if fail > 0 {
            1
        } else if strict && blocked > 0 {
            2
        } else {
            0
        }
    }
    pub fn markdown(&self) -> String {
        let (pass, fail, blocked) = self.counts();
        let mut text = format!(
            "Acceptance format v{}; platform `{}`; scope `{}`.\n\nCases: **{pass} pass / {fail} fail / {blocked} blocked-by**.\n\n| Workflow | Verdict | Pass | Fail | Blocked | Reason |\n|---|---|---:|---:|---:|---|\n",
            self.version, self.platform, self.scope
        );
        for w in &self.workflows {
            text.push_str(&format!(
                "| {} — {} | {} | {} | {} | {} | {} |\n",
                w.id,
                w.title,
                status_name(&w.status),
                w.passed,
                w.failed,
                w.blocked,
                w.reason.replace('|', "\\|")
            ));
        }
        text
    }
    pub fn e2e_csv(&self) -> String {
        fn quote(s: &str) -> String {
            format!("\"{}\"", s.replace('"', "\"\""))
        }
        let mut out =
            "spec_file,in_370_roster,classification,acceptance_ids,supporting_probe_ids,replacement_status,reason\n"
                .to_owned();
        for s in &self.specs {
            out.push_str(&format!(
                "{},{},{},{},{},{},{}\n",
                quote(&s.legacy.file),
                s.legacy.in_370_roster,
                quote(&s.legacy.classification),
                quote(&s.acceptance_ids.join(";")),
                quote(&s.supporting_probe_ids.join(";")),
                status_name(&s.replacement_status),
                quote(&s.reason)
            ));
        }
        out
    }
}
fn status_name(status: &Status) -> &'static str {
    match status {
        Status::Pass => "pass",
        Status::Fail => "fail",
        Status::BlockedBy => "blocked-by",
    }
}

fn validate_scenarios(catalog: &Catalog) -> Result<(), String> {
    let mut ids = BTreeSet::new();
    for s in scenarios::all() {
        if !ids.insert(s.id) {
            return Err(format!("duplicate scenario {}", s.id));
        }
        let w = catalog
            .workflows
            .iter()
            .find(|w| w.id == s.workflow)
            .ok_or("scenario has unknown workflow")?;
        if (s.id.starts_with('A') || s.id.starts_with('B')) && !w.cases.iter().any(|c| c.id == s.id)
        {
            return Err(format!(
                "scenario {} does not match authoritative workflow",
                s.id
            ));
        }
    }
    Ok(())
}
