//! Workspaces are the kernel's v1 records: the `workspaces` setting (presets,
//! saved and project-scoped workspaces), `workspace` (the last activated id),
//! `workspaceTrash`, and each project's saved workspace. There is no other
//! store. A window's [`Layout`] is a view of its active record's
//! `layout.docks`; [`write`] puts an edited layout back and keeps every other
//! field (generated panels, chrome, theme, unknown extension metadata).
//!
//! Panel specs keep v1's shape (`{id, flex | size}` stacked in a dock). A tab
//! stack is native: a spec with `tabbed: true` joins the previous panel's
//! stack, and `active: true` marks its showing tab when that isn’t the first.
use crate::layout::{Dock, Group, Layout, VIEWER, stacks};
use pm_design::workspace::*;
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

/// Built-in workspaces, in menu order (v1's PRESETS ids).
pub const BUILTIN: [&str; 7] = [
    "design",
    "gradient",
    "animate",
    "shaderlab",
    "edit",
    "review",
    "focus",
];
/// The workspace a window falls back to.
pub const FALLBACK: &str = "design";
/// Panels v1 retired from docks (`normalizeWorkspace`).
const RETIRED: [&str; 3] = ["chat", "layers", "library"];

fn panel(id: &str, size: Option<f32>) -> Value {
    match size {
        Some(size) => json!({"id": id, "size": rounded(size)}),
        None => json!({"id": id, "flex": true}),
    }
}
fn tab(id: &str) -> Value {
    json!({"id": id, "tabbed": true})
}
fn dock(id: &str, size: Option<f32>, panels: Vec<Value>) -> Value {
    let mut d = json!({"id": id, "panels": panels});
    match size {
        Some(size) => d["size"] = rounded(size),
        None => d["flex"] = true.into(),
    }
    d
}
fn record(id: &str, name: &str, density: &str, accent: &str, docks: Vec<Value>) -> Value {
    json!({
        "schemaVersion": 1,
        "agentDockMigration": 4,
        "id": id,
        "name": name,
        "builtin": true,
        "scope": "global",
        "projectId": null,
        "density": density,
        "theme": {"accent": accent},
        "features": {"autosave": true},
        "custom": [],
        "hiddenPanels": [],
        "layout": {"docks": docks},
    })
}

