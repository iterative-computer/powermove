//! Grouping edits (`legacy/core/layer-groups.ts`): moving layers into or out
//! of a group, ungrouping, and placing a new group's pivot.
//!
//! The structural checks (locks, cycles, "select layers in the same group")
//! stay with the renderer, which runs them before asking; these kernels do
//! what depends on geometry. Layers are passed by their index in the edited
//! composition.

use std::collections::{HashMap, HashSet};

use pm_eval::bounds::{Bounds, LayerHost};
use pm_eval::comp::IdKey;
use pm_eval::js_sem::{self, JsResult, JsThrow, JsValue, math_round};
use pm_eval::node::{self, Got};
use pm_eval::parenting::{AIM_KEYS, WorldPose, aim_properties3d};
use pm_eval::space3d::IDENTITY4;
use pm_eval::text_geom::TextMeasure;
use pm_eval::transform::IDENTITY;
use pm_model::{Node, NodeKind, Patch, Value};

use crate::channel::{self, at, join, key, lit, num, template};
use crate::pose::{
    self, ParentArg, aim_path, ensure_3d_channels, preserve_world_aim, preserve_world_pose,
};
use crate::time::Clock;
use crate::work::Edit;

/// A layer's `Map` key (`layer.id`, `SameValueZero`; `None` is `undefined`).
type Key = Option<IdKey>;

fn id_of(layer: &Node) -> JsValue {
    node::get(layer, b"id").to_js()
}

fn key_of(layer: &Node) -> Key {
    IdKey::of_js(&id_of(layer))
}

fn is_type(layer: &Node, t: &str) -> bool {
    node::is_str(node::member(Some(layer), "type"), t)
}

fn layer<'e>(edit: &'e Edit<'_>, index: usize) -> JsResult<&'e Node> {
    edit.layer(index)
        .ok_or_else(|| JsThrow::type_error("Cannot read properties of undefined (reading 'id')"))
}

/// `rebase(layers, poses)`: each layer (parents within the list first) keeps
/// the world pose captured in `poses`.
fn rebase(
    edit: &mut Edit<'_>,
    layers: &[usize],
    poses: &HashMap<Key, WorldPose>,
    time: f64,
) -> JsResult<()> {
    let mut done: HashSet<Key> = HashSet::new();
    for &l in layers {
        visit(edit, l, layers, poses, time, &mut done)?;
    }
    Ok(())
}

fn visit(
    edit: &mut Edit<'_>,
    index: usize,
    layers: &[usize],
    poses: &HashMap<Key, WorldPose>,
    time: f64,
    done: &mut HashSet<Key>,
) -> JsResult<()> {
    let l = layer(edit, index)?;
    let k = key_of(l);
    if !done.insert(k.clone()) {
        return Ok(());
    }
    // `layers.find(item => item.id === layer.parent)`.
    let parent_id = node::get(l, b"parent").to_js();
    let parent = layers.iter().copied().find(|&i| {
        edit.layer(i)
            .is_some_and(|p| js_sem::strict_eq(&id_of(p), &parent_id))
    });
    let audio = is_type(l, "audio");
    if let Some(p) = parent {
        visit(edit, p, layers, poses, time, done)?;
    }
    if !audio {
        let world = poses.get(&k).copied().ok_or_else(|| {
            JsThrow::type_error("Cannot read properties of undefined (reading 'length')")
        })?;
        preserve_world_pose(edit, index, world, time, ParentArg::Own)?;
    }
    Ok(())
}

