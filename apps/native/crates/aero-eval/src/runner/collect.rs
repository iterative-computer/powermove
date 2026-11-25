//! Seeding a clean profile and collecting evidence over the host's MCP tools.
//! Anything the host cannot report becomes a note (or a lacking capability),
//! and the graders that needed it answer Unavailable: nothing is guessed into
//! a pass. Every file of the run is reached through the run folder opened
//! once ([`paths::Anchor`]), one unfollowed component at a time, and a path
//! the host names is read only when it lies in the run's exports or work
//! folder.
use super::mcp::McpClient;
use super::paths;
use crate::artifacts::{
    Artifacts, ChangedExtension, Evaluation, ExtensionHealth, Frame, FrameTiming, PanelObservation,
    ProbeObservation, Registration, ThemeObservation,
};
use crate::graders::panel::Step;
use crate::graders::{Context, GraderSpec};
use crate::suite::Task;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The host has one window when headless.
pub const HEADLESS_WINDOW: &str = "headless";
const TIMING_SAMPLES: usize = 7;

pub struct RunDirs {
    pub root: PathBuf,
    /// The canonical spelling of `root` (`/var/…` is `/private/var/…`).
    pub real_root: PathBuf,
    /// The run folder, open: every read, walk and write below it goes
    /// through this.
    pub anchor: paths::Anchor,
    pub profile: PathBuf,
    pub exports: PathBuf,
    pub work: PathBuf,
    pub home: PathBuf,
    pub tmp: PathBuf,
    pub frames: PathBuf,
    pub host_log: PathBuf,
}

impl RunDirs {
    /// The folders of a run inside its (already reserved, empty) root.
    pub fn create(root: &Path) -> Result<Self, String> {
        let root = std::path::absolute(root).map_err(|e| e.to_string())?;
        let anchor = paths::Anchor::open(&root)?;
        let d = RunDirs {
            real_root: std::fs::canonicalize(&root).map_err(|e| e.to_string())?,
            anchor,
            profile: root.join("profile"),
            exports: root.join("exports"),
            work: root.join("work"),
            home: super::harness::home_in(&root),
            tmp: root.join("tmp"),
            frames: root.join("frames"),
            host_log: root.join("host.log"),
            root,
        };
        for p in [&d.profile, &d.exports, &d.work, &d.home, &d.tmp, &d.frames] {
            std::fs::create_dir(p).map_err(|e| format!("{}: {e}", p.display()))?;
        }
        Ok(d)
    }
}

/// Copy a suite folder into the run (relative to the run folder), creating
/// folders below it unfollowed.
fn copy_tree(from: &Path, d: &RunDirs, to: &Path) -> Result<(), String> {
    let src = paths::Anchor::open(from)?;
    for rel in src.files()? {
        let dest = to.join(&rel);
        if let Some(parent) = dest.parent() {
            mkdirs(d, parent)?;
        }
        d.anchor.write_new(&dest, &src.read(&rel)?)?;
    }
    Ok(())
}

/// Create the folders of `rel` below the run folder (existing ones must be
/// real folders, not links).
fn mkdirs(d: &RunDirs, rel: &Path) -> Result<(), String> {
    let mut at = PathBuf::new();
    for c in rel.components() {
        let parent = d.anchor.dir(&at)?;
        at.push(c);
        match parent
            .entries()?
            .iter()
            .find(|(n, _)| Path::new(n) == c.as_os_str())
        {
            Some((_, paths::Kind::Dir)) => {}
            Some(_) => return Err(format!("{} is not a folder", d.root.join(&at).display())),
            None => parent.mkdir(c.as_os_str())?,
        }
    }
    d.anchor.dir(rel).map(|_| ())
}

