//! Pre-compose (`legacy/core/compositions.ts`, `precompose`).
//!
//! Move All puts the layers in a new composition with the host's settings;
//! one pre-comp layer takes their place. A parent or group left on the other
//! side of the boundary is rebuilt as a non-rendering rig, so animated
//! coordinate spaces hold on both sides. Leave All Attributes moves only one
//! layer's source and keeps its transforms, effects and timing outside.
use std::collections::{HashMap, HashSet};

use pm_model::{Map, Value};

use super::comps::{NewComp, PrecomposeMode, comp_ids, comp_name, create, new_id_for, unique_name};
use super::{Cx, layers, track, values};
use crate::error::CommandError;

pub(super) struct Request<'a> {
    pub ids: &'a [String],
    pub comp_id: &'a str,
    pub layer_id: &'a str,
    pub name: Option<&'a str>,
    pub mode: PrecomposeMode,
    pub adjust_duration: bool,
    pub time: f64,
}

const TRANSFORM_KEYS: [&str; 8] = [
    "position.x",
    "position.y",
    "anchor.x",
    "anchor.y",
    "scale.x",
    "scale.y",
    "rotation",
    "skew",
];
/// A single layer with a source keeps its attributes outside (AE's rule).
const LEAVE_TYPES: [&str; 6] = ["solid", "image", "video", "precomp", "shader", "extension"];

fn id_of(layer: &Value) -> &str {
    values::text(layer, "id").unwrap_or_default()
}
fn link(layer: &Value, field: &str) -> Option<String> {
    values::text(layer, field)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}
fn optional(id: Option<String>) -> Value {
    id.map(|id| values::s(&id)).unwrap_or(Value::Null)
}

/// `nextName(stem)`: AE numbering, `Pre-comp 1`, `Pre-comp 2`, ...
fn next_name(cx: &Cx<'_>, stem: &str) -> String {
    let taken: HashSet<String> = comp_ids(cx).iter().map(|id| comp_name(cx, id)).collect();
    (1..)
        .map(|n| format!("{stem} {n}"))
        .find(|name| !taken.contains(name))
        .expect("unbounded")
}

/// `differs(prop, value)`: a channel or plain field away from its neutral.
fn differs(field: Option<&Value>, neutral: &Value) -> bool {
    let Some(field) = field else {
        return !neutral.is_null();
    };
    if field.as_object().is_some() {
        return field.get("expr").is_some_and(Value::is_truthy)
            || !track::keys_of(field).is_empty()
            || !field.get("v").is_some_and(|v| v.identical(neutral));
    }
    !field.identical(neutral)
}
fn styled(group: &Value) -> bool {
    differs(
        group.get("p").and_then(|p| p.get("opacity")),
        &Value::Number(100.),
    ) || group
        .get("fx")
        .and_then(Value::as_array)
        .is_some_and(|fx| !fx.is_empty())
        || group
            .get("masks")
            .and_then(Value::as_array)
            .is_some_and(|m| !m.is_empty())
        || group.get("matteSource").is_some_and(Value::is_truthy)
        || differs(group.get("blend"), &values::s("normal"))
        || differs(group.get("mblur"), &Value::Bool(false))
        || group.get("transitionIn").is_some_and(Value::is_truthy)
        || group.get("transitionOut").is_some_and(Value::is_truthy)
}

