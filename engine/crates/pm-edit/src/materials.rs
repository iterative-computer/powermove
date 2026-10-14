//! Material peer synchronization (`editing.ts` `synchronizePeers`,
//! `scene3d/materials.ts` `shaderPeers` and `materialSlots`).
//!
//! A shader material can be shared by several surfaces (material slots of
//! several 3D object layers, in any composition). Its channels keep
//! composition timing: after an edit of one surface's shader channel, every
//! other surface using the same shader gets a copy of the channel with its
//! keys moved by the difference of the two layers' starts. A peer key keeps
//! its id when it lands on one of the peer's old key times.

use pm_eval::js_sem::{JsResult, JsThrow, JsValue};
use pm_eval::node::{self, Got};
use pm_eval::{Comp, JsValue as Js};
use pm_model::{JsString, Node, NodeKind, Patch, PathPart, Value};

use crate::channel::{join, key};
use crate::time::{Clock, shifted};
use crate::work::Edit;

/// `^(shader|slots\.([^.]+)\.shader)\.([^.]+)$`: the slot (`None` for the
/// base material) and the parameter.
pub fn parse_channel(path: &str) -> Option<(Option<&str>, &str)> {
    let (head, param) = path.rsplit_once('.')?;
    if param.is_empty() || head.is_empty() {
        return None;
    }
    if head == "shader" {
        return Some((None, param));
    }
    let slot = head.strip_prefix("slots.")?.strip_suffix(".shader")?;
    if slot.is_empty() || slot.contains('.') {
        return None;
    }
    Some((Some(slot), param))
}

/// One surface of a layer (`materialSlots`): its path inside the layer and
/// its material.
struct Slot<'d> {
    path: Vec<PathPart>,
    material: Option<&'d Node>,
}

/// `materialSlots(layer)`.
fn material_slots(layer: &Node) -> JsResult<Vec<Slot<'_>>> {
    let object = node::member(node::member(node::member(Some(layer), "d"), "data"), "object");
    if !node::truthy(object) {
        return Ok(Vec::new());
    }
    let base = [key("d"), key("data"), key("object")];
    let slots = node::member(object, "slots");
    let has = slots
        .map(|s| node::get(s, b"length").truthy())
        .unwrap_or(false);
    if has {
        let Some(items) = node::array(slots) else {
            return Err(JsThrow::type_error("object.slots.map is not a function"));
        };
        return Ok(items
            .iter()
            .enumerate()
            .map(|(i, s)| Slot {
                path: join(&base, &[key("slots"), PathPart::from(i), key("material")]),
                material: node::member(Some(s), "material"),
            })
            .collect());
    }
    Ok(vec![Slot {
        path: join(&base, &[key("material")]),
        material: node::member(object, "material"),
    }])
}

/// `slot.material.shader?.id`.
fn shader_id(material: Option<&Node>) -> JsResult<Option<JsValue>> {
    let shader = node::get_checked(material, b"shader")?;
    if shader.is_nullish() {
        return Ok(None);
    }
    Ok(Some(match &shader {
        Got::Node(s) => node::get(s, b"id").to_js(),
        _ => JsValue::Undefined,
    }))
}

/// A peer channel to rewrite, with what the copy needs from it.
struct Peer {
    /// The channel's canonical path.
    prop: Vec<PathPart>,
    /// The peer layer's start.
    from: f64,
    clock: Clock,
    /// The peer's old keys: `t` and `i`.
    old: Vec<(f64, JsValue)>,
}

/// What the copy needs from the edited surface.
struct Source {
    /// `clone(prop)`.
    next: Value,
    from: f64,
    clock: Clock,
}

/// `synchronizePeers(shaderPeers(PM, layer, channel), prop, layer)` after
/// the edit of `channel` on the layer at `layer`: copies the channel to
/// every other surface sharing its shader. Returns how many were written.
pub fn synchronize_peers(edit: &mut Edit<'_>, layer: usize, channel: &str) -> JsResult<usize> {
    let Some((slot_id, param)) = parse_channel(channel) else {
        return Ok(0);
    };
    let Some((source, peers)) = find_peers(edit, layer, channel, slot_id, param)? else {
        return Ok(0);
    };
    let mut writes: Vec<Patch> = Vec::new();
    for peer in &peers {
        let mut value = source.next.clone();
        let Some(obj) = value.as_object_mut() else {
            continue;
        };
        let Some(kf) = obj.get_mut("kf") else {
            return Err(JsThrow::type_error("Cannot read properties of undefined (reading 'map')"));
        };
        let Some(keys) = kf.as_array_mut() else {
            return Err(JsThrow::type_error("next.kf.map is not a function"));
        };
        for k in keys.iter_mut() {
            let Some(k) = k.as_object_mut() else {
                return Err(JsThrow::type_error("Cannot read properties of null (reading 't')"));
            };
            let t = k.get("t").map_or(f64::NAN, pm_eval::js_sem::value_to_number);
            let time = shifted(t, source.from, peer.from, &source.clock, &peer.clock);
            // `old.find(k => Math.abs(k.t - time) < 1e-8)?.i || PM.uid('k')`.
            let kept = peer
                .old
                .iter()
                .find(|(ot, _)| (ot - time).abs() < 1e-8)
                .map(|(_, i)| i.clone())
                .filter(Js::truthy)
                .and_then(|v| crate::channel::to_value(&v));
            let id = kept.unwrap_or_else(|| Value::String(edit.ids.next("k")));
            k.insert("t", Value::Number(time));
            k.insert("i", id);
        }
        // `Object.assign(peer.prop, next)`.
        for (k, v) in obj.iter() {
            writes.push(Patch::set(join(&peer.prop, &[PathPart::Key(k.clone())]), v.clone()));
        }
    }
    edit.write(writes)?;
    Ok(peers.len())
}