/// Put the fixture into the clean profile: the project as a file entry of the
/// v1.2 library (`store/projects.json` row plus `projectMeta.<id>.file`, the
/// file route of p0-decisions), and the fixture's extensions under
/// `extensions/`. Returns the project id.
pub fn seed_profile(task: &Task, data_root: &Path, d: &RunDirs) -> Result<String, String> {
    let fixtures = data_root.join("fixtures");
    let src = fixtures.join(&task.fixture.project);
    let name = src.file_name().ok_or("fixture has no file name")?;
    let file = d.work.join(name);
    let bytes = std::fs::read(&src).map_err(|e| format!("{}: {e}", src.display()))?;
    d.anchor.write_new(&Path::new("work").join(name), &bytes)?;
    let doc = crate::pmv::read_document(&bytes)?;
    let project = crate::pmv::project_of(&doc);
    let id = project["id"]
        .as_str()
        .ok_or("fixture project has no id")?
        .to_owned();
    mkdirs(d, Path::new("profile/store"))?;
    let write = |name: String, v: Value| {
        d.anchor.write_new(
            &Path::new("profile/store").join(name),
            v.to_string().as_bytes(),
        )
    };
    write(
        "projects.json".into(),
        json!([{"id": id, "name": project["name"], "at": super::now_ms()}]),
    )?;
    write(
        format!("projectMeta.{id}.json"),
        json!({"file": file.display().to_string()}),
    )?;
    for ext in &task.fixture.extensions {
        let from = fixtures.join(ext);
        let name = from.file_name().ok_or("extension has no folder name")?;
        copy_tree(&from, d, &Path::new("profile/extensions").join(name))?;
    }
    Ok(id)
}

/// What the baseline step learned, for comparison after the run.
#[derive(Default)]
pub struct Baseline {
    /// Tree hash of each extension folder, or why it was not read.
    pub extension_hashes: BTreeMap<String, Result<String, String>>,
    /// The project's revision once opened (Aero's run is bound to it).
    pub revision: Option<u64>,
}

/// Tree hashes of the profile's extension folders, reached from the run
/// folder without following any link. An extension root that is a link (or
/// not a folder) is refused, unread, and reported under its own name.
fn extension_hashes(d: &RunDirs) -> BTreeMap<String, Result<String, String>> {
    let mut out = BTreeMap::new();
    let profile = match d.anchor.dir(Path::new("profile")) {
        Ok(p) => p,
        Err(e) => {
            out.insert("profile".into(), Err(e));
            return out;
        }
    };
    let entries = match profile.entries() {
        Ok(e) => e,
        Err(e) => {
            out.insert("profile".into(), Err(e));
            return out;
        }
    };
    for root in ["extensions", "extensions-shared"] {
        let Some((_, kind)) = entries.iter().find(|(n, _)| n == root) else {
            continue;
        };
        if *kind != paths::Kind::Dir {
            out.insert(
                root.into(),
                Err(format!(
                    "the extension root {root} is a symlink or not a folder"
                )),
            );
            continue;
        }
        let dir = match profile
            .dir(Path::new(root))
            .and_then(|a| Ok((a.entries()?, a)))
        {
            Ok(x) => x,
            Err(e) => {
                out.insert(root.into(), Err(e));
                continue;
            }
        };
        let (list, anchor) = dir;
        for (name, kind) in list {
            match kind {
                paths::Kind::Symlink => {
                    out.insert(name, Err("the extension folder is a symlink".into()));
                }
                paths::Kind::Dir => {
                    let h = anchor.dir(Path::new(&name)).and_then(|a| a.tree_hash());
                    out.insert(name, h);
                }
                _ => {}
            }
        }
    }
    out
}

fn frame_times(task: &Task) -> Vec<f64> {
    let mut t: Vec<f64> = task
        .graders
        .iter()
        .flat_map(GraderSpec::frame_times)
        .collect();
    t.sort_by(f64::total_cmp);
    t.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    t
}

fn render(
    c: &mut McpClient,
    times: &[f64],
    d: &RunDirs,
    prefix: &str,
) -> Result<Vec<Frame>, String> {
    let mut out = Vec::new();
    for chunk in times.chunks(5) {
        let r = c.call("render_frames", json!({"times": chunk, "width": 1280}))?;
        let images = r.images();
        if images.len() != chunk.len() {
            return Err(format!(
                "render_frames returned {} images for {} times",
                images.len(),
                chunk.len()
            ));
        }
        for (&t, (mime, bytes)) in chunk.iter().zip(images) {
            if mime != "image/png" {
                return Err(format!(
                    "render_frames returned {mime}; the frame graders read PNG only"
                ));
            }
            let rel = PathBuf::from("frames").join(format!("{prefix}-{t}.png"));
            d.anchor.write_new(&rel, &bytes)?;
            out.push(Frame { time: t, png: rel });
        }
    }
    Ok(out)
}