/// A built-in workspace as first shipped (v1's PRESETS metadata, native
/// geometry), or `None` for any other id.
pub fn preset(id: &str) -> Option<Value> {
    let viewer = || dock("center", None, vec![panel(VIEWER, None)]);
    let mut r = match id {
        "design" => record(
            "design",
            "Design",
            "normal",
            "#FF6B1A",
            vec![
                dock(
                    "left",
                    Some(DEFAULT_LEFT),
                    vec![panel("assets", None), tab("layer-effects")],
                ),
                viewer(),
                dock(
                    "bottom",
                    Some(DEFAULT_TIMELINE),
                    vec![panel("timeline", None)],
                ),
                dock(
                    "right",
                    Some(DEFAULT_RIGHT),
                    vec![
                        panel("inspector", None),
                        panel("fxbrowser", Some(DEFAULT_EFFECTS)),
                    ],
                ),
            ],
        ),
        "gradient" => record(
            "gradient",
            "Gradient",
            "compact",
            "#FF6B1A",
            vec![
                dock(
                    "left",
                    Some(300.),
                    vec![
                        panel("gradient-controls", None),
                        panel("assets", Some(190.)),
                    ],
                ),
                viewer(),
                dock("bottom", Some(300.), vec![panel("timeline", None)]),
                dock("right", Some(340.), vec![panel("inspector", None)]),
            ],
        ),
        "animate" => record(
            "animate",
            "Animate",
            "compact",
            "#FF6B1A",
            vec![
                dock(
                    "left",
                    Some(230.),
                    vec![panel("takes", None), panel("assets", Some(170.))],
                ),
                dock(
                    "center",
                    None,
                    vec![panel(VIEWER, Some(300.)), panel("timeline", None)],
                ),
                dock("right", Some(320.), vec![panel("inspector", None)]),
            ],
        ),
        "shaderlab" => record(
            "shaderlab",
            "Shader Lab",
            "compact",
            "#4C8DFF",
            vec![
                dock("left", Some(460.), vec![panel("shader", None)]),
                viewer(),
                dock("bottom", Some(200.), vec![panel("timeline", None)]),
                dock(
                    "right",
                    Some(290.),
                    vec![panel("inspector", None), panel("perf", Some(190.))],
                ),
            ],
        ),
        "edit" => record(
            "edit",
            "Edit",
            "normal",
            "#3FCF8E",
            vec![
                dock(
                    "left",
                    Some(250.),
                    vec![panel("assets", None), panel("fxbrowser", Some(240.))],
                ),
                viewer(),
                dock("bottom", Some(360.), vec![panel("timeline", None)]),
                dock("right", Some(280.), vec![panel("inspector", None)]),
            ],
        ),
        "review" => record(
            "review",
            "Review",
            "comfy",
            "#FF6B1A",
            vec![
                dock(
                    "center",
                    None,
                    vec![panel(VIEWER, None), panel("timeline", Some(180.))],
                ),
                dock("right", Some(300.), vec![panel("notes", None)]),
            ],
        ),
        "focus" => record("focus", "Focus", "normal", "#FF6B1A", vec![viewer()]),
        _ => return None,
    };
    match id {
        "design" | "gradient" => {
            r["features"]["adaptiveQuality"] = true.into();
        }
        "animate" => r["features"]["graphOnOpen"] = true.into(),
        _ => {}
    }
    if id == "gradient" {
        r["theme"]["radius"] = 10.into();
        // v1's generated Gradient Editor, bound to the composition background.
        r["custom"] = json!([{
            "id": "gradient-controls", "title": "Gradient Editor", "size": 220, "state": {},
            "controls": [
                {"type": "select", "label": "Type", "target": "$composition", "path": "composition.background.type", "options": ["linear", "radial"], "def": "linear"},
                {"type": "color", "label": "Start color", "target": "$composition", "path": "composition.background.startColor", "def": "#FF6B1A"},
                {"type": "color", "label": "End color", "target": "$composition", "path": "composition.background.endColor", "def": "#34144F"},
                {"type": "slider", "label": "Angle", "target": "$composition", "path": "composition.background.angle", "min": -180, "max": 180, "def": 0, "step": 1},
                {"type": "slider", "label": "Midpoint", "target": "$composition", "path": "composition.background.midpoint", "min": 1, "max": 100, "def": 100, "step": 1},
            ],
        }]);
    }
    Some(r)
}

/// v1's `WS.init`: stored records first (a saved built-in keeps the person's
/// sizes and panels), then any built-in they lack. Malformed entries are kept
/// for recovery but never offered.
pub fn seeded(stored: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = stored.to_vec();
    for id in BUILTIN {
        if !out.iter().any(|r| record_id(r) == Some(id)) {
            out.extend(preset(id));
        }
    }
    out
}
pub fn record_id(record: &Value) -> Option<&str> {
    record
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
}
pub fn record_name(record: &Value) -> &str {
    record
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| record_id(record))
        .unwrap_or("Workspace")
}
pub fn is_builtin(record: &Value) -> bool {
    record.get("builtin") == Some(&Value::Bool(true))
}
/// Has dock data a window can show (anything else falls back).
pub fn usable(record: &Value) -> bool {
    record_id(record).is_some()
        && record
            .pointer("/layout/docks")
            .and_then(Value::as_array)
            .is_some_and(|d| !d.is_empty())
}

fn size(v: Option<&Value>) -> Option<f32> {
    v.and_then(Value::as_f64)
        .filter(|n| n.is_finite() && *n > 0.)
        .map(|n| n as f32)
}

