//! Pose and aim preservation (`legacy/core/parenting.ts`).
//!
//! Reparenting, grouping, and ungrouping change the space a layer's channels
//! live in. These edits rewrite the ordinary editable channels so the layer
//! keeps its world pose (and a camera's or light's aim keeps its world
//! point): static values and every key move by the same scale and offset, so
//! keys keep their identities and easing, and an expression gains the same
//! correction as a suffix. The reads come from `pm-eval`
//! ([`Scope::world_pose_values`], [`Scope::pose_values3d`],
//! [`Scope::aim_values3d`]).

use pm_eval::comp::IdKey;
use pm_eval::js_sem::{self, JsResult, JsThrow, JsValue};
use pm_eval::node;
use pm_eval::parenting::{AIM_KEYS, Role3d, WorldPose, aim_properties3d};
use pm_eval::space3d::{CHANNELS_3D, Mat4};
use pm_eval::transform::{Mat2d, mul};
use pm_model::{Node, Patch, PathPart};

use crate::channel::{self, at, join, key, lit, num, template};
use crate::work::Edit;

/// Which parent space a rebase targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParentArg {
    /// `layer.parent ? PM.L(layer.parent) : null` (the layer's own parent,
    /// the last layer with that id).
    Own,
    /// An explicit parent layer (by index), or none.
    Layer(Option<usize>),
}

/// The index of a layer node of the edited composition.
pub(crate) fn index_of(layers: &[Node], layer: &Node) -> Option<usize> {
    layers.iter().position(|l| std::ptr::eq(l, layer))
}

fn resolve_parent<'d>(
    scope: &mut pm_eval::Scope<'_, 'd>,
    layer: &'d Node,
    parent: ParentArg,
) -> Option<&'d Node> {
    match parent {
        ParentArg::Layer(p) => p.and_then(|i| scope.layers().get(i)),
        ParentArg::Own => scope.parent_by_id(layer),
    }
}

fn layer_at<'d>(scope: &pm_eval::Scope<'_, 'd>, index: usize) -> JsResult<&'d Node> {
    scope
        .layers()
        .get(index)
        .ok_or_else(|| JsThrow::type_error("Cannot read properties of undefined (reading 'p')"))
}

/// `is3DLayer ? world3D : worldMatrix` (`worldPose` in `layer-groups.ts`).
pub fn world_pose(edit: &mut Edit<'_>, layer: usize, time: f64) -> JsResult<WorldPose> {
    edit.read(|s| {
        let l = layer_at(s, layer)?;
        s.world_pose(l, time)
    })
}

/// `worldAim3D(PM, layer, time)`.
pub fn world_aim(edit: &mut Edit<'_>, layer: usize, time: f64) -> JsResult<Option<[f64; 3]>> {
    edit.read(|s| {
        let l = layer_at(s, layer)?;
        s.world_aim3d(l, time)
    })
}

/// `preserveParentPose(PM, layer, parent, time)`: reparent `layer` under
/// `parent` (by index; `None` unparents) keeping its world pose and aim. The
/// `parent` member itself is the caller's to write, as in `setLayer`.
///
/// A camera or light whose new parent space reads its own transform (it is
/// reparented onto a member of a group that is itself parented to the
/// camera: `PM.wouldCycle` checks group ancestors only for groups) is left
/// to the legacy edit ([`crate::LEGACY_ONLY`]). There the legacy aim reads
/// the layer's keyframed channels through the evaluation cache the pose
/// write left stale (`preserveParentPose` writes the pose and reads
/// `parent3D` without `PM.touch()`), a mix of old and new values no
/// document-only kernel can reproduce, and the pose cannot be preserved in
/// such a space anyway.
pub fn reparent(
    edit: &mut Edit<'_>,
    layer: usize,
    parent: Option<usize>,
    time: f64,
) -> JsResult<()> {
    let aim = world_aim(edit, layer, time)?;
    if aim.is_some()
        && edit.read(|s| {
            let l = layer_at(s, layer)?;
            let p = parent.and_then(|i| s.layers().get(i));
            space_reads_layer(s, l, p)
        })?
    {
        return Err(crate::legacy_only(
            "Reparenting a camera or light into a space that moves with it",
        ));
    }
    let world = world_pose(edit, layer, time)?;
    preserve_world_pose(edit, layer, world, time, ParentArg::Layer(parent))?;
    let parent3 = edit.read(|s| {
        let l = layer_at(s, layer)?;
        let p = parent.and_then(|i| s.layers().get(i));
        s.parent3d(l, time, Some(p))
    })?;
    preserve_world_aim(edit, layer, aim, time, Some(parent3))
}