/// Open the seeded project, then capture the fixture's state and frames.
pub fn baseline(
    task: &Task,
    project_id: &str,
    d: &RunDirs,
    c: &mut McpClient,
    a: &mut Artifacts,
) -> Result<Baseline, String> {
    let listed = c.call("list_projects", json!({}))?;
    if !listed.text().contains(project_id) {
        return Err(format!(
            "the seeded project {project_id} is not in the clean profile's library: {}",
            listed.text()
        ));
    }
    c.call("open_project", json!({"projectId": project_id}))?;
    let work = d.anchor.dir(Path::new("work"))?;
    let (file, _) = work
        .entries()?
        .into_iter()
        .find(|(n, k)| *k == paths::Kind::File && n.ends_with(".pmv"))
        .ok_or("no working project file")?;
    let before = crate::pmv::read_document(&work.read(Path::new(&file))?)?;
    let b = Baseline {
        extension_hashes: extension_hashes(d),
        revision: crate::pmv::project_of(&before)["revision"].as_u64(),
    };
    a.project_before = Some(before);
    let times = frame_times(task);
    if !times.is_empty() && !lacks(a, super::capabilities::FRAMES) {
        match render(c, &times, d, "baseline") {
            Ok(f) => a.baseline_frames = f,
            Err(e) => a.notes.push(format!("baseline frames: {e}")),
        }
    }
    Ok(b)
}

fn lacks(a: &Artifacts, cap: &str) -> bool {
    a.host_lacks.iter().any(|l| l == cap)
}

/// The host's answer to save_project names the saved file.
fn saved_path(r: &Value) -> Option<PathBuf> {
    match r {
        Value::String(s) if s.ends_with(".pmv") => Some(PathBuf::from(s)),
        Value::Object(m) => m.values().find_map(saved_path),
        Value::Array(v) => v.iter().find_map(saved_path),
        _ => None,
    }
}

/// Save and read the project back. The path the host names is read only if
/// it is a regular file in the run's exports or work folder, reached from
/// the open run folder with no link followed at any component.
fn read_saved(c: &mut McpClient, d: &RunDirs) -> Result<Value, String> {
    let r = c.call("save_project", json!({}))?;
    let path = r
        .json()
        .as_ref()
        .and_then(saved_path)
        .or_else(|| saved_path(&Value::String(r.text())))
        .ok_or_else(|| format!("save_project did not name a file: {}", r.text()))?;
    let rel = d.anchor.relative(&path, &d.real_root).map_err(|_| {
        format!(
            "{} is not a file inside {} or {}",
            path.display(),
            d.exports.display(),
            d.work.display()
        )
    })?;
    if !rel.starts_with("exports") && !rel.starts_with("work") {
        return Err(format!(
            "{} is not a file inside {} or {}",
            path.display(),
            d.exports.display(),
            d.work.display()
        ));
    }
    crate::pmv::read_document(&d.anchor.read(&rel)?)
}

/// Keys of a panel state that name or classify an element instead of
/// showing text (a control ref like `e5` must never read as a time).
const NOT_SHOWN: [&str; 9] = [
    "ref",
    "id",
    "role",
    "type",
    "kind",
    "key",
    "testId",
    "className",
    "style",
];

/// Every displayed string of a panel state, in order.
pub fn texts(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) if !s.trim().is_empty() => out.push(s.clone()),
        Value::Object(m) => m
            .iter()
            .filter(|(k, _)| !NOT_SHOWN.contains(&k.as_str()))
            .for_each(|(_, x)| texts(x, out)),
        Value::Array(a) => a.iter().for_each(|x| texts(x, out)),
        _ => {}
    }
}

/// The displayed text of every element (object or list) of a panel state,
/// innermost first, so a grader can tell which values share a row.
pub fn rows(v: &Value, out: &mut Vec<String>) {
    let children: Vec<&Value> = match v {
        Value::Object(m) => m
            .iter()
            .filter(|(k, _)| !NOT_SHOWN.contains(&k.as_str()))
            .map(|(_, x)| x)
            .collect(),
        Value::Array(a) => a.iter().collect(),
        _ => return,
    };
    for c in children {
        rows(c, out);
    }
    let mut t = Vec::new();
    texts(v, &mut t);
    if !t.is_empty() {
        out.push(t.join(" "));
    }
}

