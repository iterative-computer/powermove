//! Seeding a clean profile and collecting evidence over the host's MCP tools.
//! Anything the host cannot report yet becomes a note, and the graders that
//! needed it answer Unavailable: nothing is guessed into a pass.
use super::mcp::McpClient;
use crate::artifacts::{
    Artifacts, ChangedExtension, ExtensionHealth, Frame, FrameTiming, PanelObservation,
    Registration, ThemeObservation,
};
use crate::graders::panel::Step;
use crate::graders::{Context, GraderSpec};
use crate::suite::{Task, tree_hash};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// The host has one window when headless.
pub const HEADLESS_WINDOW: &str = "headless";
const TIMING_SAMPLES: usize = 7;

pub struct RunDirs {
    pub root: PathBuf,
    pub profile: PathBuf,
    pub exports: PathBuf,
    pub work: PathBuf,
    pub home: PathBuf,
    pub frames: PathBuf,
    pub host_log: PathBuf,
}

impl RunDirs {
    pub fn create(root: &Path) -> Result<Self, String> {
        let root = std::path::absolute(root).map_err(|e| e.to_string())?;
        let d = RunDirs {
            profile: root.join("profile"),
            exports: root.join("exports"),
            work: root.join("work"),
            home: super::harness::home_in(&root),
            frames: root.join("frames"),
            host_log: root.join("host.log"),
            root,
        };
        for p in [&d.profile, &d.exports, &d.work, &d.home, &d.frames] {
            std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
        }
        Ok(d)
    }
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    if from.is_file() {
        std::fs::copy(from, to).map_err(|e| format!("{}: {e}", from.display()))?;
        return Ok(());
    }
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for e in std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))? {
        let e = e.map_err(|e| e.to_string())?;
        copy_tree(&e.path(), &to.join(e.file_name()))?;
    }
    Ok(())
}

/// Put the fixture into the clean profile: the project as a file entry of the
/// v1.2 library (`store/projects.json` row plus `projectMeta.<id>.file`, the
/// file route of p0-decisions), and the fixture's extensions under
/// `extensions/`. Returns the project id.
pub fn seed_profile(task: &Task, data_root: &Path, d: &RunDirs) -> Result<String, String> {
    let fixtures = data_root.join("fixtures");
    let src = fixtures.join(&task.fixture.project);
    let file = d
        .work
        .join(src.file_name().ok_or("fixture has no file name")?);
    std::fs::copy(&src, &file).map_err(|e| format!("{}: {e}", src.display()))?;
    let bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    let doc = crate::pmv::read_document(&bytes)?;
    let project = crate::pmv::project_of(&doc);
    let id = project["id"]
        .as_str()
        .ok_or("fixture project has no id")?
        .to_owned();
    let store = d.profile.join("store");
    std::fs::create_dir_all(&store).map_err(|e| e.to_string())?;
    let write = |name: String, v: Value| {
        std::fs::write(store.join(name), v.to_string()).map_err(|e| e.to_string())
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
        copy_tree(&from, &d.profile.join("extensions").join(name))?;
    }
    Ok(id)
}

/// What the baseline step learned, for comparison after the run.
#[derive(Default)]
pub struct Baseline {
    pub extension_hashes: BTreeMap<String, String>,
    pub registered: BTreeSet<(String, String)>,
    pub render_ms: Vec<f64>,
    pub timing_time: Option<f64>,
}