/// The window layout `record` describes. Never fails: v1's normalisation
/// rules apply (retired and duplicate panels dropped, the viewer always
/// present), and a record without docks shows the viewer alone. Panels no one
/// provides stay in place; the shell shows a placeholder naming them.
pub fn layout(record: &Value) -> Layout {
    let mut seen = BTreeSet::new();
    let mut docks: Vec<Dock> = vec![];
    let specs = record
        .pointer("/layout/docks")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    for (index, d) in specs.iter().enumerate() {
        let mut groups: Vec<Group> = vec![];
        for spec in d
            .get("panels")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let (id, fields) = match spec {
                Value::String(s) => (s.trim(), None),
                Value::Object(o) => (
                    o.get("id").and_then(Value::as_str).unwrap_or("").trim(),
                    Some(o),
                ),
                _ => continue,
            };
            if id.is_empty() || RETIRED.contains(&id) || !seen.insert(id.to_owned()) {
                continue;
            }
            let flag = |k: &str| fields.is_some_and(|o| o.get(k) == Some(&Value::Bool(true)));
            match groups.last_mut() {
                Some(g) if flag("tabbed") && stacks(id) && g.tabs.iter().all(|t| stacks(t)) => {
                    g.tabs.push(id.to_owned());
                    if flag("active") {
                        g.active = g.tabs.len() - 1;
                    }
                }
                _ => groups.push(Group {
                    tabs: vec![id.to_owned()],
                    active: 0,
                    size: if flag("flex") {
                        None
                    } else {
                        size(fields.and_then(|o| o.get("size")))
                    },
                }),
            }
        }
        if groups.is_empty() {
            continue;
        }
        if groups.iter().all(|g| g.size.is_some()) {
            groups[0].size = None;
        }
        let mut id = d
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map_or_else(|| format!("dock-{}", index + 1), str::to_owned);
        while docks.iter().any(|x| x.id == id) {
            id = format!("{id}-{}", index + 1);
        }
        docks.push(Dock {
            id,
            size: size(d.get("size")),
            groups,
            collapsed: d.get("hidden") == Some(&Value::Bool(true)),
        });
    }
    if !seen.contains(VIEWER) {
        let viewer = Group {
            tabs: vec![VIEWER.into()],
            active: 0,
            size: None,
        };
        match docks.iter().position(|d| d.id == "center") {
            Some(i) => docks[i].groups.insert(0, viewer),
            None => {
                let at = docks
                    .iter()
                    .position(|d| d.id == "left")
                    .map_or(0, |i| i + 1);
                docks.insert(
                    at,
                    Dock {
                        id: "center".into(),
                        size: None,
                        groups: vec![viewer],
                        collapsed: false,
                    },
                );
            }
        }
    }
    // The viewer's dock always shows: a workspace never hides the composition.
    for d in &mut docks {
        if d.groups.iter().any(|g| g.tabs.iter().any(|t| t == VIEWER)) {
            d.collapsed = false;
        }
    }
    Layout {
        version: 1,
        workspace: record_id(record).unwrap_or(FALLBACK).to_owned(),
        docks,
    }
}