/// `PM.moveToGroup(ids, target)` after its checks: `roots` (the selected
/// layers not inside another selected group) move into the group at
/// `target` (or out of every group), keeping their world poses, aims, and
/// 2.5D perspective.
pub fn move_to_group(
    edit: &mut Edit<'_>,
    roots: &[usize],
    target: Option<usize>,
    time: f64,
) -> JsResult<()> {
    let mut poses: HashMap<Key, WorldPose> = HashMap::new();
    let mut perspectives: HashMap<Key, f64> = HashMap::new();
    let mut aims: HashMap<Key, Option<[f64; 3]>> = HashMap::new();
    for &r in roots {
        if !is_type(layer(edit, r)?, "audio") {
            let k = key_of(layer(edit, r)?);
            poses.insert(k, pose::world_pose(edit, r, time)?);
        }
    }
    for &r in roots {
        let k = key_of(layer(edit, r)?);
        let p = edit.read(|s| s.perspective_amount(&s.layers()[r], time))?;
        perspectives.insert(k, p);
    }
    for &r in roots {
        let k = key_of(layer(edit, r)?);
        aims.insert(k, pose::world_aim(edit, r, time)?);
    }
    // `group?.id || null`.
    let group_id = match target {
        Some(g) => {
            let id = id_of(layer(edit, g)?);
            if id.truthy() {
                channel::to_value(&id)
            } else {
                Some(Value::Null)
            }
        }
        None => Some(Value::Null),
    };
    let patches: Vec<Patch> = roots
        .iter()
        .map(|&r| Patch {
            path: join(&edit.layer_path(r), &[key("group")]),
            exists: true,
            value: group_id.clone(),
        })
        .collect();
    edit.write(patches)?;
    rebase(edit, roots, &poses, time)?;
    for &r in roots {
        let aim = aims.get(&key_of(layer(edit, r)?)).copied().flatten();
        preserve_world_aim(edit, r, aim, time, None)?;
    }
    for &r in roots {
        let l = layer(edit, r)?;
        if !node::truthy(node::member(Some(l), "threeD")) {
            continue;
        }
        let inside_3d = edit.read(|s| {
            let l = &s.layers()[r];
            Ok(s.group_ancestors(l)?
                .iter()
                .any(|g| node::truthy(node::member(Some(g), "threeD"))))
        })?;
        if inside_3d {
            continue;
        }
        let k = key_of(layer(edit, r)?);
        let p = perspectives.get(&k).copied();
        set_property(edit, r, "perspective", p.map(Value::Number))?;
    }
    Ok(())
}

/// `layer.p[key] = PM.P(v)`.
fn set_property(edit: &mut Edit<'_>, index: usize, k: &str, v: Option<Value>) -> JsResult<()> {
    let l = layer(edit, index)?;
    let p = channel::read(Some(l), "p")?;
    match &p {
        Got::Node(p) if !node::is_nullish(Some(p)) => channel::assignable(p, k)?,
        _ => {
            return Err(JsThrow::type_error(format!(
                "Cannot set properties of {} (setting '{k}')",
                if matches!(p, Got::Node(_)) {
                    "null"
                } else {
                    "undefined"
                }
            )));
        }
    }
    let path = join(&edit.layer_path(index), &[key("p"), key(k)]);
    edit.set(path, Some(channel::property(v)))
}