/// Whether `parent3D(PM, layer, time, parent)` multiplies in `layer`'s own
/// transform: the walk of [`pm_eval::Scope::parent3d`] (enclosing groups
/// and their parent chains, then the parent chain from `parent` and each
/// link's groups), recording instead of multiplying. Each parent chain
/// starts its own `seen` set with the layer whose chain it is, so `layer`
/// is reached only through a group's chain.
fn space_reads_layer<'d>(
    s: &mut pm_eval::Scope<'_, 'd>,
    layer: &'d Node,
    parent: Option<&'d Node>,
) -> JsResult<bool> {
    struct Walk<'d> {
        target: &'d Node,
        groups: Vec<Option<IdKey>>,
        found: bool,
    }
    impl<'d> Walk<'d> {
        fn parents(
            &mut self,
            s: &mut pm_eval::Scope<'_, 'd>,
            layer: &'d Node,
            first: Option<&'d Node>,
        ) -> JsResult<()> {
            let mut seen: Vec<&'d Node> = vec![layer];
            let mut count = 0;
            let mut cur = first;
            while let Some(c) = cur {
                if seen.iter().any(|l| std::ptr::eq(*l, c)) || count >= 256 {
                    break;
                }
                seen.push(c);
                count += 1;
                self.found |= std::ptr::eq(c, self.target);
                self.groups(s, c)?;
                cur = s.parent_by_id(c);
            }
            Ok(())
        }
        fn groups(&mut self, s: &mut pm_eval::Scope<'_, 'd>, layer: &'d Node) -> JsResult<()> {
            for group in s.group_ancestors(layer)?.into_iter().rev() {
                let id = IdKey::of(&node::get(group, b"id"));
                if self.groups.contains(&id) {
                    continue;
                }
                self.groups.push(id);
                let first = s.parent_by_id(group);
                self.parents(s, group, first)?;
                self.found |= std::ptr::eq(group, self.target);
            }
            Ok(())
        }
    }
    let mut walk = Walk {
        target: layer,
        groups: Vec::new(),
        found: false,
    };
    walk.groups(s, layer)?;
    walk.parents(s, layer, parent)?;
    Ok(walk.found)
}

/// `preserveWorldPose(PM, layer, world, time, parent)`: rebase the layer's
/// editable channels so it sits at `world` inside `parent`'s space.
pub fn preserve_world_pose(
    edit: &mut Edit<'_>,
    layer: usize,
    world: WorldPose,
    time: f64,
    parent: ParentArg,
) -> JsResult<()> {
    let world = match world {
        WorldPose::Flat(m) if edit.read(|s| s.is_3d_layer(layer_at(s, layer)?))? => {
            let [a, b, c, d, x, y] = m;
            WorldPose::Space([
                a, b, 0.0, 0.0, c, d, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, x, y, 0.0, 1.0,
            ])
        }
        other => other,
    };
    match world {
        WorldPose::Space(world) => {
            ensure_3d_channels(edit, layer)?;
            let next = edit.read(|s| {
                let l = layer_at(s, layer)?;
                let p = resolve_parent(s, l, parent);
                let parent3 = s.parent3d(l, time, Some(p))?;
                s.pose_values3d(l, &world, time, Some(&parent3))
            })?;
            let values: Vec<(&str, JsValue)> = next
                .channels()
                .iter()
                .map(|(k, v)| (*k, JsValue::Number(*v)))
                .collect();
            apply_pose_values(edit, layer, &values, time)
        }
        WorldPose::Flat(world) => {
            let next = edit.read(|s| {
                let l = layer_at(s, layer)?;
                let p = resolve_parent(s, l, parent);
                let pm = s.transform_parent_matrix(l, time, Some(p))?;
                let current = mul(&pm, &s.local_matrix(l, time)?);
                if world
                    .iter()
                    .zip(current.iter())
                    .all(|(w, c)| (w - c).abs() < 1e-8)
                {
                    return Ok(None);
                }
                s.world_pose_values(l, &world, time, Some(&pm), false)
                    .map(Some)
            })?;
            let Some(next) = next else {
                return Ok(());
            };
            apply_pose_values(edit, layer, &next.channels(), time)
        }
    }
}

