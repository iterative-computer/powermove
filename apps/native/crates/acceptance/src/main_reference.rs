//! Active parity target. Tagged evidence remains a separate compatibility report.
use crate::{
    catalog::{Binding, Catalog},
    runner::{CaseResult, Filter, Report, Status, execute},
    runtime::RuntimeHook,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::OnceLock,
};
pub const REFERENCE: &str = "c7f46a05641b23944d3692c9d80f9677c7e1ef1d";
pub const LEGACY_REFERENCE: &str = "07ed5fa17ee640659cb075f9cad1af2095011d60";
pub fn snapshot() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| serde_json::from_str(include_str!("../data/catalog-main.json")).unwrap())
}
/// Owner-selected Studio contracts extend pinned main. Keep the bare source
/// snapshot unchanged, and compare every field of each exact overlay as well.
pub fn expected_catalog() -> Value {
    let mut catalog = snapshot()["catalog"].clone();
    let studio: Value =
        serde_json::from_str(include_str!("../data/fixtures/scene3d-source-catalog.json"))
            .expect("pinned Studio source contracts");
    for overlay in studio["tools"].as_array().unwrap() {
        assert!(["edit_3d", "get_3d_scene"].contains(&overlay["name"].as_str().unwrap()));
        let base = catalog["tools"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|tool| tool["name"] == overlay["name"])
            .expect("existing main tool");
        *base = overlay.clone();
    }
    catalog
}
pub fn declarations() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| serde_json::from_str(include_str!("../data/main-e2e-frozen.json")).unwrap())
}
pub fn deltas() -> &'static Vec<Value> {
    static DATA: OnceLock<Vec<Value>> = OnceLock::new();
    DATA.get_or_init(|| serde_json::from_str(include_str!("../data/main-deltas.json")).unwrap())
}
#[derive(Debug, Serialize)]
pub struct Spec {
    pub file: String,
    pub in_main: bool,
    pub in_370_roster: bool,
    pub in_legacy_396: bool,
    pub legacy_acceptance_ids: Vec<String>,
    pub main_declaration_ids: Vec<String>,
    pub replacement_status: Status,
}
#[derive(Serialize)]
pub struct MainReport {
    pub format: &'static str,
    pub version: u8,
    pub reference: &'static str,
    pub compatibility_reference: &'static str,
    pub platform: &'static str,
    pub scope: &'static str,
    pub cases: Vec<CaseResult>,
    pub workflows: Vec<crate::runner::WorkflowResult>,
    pub blocker_groups: Vec<crate::runner::BlockerGroup>,
    pub specs: Vec<Spec>,
    pub delta_routing: &'static Vec<Value>,
    /// Actual tagged requirements, never counted as current main parity passes.
    pub compatibility: Option<Report>,
    /// Required old file/extension checks; old shell/visual workflow gates are
    /// historical evidence and do not reinstate a superseded UI reference.
    pub compatibility_required_ids: Vec<String>,
}
fn blocked(
    id: String,
    workflow: &str,
    kind: &str,
    backing: &str,
    reason: String,
    mac: bool,
) -> CaseResult {
    CaseResult {
        id,
        workflow: workflow.into(),
        kind: kind.into(),
        status: Status::BlockedBy,
        reason,
        blocked_by: Some(backing.into()),
        needs_macos: mac,
    }
}
fn selected(filter: &Filter, c: &CaseResult) -> bool {
    filter.case.as_deref().is_none_or(|id| id == c.id)
        && filter.workflow.as_deref().is_none_or(|w| w == c.workflow)
}
fn object_paths(expected: &Value, actual: &Value, path: &str, out: &mut Vec<String>) {
    if expected == actual {
        return;
    }
    match (expected, actual) {
        (Value::Object(e), Value::Object(a)) => {
            for key in e.keys().chain(a.keys()).collect::<BTreeSet<_>>() {
                if !e.contains_key(key) || !a.contains_key(key) {
                    out.push(format!("{path}/{key}"));
                    continue;
                }
                object_paths(
                    &e.get(key).cloned().unwrap_or(Value::Null),
                    &a.get(key).cloned().unwrap_or(Value::Null),
                    &format!("{path}/{key}"),
                    out,
                );
            }
        }
        _ => out.push(if path.is_empty() {
            "/".into()
        } else {
            path.into()
        }),
    }
}
fn exact(expected: &Value, actual: &Value) -> Result<(), String> {
    let mut paths = vec![];
    object_paths(expected, actual, "", &mut paths);
    if paths.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "pinned main contract differs at {}",
            paths.join(", ")
        ))
    }
}
fn binding(value: &Value) -> Binding {
    Binding {
        id: value["id"].as_str().unwrap().into(),
        chord: value["chord"].as_str().unwrap().into(),
        command: value["command"].as_str().unwrap().into(),
        args: value["args"].clone(),
        priority: value["priority"].as_i64().unwrap(),
        in_fields: value["in_fields"] == true,
        repeat: value["repeat"] == true,
        loose: value["loose"] == true,
        workflow: "W010".into(),
        source: value["source"].as_str().unwrap().into(),
    }
}
fn key_check(value: &Value, context: &str) -> Result<(), String> {
    let mut b = binding(value);
    let candidates = crate::keyboard::production::shipped();
    let native = candidates
        .iter()
        .find(|c| c.chord == b.chord && c.command == b.command)
        .ok_or_else(|| {
            format!(
                "main binding {} → {} is absent from native shell",
                b.chord, b.command
            )
        })?;
    b.id = native.id.clone();
    crate::keyboard::check(&b, context)
}
pub fn run(
    catalog: &Catalog,
    filter: &Filter,
    corpus: &[PathBuf],
    runtime: Option<&dyn RuntimeHook>,
) -> Result<MainReport, String> {
    catalog.validate()?;
    if snapshot()["reference"] != REFERENCE || declarations()["revision"] != REFERENCE {
        return Err("main reference fixture drift".into());
    }
    let mut cases = vec![];
    macro_rules! check {
        ($id:expr,$workflow:expr,$kind:expr,$title:expr,$body:expr) => {{
            let id = $id.to_string();
            if filter.case.as_deref().is_none_or(|x| x == id)
                && filter.workflow.as_deref().is_none_or(|x| x == $workflow)
            {
                cases.push(execute(&id, $workflow, $kind, $title, $body));
            }
        }};
    }
    let expected = &expected_catalog();
    for tool in expected["tools"].as_array().unwrap() {
        let name = tool["name"].as_str().unwrap();
        check!(
            format!("MAIN.MCP.{name}.catalog"),
            "W033",
            "main-tool-contract",
            "Exact pinned main plus owner-selected Studio name, description and input schema",
            || exact(tool, pm_mcp::catalog::spec(name).unwrap_or(&Value::Null))
        );
        let (backing, mac) = if name == "project_memory" {
            ("delta-data:project-memory-index-and-tool".into(), false)
        } else {
            (
                format!("main:mcp:{name}"),
                matches!(
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
                ),
            )
        };
        cases.push(blocked(format!("MAIN.MCP.{name}.behavior"),"W033","main-tool-behavior",&backing,format!("Execute the complete pinned main tool body, including lifecycle/results/error precedence: {}. {}",tool["description"].as_str().unwrap(),pm_mcp::catalog::backing(name)),mac));
    }
    for field in [
        "tools",
        "inspection",
        "app",
        "instructions",
        "commandFields",
        "easePresets",
    ] {
        check!(
            format!("MAIN.MCP.{field}.identity"),
            "W033",
            "main-catalog-contract",
            "Exact pinned main plus owner-selected Studio catalog/allowlist/instruction/command/easing identity",
            || exact(&expected[field], &pm_mcp::catalog::catalog()[field])
        );
    }
    check!(
        "MAIN.A07.project-memory-call",
        "W030",
        "main-mcp-execution",
        "Actual main project_memory list call through the native MCP session",
        || {
            let mut f = crate::fixture::Fixture::new()?;
            let result = f
                .session
                .call("project_memory", json!({"operation":"list"}));
            if result["isError"] == true {
                Err(format!(
                    "main project_memory list refused: {}",
                    result["content"][0]["text"]
                ))
            } else {
                let value: Value = serde_json::from_str(
                    result["content"][0]["text"]
                        .as_str()
                        .ok_or("missing memory content")?,
                )
                .map_err(|e| e.to_string())?;
                exact(&json!({"memory":[],"total":0}), &value)
            }
        }
    );
    check!(
        "MAIN.A07.memory-index",
        "W030",
        "main-mcp-execution",
        "Current main empty project exposes the memory index without edits",
        || {
            let mut f = crate::fixture::Fixture::new()?;
            let before = f.document()?;
            let revision = f.revision();
            let state = f.call("get_project_state", json!({}))?;
            exact(
                &json!({"total":0,"goal":null,"constraints":[],"open":[],"recent":[]}),
                &state["memory"],
            )?;
            if f.document()? != before || f.revision() != revision {
                return Err("memory inspection changed authored state".into());
            }
            Ok(())
        }
    );
    // Same named MCP assertions remain useful current supporting evidence when
    // main's public contract is exactly unchanged. Do not promote full bodies.
    for scenario in crate::scenarios::all()
        .into_iter()
        .filter(|s| s.id.starts_with('A'))
    {
        if matches!(scenario.id, "A506" | "A508" | "A532") {
            continue;
        } // new 3D/effect definitions need their new owners' fixtures.
        let contract = catalog
            .workflows
            .iter()
            .flat_map(|w| &w.cases)
            .find(|c| c.id == scenario.id);
        let name = contract
            .and_then(|c| c.expected.strip_prefix("MCP &#96;"))
            .and_then(|s| s.split_once("&#96;").map(|p| p.0));
        if let Some(name) = name {
            let main = expected["tools"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["name"] == name);
            if main.is_some() && main == pm_mcp::catalog::spec(name) {
                check!(
                    format!("MAIN.support.{}", scenario.id),
                    scenario.workflow,
                    "main-mcp-supporting-assertion",
                    scenario.title,
                    scenario.run
                );
            }
        }
    }
    // Independent kernel facts are supporting evidence; no complete workflow is promoted.
    for (id, workflow, source) in [
        ("MAIN.A06.strip-library-support", "W033", "A505"),
        (
            "MAIN.A09.revision-fence-support",
            "W030",
            "W030.interleaved",
        ),
        ("MAIN.A10.capture-refusal-support", "W030", "W030.lock"),
    ] {
        let scenario = crate::scenarios::all()
            .into_iter()
            .find(|s| s.id == source)
            .unwrap();
        check!(
            id,
            workflow,
            "main-supporting-probe",
            scenario.title,
            scenario.run
        );
    }
    for (id, operation) in [
        ("MAIN.curve.set", "set_property"),
        ("MAIN.curve.replace", "replace_keyframes"),
    ] {
        check!(
            id,
            "W061",
            "main-authored-curve",
            "Pinned main Glide [0.16,0.84,0.24,1], independent authored curve oracle",
            || {
                exact(
                    &json!([0.16, 0.84, 0.24, 1]),
                    &expected["easePresets"]["glide"],
                )?;
                crate::curve_oracle::easing(operation)
            }
        );
    }
    for value in snapshot()["bindings"].as_array().unwrap() {
        let id = value["id"].as_str().unwrap();
        check!(
            format!("{id}.definition"),
            "W010",
            "main-key-definition",
            "Current main binding metadata against production shell",
            || key_check(value, "normal")
        );
        for context in crate::keyboard::CONTEXTS {
            check!(
                format!("{id}.route-{context}"),
                "W010",
                "main-key-routing",
                "Production resolver against pinned main routing flags/args",
                || key_check(value, context)
            );
        }
        for context in [
            "normal",
            "held",
            "extra-modifier",
            "input",
            "contenteditable",
            "read-only-selection",
            "button",
            "prior-decline",
            "menu-once",
        ] {
            cases.push(blocked(format!("{id}.{context}"),"W010","main-key-integration","main:native-keyboard-dispatch",format!("Real native shell dispatch {} → {} {}; priority {}, fields={}, repeat={}, loose={} under {context}. Source {REFERENCE}:{}",value["chord"],value["command"],value["args"],value["priority"],value["in_fields"],value["repeat"],value["loose"],value["source"]),true));
        }
    }
    let backings: Value = serde_json::from_str(include_str!("../data/backings.json")).unwrap();
    for w in &catalog.workflows {
        if w.id == "W050" {
            cases.push(blocked("MAIN.W050.contract".into(),"W050","main-workflow-contract","main:native-scene3d-editor","Owner requires the new Rust 3D system from feat/na-3d-source, automatic old-project upgrade, no Blender/compatibility mode, and actual native editor/render/resource verification. Old get_3d_scene fixtures do not certify this gate.".into(),true));
            continue;
        }
        if w.id == "W072" {
            cases.push(blocked("MAIN.W072.contract".into(),"W072","main-workflow-contract","delta-data:archived-web-export-preferences","Main removed standalone web-animation delivery. Preserve old project preferences/archives; verify the removed action stays absent. This gate does not request reinstating web export.".into(),true));
            continue;
        }
        cases.push(blocked(format!("MAIN.{}.contract",w.id),&w.id,"main-workflow-contract",&format!("main:{}",backings[&w.id]["id"].as_str().unwrap()),format!("Complete current-main workflow: {}. {}. Current declarations and A01–A10 deltas must execute; old tagged variants alone cannot certify this gate.",w.title,backings[&w.id]["missing"].as_str().unwrap()),backings[&w.id]["needs_macos"]==true));
    }
    for delta in deltas()
        .iter()
        .filter(|d| d["id"].as_str().unwrap().starts_with('A'))
    {
        let id = delta["id"].as_str().unwrap();
        cases.push(blocked(
            format!("MAIN.DELTA.{id}"),
            "MAIN.A",
            "main-delta-contract",
            delta["blocked_by"].as_str().unwrap(),
            format!(
                "{}; pinned source evidence {}",
                delta["expected"].as_str().unwrap(),
                delta["source_hashes"]
            ) + if id=="A02" {"; owner requires explicit consent before Claude desktop registration; acceptance never writes real client settings"} else {""},
            matches!(id, "A02" | "A04" | "A05" | "A08" | "A09"),
        ));
    }
    let mut specs = BTreeMap::<String, Spec>::new();
    for spec in &catalog.specs {
        specs.insert(
            spec.file.clone(),
            Spec {
                file: spec.file.clone(),
                in_main: false,
                in_370_roster: spec.in_370_roster,
                in_legacy_396: true,
                legacy_acceptance_ids: spec.native_triage.acceptance_ids.clone(),
                main_declaration_ids: vec![],
                replacement_status: Status::BlockedBy,
            },
        );
    }
    for source in declarations()["sources"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["declarations"].as_u64().unwrap() > 0)
    {
        let file = source["file"].as_str().unwrap();
        let spec = specs.entry(file.into()).or_insert(Spec {
            file: file.into(),
            in_main: false,
            in_370_roster: false,
            in_legacy_396: false,
            legacy_acceptance_ids: vec![],
            main_declaration_ids: vec![],
            replacement_status: Status::BlockedBy,
        });
        spec.in_main = true;
    }
    for declaration in declarations()["cases"].as_array().unwrap() {
        let file = declaration["file"].as_str().unwrap();
        let workflow = catalog
            .specs
            .iter()
            .find(|s| s.file == file)
            .and_then(|s| s.workflows.first())
            .map(String::as_str)
            .unwrap_or("W077");
        let id = format!("MAIN.E2E.{}", declaration["id"].as_str().unwrap());
        specs
            .get_mut(file)
            .unwrap()
            .main_declaration_ids
            .push(id.clone());
        let owner = match workflow {
            "W007" | "W008" | "W019" | "W020" | "W022" => "delta-data",
            "W050" | "W059" | "W064" | "W068" | "W069" | "W070" | "W071" | "W076" | "W078" => {
                "delta-fx"
            }
            _ => "native-app",
        };
        let mut result = blocked(
            id,
            workflow,
            "main-e2e-declaration",
            &format!("{owner}:{}", backings[workflow]["id"].as_str().unwrap()),
            format!(
                "{REFERENCE}:{file}:{} — {}; declaration SHA {}; parameters {}; original guards {}. Complete native controller/oracle: {}",
                declaration["line"],
                declaration["title"].as_str().unwrap(),
                declaration["sha256"],
                declaration["parameters"],
                declaration["guards"],
                backings[workflow]["missing"].as_str().unwrap()
            ),
            backings[workflow]["needs_macos"] == true,
        );
        // Main's declaration and operation parameters match the strict authored
        // curve oracle exactly. Preserve its measured failure in this verdict.
        if file.ends_with("/agent-keyframe-easing.spec.ts")
            && declaration["sha256"]
                == "44818a0df2caae3fef46a6737b7322de0e4badf97d1a2a634f3d42c33b26c598"
            && selected(filter, &result)
        {
            let curve = execute(
                "MAIN.curve.declaration",
                "W061",
                "main-authored-curve",
                "Current main complete Glide set_property variant",
                crate::curve_oracle::set,
            );
            if curve.status == Status::Fail {
                result.status = Status::Fail;
                result.blocked_by = None;
                result.needs_macos = false;
                result.reason.push_str("; measured strict variant: ");
                result.reason.push_str(&curve.reason);
            }
        }
        cases.push(result);
    }
    check!(
        "MAIN.file.utf16-ingress",
        "W007",
        "main-file-compatibility",
        "Production MCP ingress preserves kernel-valid authored UTF-16",
        crate::headless::utf16_ingress
    );
    if let Some(case) = cases
        .iter_mut()
        .find(|c| c.id == "MAIN.MCP.fork_builtin_extension.behavior")
        && selected(filter, case)
        && let Some(runtime) = runtime
    {
        *case = execute(
            &case.id,
            &case.workflow,
            &case.kind,
            "Production file-backed MCP fork preserves exact sources, pristine merge base and staged lifecycle",
            || runtime.check("A510"),
        );
    }
    // Only individually reviewed, connected portable bodies are promoted. A
    // schema match or a passing neighboring operation alone never closes one.
    for case in &mut cases {
        if case.kind == "main-tool-behavior"
            && let Some(name) = case
                .id
                .strip_prefix("MAIN.MCP.")
                .and_then(|s| s.strip_suffix(".behavior"))
            && crate::headless::TOOLS.iter().any(|t| t.0 == name)
            && selected(filter, case)
        {
            *case = execute(
                &case.id,
                &case.workflow,
                &case.kind,
                "Production MCP/kernel body, exact catalog, lifecycle/error precedence and persisted authored-state assertions",
                || crate::headless::check(name),
            );
        }
    }
    cases.retain(|c| selected(filter, c));
    if cases.is_empty() {
        return Err(
            "main filters selected no acceptance cases; use --reference v1.2.0 for legacy IDs"
                .into(),
        );
    }
    let full = filter.case.is_none() && filter.workflow.is_none();
    let compatibility = if full {
        Some(crate::runner::run_with_runtime(
            catalog,
            &Filter::default(),
            corpus,
            runtime,
        )?)
    } else {
        None
    };
    let compatibility_required_ids = compatibility
        .as_ref()
        .map(|report| {
            report
                .cases
                .iter()
                .filter(|case| {
                    case.kind == "corpus-copy"
                        || case.id == "W007.portable"
                        || (matches!(case.workflow.as_str(), "W037" | "W038" | "W040")
                            && crate::runtime::CASES.iter().any(|entry| entry.0 == case.id))
                })
                .map(|case| case.id.clone())
                .collect()
        })
        .unwrap_or_default();
    let workflows=catalog.workflows.iter().map(|w| {
        let rows=cases.iter().filter(|c|c.workflow==w.id).collect::<Vec<_>>();
        let passed=rows.iter().filter(|c|c.status==Status::Pass).count();
        let failed=rows.iter().filter(|c|c.status==Status::Fail).count();
        let blocked=rows.len()-passed-failed;
        crate::runner::WorkflowResult {id:w.id.clone(),title:if w.id=="W072" {"Retain archived web-export preferences with the retired action absent".into()} else {w.title.clone()},status:if failed>0{Status::Fail}else{Status::BlockedBy},reason:"Pinned main full workflow remains blocked; narrower assertions retain their own verdicts".into(),passed,failed,blocked}
    }).collect();
    Ok(MainReport {
        format: "powermove.acceptance.main",
        version: 2,
        reference: REFERENCE,
        compatibility_reference: LEGACY_REFERENCE,
        platform: std::env::consts::OS,
        scope: if full { "full-inventory" } else { "filtered" },
        blocker_groups: crate::runner::grouped_blockers(&cases),
        cases,
        workflows,
        specs: specs.into_values().collect(),
        delta_routing: deltas(),
        compatibility,
        compatibility_required_ids,
    })
}
impl MainReport {
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
        let required = self
            .compatibility
            .iter()
            .flat_map(|report| &report.cases)
            .filter(|case| self.compatibility_required_ids.contains(&case.id))
            .collect::<Vec<_>>();
        if fail > 0 || required.iter().any(|c| c.status == Status::Fail) {
            1
        } else if strict && (blocked > 0 || required.iter().any(|c| c.status == Status::BlockedBy))
        {
            2
        } else {
            0
        }
    }
    pub fn blockers_csv(&self) -> String {
        let mut out = "case_id,workflow,kind,needs_macos,blocked_by,reason\n".to_owned();
        for c in self.cases.iter().filter(|c| c.status == Status::BlockedBy) {
            let q = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
            out.push_str(&format!(
                "{},{},{},{},{},{}\n",
                q(&c.id),
                q(&c.workflow),
                q(&c.kind),
                c.needs_macos,
                q(c.blocked_by.as_deref().unwrap()),
                q(&c.reason)
            ));
        }
        out
    }
    pub fn e2e_csv(&self) -> String {
        let q = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
        let mut out="spec_file,in_370_roster,in_legacy_396,in_main,reference,legacy_acceptance_ids,main_declaration_ids,replacement_status\n".to_owned();
        for s in &self.specs {
            out.push_str(&format!(
                "{},{},{},{},{},{},{},blocked-by\n",
                q(&s.file),
                s.in_370_roster,
                s.in_legacy_396,
                s.in_main,
                q(REFERENCE),
                q(&s.legacy_acceptance_ids.join(";")),
                q(&s.main_declaration_ids.join(";"))
            ));
        }
        out
    }
    pub fn markdown(&self) -> String {
        let (p, f, b) = self.counts();
        format!(
            "Active main `{REFERENCE}`: **{p} pass / {f} fail / {b} blocked**. Tagged compatibility is reported separately.\n"
        )
    }
}