/// A size as the kernel stores it: tenths of a point, a whole number as an
/// integer, so a record reads back exactly as it was written.
fn rounded(n: f32) -> Value {
    let n = (f64::from(n) * 10.).round() / 10.;
    if n.fract() == 0. && n.abs() < 1e15 {
        json!(n as i64)
    } else {
        json!(n)
    }
}
/// `record` with its docks replaced by `layout`. Each panel and dock keeps its
/// own extra fields (a title, a `min`, extension metadata); only placement
/// fields are rewritten. A panel that is shown again leaves `hiddenPanels`.
pub fn write(layout: &Layout, record: &Value) -> Value {
    let mut out = record.as_object().cloned().unwrap_or_default();
    let old_docks = record
        .pointer("/layout/docks")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let hidden = record
        .get("hiddenPanels")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let old_spec = |id: &str| -> Map<String, Value> {
        old_docks
            .iter()
            .filter_map(|d| d.get("panels")?.as_array())
            .flatten()
            .chain(hidden.iter().filter_map(|h| h.get("spec")))
            .find_map(|s| s.as_object().filter(|o| o.get("id") == Some(&json!(id))))
            .cloned()
            .unwrap_or_default()
    };
    let docks: Vec<Value> = layout
        .docks
        .iter()
        .map(|d| {
            let mut o = old_docks
                .iter()
                .find(|x| x.get("id").and_then(Value::as_str) == Some(d.id.as_str()))
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            o.insert("id".into(), d.id.clone().into());
            match d.size {
                Some(n) => o.insert("size".into(), rounded(n)),
                None => o.remove("size"),
            };
            if d.collapsed {
                o.insert("hidden".into(), true.into());
            } else {
                o.remove("hidden");
            }
            if d.id == "center" {
                o.insert("flex".into(), true.into());
            }
            let mut panels = vec![];
            for g in &d.groups {
                for (i, id) in g.tabs.iter().enumerate() {
                    let mut s = old_spec(id);
                    for k in ["flex", "size", "tabbed", "active"] {
                        s.remove(k);
                    }
                    s.insert("id".into(), id.clone().into());
                    if i == 0 {
                        match g.size {
                            Some(n) => s.insert("size".into(), rounded(n)),
                            None => s.insert("flex".into(), true.into()),
                        };
                    } else {
                        s.insert("tabbed".into(), true.into());
                    }
                    if i > 0 && i == g.active {
                        s.insert("active".into(), true.into());
                    }
                    panels.push(Value::Object(s));
                }
            }
            o.insert("panels".into(), panels.into());
            Value::Object(o)
        })
        .collect();
    let open: BTreeSet<&str> = layout.open_panels().into_iter().collect();
    if !hidden.is_empty() {
        out.insert(
            "hiddenPanels".into(),
            hidden
                .into_iter()
                .filter(|h| {
                    h.get("id")
                        .and_then(Value::as_str)
                        .is_none_or(|id| !open.contains(id))
                })
                .collect::<Vec<_>>()
                .into(),
        );
    }
    let mut l = out
        .get("layout")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    l.insert("docks".into(), docks.into());
    out.insert("layout".into(), Value::Object(l));
    Value::Object(out)
}

/// A Workspace Definition as typed (S100): JSON for an object whose
/// `layout.docks`, when present, is a list of docks with panel lists. The
/// message says what to fix; nothing is applied until this passes.
pub fn check_definition(text: &str) -> Result<Value, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| format!("This isn’t valid JSON. {e}"))?;
    let bad = |why: &str| Err(format!("This workspace can’t be used. {why}"));
    if !value.is_object() {
        return bad("A workspace is an object in braces.");
    }
    if let Some(layout) = value.get("layout") {
        let Some(docks) = layout.get("docks") else {
            return bad("Its layout needs a list of docks.");
        };
        let Some(docks) = docks.as_array() else {
            return bad("Its docks must be a list.");
        };
        if docks
            .iter()
            .any(|d| !d.is_object() || !d.get("panels").is_some_and(Value::is_array))
        {
            return bad("Each dock needs a list of panels.");
        }
    }
    if value.get("custom").is_some_and(|c| !c.is_array()) {
        return bad("Its generated panels (custom) must be a list.");
    }
    Ok(value)
}
/// The generated panels `record` defines (its `custom` sections): id, title.
pub fn generated(record: &Value) -> Vec<(String, String)> {
    record
        .get("custom")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let id = c.get("id")?.as_str()?.trim();
            let title = ["title", "name"]
                .iter()
                .find_map(|k| c.get(*k)?.as_str().map(str::trim).filter(|t| !t.is_empty()))
                .unwrap_or("Controls");
            (!id.is_empty()).then(|| (id.to_owned(), title.to_owned()))
        })
        .collect()
}
/// A new record id, unique in practice (v1's `PM.uid('ws')` shape).
pub fn new_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    let n = nanos
        ^ (NEXT
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15));
    format!("ws{:x}", n & 0xFFFF_FFFF_FFFF)
}

