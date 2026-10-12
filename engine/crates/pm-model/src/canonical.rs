//! The canonical project form: every composition stored uniformly.
//!
//! A stored project keeps the open composition at its top level (`w`, `h`,
//! `fps`, `layers`, ..., plus `compId` and `compName`) and every other
//! composition in `comps` (`legacy/core/compositions.ts`). Opening a
//! composition swaps it with the top level. The engine works on the canonical
//! form instead, where the open composition lives in `comps[compId]` like the
//! others:
//!
//! ```text
//! stored:    { id, name, compId: "C1", compName: "Main", w, h, fps, layers, ..., comps: { C2 } }
//! canonical: { id, name, compId: "C1", comps: { C1: { id: "C1", name: "Main", w, h, fps, layers, ... }, C2 } }
//! ```
//!
//! [`canonical_project`] is normative and matches `canonicalProject` in
//! `apps/desktop/src/shared/canonical-json.ts` byte for byte under the
//! canonical serialization:
//!
//! 1. A value that is not an object, or has no non-empty string `compId`, or
//!    has a `comps` member that is not an object, is returned unchanged.
//! 2. The open composition becomes `{ id: compId, name: compName (when
//!    present), ...each of COMP_FIELDS present on the project }`.
//! 3. The result keeps every other top-level member (including `compId`, the
//!    pointer), drops `compName` and the composition fields, and stores
//!    `comps` with the open composition under `compId`, replacing a stale
//!    entry with that id if one exists.
//!
//! Patches canonicalized by `PM.Comps.canonicalize` ([`canonicalize_patches`])
//! address this form directly, so the engine mirror never needs to know which
//! composition is open. [`realize_project`] inverts the form for well-formed
//! projects (a non-empty string `compId` and a `comps` object without the open
//! composition).
//!
//! The legacy helpers that move between the forms are ported too:
//! [`canonical_path`], [`canonicalize_patches`], [`realize_patches`],
//! [`rotate_project`], and [`normalize_compositions`]. They follow the
//! TypeScript except where it depends on object insertion order, which the
//! engine does not keep (noted on each function).

use crate::json::{JsString, Map, Value, js_trim};
use crate::patch::{Patch, PathPart};

/// Fields that belong to a composition rather than to the project. Must equal
/// `COMP_FIELDS` in `legacy/core/compositions.ts` (a test reads that file).
pub const COMP_FIELDS: [&str; 12] = [
    "w",
    "h",
    "fps",
    "dur",
    "bg",
    "backgroundFill",
    "layers",
    "markers",
    "work",
    "shutter",
    "audioGain",
    "render3d",
];

/// Composition fields that may be absent; cleared rather than inherited when
/// the open composition changes.
pub const OPTIONAL_COMP_FIELDS: [&str; 2] = ["audioGain", "render3d"];

fn is_comp_field(key: &str) -> bool {
    COMP_FIELDS.contains(&key)
}

/// Top-level project key for a key on a stored composition (`COMP_TO_ROOT`).
fn comp_to_root(key: &str) -> Option<&'static str> {
    match key {
        "id" => Some("compId"),
        "name" => Some("compName"),
        _ => COMP_FIELDS.iter().copied().find(|f| *f == key),
    }
}

/// Key on a stored composition for a top-level project key (`ROOT_TO_COMP`).
fn root_to_comp(key: &str) -> Option<&'static str> {
    match key {
        "compId" => Some("id"),
        "compName" => Some("name"),
        _ => COMP_FIELDS.iter().copied().find(|f| *f == key),
    }
}

/// The open composition id of a project: a non-empty string `compId`.
pub fn open_comp_id(project: &Value) -> Option<&str> {
    match project.get("compId") {
        Some(Value::String(s)) if !s.as_bytes().is_empty() => s.as_str(),
        _ => None,
    }
}

fn open_comp_key(project: &Value) -> Option<JsString> {
    match project.get("compId") {
        Some(Value::String(s)) if !s.as_bytes().is_empty() => Some(s.clone()),
        _ => None,
    }
}

/// The project inside a saved document: `document.proj || document`.
pub fn project_of(document: &Value) -> &Value {
    match document.get("proj") {
        Some(proj) if proj.is_truthy() => proj,
        _ => document,
    }
}