/// `boundaryRigs`: both sides are planned before any link on the originals
/// changes. Copies are nulls, or groups for membership, with independent keys.
struct Rig {
    copies: Vec<Value>,
    links: HashMap<String, (Option<String>, Option<String>)>,
    layers: Vec<Value>,
}
fn boundary_rig(members: &[Value], source: &[Value], shared: &HashSet<String>) -> Rig {
    let originals: HashMap<&str, &Value> = source.iter().map(|l| (id_of(l), l)).collect();
    let mut local: HashMap<String, String> = members
        .iter()
        .map(|l| (id_of(l).to_owned(), id_of(l).to_owned()))
        .collect();
    let mut queue = members.to_vec();
    let mut index = 0;
    while index < queue.len() {
        for field in ["parent", "group"] {
            let Some(id) = link(&queue[index], field) else {
                continue;
            };
            if local.contains_key(&id) {
                continue;
            }
            let Some(original) = originals.get(id.as_str()) else {
                continue;
            };
            let new_id = new_id_for("L");
            let mut copy = layers::clone_layer(original, &new_id, source);
            let is_group = values::text(original, "type") == Some("group");
            let m = values::obj(&mut copy);
            m.insert("name", original.get("name").cloned().unwrap_or_default());
            m.insert("type", values::s(if is_group { "group" } else { "null" }));
            m.insert("fx", Value::Array(Vec::new()));
            m.insert("masks", Value::Array(Vec::new()));
            m.insert("matteSource", Value::Null);
            m.insert("transitionIn", Value::Null);
            m.insert("transitionOut", Value::Null);
            if shared.contains(&id) {
                if let Some(p) = m.get_mut("p").and_then(Value::as_object_mut) {
                    p.insert("opacity", values::property(Value::Number(100.)));
                }
                m.insert("blend", values::s("normal"));
                m.insert("mblur", Value::Bool(false));
            }
            if !is_group {
                m.insert("solo", Value::Bool(false));
            }
            local.insert(id, new_id);
            queue.push(copy);
        }
        index += 1;
    }
    let resolved =
        |layer: &Value, field: &str| link(layer, field).and_then(|id| local.get(&id).cloned());
    let links = queue[..members.len()]
        .iter()
        .map(|l| {
            (
                id_of(l).to_owned(),
                (resolved(l, "parent"), resolved(l, "group")),
            )
        })
        .collect();
    let mut placed: Vec<Value> = queue.clone();
    for layer in &mut placed[members.len()..] {
        let (parent, group) = (resolved(layer, "parent"), resolved(layer, "group"));
        let m = values::obj(layer);
        m.insert("parent", optional(parent));
        m.insert("group", optional(group));
    }
    let by_id: HashMap<&str, &Value> = placed.iter().map(|l| (id_of(l), l)).collect();
    let ordered = source
        .iter()
        .filter_map(|l| local.get(id_of(l)))
        .filter_map(|id| by_id.get(id.as_str()).map(|l| (*l).clone()))
        .collect();
    let copies = placed[members.len()..].to_vec();
    Rig {
        copies,
        links,
        layers: ordered,
    }
}

/// The groups a shared style stays on, after `precompose`'s safety rule.
fn shared_style(
    cx: &mut Cx<'_>,
    host: &[Value],
    picked: &[Value],
    picked_ids: &HashSet<&str>,
    time_zero: f64,
) -> Result<(Option<String>, HashSet<String>), CommandError> {
    let external: Vec<&Value> = picked
        .iter()
        .flat_map(|l| layers::ancestors(host, l))
        .filter(|g| !picked_ids.contains(id_of(g)))
        .collect();
    if !external.iter().any(|g| styled(g)) {
        return Ok((None, HashSet::new()));
    }
    let refused = || {
        CommandError::Invalid(
            "Select the whole styled group to pre-compose it without changing its appearance."
                .into(),
        )
    };
    let roots: Vec<&Value> = picked
        .iter()
        .filter(|l| {
            !layers::ancestors(host, l)
                .iter()
                .any(|g| picked_ids.contains(id_of(g)))
        })
        .collect();
    let shared = roots
        .first()
        .and_then(|l| link(l, "group"))
        .and_then(|id| host.iter().find(|l| id_of(l) == id));
    let Some(shared) = shared else {
        return Err(refused());
    };
    let mut chain = vec![shared];
    chain.extend(layers::ancestors(host, shared));
    let chain_ids: HashSet<&str> = chain.iter().map(|g| id_of(g)).collect();
    let mut safe = roots
        .iter()
        .all(|l| values::text(l, "group") == Some(id_of(shared)))
        && external.iter().all(|g| chain_ids.contains(id_of(g)))
        && !picked.iter().any(|l| values::flag(l, "threeD"));
    for group in &chain {
        let matte = values::text(group, "matteSource").unwrap_or_default();
        let keyed = TRANSFORM_KEYS.iter().any(|key| {
            group.get("p").and_then(|p| p.get(key)).is_some_and(|c| {
                c.get("expr").is_some_and(Value::is_truthy) || !track::keys_of(c).is_empty()
            })
        });
        safe &= !values::flag(group, "threeD")
            && link(group, "parent").is_none()
            && !picked_ids.contains(matte)
            && !keyed;
        let id = id_of(group).to_owned();
        let index = cx.find(&id)?.index;
        let identity = cx.kernel(None, |edit| {
            edit.read(|s| s.local_matrix(&s.layers()[index], time_zero))
        })?;
        safe &= identity
            .iter()
            .zip([1., 0., 0., 1., 0., 0.])
            .all(|(value, want)| (value - want).abs() < 1e-8);
    }
    if !safe {
        return Err(refused());
    }
    Ok((
        Some(id_of(shared).to_owned()),
        chain.iter().map(|g| id_of(g).to_owned()).collect(),
    ))
}

