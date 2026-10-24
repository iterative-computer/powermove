//! 2D layer transforms (`legacy/core/anim.ts`): `localMatrix`,
//! `transformParentMatrix`, `worldMatrix`, `worldOpacity`, and `wouldCycle`.
//!
//! Matrices are 2×3 affine transforms in the legacy column-major layout
//! `[a, b, c, d, e, f]` (x' = a·x + c·y + e, y' = b·x + d·y + f). Every
//! expression keeps the TypeScript operation order.
//!
//! # Allowed difference
//!
//! The legacy `worldMatrix` memoizes each intermediate ancestor's cumulative
//! matrix within an evaluation window (`PM.beginEval`). For a parenting
//! *cycle* the chain is cut after nine links, so a memoized intermediate
//! differs from that layer's own fresh answer and the result depends on the
//! order layers were queried in. The engine always answers as the first
//! query of a window does (a fresh walk). Without cycles the memo is exact
//! and answers are identical.

use pm_model::Node;

use crate::comp::IdKey;
use crate::js_sem::JsResult;
use crate::math;
use crate::node;
use crate::scope::{Scope, clamp, flag, is_type, same};

/// Every channel `localMatrix` reads.
const LOCAL_CHANNELS: [&str; 8] = [
    "position.x",
    "position.y",
    "anchor.x",
    "anchor.y",
    "scale.x",
    "scale.y",
    "rotation",
    "skew",
];

/// A 2×3 affine matrix, `[a, b, c, d, e, f]`.
pub type Mat2d = [f64; 6];

pub const IDENTITY: Mat2d = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `PM.mul(m, n)`: `m · n`.
#[inline]
pub fn mul(m: &Mat2d, n: &Mat2d) -> Mat2d {
    [
        m[0] * n[0] + m[2] * n[1],
        m[1] * n[0] + m[3] * n[1],
        m[0] * n[2] + m[2] * n[3],
        m[1] * n[2] + m[3] * n[3],
        m[0] * n[4] + m[2] * n[5] + m[4],
        m[1] * n[4] + m[3] * n[5] + m[5],
    ]
}

/// The inverse the legacy code writes inline (`groupBounds`,
/// `worldPoseValues`), or `None` when `|det| < 1e-10`.
pub fn invert(m: &Mat2d) -> Option<Mat2d> {
    let det = m[0] * m[3] - m[1] * m[2];
    if det.abs() < 1e-10 || det.is_nan() {
        return None;
    }
    Some([
        m[3] / det,
        -m[1] / det,
        -m[2] / det,
        m[0] / det,
        (m[2] * m[5] - m[3] * m[4]) / det,
        (m[1] * m[4] - m[0] * m[5]) / det,
    ])
}

/// Applies `m` to a point.
#[inline]
pub fn apply(m: &Mat2d, x: f64, y: f64) -> (f64, f64) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