/// What a window knows of the kernel's workspace store.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Store {
    /// The `workspaces` setting, seeded with any missing built-ins.
    pub records: Vec<Value>,
    /// The `workspace` setting: the last activated id.
    pub current: Option<String>,
    /// The `defaultWorkspace` setting: what a project without its own starts with.
    pub default: Option<String>,
    /// The window's project's saved workspace, when it has one.
    pub project: Option<Value>,
    /// The window's project id (project-scoped records of other projects hide).
    pub project_id: Option<String>,
}
impl Store {
    pub fn local() -> Self {
        Self {
            records: seeded(&[]),
            ..Default::default()
        }
    }
    pub fn get(&self, id: &str) -> Option<&Value> {
        self.records
            .iter()
            .find(|r| record_id(r) == Some(id) && usable(r))
    }
    /// The records a person can pick in this window: built-ins in menu order,
    /// then the rest in stored order. A project-scoped record shows only in
    /// its own project.
    pub fn offered(&self) -> Vec<&Value> {
        let mut out: Vec<&Value> = BUILTIN.iter().filter_map(|id| self.get(id)).collect();
        out.extend(self.records.iter().filter(|r| {
            usable(r)
                && !record_id(r).is_some_and(|id| BUILTIN.contains(&id))
                && (r.get("scope").and_then(Value::as_str) != Some("project")
                    || r.get("projectId").and_then(Value::as_str) == self.project_id.as_deref())
        }));
        out
    }
    /// The record a window shows: its project's own workspace, else `preferred`
    /// (the window's last choice), else, for a project without its own, the
    /// default for new projects, else the last activated one (v1's `workspace`).
    pub fn active(&self, preferred: Option<&str>) -> Value {
        if let Some(p) = self.project.as_ref().filter(|p| usable(p)) {
            return p.clone();
        }
        let default = self
            .default
            .as_deref()
            .filter(|_| self.project_id.is_some());
        [preferred, default, self.current.as_deref()]
            .into_iter()
            .flatten()
            .find_map(|id| self.get(id))
            .or_else(|| self.get(FALLBACK))
            .cloned()
            .or_else(|| preset(FALLBACK))
            .unwrap_or_default()
    }
    /// Applies `write` to this copy (a window without a kernel, and the
    /// window's view until the kernel publishes the write).
    pub fn apply(&mut self, write: &Write) {
        let upsert = |records: &mut Vec<Value>, record: &Value| {
            let id = record_id(record);
            match records.iter_mut().find(|r| record_id(r) == id) {
                Some(r) => *r = record.clone(),
                None => records.push(record.clone()),
            }
        };
        match write {
            Write::Active { after, .. } => {
                upsert(&mut self.records, after);
                self.current = record_id(after).map(str::to_owned);
                if self.project_id.is_some() {
                    self.project = Some(after.clone());
                }
            }
            Write::Upsert(record) => upsert(&mut self.records, record),
            Write::Archive(id) => self.records.retain(|r| record_id(r) != Some(id)),
            Write::Default(id) => self.default = Some(id.clone()),
        }
    }
}
/// One change to the kernel's workspace store.
#[derive(Clone, Debug, PartialEq)]
pub enum Write {
    /// The window's workspace became `after` (a layout edit or an activation):
    /// it is the project's saved workspace, its record in `workspaces`, and the
    /// last activated one. `before` is the project workspace the window last
    /// read; a writer that landed since wins, and the window shows theirs.
    Active { before: Value, after: Value },
    /// Adds or replaces a record in `workspaces`.
    Upsert(Value),
    /// Moves a record to `workspaceTrash` (v1's `WS.remove`).
    Archive(String),
    /// New projects start with this workspace (`defaultWorkspace`).
    Default(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_round_trip_through_layout_and_write() {
        for id in BUILTIN {
            let r = preset(id).unwrap();
            let l = layout(&r);
            l.validate().unwrap();
            assert_eq!(l.workspace, id);
            assert_eq!(write(&l, &r), r, "{id}");
        }
    }

    #[test]
    fn a_v1_record_lands_where_v1_put_it_and_keeps_its_metadata() {
        let r = json!({
            "id": "ws-1", "name": "Grading", "custom": [{"id": "custom-1", "title": "Knobs"}],
            "chrome": {"timeline": {"rowHeight": 22}}, "mine": {"x": 1},
            "layout": {"docks": [
                {"id": "tools", "size": 220, "panels": [{"id": "scopes", "size": 200, "min": 80}, {"id": "custom-1", "flex": true}]},
                {"id": "main", "flex": true, "panels": ["viewer", {"id": "timeline", "size": 240}, {"id": "agent"}, {"id": "chat"}]},
                {"id": "right", "size": 320, "panels": [{"id": "inspector", "flex": true}, {"id": "inspector"}]},
            ], "extra": true},
        });
        let l = layout(&r);
        let ids: Vec<_> = l.docks.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["tools", "main", "right"]);
        assert_eq!(l.docks[0].size, Some(220.));
        assert_eq!(l.docks[0].groups[0].size, Some(200.));
        assert_eq!(l.docks[0].groups[1].size, None);
        assert_eq!(
            l.open_panels(),
            [
                "scopes",
                "custom-1",
                "viewer",
                "timeline",
                "agent",
                "inspector"
            ]
        );
        let w = write(&l, &r);
        assert_eq!(w["custom"], r["custom"]);
        assert_eq!(w["chrome"], r["chrome"]);
        assert_eq!(w["mine"], r["mine"]);
        assert_eq!(w["layout"]["extra"], true);
        assert_eq!(w["layout"]["docks"][0]["panels"][0]["min"], 80);
        assert_eq!(layout(&w), l);
    }