/// [`project_of`] for an owned document.
pub fn into_project(document: Value) -> Value {
    match document {
        Value::Object(mut map) if map.get("proj").is_some_and(Value::is_truthy) => {
            map.remove("proj").unwrap_or(Value::Null)
        }
        other => other,
    }
}

/// The canonical project form (see the module documentation).
pub fn canonical_project(project: Value) -> Value {
    let Some(id) = open_comp_key(&project) else {
        return project;
    };
    let Value::Object(map) = project else {
        unreachable!("open_comp_key only succeeds on objects")
    };
    if map.get("comps").is_some_and(|c| c.as_object().is_none()) {
        return Value::Object(map);
    }
    let mut out = Map::new();
    let mut comp = Map::new();
    let mut comps = Map::new();
    comp.insert("id", Value::String(id.clone()));
    for (key, value) in map {
        match key.as_str() {
            Some("compName") => {
                comp.insert("name", value);
            }
            Some("comps") => {
                if let Value::Object(stored) = value {
                    for (cid, c) in stored {
                        if cid != id {
                            comps.insert(cid, c);
                        }
                    }
                }
            }
            Some(k) if is_comp_field(k) => {
                comp.insert(key, value);
            }
            _ => {
                out.insert(key, value);
            }
        }
    }
    comps.insert(id, Value::Object(comp));
    out.insert("comps", Value::Object(comps));
    Value::Object(out)
}

/// The inverse of [`canonical_project`] for well-formed projects. Returns the
/// input unchanged when `comps[compId]` is not an object. Members of
/// `comps[compId]` other than `name` and the composition fields are dropped
/// (as `realizePatches` drops patches to them).
pub fn realize_project(canonical: Value) -> Value {
    let Some(id) = open_comp_key(&canonical) else {
        return canonical;
    };
    let is_realizable = canonical
        .get("comps")
        .and_then(Value::as_object)
        .and_then(|comps| comps.get_key(&id))
        .is_some_and(|c| c.as_object().is_some());
    if !is_realizable {
        return canonical;
    }
    let Value::Object(mut map) = canonical else {
        unreachable!()
    };
    let Some(Value::Object(mut comps)) = map.remove("comps") else {
        unreachable!()
    };
    let Some(Value::Object(comp)) = comps.remove_key(&id) else {
        unreachable!()
    };
    for (key, value) in comp {
        match key.as_str() {
            Some("name") => {
                map.insert("compName", value);
            }
            Some(k) if is_comp_field(k) => {
                map.insert(key, value);
            }
            _ => {}
        }
    }
    map.insert("comps", Value::Object(comps));
    Value::Object(map)
}

/// `canonicalPath(path, rootId)`: a top-level composition path recorded while
/// `root_id` was open, as `['comps', rootId, field, ...]`.
pub fn canonical_path(path: &[PathPart], root_id: Option<&str>) -> Vec<PathPart> {
    let (Some(root), Some(PathPart::Key(first))) =
        (root_id.filter(|r| !r.is_empty()), path.first())
    else {
        return path.to_vec();
    };
    match first.as_str().and_then(root_to_comp) {
        Some(field) => {
            let mut out = vec![
                PathPart::from("comps"),
                PathPart::from(root),
                PathPart::from(field),
            ];
            out.extend_from_slice(&path[1..]);
            out
        }
        None => path.to_vec(),
    }
}

fn is_whole_comps(patch: Option<&Patch>) -> bool {
    matches!(patch, Some(p) if p.path.len() == 1 && matches!(&p.path[0], PathPart::Key(k) if k == "comps"))
}