fn extension_hashes(profile: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for root in ["extensions", "extensions-shared"] {
        if let Ok(rd) = std::fs::read_dir(profile.join(root)) {
            for e in rd.flatten() {
                if e.path().is_dir()
                    && let Ok(h) = tree_hash(&e.path())
                {
                    out.insert(e.file_name().to_string_lossy().into_owned(), h);
                }
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

fn render<R: Read, W: Write>(
    c: &mut McpClient<R, W>,
    times: &[f64],
    dir: &Path,
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
            let name = format!("{prefix}-{t}.png");
            std::fs::write(dir.join(&name), bytes).map_err(|e| e.to_string())?;
            out.push(Frame {
                time: t,
                png: PathBuf::from("frames").join(name),
            });
        }
    }
    Ok(out)
}

fn time_renders<R: Read, W: Write>(c: &mut McpClient<R, W>, t: f64) -> Result<Vec<f64>, String> {
    // One warm-up render, then the samples.
    c.call("render_frames", json!({"times": [t], "width": 1280}))?;
    let mut ms = Vec::new();
    for _ in 0..TIMING_SAMPLES {
        let s = Instant::now();
        c.call("render_frames", json!({"times": [t], "width": 1280}))?;
        ms.push(s.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(ms)
}

fn registered_effects(workspace: &Value) -> Vec<(String, usize)> {
    workspace["registeredEffects"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            Some((
                e["id"].as_str()?.to_owned(),
                e["parameters"].as_array().map_or(0, Vec::len),
            ))
        })
        .collect()
}

fn registered_panels(layout: &Value) -> Vec<String> {
    // Any object in the layout with an `id` and a `title` is a registered panel.
    let mut out = Vec::new();
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(m) => {
                if let (Some(id), true) =
                    (m.get("id").and_then(Value::as_str), m.contains_key("title"))
                {
                    out.push(id.to_owned());
                }
                m.values().for_each(|x| walk(x, out));
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    walk(layout, &mut out);
    out.sort();
    out.dedup();
    out
}

/// Open the seeded project, then capture the fixture's state and frames.
pub fn baseline<R: Read, W: Write>(
    task: &Task,
    project_id: &str,
    d: &RunDirs,
    c: &mut McpClient<R, W>,
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
    let file = std::fs::read_dir(&d.work)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "pmv"))
        .ok_or("no working project file")?;
    a.project_before = Some(crate::pmv::read_document(
        &std::fs::read(&file).map_err(|e| e.to_string())?,
    )?);
    let mut b = Baseline {
        extension_hashes: extension_hashes(&d.profile),
        ..Default::default()
    };
    match c.call("get_workspace_state", json!({})).map(|r| r.json()) {
        Ok(Some(w)) => {
            for (id, _) in registered_effects(&w) {
                b.registered.insert(("effect".into(), id));
            }
        }
        _ => a
            .notes
            .push("get_workspace_state unavailable before the run".into()),
    }
    if let Ok(Some(l)) = c.call("get_panel_layout", json!({})).map(|r| r.json()) {
        for id in registered_panels(&l) {
            b.registered.insert(("panel".into(), id));
        }
    }
    let times = frame_times(task);
    if !times.is_empty() {
        match render(c, &times, &d.frames, "baseline") {
            Ok(f) => a.baseline_frames = f,
            Err(e) => a.notes.push(format!("baseline frames: {e}")),
        }
    }
    if task
        .graders
        .iter()
        .any(|g| matches!(g, GraderSpec::FrameTime(_)))
    {
        let t = times.first().copied().unwrap_or(0.0);
        b.timing_time = Some(t);
        match time_renders(c, t) {
            Ok(ms) => b.render_ms = ms,
            Err(e) => a.notes.push(format!("baseline frame timing: {e}")),
        }
    }
    Ok(b)
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

fn read_saved<R: Read, W: Write>(c: &mut McpClient<R, W>) -> Result<Value, String> {
    let r = c.call("save_project", json!({}))?;
    let path = r
        .json()
        .as_ref()
        .and_then(saved_path)
        .or_else(|| saved_path(&Value::String(r.text())))
        .ok_or_else(|| format!("save_project did not name a file: {}", r.text()))?;
    crate::pmv::read_document(
        &std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?,
    )
}

/// Extensions in the profile whose source mentions `id` (attribution until
/// the host reports each registration's owner).
fn owner_of(profile: &Path, changed: &[ChangedExtension], id: &str) -> Option<String> {
    changed.iter().map(|c| c.id.clone()).find(|ext| {
        let dir = profile.join("extensions").join(ext);
        let mut hit = false;
        let _ = visit(&dir, &mut |p| {
            if let Ok(s) = std::fs::read_to_string(p) {
                hit |= s.contains(id);
            }
        });
        hit
    })
}

fn visit(dir: &Path, f: &mut dyn FnMut(&Path)) -> std::io::Result<()> {
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.is_dir() {
            visit(&p, f)?;
        } else {
            f(&p);
        }
    }
    Ok(())
}

/// Who registered a contribution: present before the run means it was not
/// the run's (`existing`); otherwise the changed extension that mentions it.
fn owner(
    b: &Baseline,
    kind: &str,
    id: &str,
    profile: &Path,
    changed: &[ChangedExtension],
) -> String {
    if b.registered.contains(&(kind.to_owned(), id.to_owned())) {
        return "existing".into();
    }
    owner_of(profile, changed, id).unwrap_or_else(|| "?".into())
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

fn texts(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) if !s.trim().is_empty() => out.push(s.clone()),
        Value::Object(m) => m.values().for_each(|x| texts(x, out)),
        Value::Array(a) => a.iter().for_each(|x| texts(x, out)),
        _ => {}
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

/// After the harness: save and read the project back, render, and report
/// extensions, frame times, panel probes and the active theme.
pub fn after<R: Read, W: Write>(
    task: &Task,
    _project_id: &str,
    d: &RunDirs,
    b: &Baseline,
    c: &mut McpClient<R, W>,
    a: &mut Artifacts,
) {
    match read_saved(c) {
        Ok(p) => a.project_after = Some(p),
        Err(e) => a.notes.push(format!("project after the run: {e}")),
    }
    let times = frame_times(task);
    if !times.is_empty() {
        match render(c, &times, &d.frames, "after") {
            Ok(f) => a.frames = f,
            Err(e) => a.notes.push(format!("frames after the run: {e}")),
        }
    }
    // Extensions: changed by tree hash.
    let now = extension_hashes(&d.profile);
    let mut changed = Vec::new();
    for (id, h) in &now {
        match b.extension_hashes.get(id) {
            None => changed.push(ChangedExtension {
                id: id.clone(),
                action: "created".into(),
            }),
            Some(old) if old != h => changed.push(ChangedExtension {
                id: id.clone(),
                action: "updated".into(),
            }),
            _ => {}
        }
    }
    for id in b.extension_hashes.keys().filter(|k| !now.contains_key(*k)) {
        changed.push(ChangedExtension {
            id: id.clone(),
            action: "removed".into(),
        });
    }
    a.extensions.changed = changed;
    a.extensions.windows = vec![HEADLESS_WINDOW.into()];
    let workspace = c
        .call("get_workspace_state", json!({}))
        .ok()
        .and_then(|r| r.json());
    match &workspace {
        None => a
            .notes
            .push("get_workspace_state unavailable after the run".into()),
        Some(w) => {
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
            if a.extensions.health.is_empty() {
                a.notes
                    .push("the host reports no extension health (headless health is null)".into());
            }
            for (id, params) in registered_effects(w) {
                let ext = owner(b, "effect", &id, &d.profile, &a.extensions.changed);
                a.extensions.registered.push(Registration {
                    kind: "effect".into(),
                    id,
                    extension: ext,
                    params,
                });
            }
            if let Some(t) = w.get("theme").filter(|t| t.is_object()) {
                a.theme = Some(ThemeObservation {
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
                });
                if let Some(theme) = &a.theme {
                    a.extensions.registered.push(Registration {
                        kind: "theme".into(),
                        id: theme.active.clone(),
                        extension: theme.extension.clone().unwrap_or_else(|| {
                            owner_of(&d.profile, &a.extensions.changed, &theme.active)
                                .unwrap_or_else(|| "?".into())
                        }),
                        params: 0,
                    });
                }
            }
        }
    }
    if let Ok(Some(l)) = c.call("get_panel_layout", json!({})).map(|r| r.json()) {
        for id in registered_panels(&l) {
            let ext = owner(b, "panel", &id, &d.profile, &a.extensions.changed);
            a.extensions.registered.push(Registration {
                kind: "panel".into(),
                id,
                extension: ext,
                params: 0,
            });
        }
    }
    if task
        .graders
        .iter()
        .any(|g| matches!(g, GraderSpec::Theme(_)))
        && a.theme.is_none()
    {
        a.notes
            .push("the host does not report the active theme and its tokens yet".into());
    }
    // Frame time of each budgeted target, against the baseline renders.
    for g in &task.graders {
        let GraderSpec::FrameTime(s) = g else {
            continue;
        };
        let ids = Context {
            artifacts: a,
            run_dir: &d.root,
            data_root: Path::new(""),
        }
        .resolve(&s.target);
        let (Some(t), false) = (b.timing_time, b.render_ms.is_empty()) else {
            continue;
        };
        let base = crate::image::percentile(&b.render_ms, 50.0);
        match time_renders(c, t) {
            Ok(ms) => {
                for id in ids {
                    a.frame_times.push(FrameTiming {
                        target: id,
                        ms: ms.iter().map(|m| (m - base).max(0.0)).collect(),
                        method: format!(
                            "render_frames at {t}s, 1280 wide, {TIMING_SAMPLES} samples minus the fixture's median {base:.2} ms (includes every change the run made)"
                        ),
                    })
                }
            }
            Err(e) => a.notes.push(format!("frame timing: {e}")),
        }
    }
    for g in &task.graders {
        let GraderSpec::Panel(s) = g else { continue };
        let ids = Context {
            artifacts: a,
            run_dir: &d.root,
            data_root: Path::new(""),
        }
        .resolve(&s.panel);
        let Some(id) = ids.first().cloned() else {
            continue;
        };
        let obs = probe(c, &id, s, a.project_after.as_ref(), d);
        a.panels.push(obs);
    }
}

fn probe<R: Read, W: Write>(
    c: &mut McpClient<R, W>,
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
    if !s.probe.select.is_empty() {
        let layers = project
            .map(crate::pmv::project_of)
            .map(crate::graders::project::layers)
            .unwrap_or_default();
        let ids: Vec<&str> = s
            .probe
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
    for step in &s.probe.steps {
        let state = c
            .call("get_panel_state", json!({"panelId": panel}))
            .ok()
            .and_then(|r| r.json())
            .unwrap_or(Value::Null);
        let (label, args) = match step {
            Step::Click(l) => (l, json!({"action": "click"})),
            Step::Type { field, text } => (field, json!({"action": "fill", "value": text})),
        };
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
    }
    o.playhead = c
        .call("get_timeline", json!({"limit": 1}))
        .ok()
        .and_then(|r| r.json())
        .as_ref()
        .and_then(playhead);
    o.project_after = read_saved(c).ok();
    if let Ok(r) = c.call("capture_panel", json!({"panelId": panel}))
        && let Some((_, bytes)) = r.images().into_iter().next()
    {
        let p = d
            .frames
            .join(format!("panel-{}.png", panel.replace('/', "_")));
        if std::fs::write(&p, bytes).is_ok() {
            o.capture = Some(PathBuf::from("frames").join(p.file_name().unwrap()));
        }
    }
    o
}