/// `for (const [key, fallback] of Object.entries(CHANNELS_3D)) layer.p[key] ||= PM.P(fallback); layer.threeD = true;`
pub(crate) fn ensure_3d_channels(edit: &mut Edit<'_>, layer: usize) -> JsResult<()> {
    let base = edit.layer_path(layer);
    let l = edit.layer(layer);
    let p = channel::read(l, "p")?;
    if p.is_nullish() {
        let what = if matches!(p, node::Got::Node(_)) {
            "null"
        } else {
            "undefined"
        };
        return Err(JsThrow::type_error(format!(
            "Cannot read properties of {what} (reading '{}')",
            CHANNELS_3D[0].0
        )));
    }
    let mut patches = Vec::new();
    for (k, fallback) in CHANNELS_3D {
        let current = match &p {
            node::Got::Node(p) => node::get(p, k.as_bytes()),
            _ => node::Got::Undefined,
        };
        if !current.truthy() {
            if let node::Got::Node(p) = &p {
                channel::assignable(p, k)?;
            }
            patches.push(Patch::set(
                join(&base, &[key("p"), key(k)]),
                channel::property(num(fallback)),
            ));
        }
    }
    patches.push(Patch::set(
        join(&base, &[key("threeD")]),
        pm_model::Value::Bool(true),
    ));
    edit.write(patches)
}

/// `applyPoseValues(PM, layer, next, time)`: for each channel, the scale
/// (scale channels with a non-zero current value) or the offset that moves
/// its value at `time` to `next`, applied to the static value and every key
/// (scales also scale the keys' ease speeds), or appended to an expression.
pub fn apply_pose_values(
    edit: &mut Edit<'_>,
    layer: usize,
    next: &[(&str, JsValue)],
    time: f64,
) -> JsResult<()> {
    for (k, value) in next {
        let current = edit.read(|s| s.num(layer_at(s, layer)?, k, time))?;
        let scaled = k.starts_with("scale.") && current.abs() > 1e-10;
        let scale = if scaled {
            js_sem::div(value, &JsValue::Number(current))
        } else {
            1.0
        };
        let offset = if scaled {
            0.0
        } else {
            js_sem::sub(value, &JsValue::Number(current))
        };
        let prop_path = join(&edit.layer_path(layer), &[key("p"), key(k)]);
        let prop = channel::read(edit.layer(layer), "p")?
            .node()
            .and_then(|p| node::member(Some(p), k));
        let expr = channel::read(prop, "expr")?;
        if expr.truthy() {
            if (scale - 1.0).abs() > 1e-10 || offset.abs() > 1e-10 {
                let text = template(&[
                    lit("("),
                    expr.to_js(),
                    lit(") * "),
                    JsValue::Number(scale),
                    lit(" + "),
                    JsValue::Number(offset),
                ]);
                edit.set(join(&prop_path, &[key("expr")]), channel::to_value(&text))?;
            }
            continue;
        }
        let prop = prop.expect("a prop with a readable expr is a node");
        let mut patches = Vec::new();
        channel::assignable(prop, "v")?;
        let v = node::get(prop, b"v").to_js().to_number() * scale + offset;
        patches.push(Patch::set(
            join(&prop_path, &[key("v")]),
            pm_model::Value::Number(v),
        ));
        for (i, kf) in channel::keys_of(Some(prop))?.iter().enumerate() {
            channel::assignable(kf, "v")?;
            let kp = join(&prop_path, &[key("kf"), at(i)]);
            let kv = node::get(kf, b"v").to_js().to_number() * scale + offset;
            patches.push(Patch::set(
                join(&kp, &[key("v")]),
                pm_model::Value::Number(kv),
            ));
            for ease in ["inEase", "outEase"] {
                let e = node::get(kf, ease.as_bytes());
                if e.truthy() {
                    let e = e.node().expect("a truthy member is a node");
                    channel::assignable(e, "speed")?;
                    let speed =
                        js_sem::mul(&node::get(e, b"speed").to_js(), &JsValue::Number(scale));
                    patches.push(Patch::set(
                        join(&kp, &[key(ease), key("speed")]),
                        pm_model::Value::Number(speed),
                    ));
                }
            }
        }
        edit.write(patches)?;
    }
    Ok(())
}

