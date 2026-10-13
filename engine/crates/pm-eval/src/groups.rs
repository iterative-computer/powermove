//! Layer groups (`legacy/core/layer-groups.ts`) and the group compositing
//! rules of the compositor (`legacy/gl/compositor.ts`).
//!
//! Membership (`layer.group`) supplies a transform space without changing
//! parent links or layer time. A group is a pure transform and organization
//! node until one of its visual properties needs the combined image of its
//! members; on those frames it is a compositing boundary
//! ([`Scope::group_creates_compositing_boundary`]) and its members render
//! into one offscreen stack.
//!
//! The legacy `groupSpan` and `expandGroups` read `PM.proj.layers` (the open
//! composition) even while a nested composition is evaluated; the port keeps
//! that (`Scope::project`).

use pm_model::{JsString, Node};

use crate::comp::{Comp, IdKey};
use crate::js_sem::{self, JsResult, JsValue};
use crate::node;
use crate::parenting::Role3d;
use crate::scope::{Scope, clamp, flag, is_type, js_max, js_min, or0, same};

/// A group's time span (`PM.groupSpan`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub from: f64,
    pub dur: f64,
}

/// `layer.id`'s Set/Map key.
fn id_key(layer: &Node) -> Option<IdKey> {
    IdKey::of(&node::get(layer, b"id"))
}

/// `ids.includes(id)` (`SameValueZero`).
fn includes(ids: &[JsValue], id: &JsValue) -> bool {
    ids.iter().any(|candidate| match (candidate, id) {
        (JsValue::Number(a), JsValue::Number(b)) => a == b || (a.is_nan() && b.is_nan()),
        _ => js_sem::strict_eq(candidate, id),
    })
}