pub(super) fn precompose(cx: &mut Cx<'_>, r: &Request<'_>) -> Result<(), CommandError> {
    let host = cx.layers(None);
    let wanted: HashSet<&str> = r.ids.iter().map(String::as_str).collect();
    let selected = layers::expand_groups(&host, &wanted);
    let picked: Vec<Value> = host
        .iter()
        .filter(|l| selected.iter().any(|id| id == id_of(l)))
        .cloned()
        .collect();
    if picked.is_empty() {
        return Ok(());
    }
    let requested = r
        .name
        .map(values::js_trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| next_name(cx, "Pre-comp"));
    let name = unique_name(cx, &requested, None);
    if r.mode == PrecomposeMode::LeaveAttributes {
        return leave_attributes(cx, r, &picked, name);
    }
    move_all(cx, r, &host, &picked, name)
}

fn move_all(
    cx: &mut Cx<'_>,
    r: &Request<'_>,
    host: &[Value],
    picked: &[Value],
    name: String,
) -> Result<(), CommandError> {
    let picked_ids: HashSet<&str> = picked.iter().map(id_of).collect();
    let (shared_group, shared_groups) = shared_style(cx, host, picked, &picked_ids, 0.)?;
    let comp = cx.comp_node(None)?.to_value();
    let (w, h, fps) = (
        values::num(&comp, "w").unwrap_or(1920.),
        values::num(&comp, "h").unwrap_or(1080.),
        values::num(&comp, "fps").unwrap_or(30.),
    );
    let host_duration = cx.duration(None);
    let (mut start, mut duration) = (0., host_duration);
    if r.adjust_duration {
        let from = |l: &Value| values::num(l, "from").unwrap_or(0.);
        start = picked.iter().map(from).fold(f64::INFINITY, f64::min);
        let end = picked
            .iter()
            .map(|l| from(l) + values::num(l, "dur").unwrap_or(0.))
            .fold(f64::NEG_INFINITY, f64::max);
        duration = (1. / fps).max(end - start);
    }
    create(
        cx,
        r.comp_id,
        &NewComp {
            name: Some(name.clone()),
            width: Some(w),
            height: Some(h),
            fps: Some(fps),
            duration: Some(duration),
            background: values::text(&comp, "bg").map(str::to_owned),
        },
    )?;
    let index = host
        .iter()
        .position(|l| picked_ids.contains(id_of(l)))
        .unwrap_or(0);
    let mut rest: Vec<Value> = host
        .iter()
        .filter(|l| !picked_ids.contains(id_of(l)))
        .cloned()
        .collect();
    let inner = boundary_rig(picked, host, &shared_groups);
    let outer = boundary_rig(&rest, host, &HashSet::new());
    let mut moved = inner.layers;
    for layer in &mut moved {
        let links = inner.links.get(id_of(layer)).cloned();
        let outside =
            link(layer, "matteSource").is_some_and(|id| !picked_ids.contains(id.as_str()));
        let from = values::num(layer, "from").unwrap_or(0.);
        let m = values::obj(layer);
        if let Some((parent, group)) = links {
            m.insert("parent", optional(parent));
            m.insert("group", optional(group));
        }
        if outside {
            m.insert("matteSource", Value::Null);
        }
        if start != 0. {
            m.insert("from", Value::Number(from - start));
        }
    }
    let mut path = cx.comp_path(Some(r.comp_id));
    path.push("layers".into());
    cx.set(path, Value::Array(moved))?;
    let mut layer = precomp_layer(r, &name, [w, h, duration], (w, h))?;
    let m = values::obj(&mut layer);
    m.insert("from", Value::Number(start));
    if let Some(group) = shared_group {
        m.insert("group", values::s(&group));
    }
    for item in &mut rest {
        let links = outer.links.get(id_of(item)).cloned();
        let matte_moved =
            link(item, "matteSource").is_some_and(|id| picked_ids.contains(id.as_str()));
        let m = values::obj(item);
        if let Some((parent, group)) = links {
            m.insert("parent", optional(parent));
            m.insert("group", optional(group));
        }
        if matte_moved {
            m.insert("matteSource", Value::Null);
        }
    }
    rest.insert(index.min(rest.len()), layer);
    rest.extend(outer.copies);
    layers::write_layers(cx, None, rest)
}