/// `preserveWorldAim(PM, layer, world, time, parent)`: rewrite a camera's or
/// light's stored aim (in its parent's space) so it points at `world` from
/// the parent space `parent` (default: the layer's current one).
pub fn preserve_world_aim(
    edit: &mut Edit<'_>,
    layer: usize,
    world: Option<[f64; 3]>,
    time: f64,
    parent: Option<Mat4>,
) -> JsResult<()> {
    let Some(world) = world else {
        return Ok(());
    };
    let (values, role) = edit.read(|s| {
        let l = layer_at(s, layer)?;
        let values = s.aim_values3d(l, &world, time, parent.as_ref())?;
        Ok((values, aim_properties3d(l)?.map(|(role, _)| role)))
    })?;
    let Some(role) = role else {
        return Ok(());
    };
    let base = edit.layer_path(layer);
    for (i, k) in AIM_KEYS.iter().enumerate() {
        let channel_key = format!("{}.{k}", role.name());
        let current = edit.read(|s| {
            let l = layer_at(s, layer)?;
            let props = aim_properties3d(l)?.map(|(_, p)| p).unwrap_or([None; 3]);
            Ok(s.channel_p(l, props[i], time, &channel_key)?.to_number())
        })?;
        let offset = values[i] - current;
        // `Math.abs(offset) < 1e-10` is false for NaN, which is written.
        if offset.abs() < 1e-10 {
            continue;
        }
        let prop_path = aim_path(&base, role, k);
        let prop = edit.node(&prop_path);
        let expr = channel::read(prop, "expr")?;
        if expr.truthy() {
            let text = template(&[lit("("), expr.to_js(), lit(") + "), JsValue::Number(offset)]);
            edit.set(join(&prop_path, &[key("expr")]), channel::to_value(&text))?;
            continue;
        }
        let prop = prop.expect("a prop with a readable expr is a node");
        let mut patches = Vec::new();
        let off = JsValue::Number(offset);
        channel::assignable(prop, "v")?;
        let v = js_sem::add(&node::get(prop, b"v").to_js(), &off);
        patches.push(Patch {
            path: join(&prop_path, &[key("v")]),
            exists: true,
            value: channel::to_value(&v),
        });
        for (j, kf) in channel::keys_of(Some(prop))?.iter().enumerate() {
            channel::assignable(kf, "v")?;
            let kv = js_sem::add(&node::get(kf, b"v").to_js(), &off);
            patches.push(Patch {
                path: join(&prop_path, &[key("kf"), at(j), key("v")]),
                exists: true,
                value: channel::to_value(&kv),
            });
        }
        edit.write(patches)?;
    }
    Ok(())
}

/// `layer.d.data[role].p[key]`.
pub(crate) fn aim_path(layer: &[PathPart], role: Role3d, k: &str) -> Vec<PathPart> {
    join(
        layer,
        &[key("d"), key("data"), key(role.name()), key("p"), key(k)],
    )
}

/// `Mat2d` re-export for callers building flat poses.
pub type Flat = Mat2d;