/// `canonicalizePatches(forward, backward, rootId)`: canonical paths, with
/// whole-map `['comps']` pairs split per composition (the stored map never
/// holds the open composition, so replaying it wholesale after a switch would
/// drop one).
///
/// Differences from the TypeScript, neither of which changes the document a
/// pair produces: split pairs come out in code-point order of composition id
/// rather than insertion order, and a composition counts as unchanged when its
/// canonical serializations match, where TypeScript compares
/// `JSON.stringify` output (which also differs when only key order does).
pub fn canonicalize_patches(
    forward: &[Patch],
    backward: &[Patch],
    root_id: Option<&str>,
) -> (Vec<Patch>, Vec<Patch>) {
    let Some(root) = root_id.filter(|r| !r.is_empty()) else {
        return (forward.to_vec(), backward.to_vec());
    };
    let (mut f, mut b) = (Vec::new(), Vec::new());
    for index in 0..forward.len().max(backward.len()) {
        let (fp, bp) = (forward.get(index), backward.get(index));
        if is_whole_comps(fp) && is_whole_comps(bp) {
            let record = |p: &Patch| -> Map {
                match (&p.exists, &p.value) {
                    (true, Some(Value::Object(m))) => m.clone(),
                    _ => Map::new(),
                }
            };
            let (after, before) = (record(fp.unwrap()), record(bp.unwrap()));
            let mut ids: Vec<&JsString> = after
                .keys()
                .chain(before.keys().filter(|k| after.get_key(k).is_none()))
                .collect();
            ids.sort();
            for id in ids {
                if id == root {
                    continue;
                }
                let (has, had) = (after.get_key(id), before.get_key(id));
                if let (Some(x), Some(y)) = (has, had)
                    && crate::json::to_canonical_string(x) == crate::json::to_canonical_string(y)
                {
                    continue;
                }
                let path = vec![PathPart::from("comps"), PathPart::Key(id.clone())];
                f.push(match has {
                    Some(v) => Patch::set(path.clone(), v.clone()),
                    None => Patch::delete(path.clone()),
                });
                b.push(match had {
                    Some(v) => Patch::set(path.clone(), v.clone()),
                    None => Patch::delete(path),
                });
            }
            continue;
        }
        if let Some(p) = fp {
            f.push(Patch {
                path: canonical_path(&p.path, Some(root)),
                ..p.clone()
            });
        }
        if let Some(p) = bp {
            b.push(Patch {
                path: canonical_path(&p.path, Some(root)),
                ..p.clone()
            });
        }
    }
    (f, b)
}

/// `realizePatches(patches, project)`: canonical patches translated onto a
/// stored project whose open composition is `project.compId`.
pub fn realize_patches(patches: &[Patch], project: &Value) -> Vec<Patch> {
    let root = open_comp_key(project);
    let mut out = Vec::new();
    for patch in patches {
        let path = &patch.path;
        if path.is_empty() {
            let rotated = match (&root, patch.exists, &patch.value) {
                (Some(root), true, Some(value @ Value::Object(_)))
                    if value.get("compId") != Some(&Value::String(root.clone()))
                        && value
                            .get("comps")
                            .and_then(Value::as_object)
                            .and_then(|c| c.get_key(root))
                            .is_some_and(Value::is_truthy) =>
                {
                    Some(rotate_project(value.clone(), root))
                }
                _ => None,
            };
            out.push(match rotated {
                Some(value) => Patch::set(Vec::new(), value),
                None => patch.clone(),
            });
            continue;
        }
        let addresses_root = matches!(
            (&root, path.first(), path.get(1)),
            (Some(r), Some(PathPart::Key(c)), Some(PathPart::Key(id))) if c == "comps" && id == r
        );
        if addresses_root {
            if path.len() == 2 {
                if let (true, Some(Value::Object(value))) = (patch.exists, &patch.value) {
                    for (key, v) in value {
                        if let Some(root_key) = key.as_str().and_then(comp_to_root)
                            && root_key != "compId"
                        {
                            out.push(Patch::set(vec![PathPart::from(root_key)], v.clone()));
                        }
                    }
                    for key in OPTIONAL_COMP_FIELDS {
                        if !value.contains_key(key) {
                            out.push(Patch::delete(vec![PathPart::from(key)]));
                        }
                    }
                }
                continue;
            }
            let field = path[2].property_key();
            if let Some(root_key) = field.as_str().and_then(comp_to_root) {
                let mut realized = vec![PathPart::from(root_key)];
                realized.extend_from_slice(&path[3..]);
                out.push(Patch {
                    path: realized,
                    ..patch.clone()
                });
            }
            continue;
        }
        out.push(patch.clone());
    }
    out
}