impl<'d> Scope<'_, 'd> {
    /// `PM.localMatrix(L, T)`: translate(position) · rotate · skew · scale ·
    /// translate(−anchor), composed directly.
    pub fn local_matrix(&mut self, layer: &'d Node, time: f64) -> JsResult<Mat2d> {
        if let Some(m) = self
            .local_memo
            .as_ref()
            .and_then(|memo| memo.get(layer, time))
        {
            return Ok(m);
        }
        // A matrix whose channels are all time-independent is the same at
        // every time while the document is unchanged (`channel_memo`).
        let kept = self.ev.kept_local_matrix(self.comp, layer);
        if let Some(Some(m)) = kept {
            return Ok(m);
        }
        let m = self.local_matrix_uncached(layer, time)?;
        if kept.is_none() {
            self.ev.keep_local_matrix(self.comp, layer, &LOCAL_CHANNELS, m);
        }
        if let Some(memo) = self.local_memo.as_mut() {
            memo.insert(layer, time, m);
        }
        Ok(m)
    }

    /// The channels [`Scope::local_matrix`] reads, and nothing else.
    fn local_matrix_uncached(&mut self, layer: &'d Node, time: f64) -> JsResult<Mat2d> {
        let px = self.num(layer, "position.x", time)?;
        let py = self.num(layer, "position.y", time)?;
        let ax = self.num(layer, "anchor.x", time)?;
        let ay = self.num(layer, "anchor.y", time)?;
        let sx = self.num(layer, "scale.x", time)? / 100.0;
        let sy = self.num(layer, "scale.y", time)? / 100.0;
        let r = math::deg_to_rad(self.num(layer, "rotation", time)?);
        let sk = math::tan(math::deg_to_rad(self.num(layer, "skew", time)?));
        let c = math::cos(r);
        let s = math::sin(r);
        let a = c * sx;
        let b = s * sx;
        let cc = (c * sk - s) * sy;
        let d = (s * sk + c) * sy;
        Ok([a, b, cc, d, px - a * ax - cc * ay, py - b * ax - d * ay])
    }

    /// `PM.transformParentMatrix(L, T, parent = parentOf(L))`: the
    /// coordinate space a layer's local matrix lives in. Each enclosing group
    /// (outermost first) contributes its own parent chain and local matrix
    /// once, then the layer's parent chain follows.
    pub fn transform_parent_matrix(
        &mut self,
        layer: &'d Node,
        time: f64,
        parent: Option<Option<&'d Node>>,
    ) -> JsResult<Mat2d> {
        let parent = match parent {
            Some(p) => p,
            None => self.parent_of(layer)?,
        };
        let mut walk = ParentWalk {
            groups: Vec::new(),
            matrix: IDENTITY,
        };
        walk.append_groups(self, layer, time)?;
        walk.append_parents(self, layer, parent, time)?;
        Ok(walk.matrix)
    }

    /// `PM.worldMatrix(L, T)`.
    pub fn world_matrix(&mut self, layer: &'d Node, time: f64) -> JsResult<Mat2d> {
        if !flag(layer, "parent") && !flag(layer, "group") {
            return self.local_matrix(layer, time);
        }
        let mut chain: Vec<&'d Node> = Vec::new();
        let mut cur = Some(layer);
        while let Some(c) = cur {
            if chain.len() >= 256 {
                break;
            }
            if chain.len() > 8 && chain.iter().any(|l| same(l, c)) {
                break;
            }
            chain.push(c);
            cur = self.parent_of(c)?;
        }
        if chain.iter().any(|l| flag(l, "group")) {
            let parent = self.transform_parent_matrix(layer, time, None)?;
            let local = self.local_matrix(layer, time)?;
            return Ok(mul(&parent, &local));
        }
        let mut m = self.local_matrix(chain[chain.len() - 1], time)?;
        for i in (0..chain.len() - 1).rev() {
            let local = self.local_matrix(chain[i], time)?;
            m = mul(&m, &local);
        }
        Ok(m)
    }

    /// `PM.worldOpacity(L, T)`: the layer's opacity times every enclosing
    /// group's, each clamped to 0…1 (NaN passes through, as `PM.clamp` does).
    pub fn world_opacity(&mut self, layer: &'d Node, time: f64) -> JsResult<f64> {
        let groups = self.group_ancestors(layer)?;
        let mut opacity = clamp(self.num(layer, "opacity", time)? / 100.0, 0.0, 1.0);
        for group in groups {
            opacity *= clamp(self.num(group, "opacity", time)? / 100.0, 0.0, 1.0);
        }
        Ok(opacity)
    }

    /// `PM.wouldCycle(L, parentId)`: whether parenting `layer` to `parent_id`
    /// would create a parenting or group-space cycle.
    pub fn would_cycle(&mut self, layer: &'d Node, parent_id: &node::Got<'_>) -> JsResult<bool> {
        if !parent_id.truthy() {
            return Ok(false);
        }
        let own = node::get(layer, b"id");
        let parent_js = parent_id.to_js();
        if crate::js_sem::strict_eq(&parent_js, &own.to_js()) {
            return Ok(true);
        }
        for group in self.group_ancestors(layer)? {
            if crate::js_sem::strict_eq(&node::get(group, b"id").to_js(), &parent_js) {
                return Ok(true);
            }
        }
        let is_group = is_type(layer, "group");
        let mut cur = self.layer_by_id(parent_id);
        let mut guard = 0;
        while let Some(c) = cur {
            if guard >= 256 {
                break;
            }
            guard += 1;
            if crate::js_sem::strict_eq(&node::get(c, b"id").to_js(), &own.to_js()) {
                return Ok(true);
            }
            if is_group {
                for group in self.group_ancestors(c)? {
                    if crate::js_sem::strict_eq(&node::get(group, b"id").to_js(), &own.to_js()) {
                        return Ok(true);
                    }
                }
            }
            cur = self.parent_by_id(c);
        }
        Ok(false)
    }
}

/// The shared walk of `transformParentMatrix` (`appendGroups` and
/// `appendParents`).
struct ParentWalk {
    /// Group ids already appended (`Set` semantics).
    groups: Vec<Option<IdKey>>,
    matrix: Mat2d,
}

impl ParentWalk {
    fn append_parents<'d>(
        &mut self,
        scope: &mut Scope<'_, 'd>,
        layer: &'d Node,
        first: Option<&'d Node>,
        time: f64,
    ) -> JsResult<()> {
        let mut chain: Vec<&'d Node> = Vec::new();
        let mut seen: Vec<&'d Node> = vec![layer];
        let mut cur = first;
        while let Some(c) = cur {
            if seen.iter().any(|l| same(l, c)) || chain.len() >= 256 {
                break;
            }
            seen.push(c);
            chain.push(c);
            self.append_groups(scope, c, time)?;
            cur = scope.parent_of(c)?;
        }
        for item in chain.iter().rev() {
            let local = scope.local_matrix(item, time)?;
            self.matrix = mul(&self.matrix, &local);
        }
        Ok(())
    }

    fn append_groups<'d>(
        &mut self,
        scope: &mut Scope<'_, 'd>,
        layer: &'d Node,
        time: f64,
    ) -> JsResult<()> {
        let groups = scope.group_ancestors(layer)?;
        for group in groups.iter().rev() {
            let id = IdKey::of(&node::get(group, b"id"));
            // `Set.has` by SameValueZero; `undefined` ids are one value too.
            if self.groups.contains(&id) {
                continue;
            }
            self.groups.push(id);
            let first = scope.parent_of(group)?;
            self.append_parents(scope, group, first, time)?;
            let local = scope.local_matrix(group, time)?;
            self.matrix = mul(&self.matrix, &local);
        }
        Ok(())
    }
}