fn playhead(v: &Value) -> Option<f64> {
    match v {
        Value::Object(m) => ["playhead", "time", "currentTime"]
            .iter()
            .find_map(|k| m.get(*k).and_then(Value::as_f64))
            .or_else(|| m.values().find_map(playhead)),
        _ => None,
    }
}

/// Registrations the host reports with their owning extension.
fn registrations(w: &Value) -> Vec<Registration> {
    let mut out = Vec::new();
    for (key, kind) in [
        ("registeredEffects", "effect"),
        ("registeredTransitions", "transition"),
        ("registeredPanels", "panel"),
    ] {
        for e in w[key].as_array().into_iter().flatten() {
            let Some(id) = e["id"].as_str() else { continue };
            out.push(Registration {
                kind: kind.into(),
                id: id.into(),
                extension: e["extension"].as_str().unwrap_or("?").into(),
                params: e["parameters"].as_array().map_or(0, Vec::len),
            });
        }
    }
    out
}

/// After the harness: save and read the project back, render, and report
/// extensions, frame times, panel probes, the active theme and the
/// evaluator's scores. Stops early when the collection budget runs out.
pub fn after(
    task: &Task,
    d: &RunDirs,
    b: &Baseline,
    c: &mut McpClient,
    a: &mut Artifacts,
    data_root: &Path,
) {
    match read_saved(c, d) {
        Ok(p) => a.project_after = Some(p),
        Err(e) => a.notes.push(format!("project after the run: {e}")),
    }
    let times = frame_times(task);
    if !times.is_empty() && !lacks(a, super::capabilities::FRAMES) {
        match render(c, &times, d, "after") {
            Ok(f) => a.frames = f,
            Err(e) => a.notes.push(format!("frames after the run: {e}")),
        }
    }
    // Extensions: changed by tree hash; a symlinked one is refused, unread.
    let now = extension_hashes(d);
    let mut changed = Vec::new();
    for (id, h) in &now {
        let action = match (h, b.extension_hashes.get(id)) {
            (Err(_), _) => "refused",
            (Ok(_), None) => "created",
            (Ok(h), Some(Ok(old))) if old == h => continue,
            (Ok(_), Some(_)) => "updated",
        };
        changed.push(ChangedExtension {
            id: id.clone(),
            action: action.into(),
        });
    }
    for id in b.extension_hashes.keys().filter(|k| !now.contains_key(*k)) {
        changed.push(ChangedExtension {
            id: id.clone(),
            action: "removed".into(),
        });
    }
    a.extensions.changed = changed;
    a.extensions.windows = vec![HEADLESS_WINDOW.into()];
    match c.call("get_workspace_state", json!({})).map(|r| r.json()) {
        Ok(Some(w)) => {
            for e in w["extensions"].as_array().into_iter().flatten() {
                let Some(id) = e["id"].as_str() else { continue };
                let state = match &e["health"] {
                    Value::String(s) => Some(s.clone()),
                    Value::Object(h) => h.get("state").and_then(Value::as_str).map(str::to_owned),
                    _ => None,
                };
                if let Some(state) = state {
                    a.extensions.health.push(ExtensionHealth {
                        id: id.into(),
                        window: HEADLESS_WINDOW.into(),
                        state,
                        error: e["health"]["error"].as_str().map(str::to_owned),
                    });
                }
            }
            a.extensions.registered = registrations(&w);
            if let Some(t) = w.get("theme").filter(|t| t.is_object()) {
                let theme = ThemeObservation {
                    active: t["active"].as_str().unwrap_or("").into(),
                    extension: t["extension"].as_str().map(str::to_owned),
                    tokens: t["tokens"]
                        .as_object()
                        .map(|m| {
                            m.iter()
                                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                                .collect()
                        })
                        .unwrap_or_default(),
                };
                a.extensions.registered.push(Registration {
                    kind: "theme".into(),
                    id: theme.active.clone(),
                    extension: theme.extension.clone().unwrap_or_else(|| "?".into()),
                    params: 0,
                });
                a.theme = Some(theme);
            }
        }
        Ok(None) => a.notes.push("get_workspace_state answered no JSON".into()),
        Err(e) => a.notes.push(format!("get_workspace_state: {e}")),
    }
    for g in &task.graders {
        match g {
            GraderSpec::FrameTime(s) if !lacks(a, super::capabilities::TIMING) => {
                let ids = Context {
                    artifacts: a,
                    run_dir: &d.root,
                    data_root: Path::new(""),
                }
                .resolve(&s.target);
                for id in ids {
                    match measure(c, &id, &times) {
                        Ok(t) => a.frame_times.push(t),
                        Err(e) => a.notes.push(format!("frame time of {id}: {e}")),
                    }
                }
            }
            GraderSpec::Panel(s) => {
                let ids = Context {
                    artifacts: a,
                    run_dir: &d.root,
                    data_root: Path::new(""),
                }
                .resolve(&s.panel);
                if let Some(id) = ids.first().cloned()
                    && !lacks(a, super::capabilities::PROBES)
                {
                    let obs = probe(c, &id, s, a.project_after.as_ref(), d);
                    a.panels.push(obs);
                }
            }
            GraderSpec::Rubric(r) if !lacks(a, super::capabilities::EVALUATOR) => {
                match evaluate(c, task, &data_root.join(&r.rubric), a) {
                    Ok(e) => a.evaluation = Some(e),
                    Err(e) => a.notes.push(format!("evaluator: {e}")),
                }
            }
            _ => {}
        }
    }
}