/// `rootComp(project)`: the open composition as a stand-alone record.
fn root_comp(project: &Map) -> Map {
    let mut comp = Map::new();
    if let Some(id) = project.get("compId") {
        comp.insert("id", id.clone());
    }
    if let Some(name) = project.get("compName") {
        comp.insert("name", name.clone());
    }
    for key in COMP_FIELDS {
        if let Some(v) = project.get(key) {
            comp.insert(key, v.clone());
        }
    }
    comp
}

/// `rotateProject(project, id)`: the stored project with composition `id`
/// open. Returns the input when `id` is empty, already open, or not a stored
/// composition.
pub fn rotate_project(project: Value, id: &JsString) -> Value {
    if !project.is_truthy() || id.as_bytes().is_empty() {
        return project;
    }
    if project.get("compId") == Some(&Value::String(id.clone())) {
        return project;
    }
    let Some(Value::Object(target)) = project
        .get("comps")
        .and_then(Value::as_object)
        .and_then(|c| c.get_key(id))
        .cloned()
    else {
        return project;
    };
    let Value::Object(mut next) = project else {
        return project;
    };
    let mut comps = Map::new();
    if let Some(Value::Object(stored)) = next.get("comps") {
        for (key, comp) in stored {
            if key != id {
                comps.insert(key.clone(), comp.clone());
            }
        }
    }
    if let Some(open) = next.get("compId").filter(|v| v.is_truthy()) {
        let key = open.to_js_string();
        comps.insert(key, Value::Object(root_comp(&next)));
    }
    next.insert("comps", Value::Object(comps));
    next.insert("compId", Value::String(id.clone()));
    let name = match target.get("name") {
        Some(n) if n.is_truthy() => n.clone(),
        _ => Value::from("Comp"),
    };
    next.insert("compName", name);
    for key in COMP_FIELDS {
        match target.get(key) {
            Some(v) => {
                next.insert(key, v.clone());
            }
            None if OPTIONAL_COMP_FIELDS.contains(&key) => {
                next.remove(key);
            }
            None => {}
        }
    }
    for key in ["layers", "markers"] {
        if next.get(key).and_then(Value::as_array).is_none() {
            next.insert(key, Value::Array(Vec::new()));
        }
    }
    let work_ok = matches!(next.get("work"), Some(Value::Array(w)) if w.len() == 2);
    if !work_ok {
        let dur = next.get("dur").cloned().unwrap_or(Value::Null);
        next.insert("work", Value::Array(vec![Value::from(0), dur]));
    }
    Value::Object(next)
}

/// `normalizeCompositions(project, uid)`: gives the open composition an id
/// and name and flattens nested `comp.comps` maps from older files into the
/// project's one `comps` map.
///
/// When the same composition id appears more than once across nesting levels,
/// TypeScript keeps the first in depth-first insertion order; this keeps the
/// first in depth-first code-point order of ids.
pub fn normalize_compositions(project: &mut Value, mut uid: impl FnMut(&str) -> String) {
    let Value::Object(map) = project else {
        return;
    };
    if open_comp_key(&Value::Object(map.clone())).is_none() {
        map.insert("compId", Value::from(uid("C")));
    }
    let name_ok = matches!(map.get("compName"), Some(Value::String(s)) if s.as_str().is_none_or(|t| !js_trim(t).is_empty()));
    if !name_ok {
        let name = match map.get("name") {
            Some(n) if n.is_truthy() => n.to_js_string(),
            _ => JsString::from("Comp 1"),
        };
        map.insert("compName", Value::String(name));
    }
    let open = open_comp_key(&Value::Object(map.clone())).expect("set above");
    let mut flat = Map::new();
    fn visit(comps: Value, open: &JsString, flat: &mut Map) {
        let Value::Object(comps) = comps else { return };
        for (id, comp) in comps {
            let Value::Object(mut comp) = comp else {
                continue;
            };
            if id == *open {
                continue;
            }
            let nested = comp.insert("comps", Value::Object(Map::new()));
            comp.insert("id", Value::String(id.clone()));
            let first = flat.get_key(&id).is_none();
            if first {
                flat.insert(id.clone(), Value::Object(comp));
            }
            if let Some(nested) = nested {
                visit(nested, open, flat);
            }
        }
    }
    if let Some(comps) = map.remove("comps") {
        visit(comps, &open, &mut flat);
    }
    map.insert("comps", Value::Object(flat));
}