/// `prop.kf.length || prop.expr` for every channel of `layer.p` and the aim
/// channels (`hasAnimation` in `ungroupLayers`).
fn has_animation(l: &Node) -> JsResult<bool> {
    let p = channel::read(Some(l), "p")?;
    let props: Vec<Option<&Node>> = match &p {
        Got::Node(p) if !node::is_nullish(Some(p)) => node::entries(p)
            .into_iter()
            .map(|(_, v)| v.node())
            .collect(),
        _ => {
            return Err(JsThrow::type_error(
                "Cannot convert undefined or null to object",
            ));
        }
    };
    let aims = aim_properties3d(l)?.map(|(_, p)| p);
    for prop in props.into_iter().chain(aims.into_iter().flatten()) {
        let kf = channel::read(prop, "kf")?;
        let length = match &kf {
            Got::Node(n) => node::get_checked(Some(n), b"length")?.truthy(),
            Got::Value(v) => v.truthy(),
            Got::Undefined => {
                return Err(JsThrow::type_error(
                    "Cannot read properties of undefined (reading 'length')",
                ));
            }
        };
        if length || channel::read(prop, "expr")?.truthy() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// What one sample of an animated ungroup records per member.
#[derive(Clone, Copy)]
struct Sampled {
    world: WorldPose,
    aim: Option<[f64; 3]>,
    perspective: f64,
    opacity: f64,
}

/// The transform channels an ungroup bakes, flat members first.
const FLAT_CHANNELS: [&str; 7] = [
    "position.x",
    "position.y",
    "scale.x",
    "scale.y",
    "rotation",
    "skew",
    "opacity",
];
const SPACE_CHANNELS: [&str; 8] = [
    "perspective",
    "position.z",
    "scale.z",
    "rotation.x",
    "rotation.y",
    "orientation.x",
    "orientation.y",
    "orientation.z",
];

/// Original layer positions, followed through deletions.
struct Tracked {
    alive: Vec<usize>,
}

impl Tracked {
    fn current(&self, original: usize) -> Option<usize> {
        self.alive.iter().position(|&o| o == original)
    }
}

/// `PM.ungroupLayers(ids)` after its checks: dissolves each group in
/// `groups` (by index), moving its members to the group's own group with
/// their world poses, aims, perspective, and opacity. A group that moves its
/// members while anything is animated is baked: the members' transform
/// channels get linear keys at every composition frame (and at `time`).
/// Returns the members' ids, as `{ ids }` reports them.
///
/// Baked keys are written at TypeScript's float times (`time - from`) while
/// the edit runs, so every later sample (a child reading its baked parent, a
/// group reading a group baked earlier in the same ungroup) reads them where
/// evaluation puts them, exactly as the legacy edit does. Once every group
/// is done the keys move to their times on the tick grid ([`settle_times`],
/// section 5, rule 4 with amendment 1).
pub fn ungroup(edit: &mut Edit<'_>, groups: &[usize], time: f64) -> JsResult<Vec<JsValue>> {
    let mut placed: HashMap<String, f64> = HashMap::new();
    let children = ungroup_each(edit, groups, time, &mut placed)?;
    settle_times(edit, &placed)?;
    Ok(children)
}

fn ungroup_each(
    edit: &mut Edit<'_>,
    groups: &[usize],
    time: f64,
    placed: &mut HashMap<String, f64>,
) -> JsResult<Vec<JsValue>> {
    let mut tracked = Tracked {
        alive: (0..edit.layers().len()).collect(),
    };
    let clock = edit.read(|s| {
        let fps = s.project.member("fps");
        Ok(Clock::new(s.project.rate(), node::to_number(fps)))
    })?;
    let (dur, fps) = edit.read(|s| {
        Ok((
            node::to_number(s.project.member("dur")),
            node::to_number(s.project.member("fps")),
        ))
    })?;
    let mut children: Vec<JsValue> = Vec::new();
    for &original in groups {
        let g = tracked.current(original).ok_or_else(|| {
            JsThrow::type_error("Cannot read properties of undefined (reading 'id')")
        })?;
        let group_id = id_of(layer(edit, g)?);
        let members: Vec<usize> = edit
            .layers()
            .iter()
            .enumerate()
            .filter(|(_, l)| js_sem::strict_eq(&node::get(l, b"group").to_js(), &group_id))
            .map(|(i, _)| i)
            .collect();

        let changes_3d = edit.read(|s| {
            let gl = &s.layers()[g];
            if !s.is_3d_layer(gl)? {
                return Ok(false);
            }
            let m = s.local3d(gl, time)?;
            Ok(m.iter()
                .zip(IDENTITY4.iter())
                .any(|(v, i)| (v - i).abs() > 1e-8))
        })?;
        let changes_pose = changes_3d
            || edit.read(|s| {
                let gl = &s.layers()[g];
                let m = s.local_matrix(gl, time)?;
                if m.iter()
                    .zip(IDENTITY.iter())
                    .any(|(v, i)| (v - i).abs() > 1e-8)
                {
                    return Ok(true);
                }
                Ok(!js_sem::strict_eq(
                    &s.channel(gl, "opacity", time)?,
                    &JsValue::Number(100.0),
                ))
            })?;
        let animated = has_animation(layer(edit, g)?)?
            || changes_pose && {
                let mut any = false;
                for l in edit.layers() {
                    if has_animation(l)? {
                        any = true;
                        break;
                    }
                }
                any
            };
        let times: Vec<f64> = if animated {
            let count = (dur * fps).ceil() + 1.0;
            let mut times = vec![time];
            if count.is_finite() && count > 0.0 {
                // `Array.from({ length })` throws past 2^32 - 1 elements.
                if count > 4_294_967_295.0 {
                    return Err(JsThrow {
                        kind: "RangeError",
                        message: "Invalid array length".into(),
                    });
                }
                for i in 0..count as i64 {
                    times.push(clock.frame(i));
                }
            }
            let mut unique: Vec<f64> = Vec::with_capacity(times.len());
            let mut seen: HashSet<u64> = HashSet::new();
            for t in times {
                let bits = if t == 0.0 {
                    0.0f64.to_bits()
                } else {
                    t.to_bits()
                };
                if seen.insert(bits) {
                    unique.push(t);
                }
            }
            // `sort((a, b) => a - b)`: a stable sort that leaves NaN where
            // the comparator cannot order it.
            unique.sort_by(|a, b| {
                (a - b)
                    .partial_cmp(&0.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            unique
        } else {
            Vec::new()
        };
        let solid: Vec<usize> = members
            .iter()
            .copied()
            .filter(|&m| !is_type(&edit.layers()[m], "audio"))
            .collect();
        let mut samples: Vec<(f64, HashMap<Key, Sampled>)> = Vec::with_capacity(times.len());
        for &t in &times {
            let mut values = HashMap::new();
            for &m in &solid {
                let sampled = edit.read(|s| {
                    let l = &s.layers()[m];
                    let gl = &s.layers()[g];
                    let world = s.world_pose(l, t)?;
                    let aim = s.world_aim3d(l, t)?;
                    let perspective = s.perspective_amount(l, t)?;
                    let opacity =
                        js_sem::mul(&s.channel(l, "opacity", t)?, &s.channel(gl, "opacity", t)?)
                            / 100.0;
                    Ok(Sampled {
                        world,
                        aim,
                        perspective,
                        opacity,
                    })
                })?;
                values.insert(key_of(&edit.layers()[m]), sampled);
            }
            samples.push((t, values));
        }
        let mut poses: HashMap<Key, WorldPose> = HashMap::new();
        for &m in &solid {
            let w = pose::world_pose(edit, m, time)?;
            poses.insert(key_of(&edit.layers()[m]), w);
        }
        let mut aims: HashMap<Key, Option<[f64; 3]>> = HashMap::new();
        for &m in &members {
            let a = pose::world_aim(edit, m, time)?;
            aims.insert(key_of(&edit.layers()[m]), a);
        }
        let mut perspectives: HashMap<Key, f64> = HashMap::new();
        for &m in &members {
            let p = edit.read(|s| s.perspective_amount(&s.layers()[m], time))?;
            perspectives.insert(key_of(&edit.layers()[m]), p);
        }
        // Membership, 2.5D conversion, and the group's opacity.
        let parent_group = {
            let v = node::get(layer(edit, g)?, b"group").to_js();
            if v.truthy() {
                channel::to_value(&v)
            } else {
                Some(Value::Null)
            }
        };
        for &m in &members {
            edit.set(
                join(&edit.layer_path(m), &[key("group")]),
                parent_group.clone(),
            )?;
            let k = key_of(layer(edit, m)?);
            children.push(id_of(layer(edit, m)?));
            if matches!(poses.get(&k), Some(WorldPose::Space(_))) {
                ensure_3d_channels(edit, m)?;
                set_property(
                    edit,
                    m,
                    "perspective",
                    perspectives.get(&k).copied().map(Value::Number),
                )?;
            }
            let factor = edit.read(|s| {
                let gl = &s.layers()[g];
                Ok(s.channel(gl, "opacity", time)?.to_number() / 100.0)
            })?;
            fade(edit, m, factor)?;
        }
        if animated {
            bake(edit, &members, &poses, &samples, &clock, placed)?;
        } else {
            rebase(edit, &members, &poses, time)?;
            for &m in &members {
                let aim = aims.get(&key_of(layer(edit, m)?)).copied().flatten();
                preserve_world_aim(edit, m, aim, time, None)?;
            }
        }
        // `PM.proj.layers = PM.proj.layers.filter(layer => layer.id !== group.id)`.
        let doomed: Vec<usize> = edit
            .layers()
            .iter()
            .enumerate()
            .filter(|(_, l)| js_sem::strict_eq(&id_of(l), &group_id))
            .map(|(i, _)| i)
            .collect();
        remove_layers(edit, &doomed)?;
        for &i in doomed.iter().rev() {
            tracked.alive.remove(i);
        }
    }
    Ok(children)
}

/// Removes layers by index (as `filter` does: by position, whatever their
/// ids).
fn remove_layers(edit: &mut Edit<'_>, doomed: &[usize]) -> JsResult<()> {
    if doomed.is_empty() {
        return Ok(());
    }
    let mut path = edit.comp_path();
    path.push(key("layers"));
    let kept: Vec<Value> = edit
        .layers()
        .iter()
        .enumerate()
        .filter(|(i, _)| !doomed.contains(i))
        .map(|(_, l)| l.to_value())
        .collect();
    edit.set(path, Some(Value::Array(kept)))
}

/// The group's opacity folded into a member (`opacity.v *= factor`, keys and
/// ease speeds too, or `(${expr}) * ${factor}`).
fn fade(edit: &mut Edit<'_>, m: usize, factor: f64) -> JsResult<()> {
    let l = layer(edit, m)?;
    let p = channel::read(Some(l), "p")?;
    let opacity = match &p {
        Got::Node(p) => node::member(Some(p), "opacity"),
        _ => None,
    };
    if !node::truthy(opacity) || factor == 1.0 {
        return Ok(());
    }
    let opacity = opacity.expect("truthy");
    let path = join(&edit.layer_path(m), &[key("p"), key("opacity")]);
    let expr = node::get(opacity, b"expr");
    if expr.truthy() {
        let text = template(&[lit("("), expr.to_js(), lit(") * "), JsValue::Number(factor)]);
        return edit.set(join(&path, &[key("expr")]), channel::to_value(&text));
    }
    let f = JsValue::Number(factor);
    channel::assignable(opacity, "v")?;
    let mut patches = vec![Patch::set(
        join(&path, &[key("v")]),
        Value::Number(js_sem::mul(&node::get(opacity, b"v").to_js(), &f)),
    )];
    for (i, kf) in channel::keys_of(Some(opacity))?.iter().enumerate() {
        channel::assignable(kf, "v")?;
        let kp = join(&path, &[key("kf"), at(i)]);
        patches.push(Patch::set(
            join(&kp, &[key("v")]),
            Value::Number(js_sem::mul(&node::get(kf, b"v").to_js(), &f)),
        ));
        for ease in ["inEase", "outEase"] {
            let e = node::get(kf, ease.as_bytes());
            if e.truthy() {
                let e = e.node().expect("truthy");
                channel::assignable(e, "speed")?;
                patches.push(Patch::set(
                    join(&kp, &[key(ease), key("speed")]),
                    Value::Number(js_sem::mul(&node::get(e, b"speed").to_js(), &f)),
                ));
            }
        }
    }
    edit.write(patches)
}

/// The members' channels for a bake: the flat transform plus, for members
/// that became 2.5D, the 3D channels (`channelsFor`).
fn channels_for(pose: Option<&WorldPose>) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = FLAT_CHANNELS.to_vec();
    if matches!(pose, Some(WorldPose::Space(_))) {
        out.extend_from_slice(&SPACE_CHANNELS);
    }
    out
}

/// The members in bake order: each after the members it hangs from (its
/// parent chain and their groups), audio left out.
fn bake_order(edit: &mut Edit<'_>, members: &[usize]) -> JsResult<Vec<usize>> {
    let mut ordered = Vec::new();
    let mut seen: HashSet<Key> = HashSet::new();
    for &m in members {
        order_visit(edit, m, members, &mut seen, &mut ordered)?;
    }
    Ok(ordered)
}

fn order_visit(
    edit: &mut Edit<'_>,
    m: usize,
    members: &[usize],
    seen: &mut HashSet<Key>,
    ordered: &mut Vec<usize>,
) -> JsResult<()> {
    if !seen.insert(key_of(layer(edit, m)?)) {
        return Ok(());
    }
    // The parent chain through `PM.L` (the last layer per id), at most 256
    // links, with every member it reaches (and every member group around it)
    // visited first.
    let chain: Vec<(usize, Vec<usize>)> = edit.read(|s| {
        let layers = s.layers();
        let mut out = Vec::new();
        let mut parent = s.parent_by_id(&layers[m]);
        let mut guard = 0;
        while let Some(p) = parent {
            guard += 1;
            if guard > 256 {
                break;
            }
            let pi = pose::index_of(layers, p).expect("a layer of the composition");
            let groups: Vec<usize> = s
                .group_ancestors(p)?
                .iter()
                .filter_map(|g| pose::index_of(layers, g))
                .collect();
            out.push((pi, groups));
            parent = s.parent_by_id(p);
        }
        Ok(out)
    })?;
    for (p, groups) in chain {
        if members.contains(&p) {
            order_visit(edit, p, members, seen, ordered)?;
        }
        for g in groups {
            if members.contains(&g) {
                order_visit(edit, g, members, seen, ordered)?;
            }
        }
    }
    if !is_type(layer(edit, m)?, "audio") {
        ordered.push(m);
    }
    Ok(())
}

/// The animated ungroup: every member's transform channels become linear
/// keys at the sampled times reproducing the sampled world poses, aims,
/// perspective, and opacity.
fn bake(
    edit: &mut Edit<'_>,
    members: &[usize],
    poses: &HashMap<Key, WorldPose>,
    samples: &[(f64, HashMap<Key, Sampled>)],
    clock: &Clock,
    placed: &mut HashMap<String, f64>,
) -> JsResult<()> {
    let ordered = bake_order(edit, members)?;
    // Reset the baked channels: `{ v: layer.p[key].v, kf: [], expr: null }`.
    for &m in &ordered {
        let k = key_of(layer(edit, m)?);
        let base = edit.layer_path(m);
        let mut patches = Vec::new();
        let l = layer(edit, m)?;
        let p = channel::read(Some(l), "p")?;
        for name in channels_for(poses.get(&k)) {
            let prop = match &p {
                Got::Node(p) => node::member(Some(p), name),
                _ => None,
            };
            let v = channel::read(prop, "v")?;
            if let Got::Node(p) = &p {
                channel::assignable(p, name)?;
            }
            patches.push(Patch::set(
                join(&base, &[key("p"), key(name)]),
                channel::property(channel::to_value(&v.to_js())),
            ));
        }
        edit.write(patches)?;
    }
    for &m in &ordered {
        let l = layer(edit, m)?;
        let Some((role, props)) = aim_properties3d(l)? else {
            continue;
        };
        let base = edit.layer_path(m);
        let mut patches = Vec::new();
        for (i, prop) in props.iter().enumerate() {
            let Some(prop) = prop else {
                return Err(JsThrow::type_error(
                    "Cannot set properties of undefined (setting 'kf')",
                ));
            };
            channel::assignable(prop, "kf")?;
            let path = aim_path(&base, role, AIM_KEYS[i]);
            patches.push(Patch::set(
                join(&path, &[key("kf")]),
                Value::Array(Vec::new()),
            ));
            patches.push(Patch::set(join(&path, &[key("expr")]), Value::Null));
        }
        edit.write(patches)?;
    }
    for (time, values) in samples {
        let time = *time;
        for &m in &ordered {
            let k = key_of(layer(edit, m)?);
            let source = *values.get(&k).ok_or_else(|| {
                JsThrow::type_error("Cannot read properties of undefined (reading 'world')")
            })?;
            let mut values: Vec<(&'static str, JsValue)> = edit.read(|s| {
                let l = &s.layers()[m];
                // `worldPoseValues(PM, layer, world, time, undefined, true)`:
                // the default parent matrix is evaluated first.
                let p = s.transform_parent_matrix(l, time, None)?;
                match source.world {
                    WorldPose::Flat(world) => Ok(s
                        .world_pose_values(l, &world, time, Some(&p), true)?
                        .channels()
                        .to_vec()),
                    WorldPose::Space(world) => {
                        let parent = s.parent3d(l, time, None)?;
                        Ok(s.pose_values3d(l, &world, time, Some(&parent))?
                            .channels()
                            .iter()
                            .map(|(k, v)| (*k, JsValue::Number(*v)))
                            .collect())
                    }
                }
            })?;
            values.push(("opacity", JsValue::Number(source.opacity)));
            values.push(("perspective", JsValue::Number(source.perspective)));
            // `sample.time - layer.from` now; `clock.local` once the
            // ungroup is done.
            let from = node::get(layer(edit, m)?, b"from").to_js().to_number();
            let t = time - from;
            let settled = clock.local(time, from);
            let base = edit.layer_path(m);
            let mut patches = Vec::new();
            for name in channels_for(poses.get(&k)) {
                let prop = channel::read(layer(edit, m)?.get("p"), name)?
                    .node()
                    .ok_or_else(|| {
                        JsThrow::type_error("Cannot read properties of undefined (reading 'kf')")
                    })?;
                let mut value = values
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, v)| v.clone())
                    .unwrap_or(JsValue::Undefined);
                let keys = channel::keys_of(Some(prop))?;
                if (name == "rotation" || name.starts_with("rotation.")) && !keys.is_empty() {
                    let last = node::get(&keys[keys.len() - 1], b"v").to_js();
                    let turns = math_round(js_sem::sub(&last, &value) / 360.0) * 360.0;
                    value = js_sem::add(&value, &JsValue::Number(turns));
                }
                let path = join(&base, &[key("p"), key(name)]);
                let n = keys.len();
                let kf = channel::linear_key(t, channel::to_value(&value), &mut edit.ids);
                place(&kf, settled, placed);
                patches.push(Patch::set(join(&path, &[key("kf"), at(n)]), kf));
                patches.push(Patch {
                    path: join(&path, &[key("v")]),
                    exists: true,
                    value: channel::to_value(&value),
                });
            }
            edit.write(patches)?;
            if let Some(aim) = source.aim {
                let values = edit.read(|s| s.aim_values3d(&s.layers()[m], &aim, time, None))?;
                let l = layer(edit, m)?;
                let found = match aim_properties3d(l)? {
                    Some((role, props)) => {
                        let mut lengths = [0usize; 3];
                        for (i, prop) in props.iter().enumerate() {
                            lengths[i] = channel::keys_of(*prop)?.len();
                        }
                        Some((role, lengths))
                    }
                    None => None,
                };
                if let Some((role, lengths)) = found {
                    let mut patches = Vec::new();
                    for (i, n) in lengths.into_iter().enumerate() {
                        let path = aim_path(&base, role, AIM_KEYS[i]);
                        let kf = channel::linear_key(t, num(values[i]), &mut edit.ids);
                        place(&kf, settled, placed);
                        patches.push(Patch::set(join(&path, &[key("kf"), at(n)]), kf));
                        patches.push(Patch::set(
                            join(&path, &[key("v")]),
                            Value::Number(values[i]),
                        ));
                    }
                    edit.write(patches)?;
                }
            }
        }
    }
    Ok(())
}

/// Notes the time on the tick grid of a baked key (by its created id).
fn place(kf: &Value, settled: f64, placed: &mut HashMap<String, f64>) {
    if let Value::Object(m) = kf
        && let Some(id) = m.get("i").and_then(Value::as_js_string)
    {
        placed.insert(id.to_string_lossy(), settled);
    }
}

/// Moves every baked key of the edited composition (found by its created
/// id, wherever an ungroup left it) to its time on the tick grid.
fn settle_times(edit: &mut Edit<'_>, placed: &HashMap<String, f64>) -> JsResult<()> {
    fn walk(
        node: &Node,
        path: &mut Vec<pm_model::PathPart>,
        placed: &HashMap<String, f64>,
        out: &mut Vec<Patch>,
    ) {
        match node.kind() {
            NodeKind::Object(map) => {
                if let Some(NodeKind::String(id)) =
                    map.get(&pm_model::JsString::from("i")).map(Node::kind)
                    && let Some(&t) = placed.get(&id.to_string_lossy())
                    && let Some(NodeKind::Number(was)) =
                        map.get(&pm_model::JsString::from("t")).map(Node::kind)
                    && was.to_bits() != t.to_bits()
                {
                    out.push(Patch::set(join(path, &[key("t")]), Value::Number(t)));
                }
                for (k, child) in map {
                    path.push(pm_model::PathPart::Key(k.clone()));
                    walk(child, path, placed, out);
                    path.pop();
                }
            }
            NodeKind::Array(items) => {
                for (i, child) in items.iter().enumerate() {
                    path.push(at(i));
                    walk(child, path, placed, out);
                    path.pop();
                }
            }
            _ => {}
        }
    }
    if placed.is_empty() {
        return Ok(());
    }
    let mut path = edit.comp_path();
    path.push(key("layers"));
    let mut patches = Vec::new();
    if let Some(layers) = edit.node(&path) {
        walk(layers, &mut path.clone(), placed, &mut patches);
    }
    edit.write(patches)
}

/// The pivot of a new group: `PM.groupBounds(group, time)`'s center becomes
/// the group's position and anchor (an identity transform, so members do not
/// move). Returns the bounds. Model groups (three.js bounds in Phase 1) throw
/// the host-only error.
pub fn group_placement(
    edit: &mut Edit<'_>,
    group: usize,
    time: f64,
    measure: &mut dyn TextMeasure,
    host: &mut dyn LayerHost,
) -> JsResult<Option<Bounds>> {
    let bounds = edit.read(|s| {
        let g = s
            .layers()
            .get(group)
            .ok_or_else(|| JsThrow::type_error("Group not found"))?;
        s.group_bounds(g, time, measure, host)
    })?;
    let Some(b) = bounds else {
        return Ok(None);
    };
    let base = edit.layer_path(group);
    let p = channel::read(edit.layer(group), "p")?;
    let mut patches = Vec::new();
    // `if (group.threeD && bounds.z0 !== undefined)`: a model group's depth.
    let three_d = node::truthy(node::member(edit.layer(group), "threeD"));
    let mut axes = vec![("x", (b.x0 + b.x1) / 2.0), ("y", (b.y0 + b.y1) / 2.0)];
    if let (true, Some((z0, z1))) = (three_d, b.z) {
        axes.push(("z", (z0 + z1) / 2.0));
    }
    for (axis, value) in axes {
        // `group.p['position.x'].v = group.p['anchor.x'].v = …` (anchor first).
        for name in ["anchor", "position"] {
            let k = format!("{name}.{axis}");
            let prop = match &p {
                Got::Node(p) => node::member(Some(p), &k),
                _ => None,
            };
            let Some(prop) = prop.filter(|n| !matches!(n.kind(), NodeKind::Null)) else {
                return Err(JsThrow::type_error(
                    "Cannot set properties of undefined (setting 'v')",
                ));
            };
            channel::assignable(prop, "v")?;
            patches.push(Patch::set(
                join(&base, &[key("p"), key(&k), key("v")]),
                Value::Number(value),
            ));
        }
    }
    edit.write(patches)?;
    Ok(Some(b))
}