/// Paired timings of one target at active frames: the same revision rendered
/// with the target enabled and disabled; each sample is their difference.
fn measure(c: &mut McpClient, target: &str, times: &[f64]) -> Result<FrameTiming, String> {
    let r = c
        .call(
            "measure_frame_time",
            json!({"target": target, "times": times, "samples": TIMING_SAMPLES, "width": 1280}),
        )?
        .json()
        .ok_or("measure_frame_time answered no JSON")?;
    let list = |k: &str| -> Vec<f64> {
        r[k].as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_f64)
            .collect()
    };
    let (on, off) = (list("enabledMs"), list("disabledMs"));
    if on.len() != off.len() || on.is_empty() {
        return Err(format!(
            "{} enabled and {} disabled samples are not pairs",
            on.len(),
            off.len()
        ));
    }
    Ok(FrameTiming {
        target: target.into(),
        ms: on.iter().zip(&off).map(|(x, y)| x - y).collect(),
        method: format!(
            "paired enabled/disabled renders at revision {}, active frames {times:?}",
            r["revision"]
        ),
    })
}

/// A fresh evaluator session's scores for one rubric (never the generator's
/// context): the rubric, the prompt and the run's frames and captures.
fn evaluate(
    c: &mut McpClient,
    task: &Task,
    rubric: &Path,
    a: &Artifacts,
) -> Result<Evaluation, String> {
    let r = crate::graders::rubric::Rubric::load(rubric)?;
    let mut sees: Vec<String> = a
        .frames
        .iter()
        .map(|f| f.png.display().to_string())
        .collect();
    sees.extend(
        a.panels
            .iter()
            .filter_map(|p| p.capture.as_ref().map(|c| c.display().to_string())),
    );
    let v = c
        .call(
            "evaluate_rubric",
            json!({"rubric": r, "prompt": task.prompt, "sees": sees}),
        )?
        .json()
        .ok_or("evaluate_rubric answered no JSON")?;
    serde_json::from_value(v).map_err(|e| format!("evaluate_rubric answer: {e}"))
}

/// Find the ref of a control by its label anywhere in a panel state: an exact
/// (case-insensitive) label first, else a label containing the text.
fn control_ref(state: &Value, label: &str) -> Option<String> {
    control_ref_by(state, label, true).or_else(|| control_ref_by(state, label, false))
}

