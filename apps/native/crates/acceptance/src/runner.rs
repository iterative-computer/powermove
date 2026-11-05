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
pub struct BlockerGroup {
    pub backing: String,
    pub needs_macos: bool,
    pub workflows: BTreeSet<String>,
    pub case_ids: Vec<String>,
}
fn grouped_blockers(cases: &[CaseResult]) -> Vec<BlockerGroup> {
    let mut groups = std::collections::BTreeMap::<String, BlockerGroup>::new();
    for case in cases.iter().filter(|c| c.status == Status::BlockedBy) {
        let backing = case
            .blocked_by
            .as_ref()
            .expect("blocked case needs backing");
        let group = groups
            .entry(backing.clone())
            .or_insert_with(|| BlockerGroup {
                backing: backing.clone(),
                needs_macos: false,
                workflows: BTreeSet::new(),
                case_ids: vec![],
            });
        group.needs_macos |= case.needs_macos;
        group.workflows.insert(case.workflow.clone());
        group.case_ids.push(case.id.clone());
    }
    groups.into_values().collect()
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub format: String,
    pub version: u32,
    pub platform: String,
    pub scope: String,
    pub cases: Vec<CaseResult>,
    pub blocker_groups: Vec<BlockerGroup>,
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
#[derive(serde::Deserialize)]
struct Backing {
    id: String,
    missing: String,
    needs_macos: bool,
}
fn backings() -> &'static std::collections::BTreeMap<String, Backing> {
    static BACKINGS: std::sync::OnceLock<std::collections::BTreeMap<String, Backing>> =
        std::sync::OnceLock::new();
    BACKINGS.get_or_init(|| {
        serde_json::from_str(include_str!("../data/backings.json"))
            .expect("acceptance backing inventory")
    })
}
fn blocker(workflow: &str) -> (&'static str, &'static str, bool) {
    let backing = &backings()[workflow];
    (&backing.id, &backing.missing, backing.needs_macos)
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

/// A case names the unverified assertion and its actual backing, rather than
/// treating every member of a workflow as blocked on the same missing crate.
fn blocked_original(c: &crate::catalog::Contract, workflow: &str) -> CaseResult {
    let mut result = blocked(&c.id, workflow, "original");
    if matches!(c.id.as_str(), "A511" | "A512") {
        result.blocked_by = Some("memory-post-tag-requirement".into());
        result.needs_macos = false;
        result.reason = format!(
            "Frozen-source reconciliation (2026-11-04): tag 07ed5fa17 has no project_memory reference in agent-tools or renderer; its 67-tool union and get_project_state description have no memory index. W1 misattributes this later requirement to the tag. Preserve the original ID/claim as an explicit additional native requirement: no memory index/body is implemented. Missing backing: kernel memory storage/index plus versioned tool admission; retained unverified claim: {}",
            c.expected
        );
        return result;
    }
    if workflow == "W040" {
        let capability = match c.id.as_str() {
            "A658" | "A697" | "A699" => Some("compat:panel-render-mount"),
            "A664" => Some("compat:scene3d-backend"),
            "A665" => Some("compat:asset-byte-import"),
            "A667" => Some("compat:palette-host"),
            "A670" => Some("compat:project-kernel"),
            "A671" => Some("compat:animation-evaluator"),
            "A672" => Some("compat:model-constructors"),
            "A673" => Some("compat:selection-kernel"),
            "A674" => Some("compat:group-query-edit"),
            "A675" => Some("compat:transport-engine"),
            "A676" => Some("compat:history-selection-adapter"),
            "A677" => Some("compat:structural-mutation-lease-adapter"),
            "A679" => Some("compat:media-registry"),
            "A680" => Some("compat:complete-canvas-engine-resources"),
            "A681" => Some("compat:ui-state-kernel"),
            "A682" => Some("compat:remote-dom-controls"),
            "A683" | "A694" | "A701" => Some("compat:native-host-actions"),
            "A684" => Some("compat:native-drag-drop"),
            "A685" => Some("compat:workspace-kernel"),
            "A686" => Some("compat:legacy-util-backend"),
            "A687" => Some("compat:ease-query-backend"),
            "A688" => Some("compat:space3d-backend"),
            "A690" => Some("compat:storage-kernel-persistence"),
            "A692" => Some("compat:extension-lifecycle"),
            "A700" => Some("compat:store-namespaced-events"),
            _ => None,
        };
        if let Some(capability) = capability {
            result.blocked_by = Some(capability.into());
            result.needs_macos = !matches!(
                c.id.as_str(),
                "A670"
                    | "A671"
                    | "A672"
                    | "A673"
                    | "A674"
                    | "A676"
                    | "A677"
                    | "A681"
                    | "A685"
                    | "A686"
                    | "A687"
                    | "A688"
                    | "A690"
                    | "A700"
            );
            result.reason = format!(
                "Missing connected {capability}; existing facade/registry or component probes do not execute every listed member against the real backend; unverified case: {}",
                c.expected
            );
            if c.id == "A676" {
                result.reason.push_str("; native V8/kernel begin/do/cancel/undo/redo/external paths execute, but KernelAdapter has no history.selection dispatch arm");
            }
            if c.id == "A677" {
                result.reason.push_str("; native V8/kernel apply/begin/dispatch/commit/cancel paths execute, but KernelAdapter lacks edit.mutateStart, edit.mutateFinish and edit.mutateAbort");
            }
            if c.id == "A680" {
                result.reason.push_str("; real JS recorder/native CPU fill/drawImage/readback now executes; complete text, paint, encoding and engine frame resources remain required");
            }
            return result;
        }
    }
    if let Some(name) = c
        .expected
        .strip_prefix("MCP &#96;")
        .and_then(|s| s.split_once("&#96;").map(|(name, _)| name))
        && pm_mcp::catalog::spec(name).is_some()
    {
        let implemented = pm_mcp::catalog::IMPLEMENTED.contains(&name);
        result.blocked_by = Some(format!("mcp:{name}"));
        result.needs_macos = matches!(
            name,
            "render_frames"
                | "probe_media"
                | "sample_media_frames"
                | "media_contact_sheet"
                | "media_waveform"
                | "export_video"
                | "get_workspace_state"
                | "open_panel"
                | "get_panel_state"
                | "interact_panel"
                | "capture_panel"
                | "computer_use_panel"
                | "inspect_creative_workspace"
                | "inspect_creative_extension"
        );
        result.reason = format!(
            "{}: {}; {}",
            if implemented {
                "Adapter/body merged; remaining full-case verification"
            } else {
                "Public tool body unavailable"
            },
            pm_mcp::catalog::backing(name),
            c.expected
        );
        if name == "save_project" {
            result.reason.push_str("; kernel container-copy probes preserve embedded media, but Automation exposes neither file-open/adopt nor durable media byte admission to seed this MCP case. Needs that host ingress and byte-for-byte packed media verification");
        }
    } else {
        result.reason.push_str("; unverified case: ");
        result.reason.push_str(&c.expected);
    }
    result
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
fn runtime_case(
    case: (&str, &str, &str, bool),
    kind: &str,
    title: &str,
    runtime: Option<&dyn crate::runtime::RuntimeHook>,
) -> CaseResult {
    let (id, workflow, _, requires_bundle) = case;
    if let Some(runtime) = runtime.filter(|r| !requires_bundle || r.has_compat_bundle()) {
        execute(id, workflow, kind, title, || runtime.check(id))
    } else {
        CaseResult {
            id: id.into(),
            workflow: workflow.into(),
            kind: kind.into(),
            status: Status::BlockedBy,
            reason: format!(
                "{title}; supply --runtime-runner{}",
                if requires_bundle {
                    " and --compat-bundle"
                } else {
                    ""
                }
            ),
            blocked_by: Some(
                if requires_bundle {
                    "compat-bundle"
                } else {
                    "native-v8-backing"
                }
                .into(),
            ),
            needs_macos: false,
        }
    }
}
pub fn run(catalog: &Catalog, filter: &Filter, corpus: &[PathBuf]) -> Result<Report, String> {
    run_with_runtime(catalog, filter, corpus, None)
}
pub fn run_with_runtime(
    catalog: &Catalog,
    filter: &Filter,
    corpus: &[PathBuf],
    runtime: Option<&dyn crate::runtime::RuntimeHook>,
) -> Result<Report, String> {
    catalog.validate()?;
    if backings().len() != catalog.workflows.len()
        || catalog
            .workflows
            .iter()
            .any(|w| !backings().contains_key(&w.id))
    {
        return Err("workflow/backing inventory drift".into());
    }
    let scenarios = scenarios::all();
    validate_scenarios(catalog)?;
    let mut known = BTreeSet::new();
    let mut cases = vec![];
    for w in &catalog.workflows {
        let id = format!("{}.contract", w.id);
        known.insert(id.clone());
        if filter.accepts(&id, &w.id) {
            let mut result = blocked(&id, &w.id, "workflow-contract");
            result.reason.push_str("; full W1 contract: ");
            result.reason.push_str(
                w.contract
                    .split("| Case |")
                    .next()
                    .unwrap_or(&w.contract)
                    .trim(),
            );
            cases.push(result);
        }
        for c in &w.cases {
            known.insert(c.id.clone());
            if !filter.accepts(&c.id, &w.id) {
                continue;
            }
            if let Some(s) = scenarios.iter().find(|s| s.id == c.id) {
                cases.push(execute(s.id, s.workflow, "original", s.title, s.run));
            } else if let Some(s) = crate::runtime::CASES.iter().find(|s| s.0 == c.id) {
                cases.push(runtime_case(*s, "original", &c.expected, runtime));
            } else {
                cases.push(blocked_original(c, &w.id));
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
    for &(id, workflow, title, requires_bundle) in crate::runtime::CASES {
        if id.starts_with(['A', 'B']) {
            continue;
        }
        known.insert(id.into());
        if filter.accepts(id, workflow) {
            cases.push(runtime_case(
                (id, workflow, title, requires_bundle),
                "runtime-probe",
                title,
                runtime,
            ));
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
            for c in w.cases.iter().filter(|c|c.source.contains(&format!("/{basename}"))) {acceptance_ids.push(c.id.clone());}
            for s in scenarios.iter().filter(|s|s.workflow==w.id) {supporting_probe_ids.push(s.id.into());}
            supporting_probe_ids.extend(crate::runtime::CASES.iter().filter(|(_,workflow,_,_)|*workflow==w.id).map(|(id,_,_,_)|id.to_string()));
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
        blocker_groups: grouped_blockers(&cases),
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
    /// Exact per-case open work, generated from this run (including filters).
    pub fn blockers_csv(&self) -> String {
        fn quote(s: &str) -> String {
            format!("\"{}\"", s.replace('"', "\"\""))
        }
        let mut text = "case_id,workflow,kind,needs_macos,blocked_by,reason\n".to_owned();
        for case in self.cases.iter().filter(|c| c.status == Status::BlockedBy) {
            text.push_str(&format!(
                "{},{},{},{},{},{}\n",
                quote(&case.id),
                quote(&case.workflow),
                quote(&case.kind),
                case.needs_macos,
                quote(case.blocked_by.as_deref().unwrap_or("")),
                quote(&case.reason)
            ));
        }
        text
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
    for &(id, workflow, _, _) in crate::runtime::CASES {
        if !ids.insert(id) {
            return Err(format!("duplicate native case {id}"));
        }
        let group = catalog
            .workflows
            .iter()
            .find(|w| w.id == workflow)
            .ok_or("native case has unknown workflow")?;
        if id.starts_with(['A', 'B']) && !group.cases.iter().any(|c| c.id == id) {
            return Err(format!("native case {id} has no authoritative contract"));
        }
    }
    Ok(())
}