    #[test]
    fn missing_viewer_and_empty_records_still_show_the_composition() {
        let l = layout(
            &json!({"id": "x", "layout": {"docks": [{"id": "left", "panels": ["assets"]}]}}),
        );
        assert_eq!(l.open_panels(), ["assets", "viewer"]);
        l.validate().unwrap();
        let l = layout(&json!(null));
        assert_eq!(l.open_panels(), ["viewer"]);
        l.validate().unwrap();
    }

    #[test]
    fn written_sizes_read_back_exactly_after_the_kernel_round_trip() {
        let mut l = layout(&preset("edit").unwrap());
        l.docks[0].size = Some(312.);
        l.docks[3].size = Some(287.25);
        let w = write(&l, &preset("edit").unwrap());
        // The kernel writes canonical JSON; integral numbers come back as integers.
        let back: Value = serde_json::from_str(&w.to_string().replace(".0,", ",")).unwrap();
        assert_eq!(back, w);
        assert_eq!(w["layout"]["docks"][0]["size"], json!(312));
        assert_eq!(w["layout"]["docks"][3]["size"], json!(287.3));
    }

    #[test]
    fn tabs_round_trip_with_their_showing_tab() {
        let mut l = layout(&preset("design").unwrap());
        l.docks[0].groups[0].active = 1;
        let w = write(&l, &preset("design").unwrap());
        assert_eq!(layout(&w), l);
    }

    #[test]
    fn store_offers_builtins_then_saved_and_hides_other_projects_records() {
        let mut s = Store {
            records: seeded(&[
                json!({"id": "mine", "name": "Mine", "layout": {"docks": [{"id": "center", "panels": ["viewer"]}]}}),
                json!({"id": "theirs", "scope": "project", "projectId": "p2", "layout": {"docks": [{"id": "center", "panels": ["viewer"]}]}}),
                json!({"id": "broken"}),
            ]),
            project_id: Some("p1".into()),
            ..Default::default()
        };
        let names: Vec<_> = s.offered().iter().map(|r| record_name(r)).collect();
        assert_eq!(
            names,
            [
                "Design",
                "Gradient",
                "Animate",
                "Shader Lab",
                "Edit",
                "Review",
                "Focus",
                "Mine"
            ]
        );
        assert_eq!(record_id(&s.active(Some("mine"))), Some("mine"));
        assert_eq!(record_id(&s.active(Some("broken"))), Some("design"));
        s.apply(&Write::Archive("mine".into()));
        assert!(s.get("mine").is_none());
    }
}