fn control_ref_by(state: &Value, label: &str, exact: bool) -> Option<String> {
    let want = label.to_lowercase();
    let hit = |s: &str| {
        let s = s.trim().to_lowercase();
        if exact { s == want } else { s.contains(&want) }
    };
    match state {
        Value::Object(m) => {
            if let Some(r) = m.get("ref").and_then(Value::as_str)
                && m.iter()
                    .any(|(k, v)| k != "ref" && v.as_str().is_some_and(hit))
            {
                return Some(r.to_owned());
            }
            m.values().find_map(|v| control_ref_by(v, label, exact))
        }
        Value::Array(a) => a.iter().find_map(|v| control_ref_by(v, label, exact)),
        _ => None,
    }
}

/// Run a panel's probes in order: open it, then for each probe select,
/// snapshot the project and frames, run the steps, and read the panel, its
/// rows, the playhead, the project and the frames again.
fn probe(
    c: &mut McpClient,
    panel: &str,
    s: &crate::graders::panel::PanelSpec,
    project: Option<&Value>,
    d: &RunDirs,
) -> PanelObservation {
    let mut o = PanelObservation {
        panel: panel.into(),
        ..Default::default()
    };
    if c.call("open_panel", json!({"panelId": panel})).is_err() {
        return o;
    }
    o.opened = true;
    let tag = panel.replace('/', "_");
    for (i, p) in s.probes.iter().enumerate() {
        o.probes
            .push(run_probe(c, panel, p, project, d, &format!("{tag}-{i}")));
    }
    if let Ok(r) = c.call("capture_panel", json!({"panelId": panel}))
        && let Some((_, bytes)) = r.images().into_iter().next()
    {
        let rel = PathBuf::from("frames").join(format!("panel-{tag}.png"));
        if d.anchor.write_new(&rel, &bytes).is_ok() {
            o.capture = Some(rel);
        }
    }
    o
}

fn run_probe(
    c: &mut McpClient,
    panel: &str,
    p: &crate::graders::panel::Probe,
    project: Option<&Value>,
    d: &RunDirs,
    tag: &str,
) -> ProbeObservation {
    let mut o = ProbeObservation::default();
    if !p.select.is_empty() {
        let layers = project
            .map(crate::pmv::project_of)
            .map(crate::graders::project::layers)
            .unwrap_or_default();
        let ids: Vec<&str> = p
            .select
            .iter()
            .filter_map(|n| {
                layers
                    .iter()
                    .find(|l| l["name"].as_str() == Some(n))
                    .and_then(|l| l["id"].as_str())
            })
            .collect();
        let _ = c.call("select_layers", json!({"layerIds": ids}));
    }
    let times = p.frame_times();
    o.project_before = read_saved(c, d).ok();
    if !times.is_empty() {
        o.baseline_frames =
            render(c, &times, d, &format!("panel-{tag}-before")).unwrap_or_default();
    }
    for step in &p.steps {
        let (label, args) = match step {
            Step::Seek(t) => {
                o.steps
                    .push(match c.call("set_playhead", json!({"time": t})) {
                        Ok(_) => "ok".into(),
                        Err(e) => e,
                    });
                continue;
            }
            Step::Click(l) => (l, json!({"action": "click"})),
            Step::Type { field, text } => (field, json!({"action": "fill", "value": text})),
        };
        let state = c
            .call("get_panel_state", json!({"panelId": panel}))
            .ok()
            .and_then(|r| r.json())
            .unwrap_or(Value::Null);
        let Some(r) = control_ref(&state, label) else {
            o.steps.push(format!("no control labelled \"{label}\""));
            continue;
        };
        let mut args = args;
        args["panelId"] = json!(panel);
        args["ref"] = json!(r);
        o.steps.push(match c.call("interact_panel", args) {
            Ok(_) => "ok".into(),
            Err(e) => e,
        });
    }
    if let Ok(Some(state)) = c
        .call("get_panel_state", json!({"panelId": panel}))
        .map(|r| r.json())
    {
        texts(&state, &mut o.text);
        rows(&state, &mut o.rows);
    }
    o.playhead = c
        .call("get_timeline", json!({"limit": 1}))
        .ok()
        .and_then(|r| r.json())
        .as_ref()
        .and_then(playhead);
    o.project_after = read_saved(c, d).ok();
    if !times.is_empty() {
        o.frames = render(c, &times, d, &format!("panel-{tag}-after")).unwrap_or_default();
    }
    o
}