/// `makeLayer`: AE's placement, the source's centre on the host's centre.
fn precomp_layer(
    r: &Request<'_>,
    name: &str,
    source: [f64; 3],
    host: (f64, f64),
) -> Result<Value, CommandError> {
    let [w, h, duration] = source;
    let mut content = Map::new();
    content.insert("comp", values::s(r.comp_id));
    content.insert("w", Value::Number(w));
    content.insert("h", Value::Number(h));
    let mut layer = layers::make_layer(
        "precomp",
        r.layer_id,
        Some(name),
        (host.0, host.1, duration),
        &content,
        &Map::new(),
    )?;
    let m = values::obj(&mut layer);
    m.insert("d", Value::Object(content));
    if let Some(p) = m.get_mut("p").and_then(Value::as_object_mut) {
        for (key, value) in [
            ("anchor.x", w / 2.),
            ("anchor.y", h / 2.),
            ("position.x", host.0 / 2.),
            ("position.y", host.1 / 2.),
        ] {
            p.insert(key, values::property(Value::Number(value)));
        }
    }
    Ok(layer)
}

/// `leaveAttributes`: the source, its trim, speed and remap move inside; the
/// layer becomes the pre-comp and its anchor shifts so nothing moves.
fn leave_attributes(
    cx: &mut Cx<'_>,
    r: &Request<'_>,
    picked: &[Value],
    name: String,
) -> Result<(), CommandError> {
    let [layer] = picked else {
        return Ok(());
    };
    let kind = values::text(layer, "type").unwrap_or_default();
    if !LEAVE_TYPES.contains(&kind) {
        return Ok(());
    }
    let id = id_of(layer).to_owned();
    let loc = cx.find(&id)?;
    let index = loc.index;
    let mut measure = cx.measure(false)?;
    let bounds = cx.kernel(None, |edit| {
        pm_edit::comp_bounds::leave_attributes(
            edit,
            index,
            r.time,
            &mut *measure,
            &mut pm_eval::bounds::NoHost,
        )
    })?;
    let comp = cx.comp_node(None)?.to_value();
    let fps = values::num(&comp, "fps").unwrap_or(30.);
    let duration = values::num(layer, "dur").unwrap_or(0.);
    create(
        cx,
        r.comp_id,
        &NewComp {
            name: Some(name),
            width: Some(bounds.w),
            height: Some(bounds.h),
            fps: Some(fps),
            duration: Some(duration),
            background: values::text(&comp, "bg").map(str::to_owned),
        },
    )?;
    let mut inner = layers::make_layer(
        kind,
        r.layer_id,
        values::text(layer, "name"),
        (bounds.w, bounds.h, duration),
        &Map::new(),
        &Map::new(),
    )?;
    let m = values::obj(&mut inner);
    m.insert("d", layer.get("d").cloned().unwrap_or_default());
    m.insert("from", Value::Number(0.));
    m.insert("dur", Value::Number(duration));
    let mut path = cx.comp_path(Some(r.comp_id));
    path.push("layers".into());
    cx.set(path, Value::Array(vec![inner]))?;
    let mut content = Map::new();
    content.insert("comp", values::s(r.comp_id));
    content.insert("w", Value::Number(bounds.w));
    content.insert("h", Value::Number(bounds.h));
    let mut type_path = loc.path();
    type_path.push("type".into());
    cx.set(type_path, values::s("precomp"))?;
    let mut d_path = loc.path();
    d_path.push("d".into());
    cx.set(d_path, Value::Object(content))
}