impl<'d> Scope<'_, 'd> {
    /// `groupAncestors(layer, PM.proj.layers)`.
    fn project_ancestors(&mut self, layer: &'d Node) -> JsResult<Vec<&'d Node>> {
        let project: &Comp<'d> = self.project;
        self.ev.group_ancestors(project, layer)
    }

    /// `PM.expandGroups(ids)`: the ids of the open composition's layers that
    /// are in `ids` or inside a group in `ids`, in stack order.
    pub fn expand_groups(&mut self, ids: &[JsValue]) -> JsResult<Vec<&'d Node>> {
        let mut out = Vec::new();
        for layer in self.project.layers() {
            let id = node::get(layer, b"id").to_js();
            let inside = includes(ids, &id)
                || self
                    .project_ancestors(layer)?
                    .iter()
                    .any(|g| includes(ids, &node::get(g, b"id").to_js()));
            if inside {
                out.push(layer);
            }
        }
        Ok(out)
    }

    /// `PM.groupSpan(group)`: from the earliest member start to the latest
    /// member end (nested groups excluded), or `0` to the open composition's
    /// duration for an empty group.
    pub fn group_span(&mut self, group: &'d Node) -> JsResult<Span> {
        let members = self.expand_groups(&[node::get(group, b"id").to_js()])?;
        let ids: Vec<JsValue> = members
            .iter()
            .map(|l| node::get(l, b"id").to_js())
            .collect();
        let children: Vec<&'d Node> = self
            .project
            .layers()
            .iter()
            .filter(|l| includes(&ids, &node::get(l, b"id").to_js()) && !is_type(l, "group"))
            .collect();
        if children.is_empty() {
            let end = node::to_number(self.project.member("dur"));
            return Ok(Span {
                from: 0.0,
                dur: end,
            });
        }
        let mut from = f64::INFINITY;
        let mut end = f64::NEG_INFINITY;
        for l in &children {
            let start = node::js_value_opt(node::member(Some(l), "from"));
            let dur = node::js_value_opt(node::member(Some(l), "dur"));
            from = js_min(from, start.to_number());
            end = js_max(end, js_sem::add(&start, &dur).to_number());
        }
        Ok(Span {
            from,
            dur: end - from,
        })
    }

    /// `PM.transformRoots(ids)`: the selected layers that move on their own
    /// (not audio, not locked or inside a locked group, and not under a
    /// selected parent or group).
    pub fn transform_roots(&mut self, ids: &[JsValue]) -> JsResult<Vec<&'d Node>> {
        let layers = self.layers();
        let mut selected = Vec::new();
        for layer in layers {
            if !includes(ids, &node::get(layer, b"id").to_js())
                || is_type(layer, "audio")
                || flag(layer, "lock")
            {
                continue;
            }
            if self.group_ancestors(layer)?.iter().any(|g| flag(g, "lock")) {
                continue;
            }
            selected.push(layer);
        }
        let selected_ids: Vec<JsValue> = selected
            .iter()
            .map(|l| node::get(l, b"id").to_js())
            .collect();
        let mut out = Vec::new();
        'layers: for &layer in &selected {
            let mut seen: Vec<JsValue> = Vec::new();
            let mut current = Some(layer);
            while let Some(c) = current {
                let id = node::get(c, b"id").to_js();
                if includes(&seen, &id) {
                    break;
                }
                seen.push(id.clone());
                if (!same(c, layer) && includes(&selected_ids, &id))
                    || self
                        .group_ancestors(c)?
                        .iter()
                        .any(|g| includes(&selected_ids, &node::get(g, b"id").to_js()))
                {
                    continue 'layers;
                }
                let parent = node::get(c, b"parent").to_js();
                current = layers
                    .iter()
                    .find(|item| js_sem::strict_eq(&node::get(item, b"id").to_js(), &parent));
            }
            out.push(layer);
        }
        Ok(out)
    }

    /// `isModelGroup(PM, layer)`: a group whose members (other than groups
    /// and audio) are all 3D layers, and at least one.
    pub fn is_model_group(&mut self, layer: &'d Node) -> JsResult<bool> {
        if !is_type(layer, "group") {
            return Ok(false);
        }
        let id = id_key(layer);
        let mut count = 0;
        for child in self.layers() {
            if is_type(child, "group") || is_type(child, "audio") {
                continue;
            }
            let inside = self
                .group_ancestors(child)?
                .iter()
                .any(|g| id.is_some() && id_key(g) == id);
            if !inside {
                continue;
            }
            if Role3d::of(child).is_none() {
                return Ok(false);
            }
            count += 1;
        }
        Ok(count > 0)
    }

    /// `hasRenderableEffects(L.fx, PM, L, T)`: some effect is enabled
    /// (`evaluatedValue` of `on`, truthy) and not marked `missing`.
    pub fn has_renderable_effects(&mut self, layer: &'d Node, time: f64) -> JsResult<bool> {
        let fx = node::member(Some(layer), "fx");
        let Some(effects) = node::array(fx) else {
            return Ok(false);
        };
        for effect in effects {
            let id = node::to_js_string(node::member(Some(effect), "id"));
            let key = format!("{}.$enabled", id.to_string_lossy());
            let on = node::member_checked(Some(effect), "on")?;
            let enabled = self.evaluated(layer, on, time, &key)?.truthy();
            let missing = node::member(Some(effect), "missing");
            if enabled
                && !matches!(
                    missing.map(Node::kind),
                    Some(pm_model::NodeKind::Bool(true))
                )
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `activeTransition(L, T)`: the transition edge in effect at `time`, if
    /// any. `known` answers `PM.transitionDef(type)`'s existence.
    pub fn active_transition(
        &mut self,
        layer: &'d Node,
        time: f64,
        known: &dyn Fn(&JsString) -> bool,
    ) -> JsResult<Option<ActiveTransition<'d>>> {
        let tin = node::member(Some(layer), "transitionIn");
        let tout = node::member(Some(layer), "transitionOut");
        if !node::truthy(tin) && !node::truthy(tout) {
            return Ok(None);
        }
        let span = if is_type(layer, "group") {
            Some(self.group_span(layer)?)
        } else {
            None
        };
        let start = or0(match span {
            Some(s) => s.from,
            None => node::to_number(node::member(Some(layer), "from")),
        });
        let length = js_max(
            0.0,
            or0(match span {
                Some(s) => s.dur,
                None => node::to_number(node::member(Some(layer), "dur")),
            }),
        );
        let end = start + length;
        let at = |transition: Option<&'d Node>, edge_in: bool| -> Option<ActiveTransition<'d>> {
            let t = transition.filter(|t| node::truthy(Some(t)))?;
            if node::truthy(node::member(Some(t), "missing")) {
                return None;
            }
            let ty = node::string(node::member(Some(t), "type"))?;
            if !known(ty) {
                return None;
            }
            let raw = or0(node::to_number(node::member(Some(t), "dur")));
            let dur = js_min(length, js_max(0.02, if raw == 0.0 { 0.5 } else { raw }));
            if dur <= 0.0 {
                return None;
            }
            if edge_in && time >= start && time <= start + dur {
                return Some(ActiveTransition {
                    transition: t,
                    edge_in: true,
                    progress: clamp((time - start) / dur, 0.0, 1.0),
                });
            }
            if !edge_in && time >= end - dur && time <= end {
                return Some(ActiveTransition {
                    transition: t,
                    edge_in: false,
                    progress: clamp((time - (end - dur)) / dur, 0.0, 1.0),
                });
            }
            None
        };
        Ok(at(tin, true).or_else(|| at(tout, false)))
    }

    /// `groupCreatesCompositingBoundary(L, T, opt)`: whether a group renders
    /// its members into one offscreen stack at `time` (opacity below 1,
    /// enabled effects or masks, a matte, a blend mode, motion blur when the
    /// render uses it, or an active transition).
    pub fn group_creates_compositing_boundary(
        &mut self,
        layer: &'d Node,
        time: f64,
        motion_blur_pass: bool,
        transitions: &dyn Fn(&JsString) -> bool,
    ) -> JsResult<bool> {
        if !is_type(layer, "group") {
            return Ok(false);
        }
        let opacity = clamp(self.num(layer, "opacity", time)? / 100.0, 0.0, 1.0);
        let blend = self.evaluated(layer, node::member(Some(layer), "blend"), time, "l.blend")?;
        let mblur = self.evaluated(layer, node::member(Some(layer), "mblur"), time, "l.mblur")?;
        if opacity < 1.0 - 1e-6 || self.has_renderable_effects(layer, time)? {
            return Ok(true);
        }
        if let Some(masks) = node::array(node::member(Some(layer), "masks")) {
            for mask in masks {
                let id = node::to_js_string(node::member(Some(mask), "id"));
                let key = format!("m.{}.on", id.to_string_lossy());
                let on = node::member_checked(Some(mask), "on")?;
                if self.evaluated(layer, on, time, &key)? != JsValue::Bool(false) {
                    return Ok(true);
                }
            }
        }
        if flag(layer, "matteSource")
            || (blend.truthy() && blend != JsValue::from("normal"))
            || (mblur.truthy() && motion_blur_pass)
        {
            return Ok(true);
        }
        Ok(self.active_transition(layer, time, transitions)?.is_some())
    }

    /// `compositingLayers(layers, parentGroup, boundary)`: the members of one
    /// compositing pass, in stack order. Groups that are not boundaries are
    /// flattened into the pass (recursively); boundary groups stay as one
    /// entry that renders its own members.
    pub fn compositing_layers(
        &mut self,
        layers: &[&'d Node],
        parent_group: Option<&JsValue>,
        boundary: &mut dyn FnMut(&mut Self, &'d Node) -> JsResult<bool>,
    ) -> JsResult<Vec<&'d Node>> {
        let owner = |l: &Node| {
            let g = node::get(l, b"group");
            if g.truthy() { Some(g.to_js()) } else { None }
        };
        let matches = |l: &Node, parent: Option<&JsValue>| match (owner(l), parent) {
            (None, None) => true,
            (Some(a), Some(b)) => js_sem::strict_eq(&a, b),
            _ => false,
        };
        let mut result = Vec::new();
        let mut stack: Vec<std::vec::IntoIter<&'d Node>> = vec![
            layers
                .iter()
                .copied()
                .filter(|l| matches(l, parent_group))
                .collect::<Vec<_>>()
                .into_iter(),
        ];
        while let Some(level) = stack.last_mut() {
            let Some(layer) = level.next() else {
                stack.pop();
                continue;
            };
            if is_type(layer, "group") && !boundary(self, layer)? {
                let id = node::get(layer, b"id").to_js();
                let children: Vec<&'d Node> = layers
                    .iter()
                    .copied()
                    .filter(|l| matches(l, Some(&id)))
                    .collect();
                stack.push(children.into_iter());
            } else {
                result.push(layer);
            }
        }
        Ok(result)
    }

    /// `validMatteSource(layers, layer, id)` over the composition's layers.
    pub fn valid_matte_source(&self, layer: &Node, id: &JsValue) -> bool {
        crate::matte::valid_matte_source(self.layers(), layer, id)
    }
}

/// An active transition edge (`activeTransition`'s answer, without the
/// definition).
#[derive(Clone, Copy, Debug)]
pub struct ActiveTransition<'d> {
    pub transition: &'d Node,
    /// `edge === 'in'`.
    pub edge_in: bool,
    /// 0…1.
    pub progress: f64,
}