fn find_peers(
    edit: &Edit<'_>,
    layer: usize,
    channel: &str,
    slot_id: Option<&str>,
    param: &str,
) -> JsResult<Option<(Source, Vec<Peer>)>> {
    let source_comp = edit.comp_id().clone();
    let root = edit
        .node(&[])
        .ok_or_else(|| JsThrow::type_error("The mirror holds no document"))?;
    let revision = edit.document().revision();
    let comps_node = node::member(Some(root), "comps");
    let open = node::string(node::member(Some(root), "compId")).cloned();
    // `ProjectIndex.allLayers()`: the open composition first, then the others.
    let mut comp_ids: Vec<JsString> = Vec::new();
    if let Some(o) = &open {
        comp_ids.push(o.clone());
    }
    if let Some(NodeKind::Object(map)) = comps_node.map(Node::kind) {
        for k in map.keys() {
            if Some(k) != open.as_ref() {
                comp_ids.push(k.clone());
            }
        }
    }
    let comp_layers = |id: &JsString| -> &[Node] {
        node::array(node::member(node::member(comps_node, &id.to_string_lossy()), "layers")).unwrap_or(&[])
    };
    let clock_of = |id: &JsString| -> Clock {
        match Comp::in_root(root, &id.to_string_lossy(), revision) {
            Ok(c) => Clock::new(c.rate(), node::to_number(c.member("fps"))),
            Err(_) => Clock::new(None, f64::NAN),
        }
    };
    let source_layer = comp_layers(&source_comp)
        .get(layer)
        .ok_or_else(|| JsThrow::type_error("Layer not found"))?;
    // The edited surface's material.
    let object = node::get_checked(
        node::get_checked(node::get_checked(Some(source_layer), b"d")?.node(), b"data")?.node(),
        b"object",
    )?;
    let material = match slot_id {
        Some(id) => {
            let slots = node::get_checked(object.node(), b"slots")?;
            let items = match slots.node().map(Node::kind) {
                Some(NodeKind::Array(items)) => items,
                _ => return Err(JsThrow::type_error("slots.find is not a function")),
            };
            items
                .iter()
                .find(|s| node::is_str(node::member(Some(s), "id"), id))
                .and_then(|s| node::member(Some(s), "material"))
        }
        None => node::get_checked(object.node(), b"material")?.node(),
    };
    let shader = node::member(material, "shader");
    if !node::truthy(shader) {
        return Ok(None);
    }
    let wanted = node::get(shader.expect("truthy"), b"id").to_js();
    // The edited channel itself (`PM.findProp(layer, channel) || layer.p[channel]`).
    let source_prop: Option<&Node> = match pm_eval::Evaluator::new().find_prop(source_layer, &JsString::from(channel))? {
        Got::Node(n) if node::truthy(Some(n)) => Some(n),
        _ => node::member(node::member(Some(source_layer), "p"), channel).filter(|n| node::truthy(Some(n))),
    };
    let source_prop = source_prop.ok_or_else(|| JsThrow::type_error("Property not found"))?;
    let mut peers: Vec<Peer> = Vec::new();
    for id in &comp_ids {
        let clock = clock_of(id);
        for (i, candidate) in comp_layers(id).iter().enumerate() {
            for slot in material_slots(candidate)? {
                let Some(sid) = shader_id(slot.material)? else { continue };
                if !pm_eval::js_sem::strict_eq(&sid, &wanted) {
                    continue;
                }
                let prop = node::member(node::member(node::member(slot.material, "shader"), "p"), param);
                let Some(prop) = prop.filter(|p| node::truthy(Some(p))) else { continue };
                if std::ptr::eq(prop, source_prop) {
                    continue;
                }
                let mut path = vec![key("comps"), PathPart::Key(id.clone()), key("layers"), PathPart::from(i)];
                path.extend(slot.path.iter().cloned());
                path.extend([key("shader"), key("p"), key(param)]);
                let old = node::array(node::member(Some(prop), "kf"))
                    .unwrap_or(&[])
                    .iter()
                    .map(|k| (node::get(k, b"t").to_js().to_number(), node::get(k, b"i").to_js()))
                    .collect();
                peers.push(Peer {
                    prop: path,
                    from: node::get(candidate, b"from").to_js().to_number(),
                    clock,
                    old,
                });
            }
        }
    }
    let next = pm_model::patch::clone_patch_value(Some(&source_prop.to_value())).unwrap_or(Value::Null);
    let source = Source {
        next,
        from: node::get(source_layer, b"from").to_js().to_number(),
        clock: clock_of(&source_comp),
    };
    Ok(Some((source, peers)))
}
